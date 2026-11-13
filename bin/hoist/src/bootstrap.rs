//! Starts the `hoistd` installed next to `hoist`, never one found on `PATH`.
//! The daemon must not write to stdout, which carries MCP.

use anyhow::{Context, bail};
use hoista_server::{
    VERSION,
    infrastructure::paths::StatePaths,
    {
        client::{ClientError, LocalClient},
        contracts::HealthDto,
    },
};
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};

/// How long `hoistd` gets to start before the CLI gives up.
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a started `hoistd` gets to answer at all.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the CLI says it's still waiting.
const NOTICE_AFTER: Duration = Duration::from_secs(10);
const START_POLL: Duration = Duration::from_millis(100);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(1);

/// The daemon is a persistent service. Keep its child handle while this CLI session runs;
/// collect an exited child on drop, but never terminate a successfully started service.
pub(crate) struct ReadyDaemon {
    pub(crate) health: HealthDto,
    child: Option<Child>,
}

impl Drop for ReadyDaemon {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.try_wait();
        }
    }
}

fn compatible(health: HealthDto) -> anyhow::Result<HealthDto> {
    if health.version != VERSION {
        bail!(
            "The running daemon and CLI versions differ. Restart Hoista using the matching installation."
        );
    }
    Ok(health)
}

async fn probe(
    client: &LocalClient,
    previous_pid: Option<u32>,
) -> anyhow::Result<Option<HealthDto>> {
    match tokio::time::timeout(HEALTH_TIMEOUT, client.health()).await {
        Ok(Ok(health)) if Some(health.pid) == previous_pid => Ok(None),
        Ok(Ok(health)) => Ok(Some(compatible(health)?)),
        Ok(Err(ClientError::Rejected(_))) => {
            bail!(
                "The running daemon does not support this CLI's readiness API. Restart Hoista using the matching installation."
            )
        }
        Ok(Err(ClientError::Transport)) | Err(_) => Ok(None),
    }
}

/// Waits for the service manager's daemon without racing it with a detached child.
pub(crate) async fn wait(
    paths: &StatePaths,
    previous_pid: Option<u32>,
) -> anyhow::Result<HealthDto> {
    let client = LocalClient::new(paths)?;
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    let mut notice = tokio::time::Instant::now() + NOTICE_AFTER;
    loop {
        if let Some(health) = probe(&client, previous_pid).await?
            && Some(health.pid) != previous_pid
        {
            return Ok(health);
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "The service did not provide a ready daemon within 120 seconds. Check hoist service status and the private state logs. A daemon started separately may still own this state directory."
            );
        }
        if tokio::time::Instant::now() >= notice {
            eprintln!("Waiting for the service manager and any accepted work to finish...");
            notice = tokio::time::Instant::now() + NOTICE_AFTER;
        }
        tokio::time::sleep(START_POLL).await;
    }
}

pub(crate) async fn ensure(paths: &StatePaths) -> anyhow::Result<ReadyDaemon> {
    let client = LocalClient::new(paths)?;
    if let Some(health) = probe(&client, None).await? {
        return Ok(ReadyDaemon {
            health,
            child: None,
        });
    }
    hoista_server::infrastructure::tool_paths::read(paths).context(
        "Could not read deployment tools; run hoist tools configure for this state directory",
    )?;
    let executable = std::env::current_exe().context("Could not locate this CLI installation")?;
    let executable =
        std::fs::canonicalize(executable).context("Could not resolve this CLI installation")?;
    let daemon = executable
        .parent()
        .context("CLI installation has no parent directory")?
        .join(if cfg!(windows) {
            "hoistd.exe"
        } else {
            "hoistd"
        });
    if !daemon.is_file() {
        bail!(
            "Install hoistd alongside hoist, or start the daemon manually for this state directory."
        );
    }
    let mut command = Command::new(daemon);
    command
        .arg("--state-dir")
        .arg(paths.root())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    let mut child = command
        .spawn()
        .context("Could not start the installed hoistd binary")?;
    let ready = async {
        let deadline = tokio::time::Instant::now() + SPAWN_TIMEOUT;
        loop {
            if let Some(health) = probe(&client, None).await? {
                return Ok(health);
            }
            // A racing starter may win the SQLite ownership lock. Keep probing even
            // when this child exits; never remove a socket or stop the other owner.
            let _ = child
                .try_wait()
                .context("Could not observe daemon startup")?;
            if tokio::time::Instant::now() >= deadline {
                bail!("Hoista did not become ready within 15 seconds. Check the private state logs or run hoistd --state-dir with this directory.");
            }
            tokio::time::sleep(START_POLL).await;
        }
    }
    .await;
    match ready {
        Ok(health) => Ok(ReadyDaemon {
            health,
            child: Some(child),
        }),
        Err(error) => {
            if child.try_wait()?.is_none() {
                child.kill()?;
                let _ = child.wait()?;
            }
            Err(error)
        }
    }
}

/// Requests a graceful, daemon-owned restart; the request survives this CLI disconnecting.
pub(crate) async fn restart(
    paths: &StatePaths,
    handoff: bool,
) -> anyhow::Result<hoista_server::contracts::DaemonStatus> {
    use hoista_server::contracts::DaemonStatus;
    hoista_server::infrastructure::tool_paths::read(paths)
        .context("Could not read deployment tools; run hoist tools configure before restarting")?;
    let client = LocalClient::new(paths)?;
    let before: DaemonStatus = serde_json::from_value(client.get_json("/daemon").await?)?;
    let accepted: DaemonStatus = serde_json::from_value(
        client
            .post_json(
                "/daemon/restart",
                &serde_json::json!({"instance_id":before.instance_id,"handoff":handoff}),
            )
            .await
            .context(
                "Could not confirm the restart request; check hoist daemon status before retrying",
            )?,
    )?;
    if accepted.instance_id != before.instance_id || !accepted.restart_requested {
        bail!(
            "The daemon returned an invalid restart acknowledgment. Check daemon status before retrying."
        );
    }
    eprintln!(
        "Restart requested. Waiting for accepted work to finish and a new daemon to become ready..."
    );
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    let mut notice = tokio::time::Instant::now() + NOTICE_AFTER;
    loop {
        if let Ok(Ok(value)) =
            tokio::time::timeout(HEALTH_TIMEOUT, client.get_json("/daemon")).await
        {
            let status: DaemonStatus = serde_json::from_value(value)?;
            if status.instance_id != before.instance_id {
                if status.version != VERSION {
                    bail!(
                        "The replacement daemon version differs from this CLI. Check the installed binaries and service descriptor."
                    );
                }
                return Ok(status);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "Restart was accepted but no replacement was ready within 120 seconds. Accepted work may still be draining. Check daemon status and private logs; no process was force-killed."
            );
        }
        if tokio::time::Instant::now() >= notice {
            eprintln!(
                "Still waiting for accepted work to drain and the replacement daemon to start..."
            );
            notice = tokio::time::Instant::now() + NOTICE_AFTER;
        }
        tokio::time::sleep(START_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_requires_the_installed_version() {
        assert!(
            compatible(HealthDto {
                version: VERSION.into(),
                pid: 123
            })
            .is_ok()
        );
        assert!(
            compatible(HealthDto {
                version: "other".into(),
                pid: 123
            })
            .is_err()
        );
    }
}
