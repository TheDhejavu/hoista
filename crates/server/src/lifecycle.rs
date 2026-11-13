//! Restarting `hoistd` when the person asks: the request stops new work
//! coming in, and `hoistd` finishes what's running before it exits.

use super::contracts::{DaemonStatus, RestartRequest};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Per-process lifecycle identity and one-way restart intent.
pub struct DaemonLifecycle {
    instance_id: String,
    supervised: bool,
    stop_admission: CancellationToken,
    handoff: Mutex<Option<bool>>,
    mcp_command: Vec<String>,
}

impl DaemonLifecycle {
    /// Creates one identity for this daemon instance, before opening its listener.
    pub fn new(stop_admission: CancellationToken, supervised: bool) -> Self {
        Self {
            instance_id: uuid::Uuid::new_v4().to_string(),
            supervised,
            stop_admission,
            handoff: Mutex::new(None),
            mcp_command: Vec::new(),
        }
    }

    /// Records the command coding tools run to reach this daemon (see
    /// [`DaemonStatus::mcp_command`]).
    #[must_use]
    pub fn with_mcp_command(mut self, command: Vec<String>) -> Self {
        self.mcp_command = command;
        self
    }

    /// Observes this process; a restart acknowledgment is not proof of a replacement process.
    pub fn status(&self) -> DaemonStatus {
        DaemonStatus {
            version: crate::VERSION.into(),
            pid: std::process::id(),
            instance_id: self.instance_id.clone(),
            supervised: self.supervised,
            restart_requested: self.handoff.lock().expect("lifecycle lock").is_some(),
            mcp_command: self.mcp_command.clone(),
        }
    }

    /// Records restart intent exactly once for the observed instance, then closes admission.
    ///
    /// # Errors
    /// Rejects stale instance IDs or a conflicting duplicate handoff policy.
    pub fn request_restart(&self, request: &RestartRequest) -> Result<DaemonStatus, &'static str> {
        if request.instance_id != self.instance_id {
            return Err("The daemon instance changed. Check daemon status before retrying.");
        }
        let policy = self.supervised || request.handoff;
        {
            let mut handoff = self.handoff.lock().expect("lifecycle lock");
            if handoff.is_some_and(|old| old != policy) {
                return Err("A restart with a different handoff policy is already pending.");
            }
            if handoff.is_none() {
                tracing::info!(code = "daemon.restart_requested");
            }
            *handoff = Some(policy);
        }
        self.stop_admission.cancel();
        Ok(self.status())
    }

    /// True after an accepted owner request; signal shutdown alone never requests restart.
    pub fn restart_requested(&self) -> bool {
        self.handoff.lock().expect("lifecycle lock").is_some()
    }

    /// Whether the installed supervisor must start the replacement instead of this process.
    pub fn handoff(&self) -> bool {
        self.handoff
            .lock()
            .expect("lifecycle lock")
            .unwrap_or(self.supervised)
    }
}
