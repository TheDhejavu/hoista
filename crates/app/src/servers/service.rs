//! What agents and the person do on a server Hoista made: look at it, read
//! its logs and files, run commands, write files and make deploy keys. Every
//! call signs in with the key Hoista keeps, checks the server's host key,
//! and passes what comes back through the secret scanner.

use super::{ServerAccess, ServerSetup, keys::ServerKey, pending_reference, vault_reference};
use crate::{
    domain::{
        Secret,
        model::{GatedAction, Resource, ResourceKind, ResourceStatus},
        servers::{plain_path, shell_quote},
    },
    engine::{Actor, Engine},
    ports::{
        catalog::EngineError,
        servers::{CommandOutput, CommandRun, ServerAddress},
    },
};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// SSH's port.
const SSH_PORT: u16 = 22;
/// How long a look or a read may take.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a command may take at most, and by default.
const MAX_RUN_SECONDS: u64 = 900;
const DEFAULT_RUN_SECONDS: u64 = 120;
/// How much output reaches the agent, each of standard output and errors.
const MAX_OUTPUT: usize = 64 * 1024;
/// The largest file read or written.
const MAX_FILE: usize = 1024 * 1024;
/// How long the setup check may take.
const SETUP_CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// How many things done on a server are shown.
const MAX_ACTIVITY: u32 = 50;
/// The first-boot setup's state on its first line (`status: done`), then,
/// when it failed, the last lines of what it printed.
const SETUP_SCRIPT: &str = r#"state="$(cloud-init status 2>/dev/null | sed -n 's/^status: //p')"
echo "${state:-unknown}"
case "$state" in *error*) sudo -n tail -n 5 /var/log/cloud-init-output.log 2>/dev/null ;; esac"#;
/// How many log lines at most, and by default.
const MAX_LOG_LINES: u32 = 2000;
const DEFAULT_LOG_LINES: u32 = 200;
/// What a server says on a look: setup, load, memory, disk, failed services,
/// containers, waiting updates and whether a reboot is due.
const STATUS_SCRIPT: &str = r#"echo "setup version: $(cat /etc/hoista/version 2>/dev/null || echo none)"
echo "setup: $(cloud-init status 2>/dev/null | sed 's/^status: //' || echo unknown)"
echo "system: $(. /etc/os-release 2>/dev/null; echo "$PRETTY_NAME")"
echo "uptime: $(uptime)"
echo
free -m
echo
df -h -x tmpfs -x devtmpfs -x overlay
echo
echo "failed services:"
systemctl --failed --no-legend --plain 2>/dev/null || true
echo
echo "containers:"
sudo -n docker ps --format '{{.Names}}  {{.Image}}  {{.Status}}' 2>/dev/null || echo "Docker isn't running"
echo
echo "updates waiting: $(apt-get -s upgrade 2>/dev/null | grep -c '^Inst ')"
if [ -f /var/run/reboot-required ]; then echo "reboot required"; fi"#;

/// How a command on a server went, as an agent sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunResult {
    /// Its exit code; null when it was stopped or the server didn't say.
    pub exit_code: Option<u32>,
    /// What it printed, with secrets taken out.
    pub output: String,
    /// What it printed as errors, with secrets taken out.
    pub errors: String,
    /// Output past the limit was cut.
    pub truncated: bool,
    /// It ran past its time and was stopped.
    pub timed_out: bool,
}

/// What a log read asks for.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogsAsk<'a> {
    /// A systemd unit, such as `caddy` or `docker`.
    pub unit: Option<&'a str>,
    /// A Docker container, by name or id.
    pub container: Option<&'a str>,
    /// How many of the newest lines.
    pub lines: Option<u32>,
    /// Only lines after this, as `journalctl --since` and `docker logs
    /// --since` read it: `2026-11-22 10:00:00`, or `1h` and the like.
    pub since: Option<&'a str>,
    /// Only lines before this, read the same way.
    pub until: Option<&'a str>,
    /// Keep reading new lines as they come, for this many seconds (at most
    /// 60), after the newest ones.
    pub follow_seconds: Option<u32>,
}

/// The longest a log read follows new lines.
const MAX_FOLLOW_SECONDS: u32 = 60;
/// What `timeout` exits with when it stopped a command.
const TIMED_OUT: u32 = 124;

/// A time as `journalctl` reads it: a span such as `1h` means "1h ago".
fn journal_time(time: &str) -> String {
    if time.bytes().all(|b| b.is_ascii_alphanumeric())
        && time.starts_with(|c: char| c.is_ascii_digit())
    {
        format!("-{time}")
    } else {
        time.to_owned()
    }
}

/// The command that reads the logs `ask` names, and how long it may take.
fn log_command(ask: LogsAsk<'_>) -> Result<(String, Duration), EngineError> {
    let lines = ask
        .lines
        .unwrap_or(DEFAULT_LOG_LINES)
        .clamp(1, MAX_LOG_LINES);
    if ask.unit.is_some() && ask.container.is_some() {
        return Err(EngineError::Invalid(
            "Give a unit or a container, not both.",
        ));
    }
    if [ask.unit, ask.container]
        .into_iter()
        .flatten()
        .any(|name| !plain_name(name))
        || [ask.since, ask.until]
            .into_iter()
            .flatten()
            .any(|time| !plain_since(time))
    {
        return Err(EngineError::Invalid(
            "Give the unit or container by name, and since and until as a time such as 2026-11-22 10:00 or 1h.",
        ));
    }
    if ask.follow_seconds.is_some() && ask.until.is_some() {
        return Err(EngineError::Invalid(
            "Follow new lines or read up to a time, not both.",
        ));
    }
    let follow = ask
        .follow_seconds
        .map(|seconds| seconds.clamp(1, MAX_FOLLOW_SECONDS));
    let (prefix, flag) = match follow {
        Some(seconds) => (format!("sudo -n timeout {seconds} "), " -f"),
        None => ("sudo -n ".to_owned(), ""),
    };
    let command = match ask.container {
        Some(container) => {
            let times: String = [("--since", ask.since), ("--until", ask.until)]
                .into_iter()
                .filter_map(|(name, time)| {
                    time.map(|time| format!(" {name} {}", shell_quote(time)))
                })
                .collect();
            format!(
                "{prefix}docker logs --timestamps --tail {lines}{flag}{times} {} 2>&1",
                shell_quote(container)
            )
        }
        None => {
            let unit = ask
                .unit
                .map(|unit| format!(" -u {}", shell_quote(unit)))
                .unwrap_or_default();
            let times: String = [("--since", ask.since), ("--until", ask.until)]
                .into_iter()
                .filter_map(|(name, time)| {
                    time.map(|time| format!(" {name} {}", shell_quote(&journal_time(time))))
                })
                .collect();
            format!("{prefix}journalctl --no-pager -o short-iso -n {lines}{flag}{unit}{times}")
        }
    };
    let timeout = READ_TIMEOUT + Duration::from_secs(u64::from(follow.unwrap_or(0)));
    Ok((command, timeout))
}

/// A file to write.
#[derive(Debug, Clone, Copy)]
pub struct FileWrite<'a> {
    /// Its full path.
    pub path: &'a str,
    /// What goes in it.
    pub content: &'a [u8],
    /// Its mode, in octal such as `644` or `0600`.
    pub mode: Option<&'a str>,
    /// Who owns it: `user` or `user:group`; `root` when left out.
    pub owner: Option<&'a str>,
}

/// A deploy key just made: the private half is shown once and kept nowhere.
#[derive(Debug)]
pub struct DeployKey {
    /// The public half, as added to `authorized_keys`.
    pub public_key: String,
    /// The private half, in OpenSSH's format, for a CI secret.
    pub private_key: Secret,
    /// Who signs in with it.
    pub user: String,
}

/// Servers Hoista made, and what's done on them.
#[derive(Clone, Copy)]
pub struct ServerService<'a> {
    engine: &'a Engine,
}

impl Engine {
    /// Servers Hoista made, and what's done on them.
    #[must_use]
    pub fn servers(&self) -> ServerService<'_> {
        ServerService { engine: self }
    }
}

/// Where a server is and how to sign in to it.
struct Reached {
    resource: Resource,
    host: String,
    access: ServerAccess,
}

/// What an approval for something on a server is for: the server, what's
/// done and a digest of exactly what, so an approval fits one change only.
#[must_use]
pub fn approval_target(resource_id: &str, what: &str, detail: &[u8]) -> String {
    let digest: String = Sha256::digest(detail)
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{resource_id}:{what}:{digest}")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A unit or container name, as systemd and Docker allow them.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._:-".contains(&b))
}

/// A `--since` value: a date and time, or a span such as `30m`.
fn plain_since(since: &str) -> bool {
    !since.is_empty()
        && since.len() <= 32
        && since
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" -:+.".contains(&b))
}

impl<'a> ServerService<'a> {
    /// The server with this resource id, where it answers and the key to
    /// sign in with. A server whose host key is still the one it was made
    /// with gets a new one first.
    async fn reach(
        self,
        actor: &Actor,
        resource_id: &str,
        scope: &str,
    ) -> Result<Reached, EngineError> {
        self.engine.authorize(actor, scope).await?;
        self.reach_resource(resource_id).await
    }

    /// As [`Self::reach`], for Hoista itself checking on the server.
    async fn reach_resource(self, resource_id: &str) -> Result<Reached, EngineError> {
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.kind != ResourceKind::Server {
            return Err(EngineError::Invalid("That resource isn't a server."));
        }
        let adapter = self.engine.adapter(resource.provider)?;
        if !adapter.capabilities().server_access {
            return Err(EngineError::Conflict(
                "Hoista can't sign in to servers on this provider.",
            ));
        }
        if self.engine.remote_servers.is_none() {
            return Err(EngineError::Conflict(
                "Hoista signs in to servers only when it runs on your computer, not on a remote.",
            ));
        }
        let access = self.access(&resource).await?;
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let details = adapter
            .provider_details(&token, &account.external_id, &resource.provider_ref)
            .await?;
        drop(token);
        let host = details
            .public_ips
            .iter()
            .find(|ip| ip.contains('.'))
            .or_else(|| details.public_ips.first())
            .cloned()
            .ok_or(EngineError::Conflict(
                "The server has no public address, so Hoista can't reach it.",
            ))?;
        let mut reached = Reached {
            resource,
            host,
            access,
        };
        if reached.access.replace_host_key {
            self.replace_host_key(&mut reached).await?;
        }
        Ok(reached)
    }

    /// The key Hoista keeps for a server. One made but not yet saved under
    /// its resource (the server was found after it was made) is moved there.
    async fn access(self, resource: &Resource) -> Result<ServerAccess, EngineError> {
        let reference = vault_reference(&resource.id);
        match self.engine.vault.read(&reference).await {
            Ok(kept) => return ServerAccess::from_kept(&kept),
            Err(EngineError::NotFound) => {}
            Err(error) => return Err(error),
        }
        let pending = pending_reference(&resource.account_id, &resource.name);
        match self.engine.vault.read(&pending).await {
            Ok(saved) => {
                let kept = ServerSetup::from_secret(&saved)?.kept();
                self.engine.vault.write(&reference, kept).await?;
                self.engine.vault.delete(&pending).await?;
                ServerAccess::from_kept(&self.engine.vault.read(&reference).await?)
            }
            Err(EngineError::NotFound) => Err(EngineError::Conflict(
                "Hoista didn't make this server, so it has no key for it. Agents can work on servers made with add_to_project.",
            )),
            Err(error) => Err(error),
        }
    }

    /// Replaces the host key the server was made with, which sits in its
    /// `user_data` for anyone with a Read token, with one the server makes
    /// now, read back over the connection the old key just proved.
    async fn replace_host_key(self, reached: &mut Reached) -> Result<(), EngineError> {
        let script = "set -e
sudo rm -f /etc/ssh/ssh_host_ed25519_key.next /etc/ssh/ssh_host_ed25519_key.next.pub
sudo ssh-keygen -q -t ed25519 -N '' -C \"root@$(hostname)\" -f /etc/ssh/ssh_host_ed25519_key.next
sudo mv /etc/ssh/ssh_host_ed25519_key.next.pub /etc/ssh/ssh_host_ed25519_key.pub
sudo mv /etc/ssh/ssh_host_ed25519_key.next /etc/ssh/ssh_host_ed25519_key
cat /etc/ssh/ssh_host_ed25519_key.pub
sudo systemctl reload ssh 2>/dev/null || sudo systemctl reload sshd";
        let done = self.send(reached, script, None, READ_TIMEOUT).await?;
        let line = text(&done.stdout);
        let line = line.trim();
        if done.exit_code != Some(0) || !line.starts_with("ssh-ed25519 ") || line.contains('\n') {
            return Err(EngineError::Conflict(
                "Hoista couldn't give the server a new host key. Try again in a minute, once it has finished starting.",
            ));
        }
        reached.access.host_key = line.to_owned();
        reached.access.replace_host_key = false;
        self.engine
            .vault
            .write(
                &vault_reference(&reached.resource.id),
                reached.access.to_secret(),
            )
            .await
    }

    async fn send(
        self,
        reached: &Reached,
        command: &str,
        input: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<CommandOutput, EngineError> {
        let remote = self
            .engine
            .remote_servers
            .as_ref()
            .ok_or(EngineError::Store)?;
        remote
            .run(
                ServerAddress {
                    host: &reached.host,
                    port: SSH_PORT,
                },
                &reached.access,
                CommandRun {
                    command,
                    input,
                    timeout,
                    max_output: MAX_OUTPUT,
                },
            )
            .await
    }

    /// Runs `command` and returns what it printed, secrets taken out.
    async fn run_on(
        self,
        reached: &Reached,
        command: &str,
        input: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<RunResult, EngineError> {
        let done = self.send(reached, command, input, timeout).await?;
        let scanner = crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
        Ok(RunResult {
            exit_code: done.exit_code,
            output: scanner.redact(&text(&done.stdout), &[]),
            errors: scanner.redact(&text(&done.stderr), &[]),
            truncated: done.truncated,
            timed_out: done.timed_out,
        })
    }

    /// Where a server Hoista made is in its first-boot setup, while it runs:
    /// None once it's done (or for a server Hoista can't sign in to), so the
    /// provider's word stands. A server still setting up reads as deploying,
    /// and one whose setup failed as failing.
    ///
    /// # Errors
    /// Reports a vault that can't be read or written.
    pub async fn setup_status(
        self,
        resource: &Resource,
    ) -> Result<Option<ResourceStatus>, EngineError> {
        if resource.kind != ResourceKind::Server || self.engine.remote_servers.is_none() {
            return Ok(None);
        }
        let Ok(access) = self.access(resource).await else {
            return Ok(None);
        };
        if access.setup_finished {
            return Ok(None);
        }
        // Not reachable yet: it's still starting.
        let Ok(mut reached) = self.reach_resource(&resource.id).await else {
            return Ok(Some(ResourceStatus::Deploying));
        };
        let Ok(said) = self
            .send(&reached, SETUP_SCRIPT, None, SETUP_CHECK_TIMEOUT)
            .await
        else {
            return Ok(Some(ResourceStatus::Deploying));
        };
        let said = text(&said.stdout);
        let state = said.lines().next().unwrap_or_default().trim();
        let (status, summary) = if state.contains("error") {
            let scanner =
                crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
            let last: Vec<&str> = said
                .lines()
                .skip(1)
                .filter(|line| !line.trim().is_empty())
                .collect();
            (
                ResourceStatus::Failing,
                format!(
                    "{} couldn't finish setting itself up: {}",
                    resource.name,
                    scanner.redact(&last.join(" / "), &[])
                ),
            )
        } else if state.starts_with("done") || state.starts_with("degraded done") {
            (
                ResourceStatus::Live,
                format!("{} finished setting itself up", resource.name),
            )
        } else {
            return Ok(Some(ResourceStatus::Deploying));
        };
        reached.access.setup_finished = true;
        self.engine
            .vault
            .write(&vault_reference(&resource.id), reached.access.to_secret())
            .await?;
        self.engine
            .repository
            .event_about(
                "engine",
                resource.project_id.as_deref(),
                &resource.id,
                if status == ResourceStatus::Live {
                    "server.ready"
                } else {
                    "server.setup_failed"
                },
                &summary,
            )
            .await?;
        Ok(Some(status))
    }

    /// The newest things done on a server (commands, files, keys, its setup),
    /// newest first.
    ///
    /// # Errors
    /// Missing permission or unavailable state.
    pub async fn activity(
        self,
        actor: &Actor,
        resource_id: &str,
    ) -> Result<Vec<crate::Event>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        self.engine
            .repository
            .events_about(resource_id, "server.", MAX_ACTIVITY)
            .await
    }

    /// Records what was done on a server, for the activity log.
    async fn record(
        self,
        actor: &Actor,
        reached: &Reached,
        kind: &str,
        summary: String,
    ) -> Result<(), EngineError> {
        self.engine
            .repository
            .event_about(
                actor.id(),
                reached.resource.project_id.as_deref(),
                &reached.resource.id,
                kind,
                &summary,
            )
            .await
    }

    /// How the server is: its setup, load, memory, disk, failed services,
    /// containers and waiting updates.
    ///
    /// # Errors
    /// Missing permission, a resource that isn't a server Hoista made, or a
    /// server that can't be reached.
    pub async fn status(self, actor: &Actor, resource_id: &str) -> Result<RunResult, EngineError> {
        let reached = self.reach(actor, resource_id, "apps:read").await?;
        self.run_on(&reached, STATUS_SCRIPT, None, READ_TIMEOUT)
            .await
    }

    /// The newest lines of a systemd unit's journal, a container's logs, or
    /// the whole journal; between two times, or as they come for a while.
    ///
    /// # Errors
    /// As [`Self::status`], and a unit, container or time Hoista can't pass on.
    pub async fn logs(
        self,
        actor: &Actor,
        resource_id: &str,
        ask: LogsAsk<'_>,
    ) -> Result<RunResult, EngineError> {
        let (command, timeout) = log_command(ask)?;
        let reached = self.reach(actor, resource_id, "apps:read").await?;
        let mut read = self.run_on(&reached, &command, None, timeout).await?;
        // Following ends when its time is up, which `timeout` reports as 124.
        if ask.follow_seconds.is_some() && read.exit_code == Some(TIMED_OUT) {
            read.exit_code = Some(0);
        }
        Ok(read)
    }

    /// A file's contents, up to 1 MB. Password hashes, private keys and
    /// other processes' environments are never read, wherever a link points.
    ///
    /// # Errors
    /// As [`Self::status`], and a path that isn't plain or is never read.
    pub async fn read_file(
        self,
        actor: &Actor,
        resource_id: &str,
        path: &str,
    ) -> Result<RunResult, EngineError> {
        if !plain_path(path) {
            return Err(EngineError::Invalid(
                "Give the file's full path, such as /srv/app/compose.yaml.",
            ));
        }
        let reached = self.reach(actor, resource_id, "apps:read").await?;
        let resolved = self
            .send(
                &reached,
                &format!("sudo -n readlink -e -- {}", shell_quote(path)),
                None,
                READ_TIMEOUT,
            )
            .await?;
        let real = text(&resolved.stdout).trim().to_owned();
        if resolved.exit_code != Some(0) || real.is_empty() {
            return Err(EngineError::Invalid(
                "There's no file at that path on the server.",
            ));
        }
        if crate::domain::servers::path_never_read(path)
            || crate::domain::servers::path_never_read(&real)
        {
            return Err(EngineError::Conflict(
                "Hoista never reads that file for an agent: it holds passwords or private keys.",
            ));
        }
        let command = format!(
            "sudo -n test -f {real_q} && sudo -n head -c {limit} -- {real_q}",
            real_q = shell_quote(&real),
            limit = MAX_FILE + 1,
        );
        let mut read = self.run_on(&reached, &command, None, READ_TIMEOUT).await?;
        if read.exit_code != Some(0) {
            return Err(EngineError::Invalid(
                "That path isn't a file Hoista can read.",
            ));
        }
        if read.output.len() > MAX_FILE {
            read.output.truncate(MAX_FILE);
            read.truncated = true;
        }
        Ok(read)
    }

    /// What an approval to run `command` is for.
    #[must_use]
    pub fn run_target(resource_id: &str, command: &str) -> String {
        let what = if crate::domain::servers::command_always_asks(command) {
            "run_always_ask"
        } else {
            "run"
        };
        approval_target(resource_id, what, command.as_bytes())
    }

    /// What an approval to write a file is for.
    #[must_use]
    pub fn write_target(resource_id: &str, write: &FileWrite<'_>) -> String {
        let mut detail = Vec::new();
        for part in [
            write.path.as_bytes(),
            write.mode.unwrap_or_default().as_bytes(),
            write.owner.unwrap_or_default().as_bytes(),
            write.content,
        ] {
            detail.extend_from_slice(&(part.len() as u64).to_be_bytes());
            detail.extend_from_slice(part);
        }
        let what = if crate::domain::servers::write_always_asks(write.path) {
            "write_always_ask"
        } else {
            "write"
        };
        approval_target(resource_id, what, &detail)
    }

    /// What an approval to make a deploy key is for.
    #[must_use]
    pub fn deploy_key_target(resource_id: &str, name: &str, command: &str) -> String {
        approval_target(
            resource_id,
            "deploy_key",
            format!("{name}\n{command}").as_bytes(),
        )
    }

    /// Whether an agent with Full access may do this on the server without
    /// asking: the person turned that on for the server, and it isn't on the
    /// always-ask list. Making a deploy key always asks.
    ///
    /// # Errors
    /// Reports a vault that can't be read.
    pub async fn allowed_without_asking(self, target: &str) -> Result<bool, EngineError> {
        let mut parts = target.splitn(3, ':');
        let (Some(resource_id), Some(what)) = (parts.next(), parts.next()) else {
            return Ok(false);
        };
        if !matches!(what, "run" | "write") {
            return Ok(false);
        }
        let Some(resource) = self.engine.repository.resource(resource_id).await? else {
            return Ok(false);
        };
        Ok(self
            .access(&resource)
            .await
            .is_ok_and(|access| access.agents_full_access))
    }

    /// Runs one command as `hoista`, who can use `sudo`.
    ///
    /// # Errors
    /// As [`Self::status`], and an agent without an approval for exactly
    /// this command.
    pub async fn run(
        self,
        actor: &Actor,
        resource_id: &str,
        command: &str,
        timeout_seconds: Option<u64>,
        approval: Option<&str>,
    ) -> Result<RunResult, EngineError> {
        if command.trim().is_empty() || command.len() > 16 * 1024 || command.contains('\0') {
            return Err(EngineError::Invalid("Give one command of at most 16 KB."));
        }
        self.engine.authorize(actor, "apps:write").await?;
        self.engine
            .gate(
                actor,
                GatedAction::ServerCommand,
                &Self::run_target(resource_id, command),
                approval,
            )
            .await?;
        let reached = self.reach(actor, resource_id, "apps:write").await?;
        let seconds = timeout_seconds
            .unwrap_or(DEFAULT_RUN_SECONDS)
            .clamp(1, MAX_RUN_SECONDS);
        let done = self
            .run_on(&reached, command, None, Duration::from_secs(seconds))
            .await?;
        let mut shown: String = command.chars().take(200).collect();
        if shown.len() < command.len() {
            shown.push('…');
        }
        self.record(
            actor,
            &reached,
            "server.command",
            format!(
                "Ran a command on {}: {shown} (exit {})",
                reached.resource.name,
                done.exit_code
                    .map_or_else(|| "unknown".to_owned(), |code| code.to_string())
            ),
        )
        .await?;
        Ok(done)
    }

    /// Writes a file, making its folder if needed.
    ///
    /// # Errors
    /// As [`Self::run`], and a path, mode or owner Hoista can't pass on.
    pub async fn write_file(
        self,
        actor: &Actor,
        resource_id: &str,
        write: FileWrite<'_>,
        approval: Option<&str>,
    ) -> Result<RunResult, EngineError> {
        if !plain_path(write.path) || write.path.ends_with('/') {
            return Err(EngineError::Invalid(
                "Give the file's full path, such as /srv/app/compose.yaml.",
            ));
        }
        if write.content.len() > MAX_FILE {
            return Err(EngineError::Invalid("A file can be at most 1 MB."));
        }
        let mode = write.mode.unwrap_or("644");
        if !(3..=4).contains(&mode.len()) || !mode.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            return Err(EngineError::Invalid(
                "Give mode in octal, such as 644 or 600.",
            ));
        }
        let owner = write.owner.unwrap_or("root");
        let (user, group) = owner.split_once(':').unwrap_or((owner, owner));
        if !plain_name(user) || !plain_name(group) {
            return Err(EngineError::Invalid("Give owner as user or user:group."));
        }
        self.engine.authorize(actor, "apps:write").await?;
        self.engine
            .gate(
                actor,
                GatedAction::ServerCommand,
                &Self::write_target(resource_id, &write),
                approval,
            )
            .await?;
        let reached = self.reach(actor, resource_id, "apps:write").await?;
        let command = format!(
            "sudo -n install -D -m {mode} -o {} -g {} /dev/stdin {}",
            shell_quote(user),
            shell_quote(group),
            shell_quote(write.path)
        );
        let done = self
            .run_on(&reached, &command, Some(write.content), READ_TIMEOUT)
            .await?;
        if done.exit_code != Some(0) {
            return Ok(done);
        }
        self.record(
            actor,
            &reached,
            "server.file_written",
            format!(
                "Wrote {} on {} ({} bytes, mode {mode}, owner {owner})",
                write.path,
                reached.resource.name,
                write.content.len()
            ),
        )
        .await?;
        Ok(done)
    }

    /// Makes a key that can sign in as `hoista` and run only `command`, for
    /// a CI job to deploy with. The private half is returned once.
    ///
    /// # Errors
    /// As [`Self::run`], and a name or command Hoista can't pass on.
    pub async fn deploy_key(
        self,
        actor: &Actor,
        resource_id: &str,
        name: &str,
        command: &str,
        approval: Option<&str>,
    ) -> Result<DeployKey, EngineError> {
        if !crate::domain::model::valid_slug(name) {
            return Err(EngineError::Invalid(
                "Name the key with lowercase letters, digits and hyphens, such as github-actions.",
            ));
        }
        if command.trim().is_empty()
            || command.len() > 1024
            || command.contains(['"', '\n', '\0', '\\'])
        {
            return Err(EngineError::Invalid(
                "Give the one command the key may run, without double quotes, such as /srv/app/deploy.sh.",
            ));
        }
        self.engine.authorize(actor, "apps:write").await?;
        self.engine
            .gate(
                actor,
                GatedAction::ServerCommand,
                &Self::deploy_key_target(resource_id, name, command),
                approval,
            )
            .await?;
        let reached = self.reach(actor, resource_id, "apps:write").await?;
        let key = ServerKey::generate(&format!("{name}@hoista"))?;
        let line = format!("restrict,command=\"{command}\" {}", key.public_line());
        let script = format!(
            "set -e; umask 077; mkdir -p ~/.ssh; printf '%s\\n' {} >> ~/.ssh/authorized_keys",
            shell_quote(&line)
        );
        let done = self.send(&reached, &script, None, READ_TIMEOUT).await?;
        if done.exit_code != Some(0) {
            return Err(EngineError::Conflict(
                "The server didn't take the deploy key.",
            ));
        }
        self.record(
            actor,
            &reached,
            "server.deploy_key",
            format!(
                "Made deploy key {name} on {}, which can only run {command}",
                reached.resource.name
            ),
        )
        .await?;
        Ok(DeployKey {
            public_key: key.public_line(),
            private_key: key.private_openssh()?,
            user: reached.access.user.clone(),
        })
    }

    /// Whether agents with Full access may run commands and write files on
    /// this server without asking.
    ///
    /// # Errors
    /// Missing permission, or a resource that isn't a server Hoista made.
    pub async fn agents_full_access(
        self,
        actor: &Actor,
        resource_id: &str,
    ) -> Result<bool, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        Ok(self.access(&resource).await?.agents_full_access)
    }

    /// Lets agents with Full access run commands and write files on this
    /// server without asking, or stops that. Only the person can change it,
    /// and turning it on asks them to confirm it's them.
    ///
    /// # Errors
    /// An agent asking, a declined confirmation, or a resource that isn't a
    /// server Hoista made.
    pub async fn set_agents_full_access(
        self,
        actor: &Actor,
        resource_id: &str,
        on: bool,
    ) -> Result<(), EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the person can change what agents may do on a server.",
            ));
        }
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let mut access = self.access(&resource).await?;
        if on && !access.agents_full_access {
            self.engine
                .confirm_owner(&format!(
                    "let agents with Full access run commands on {} without asking",
                    resource.name
                ))
                .await?;
        }
        access.agents_full_access = on;
        self.engine
            .vault
            .write(&vault_reference(&resource.id), access.to_secret())
            .await?;
        self.engine
            .repository
            .event_about(
                actor.id(),
                resource.project_id.as_deref(),
                &resource.id,
                "server.agent_access",
                &if on {
                    format!(
                        "Let agents with Full access work on {} without asking",
                        resource.name
                    )
                } else {
                    format!("Agents ask before working on {}", resource.name)
                },
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approvals_fit_one_change() {
        let one = ServerService::run_target("r1", "docker compose up -d");
        assert!(one.starts_with("r1:run:"));
        assert_ne!(one, ServerService::run_target("r1", "docker compose down"));
        assert_ne!(one, ServerService::run_target("r2", "docker compose up -d"));
        assert!(ServerService::run_target("r1", "sudo reboot").starts_with("r1:run_always_ask:"));
        let write = FileWrite {
            path: "/srv/a",
            content: b"x",
            mode: None,
            owner: None,
        };
        let other = FileWrite {
            content: b"y",
            ..write
        };
        assert_ne!(
            ServerService::write_target("r1", &write),
            ServerService::write_target("r1", &other)
        );
    }

    #[test]
    fn logs_read_a_range_or_follow_for_a_while() {
        let (range, _) = log_command(LogsAsk {
            unit: Some("caddy"),
            since: Some("2h"),
            until: Some("1h"),
            ..LogsAsk::default()
        })
        .unwrap();
        assert_eq!(
            range,
            "sudo -n journalctl --no-pager -o short-iso -n 200 -u 'caddy' --since '-2h' --until '-1h'"
        );
        let (follow, timeout) = log_command(LogsAsk {
            container: Some("app-web-1"),
            follow_seconds: Some(500),
            ..LogsAsk::default()
        })
        .unwrap();
        assert_eq!(
            follow,
            "sudo -n timeout 60 docker logs --timestamps --tail 200 -f 'app-web-1' 2>&1"
        );
        assert_eq!(timeout, READ_TIMEOUT + Duration::from_secs(60));
        assert!(
            log_command(LogsAsk {
                follow_seconds: Some(5),
                until: Some("1h"),
                ..LogsAsk::default()
            })
            .is_err()
        );
    }

    #[test]
    fn names_and_times_are_checked() {
        assert!(plain_name("caddy.service"));
        assert!(plain_name("app-web-1"));
        assert!(!plain_name("x; rm -rf /"));
        assert!(plain_since("2026-11-22 10:00:00"));
        assert!(plain_since("1h"));
        assert!(!plain_since("$(id)"));
    }
}
