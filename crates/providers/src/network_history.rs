//! Recent HTTP requests from this process, kept in memory by host. No paths, bodies
//! or error text are kept.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use crate::application::ports::network::{HostObservation, NetworkOutcome, NetworkReport};

/// Hosts kept at once; the one seen longest ago goes first.
const CAPACITY: usize = 64;
/// A host not contacted for this long is dropped.
const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

struct Entry {
    observation: HostObservation,
    last_seen: Instant,
    /// Which request last updated it, so an older request finishing late
    /// doesn't overwrite a newer one.
    generation: uuid::Uuid,
}

/// The hosts this process has sent requests to lately.
#[derive(Default)]
pub(crate) struct NetworkHistory {
    entries: Mutex<BTreeMap<String, Entry>>,
}

/// The one history for this process. Never saved, so it starts empty each run.
pub(crate) fn shared() -> Arc<NetworkHistory> {
    static HISTORY: OnceLock<Arc<NetworkHistory>> = OnceLock::new();
    HISTORY
        .get_or_init(|| Arc::new(NetworkHistory::default()))
        .clone()
}

/// The report as it stands. Makes no request itself.
pub fn report() -> NetworkReport {
    shared().snapshot()
}

impl NetworkHistory {
    /// Notes a request to `url`'s host starting now.
    pub(crate) fn begin(self: &Arc<Self>, url: &reqwest::Url) -> Attempt {
        let host = url
            .host_str()
            .filter(|host| host.len() <= crate::domain::domains::MAX_HOSTNAME_CHARS)
            .map(str::to_owned);
        let generation = uuid::Uuid::new_v4();
        if let Some(host) = &host {
            let now = Instant::now();
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.retain(|_, entry| now.saturating_duration_since(entry.last_seen) < RETENTION);
            if !entries.contains_key(host) && entries.len() >= CAPACITY {
                let oldest = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_seen)
                    .map(|(host, _)| host.clone());
                if let Some(oldest) = oldest {
                    entries.remove(&oldest);
                }
            }
            let attempts = entries
                .get(host)
                .map_or(1, |entry| entry.observation.attempts.saturating_add(1));
            entries.insert(
                host.clone(),
                Entry {
                    observation: HostObservation {
                        host: host.clone(),
                        last_attempt_at: chrono::Utc::now().to_rfc3339(),
                        attempts,
                        outcome: NetworkOutcome::InFlight,
                        http_status: None,
                    },
                    last_seen: now,
                    generation,
                },
            );
        }
        Attempt {
            history: self.clone(),
            host,
            generation,
            finished: false,
        }
    }

    pub(crate) fn snapshot(&self) -> NetworkReport {
        let now = Instant::now();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, entry| now.saturating_duration_since(entry.last_seen) < RETENTION);
        NetworkReport {
            allowed_api_hosts: crate::api_hosts().into_iter().map(Into::into).collect(),
            recent_hosts: entries
                .values()
                .map(|entry| entry.observation.clone())
                .collect(),
            limitations: vec![
                "Owned provider HTTP and public health-check attempts only; subprocess Git/flyctl/package tools, relay/tail WebSockets and other clients are not covered.".into(),
                "Current process only; at most 64 hosts, evicted after 24 hours without an attempt or when capacity is reached. Counts reset on eviction/restart.".into(),
                "Response means headers arrived, not that body parsing, permissions or the application succeeded. The latest-started request wins over older completions.".into(),
                "Hosts may identify your projects. This owner-local history is not included in diagnostic exports and is never uploaded automatically.".into(),
            ],
        }
    }
}

/// One request in progress. Dropped without [`Attempt::finish`], it's
/// recorded as cancelled.
pub(crate) struct Attempt {
    history: Arc<NetworkHistory>,
    host: Option<String>,
    generation: uuid::Uuid,
    finished: bool,
}

impl Attempt {
    fn update(&self, outcome: NetworkOutcome, status: Option<u16>) {
        let Some(host) = &self.host else { return };
        let mut entries = self
            .history
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = entries.get_mut(host)
            && entry.generation == self.generation
        {
            entry.observation.outcome = outcome;
            entry.observation.http_status = status;
        }
    }

    /// Records how the request ended: with a status, or `None` when no
    /// answer came.
    pub(crate) fn finish(mut self, status: Option<u16>) {
        self.update(
            if status.is_some() {
                NetworkOutcome::Response
            } else {
                NetworkOutcome::TransportFailure
            },
            status,
        );
        self.finished = true;
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            self.update(NetworkOutcome::Cancelled, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_excludes_url_details_and_old_completions_cannot_replace_new_outcomes() {
        let history = Arc::new(NetworkHistory::default());
        let url = reqwest::Url::parse(
            "https://user:private-token@api.cloudflare.com/private-path?secret=private-query",
        )
        .unwrap();
        let first = history.begin(&url);
        let second = history.begin(&url);
        second.finish(Some(429));
        first.finish(Some(200));
        let report = history.snapshot();
        assert_eq!(report.recent_hosts[0].attempts, 2);
        assert_eq!(report.recent_hosts[0].http_status, Some(429));
        assert_eq!(report.recent_hosts[0].outcome, NetworkOutcome::Response);
        let encoded = serde_json::to_string(&report).unwrap();
        for value in ["private-token", "private-path", "private-query", "user:"] {
            assert!(!encoded.contains(value));
        }
        drop(history.begin(&url));
        assert_eq!(
            history.snapshot().recent_hosts[0].outcome,
            NetworkOutcome::Cancelled
        );
        history.begin(&url).finish(None);
        assert_eq!(
            history.snapshot().recent_hosts[0].outcome,
            NetworkOutcome::TransportFailure
        );
    }

    #[test]
    fn history_bounds_hosts_and_expires_old_entries() {
        let history = Arc::new(NetworkHistory::default());
        for n in 0..70 {
            history
                .begin(&reqwest::Url::parse(&format!("https://app-{n}.workers.dev/")).unwrap())
                .finish(Some(200));
        }
        assert_eq!(history.snapshot().recent_hosts.len(), CAPACITY);
        {
            let mut entries = history.entries.lock().unwrap();
            for entry in entries.values_mut() {
                entry.last_seen = Instant::now() - RETENTION - Duration::from_secs(1);
            }
        }
        assert!(history.snapshot().recent_hosts.is_empty());
    }
}
