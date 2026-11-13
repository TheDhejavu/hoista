//! Reading a provider again after something changed it there. Providers take a
//! moment to show what a change made, so a changed account is read a few
//! seconds later and once more after that, by the background monitor.

use crate::accounts::AccountService;
use crate::engine::Engine;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// When a changed account is read again: soon, then once the provider has
/// caught up with slower changes.
const AFTER: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(30)];

/// Accounts waiting to be read again, and when.
#[derive(Default)]
pub(crate) struct Rescans {
    due: Mutex<Vec<(String, Instant)>>,
    wake: tokio::sync::Notify,
}

impl Rescans {
    /// Reads `account` again shortly, and once more a little later.
    pub(crate) fn after_change(&self, account: &str) {
        let now = Instant::now();
        if let Ok(mut due) = self.due.lock() {
            due.retain(|(id, _)| id != account);
            due.extend(AFTER.iter().map(|after| (account.to_owned(), now + *after)));
        }
        self.wake.notify_one();
    }

    /// The accounts due now, each once, taken off the list.
    fn take_due(&self) -> Vec<String> {
        let now = Instant::now();
        let Ok(mut due) = self.due.lock() else {
            return Vec::new();
        };
        let mut ready: Vec<String> = Vec::new();
        due.retain(|(id, at)| {
            if *at > now {
                return true;
            }
            if !ready.contains(id) {
                ready.push(id.clone());
            }
            false
        });
        ready
    }

    /// How long until the next account is due, if any is waiting.
    pub(crate) fn next_in(&self) -> Option<Duration> {
        let now = Instant::now();
        self.due
            .lock()
            .ok()?
            .iter()
            .map(|(_, at)| at.saturating_duration_since(now))
            .min()
    }

    /// Wakes when a new change is noted.
    pub(crate) async fn changed(&self) {
        self.wake.notified().await;
    }
}

impl Engine {
    /// Reads again each changed account that's due. What it finds shows in
    /// the project; a line in the activity log says what's new there, and a
    /// failed read keeps what Hoista already knows and says why.
    pub(crate) async fn run_due_rescans(&self) -> Result<(), crate::ports::catalog::EngineError> {
        use crate::ports::catalog::EngineError;
        for account in self.rescans.take_due() {
            let before = self.accounts().account_resources(&account).await?;
            match self.accounts().refresh_found(&account).await {
                Ok(()) => {}
                Err(EngineError::NotFound) => continue,
                Err(error) => {
                    self.repository
                        .event("engine", None, "account.refresh_unavailable", error.code())
                        .await?;
                    continue;
                }
            }
            let after = self.accounts().account_resources(&account).await?;
            let Some(summary) = what_changed(&before, &after) else {
                continue;
            };
            let provider = after
                .first()
                .or(before.first())
                .map_or("The provider", |resource| resource.provider.display_name());
            self.repository
                .event(
                    "engine",
                    None,
                    "resource.refreshed",
                    &format!("After the change, {provider} shows {summary}."),
                )
                .await?;
        }
        Ok(())
    }
}

impl<'a> AccountService<'a> {
    async fn account_resources(
        self,
        account: &str,
    ) -> Result<Vec<crate::domain::model::Resource>, crate::ports::catalog::EngineError> {
        Ok(self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| resource.account_id == account)
            .collect())
    }
}

/// What a read found that wasn't there before, in a few words, or None
/// when nothing changed.
fn what_changed(
    before: &[crate::domain::model::Resource],
    after: &[crate::domain::model::Resource],
) -> Option<String> {
    let mut new = 0;
    let mut changed = 0;
    for resource in after {
        match before.iter().find(|old| old.id == resource.id) {
            None => new += 1,
            Some(old) if old.status != resource.status => changed += 1,
            Some(_) => {}
        }
    }
    let count =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match (new, changed) {
        (0, 0) => None,
        (new, 0) => Some(count(new, "new resource", "new resources")),
        (0, changed) => Some(count(
            changed,
            "resource with a new status",
            "resources with a new status",
        )),
        (new, changed) => Some(format!(
            "{} and {}",
            count(new, "new resource", "new resources"),
            count(changed, "with a new status", "with a new status")
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::accounts::rescan::*;

    #[test]
    fn a_changed_account_is_read_twice_and_once_at_a_time() {
        let rescans = Rescans::default();
        assert!(rescans.next_in().is_none());
        rescans.after_change("a");
        rescans.after_change("a");
        // A second change moves the reads later rather than adding more.
        assert_eq!(rescans.due.lock().unwrap().len(), 2);
        assert!(rescans.take_due().is_empty());
        let next = rescans.next_in().unwrap();
        assert!(next <= AFTER[0] && next > Duration::from_secs(4));
        rescans
            .due
            .lock()
            .unwrap()
            .iter_mut()
            .for_each(|(_, at)| *at = Instant::now());
        assert_eq!(rescans.take_due(), vec!["a".to_owned()]);
        assert!(rescans.next_in().is_none());
    }

    #[test]
    fn says_what_a_read_found() {
        use crate::domain::model::{Mode, Resource, ResourceKind, ResourceStatus};
        let resource = |id: &str, status| Resource {
            id: id.into(),
            project_id: None,
            kind: ResourceKind::Web,
            provider: crate::Provider::Render,
            account_id: "a".into(),
            provider_ref: id.into(),
            name: id.into(),
            region: None,
            status,
            mode: Mode::Watching,
            url: None,
            group: None,
        };
        let before = vec![resource("1", ResourceStatus::Deploying)];
        assert_eq!(what_changed(&before, &before), None);
        let after = vec![
            resource("1", ResourceStatus::Live),
            resource("2", ResourceStatus::Live),
        ];
        assert_eq!(
            what_changed(&before, &after).as_deref(),
            Some("1 new resource and 1 with a new status")
        );
        assert_eq!(
            what_changed(&[], &after[1..]).as_deref(),
            Some("1 new resource")
        );
    }
}
