//! Reading a resource's logs: live tails the engine runs, short captures
//! from a tail, and the history a provider keeps. Every tail ends with a
//! frame saying why.

use crate::engine::{Actor, now};
use crate::observe::ObserveService;
use crate::{
    domain::{
        Secret,
        logs::{LogEntry, LogLevel},
    },
    ports::{
        catalog::{CatalogRepository, EngineError},
        providers::{LogSession, ProviderAdapter},
    },
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

/// Log viewers and history reads open at once, across all resources.
const MAX_VIEWERS: usize = 8;
const MAX_QUERY_BYTES: usize = 256;
/// Frames a viewer can fall behind by before its tail is ended.
const QUEUE_FRAMES: usize = 64;
/// A tail ends after this long, to be opened again if still wanted.
const TAIL_LIFETIME: Duration = Duration::from_secs(5 * 60);
/// How often an agent's access is checked again while its tail runs.
const ACCESS_CHECK_EVERY: Duration = Duration::from_secs(5);
const MAX_TAIL_ENTRIES: usize = 10_000;
const MAX_TAIL_BYTES: usize = 16 * 1024 * 1024;
/// A capture waits this long for the tail to connect...
const CAPTURE_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// ...then reads for this long.
const CAPTURE_DURATION: Duration = Duration::from_secs(5);
const MAX_CAPTURE_ENTRIES: usize = 100;
/// How long a history read waits for a free viewer slot.
const HISTORY_WAIT: Duration = Duration::from_secs(20);

/// What a live tail sends. The last frame says why it ended.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogFrame {
    /// The tail at the provider is connected; no history was fetched.
    Ready {
        /// When it connected.
        started_at: String,
        /// Always false: live tails carry no history.
        historical: bool,
    },
    /// One redacted log entry from the provider.
    Log {
        /// The log entry.
        entry: LogEntry,
    },
    /// The tail ended; reconnecting doesn't replay what was missed.
    End {
        /// Why it ended, as a code.
        reason: String,
        /// Whether the tail was deleted at the provider.
        cleanup_complete: bool,
    },
}

/// A live tail's frames. Dropping it stops the tail and removes it at the provider.
pub struct LogSubscription {
    receiver: mpsc::Receiver<LogFrame>,
    cancel: CancellationToken,
}

impl LogSubscription {
    /// The next frame; `None` once the tail has finished.
    pub async fn recv(&mut self) -> Option<LogFrame> {
        self.receiver.recv().await
    }
}

impl Drop for LogSubscription {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The engine's running tails, and the slots they and history reads share.
pub(crate) struct LogTasks {
    tasks: Mutex<JoinSet<()>>,
    slots: Arc<Semaphore>,
    shutdown: CancellationToken,
}

impl Default for LogTasks {
    fn default() -> Self {
        Self {
            tasks: Mutex::new(JoinSet::new()),
            slots: Arc::new(Semaphore::new(MAX_VIEWERS)),
            shutdown: CancellationToken::new(),
        }
    }
}

/// Whether the actor may still read logs. Checked again while a tail runs,
/// so a revoked agent stops getting them.
async fn authorized(repository: &dyn CatalogRepository, actor: &Actor) -> Result<(), EngineError> {
    if let Actor::Agent { id } = actor {
        let settings = repository.settings().await?;
        if !settings.agents_enabled {
            return Err(EngineError::AgentsDisabled);
        }
        repository
            .authorize_agent(id, "apps:read", &now(), settings.agent_expiry_days)
            .await?;
    }
    Ok(())
}

impl<'a> ObserveService<'a> {
    /// Opens a live tail of one of the project's resources. Only new lines
    /// come through; no history is read.
    ///
    /// # Errors
    /// Fails without access, for a query over 256 bytes, a resource that
    /// isn't in the project, a provider without live logs, or when every
    /// viewer slot is taken.
    pub async fn live_logs(
        self,
        actor: &Actor,
        project_id: &str,
        resource_id: &str,
        query: &str,
        level: Option<LogLevel>,
    ) -> Result<LogSubscription, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        if query.len() > MAX_QUERY_BYTES {
            return Err(EngineError::Invalid(
                "Log queries must be at most 256 bytes.",
            ));
        }
        let permit = self
            .engine
            .log_tasks
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                EngineError::Conflict(
                    "Eight live log sessions are already open. Close a viewer and retry.",
                )
            })?;
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.project_id.as_deref() != Some(project_id) {
            return Err(EngineError::NotFound);
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let adapter = self.engine.adapter(resource.provider)?.clone();
        if !adapter.capabilities().logs {
            return Err(crate::ports::providers::unsupported());
        }
        let token = self.engine.accounts().account_token(&account.id).await?;
        let mut tasks = self.engine.log_tasks.tasks.lock().await;
        if self.engine.log_tasks.shutdown.is_cancelled() {
            return Err(EngineError::Conflict("The engine is shutting down."));
        }
        while tasks.try_join_next().is_some() {}
        self.engine
            .repository
            .event(
                actor.id(),
                Some(project_id),
                "logs.opened",
                "Opened a live-only provider log viewer.",
            )
            .await?;
        let cancel = self.engine.log_tasks.shutdown.child_token();
        // One more slot than log frames may use, kept for the last frame.
        let (sender, receiver) = mpsc::channel(QUEUE_FRAMES + 1);
        let job = LogJob {
            repository: self.engine.repository.clone(),
            actor: actor.clone(),
            adapter,
            token,
            account: account.external_id,
            reference: resource.provider_ref,
            query: query.into(),
            level,
            cancel: cancel.clone(),
            sender,
        };
        tasks.spawn(async move {
            let _permit = permit;
            job.run().await;
        });
        Ok(LogSubscription { receiver, cancel })
    }

    /// Stops every tail and waits for them to finish, before the database
    /// is closed at shutdown.
    pub async fn stop_logs(self) {
        self.engine.log_tasks.shutdown.cancel();
        let mut tasks = self.engine.log_tasks.tasks.lock().await;
        // Provider HTTP and socket calls have their own deadlines.
        while tasks.join_next().await.is_some() {}
    }
}

struct LogJob {
    repository: Arc<dyn CatalogRepository>,
    actor: Actor,
    adapter: Arc<dyn ProviderAdapter>,
    token: Secret,
    account: String,
    reference: String,
    query: String,
    level: Option<LogLevel>,
    cancel: CancellationToken,
    sender: mpsc::Sender<LogFrame>,
}

impl LogJob {
    async fn run(self) {
        // Opening isn't cancelled: the tail it makes has to be known to be removed.
        let opened = self
            .adapter
            .open_logs(&self.token, &self.account, &self.reference, &self.query)
            .await;
        let (reason, cleanup_complete) = match opened {
            Ok(mut session) => {
                let reason = self.observe(session.as_mut()).await;
                let cleanup = session.close().await;
                if cleanup.is_err() {
                    tracing::warn!(code = "logs.cleanup_failed");
                }
                (reason, cleanup.is_ok())
            }
            Err(error) => (error.code().to_owned(), false),
        };
        let _ = self.sender.try_send(LogFrame::End {
            reason,
            cleanup_complete,
        });
    }

    async fn observe(&self, session: &mut dyn LogSession) -> String {
        if self.cancel.is_cancelled() {
            return "logs.cancelled".into();
        }
        if let Err(error) = authorized(self.repository.as_ref(), &self.actor).await {
            return error.code().into();
        }
        if self
            .sender
            .try_send(LogFrame::Ready {
                started_at: now(),
                historical: false,
            })
            .is_err()
        {
            return "logs.slow_consumer".into();
        }
        let deadline = tokio::time::sleep(TAIL_LIFETIME);
        tokio::pin!(deadline);
        let mut checks = tokio::time::interval(ACCESS_CHECK_EVERY);
        let mut records = 0;
        let mut bytes = 0;
        loop {
            let next = tokio::select! {
                () = self.cancel.cancelled() => return "logs.cancelled".into(),
                () = &mut deadline => return "logs.session_expired".into(),
                _ = checks.tick() => {
                    if let Err(error) = authorized(self.repository.as_ref(), &self.actor).await {
                        return error.code().into();
                    }
                    continue;
                }
                next = session.next() => next,
            };
            let entry = match next {
                Ok(Some(entry)) => entry,
                Ok(None) => return "logs.closed".into(),
                Err(error) => return error.code().into(),
            };
            // Lines the level filter hides still count towards the limits.
            records += 1;
            bytes += entry.message.len();
            if records > MAX_TAIL_ENTRIES
                || bytes > MAX_TAIL_BYTES
                || entry.message.len() > crate::domain::logs::MAX_MESSAGE_BYTES
            {
                return "logs.limit".into();
            }
            if self.level.is_some_and(|level| level != entry.level) {
                continue;
            }
            if let Err(error) = authorized(self.repository.as_ref(), &self.actor).await {
                return error.code().into();
            }
            if self.sender.capacity() <= 1 || self.sender.try_send(LogFrame::Log { entry }).is_err()
            {
                return "logs.slow_consumer".into();
            }
        }
    }
}

/// A few seconds of live logs; never a search of past logs.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct LogCapture {
    /// Up to 100 redacted log entries.
    pub items: Vec<LogEntry>,
    /// Always false: this never fills in past logs.
    pub historical: bool,
    /// When the tail connected; null if it failed to connect.
    pub started_at: Option<String>,
    /// When capture and cleanup at the provider finished.
    pub finished_at: String,
    /// Why it ended, such as a limit, a disconnect or a missing permission.
    pub end_reason: String,
    /// Whether the tail was deleted at the provider.
    pub cleanup_complete: bool,
    /// Always null: live captures can't be resumed.
    pub next_cursor: Option<String>,
}

impl<'a> ObserveService<'a> {
    /// Up to five seconds, or 100 lines, of a live tail. `since` drops older
    /// lines; it never fetches earlier ones.
    ///
    /// # Errors
    /// As [`Self::live_logs`], or for a `since` that isn't an RFC 3339 time.
    pub async fn logs(
        self,
        actor: &Actor,
        project_id: &str,
        resource_id: &str,
        since: &str,
        query: &str,
    ) -> Result<LogCapture, EngineError> {
        let since = chrono::DateTime::parse_from_rfc3339(since)
            .map_err(|_| EngineError::Invalid("Use an RFC 3339 start time."))?;
        let mut subscription = self
            .engine
            .observe()
            .live_logs(actor, project_id, resource_id, query, None)
            .await?;
        let mut capture = LogCapture {
            items: vec![],
            historical: false,
            started_at: None,
            finished_at: String::new(),
            end_reason: "logs.closed".into(),
            cleanup_complete: false,
            next_cursor: None,
        };
        let deadline = tokio::time::sleep(CAPTURE_CONNECT_TIMEOUT);
        tokio::pin!(deadline);
        let mut stopping = false;
        loop {
            let frame = tokio::select! {
                () = &mut deadline, if !stopping => {
                    subscription.cancel.cancel();
                    capture.end_reason = if capture.started_at.is_some() {
                        "logs.capture_complete"
                    } else {
                        "logs.connect_timeout"
                    }
                    .into();
                    stopping = true;
                    continue;
                }
                frame = subscription.recv() => frame,
            };
            match frame {
                Some(LogFrame::Ready { started_at, .. }) => {
                    capture.started_at = Some(started_at);
                    if !stopping {
                        deadline
                            .as_mut()
                            .reset(tokio::time::Instant::now() + CAPTURE_DURATION);
                    }
                }
                Some(LogFrame::Log { entry }) if !stopping => {
                    let timestamp = entry.timestamp.as_deref().unwrap_or(&entry.observed_at);
                    if chrono::DateTime::parse_from_rfc3339(timestamp)
                        .is_ok_and(|time| time >= since)
                    {
                        capture.items.push(entry);
                    }
                    if capture.items.len() >= MAX_CAPTURE_ENTRIES {
                        subscription.cancel.cancel();
                        capture.end_reason = "logs.capture_limit".into();
                        stopping = true;
                    }
                }
                Some(LogFrame::End {
                    reason,
                    cleanup_complete,
                }) => {
                    if !stopping || reason != "logs.cancelled" {
                        capture.end_reason = reason;
                    }
                    capture.cleanup_complete = cleanup_complete;
                    break;
                }
                None => break,
                _ => {}
            }
        }
        capture.finished_at = now();
        Ok(capture)
    }
}

impl<'a> ObserveService<'a> {
    /// The logs a provider keeps for one of the project's resources. Access
    /// is checked again before they're returned. Shares the viewer slots
    /// with live tails.
    ///
    /// # Errors
    /// Fails for a bad range, a resource that isn't in the project or
    /// changed during the read, access that was revoked, no free slot after
    /// 20 seconds, a provider without log history, or at shutdown.
    pub async fn log_history(
        self,
        actor: &Actor,
        project_id: &str,
        resource_id: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        query.milliseconds().map_err(EngineError::Invalid)?;
        self.engine.authorize(actor, "apps:read").await?;
        // A project with many services reads them all at once, so a read
        // waits its turn for a while rather than being turned away.
        let _permit = tokio::time::timeout(
            HISTORY_WAIT,
            self.engine.log_tasks.slots.clone().acquire_owned(),
        )
        .await
        .map_err(|_| {
            EngineError::Conflict("Hoista is busy reading other logs. Retry in a few seconds.")
        })?
        .map_err(|_| EngineError::Conflict("The engine is shutting down."))?;
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.project_id.as_deref() != Some(project_id) {
            return Err(EngineError::NotFound);
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let adapter = self.engine.adapter(resource.provider)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        if self.engine.log_tasks.shutdown.is_cancelled() {
            return Err(EngineError::Conflict("The engine is shutting down."));
        }
        let read = adapter.log_history(&token, &account.external_id, &resource.provider_ref, query);
        let history = tokio::select! {
            () = self.engine.log_tasks.shutdown.cancelled() => {
                return Err(EngineError::Conflict("The engine is shutting down."));
            }
            result = read => result?,
        };
        self.engine.authorize(actor, "apps:read").await?;
        let current = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if current.project_id != resource.project_id
            || current.account_id != resource.account_id
            || current.provider_ref != resource.provider_ref
        {
            return Err(EngineError::Conflict(
                "The resource changed during the log query. Retry.",
            ));
        }
        Ok(history)
    }
}
