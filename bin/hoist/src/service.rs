//! Registering the daemon as a per-user service. Service files contain paths,
//! never credentials.

mod windows;

use anyhow::{Context, bail};
use hoista_server::infrastructure::paths::StatePaths;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};

const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const MAX_SERVICE_FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy)]
enum Platform {
    Mac,
    Linux,
    Windows,
}

struct Registration {
    platform: Platform,
    name: String,
    file: PathBuf,
    content: String,
}

fn path_text(path: &Path) -> anyhow::Result<&str> {
    let value = path.to_str().context("Service paths must be UTF-8")?;
    if !path.is_absolute() || value.chars().any(char::is_control) {
        bail!("Service paths must be absolute and contain no control characters.");
    }
    Ok(value)
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn unit_arg(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    )
}

fn registration(
    platform: Platform,
    home: &Path,
    config: &Path,
    daemon: &Path,
    root: &Path,
) -> anyhow::Result<Registration> {
    let executable = path_text(daemon)?;
    let state = path_text(root)?;
    let digest = format!("{:x}", Sha256::digest(state.as_bytes()));
    let name = format!("com.hoista.daemon.{}", &digest[..24]);
    let (file, content) = match platform {
        Platform::Windows => bail!("Use the Windows identity-aware registration factory."),
        Platform::Mac => (
            home.join("Library/LaunchAgents")
                .join(format!("{name}.plist")),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{name}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--state-dir</string><string>{}</string><string>--supervised</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>ThrottleInterval</key><integer>10</integer>\n<key>ProcessType</key><string>Background</string>\n<key>StandardOutPath</key><string>/dev/null</string>\n<key>StandardErrorPath</key><string>/dev/null</string>\n</dict></plist>\n",
                xml(executable),
                xml(state)
            ),
        ),
        Platform::Linux => (
            config.join("systemd/user").join(format!("{name}.service")),
            format!(
                "[Unit]\nDescription=Hoista cloud management daemon\nStartLimitIntervalSec=0\n\n[Service]\nType=simple\nExecStart={} --state-dir {} --supervised\nRestart=always\nRestartSec=5\nTimeoutStopSec=30\nUMask=0077\n\n[Install]\nWantedBy=default.target\n",
                unit_arg(executable),
                unit_arg(state)
            ),
        ),
    };
    Ok(Registration {
        platform,
        name,
        file,
        content,
    })
}

async fn command(program: &str, args: &[&str], success_required: bool) -> anyhow::Result<bool> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(COMMAND_TIMEOUT, command.status())
        .await
        .context("The user service manager did not respond within 20 seconds")?
        .context("Could not run the user service manager")?;
    if success_required && !status.success() {
        bail!(
            "The user service manager rejected the request. Check your login session and hoist service status; the descriptor is retained for retry."
        );
    }
    Ok(status.success())
}

async fn domain() -> anyhow::Result<String> {
    let result = tokio::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .await?;
    let uid = std::str::from_utf8(&result.stdout)?.trim();
    if !result.status.success() || uid.is_empty() || !uid.bytes().all(|c| c.is_ascii_digit()) {
        bail!("Could not determine the login user.");
    }
    Ok(format!("gui/{uid}"))
}

impl Registration {
    fn same_file(&self) -> anyhow::Result<bool> {
        match std::fs::symlink_metadata(&self.file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
            Ok(metadata) => {
                if !metadata.is_file()
                    || metadata.len() > MAX_SERVICE_FILE_BYTES
                    || std::fs::read_to_string(&self.file)? != self.content
                {
                    bail!(
                        "The service descriptor differs from this installation. Review it before replacing it: {}",
                        self.file.display()
                    );
                }
                Ok(true)
            }
        }
    }

    fn save(&self) -> anyhow::Result<()> {
        if self.same_file()? {
            return Ok(());
        }
        std::fs::create_dir_all(self.file.parent().context("Missing service directory")?)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.file)?;
        file.write_all(self.content.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    async fn loaded(&self) -> anyhow::Result<bool> {
        match self.platform {
            Platform::Windows => windows::manager(self, "status").await,
            Platform::Mac => {
                command(
                    "/bin/launchctl",
                    &["print", &format!("{}/{}", domain().await?, self.name)],
                    false,
                )
                .await
            }
            Platform::Linux => {
                command(
                    "systemctl",
                    &["--user", "is-active", &format!("{}.service", self.name)],
                    false,
                )
                .await
            }
        }
    }

    async fn install(&self) -> anyhow::Result<()> {
        self.save()?;
        match self.platform {
            Platform::Windows => {
                windows::manager(self, "install").await?;
            }
            Platform::Mac => {
                let domain = domain().await?;
                command(
                    "/bin/launchctl",
                    &["enable", &format!("{domain}/{}", self.name)],
                    true,
                )
                .await?;
                if !self.loaded().await? {
                    command(
                        "/bin/launchctl",
                        &["bootstrap", &domain, path_text(&self.file)?],
                        true,
                    )
                    .await?;
                }
            }
            Platform::Linux => {
                command("systemctl", &["--user", "daemon-reload"], true).await?;
                command(
                    "systemctl",
                    &[
                        "--user",
                        "enable",
                        "--now",
                        &format!("{}.service", self.name),
                    ],
                    true,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn uninstall(&self) -> anyhow::Result<()> {
        if !self.same_file()? {
            bail!("No matching Hoista service descriptor is installed for this state directory.");
        }
        match self.platform {
            Platform::Windows => {
                windows::manager(self, "uninstall").await?;
            }
            Platform::Mac => {
                if self.loaded().await? {
                    command(
                        "/bin/launchctl",
                        &["bootout", &format!("{}/{}", domain().await?, self.name)],
                        true,
                    )
                    .await?;
                }
            }
            Platform::Linux => {
                command(
                    "systemctl",
                    &[
                        "--user",
                        "disable",
                        "--now",
                        &format!("{}.service", self.name),
                    ],
                    true,
                )
                .await?;
            }
        }
        std::fs::remove_file(&self.file)?;
        if matches!(self.platform, Platform::Linux) {
            command("systemctl", &["--user", "daemon-reload"], true).await?;
        }
        Ok(())
    }
}

pub(crate) async fn run(paths: &StatePaths, action: &str) -> anyhow::Result<()> {
    let platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        bail!(
            "Persistent user-service installation requires macOS, Windows or Linux with systemd."
        );
    };
    paths.prepare()?;
    let root = std::fs::canonicalize(paths.root())?;
    let executable = std::fs::canonicalize(std::env::current_exe()?)?;
    let daemon = executable
        .parent()
        .context("Missing installation directory")?
        .join(if cfg!(windows) {
            "hoistd.exe"
        } else {
            "hoistd"
        });
    if !daemon.is_file() {
        bail!("Install hoistd alongside hoist before setting up a service.");
    }
    let home = dirs::home_dir().context("Cannot find your home directory")?;
    let config = dirs::config_dir().context("Cannot find your configuration directory")?;
    let registration = if matches!(platform, Platform::Windows) {
        windows::registration(&daemon, &root).await?
    } else {
        registration(platform, &home, &config, &daemon, &root)?
    };
    match action {
        "preview" => println!("{}", registration.content),
        "install" => {
            hoista_server::infrastructure::tool_paths::configure(paths, false)?;
            registration.install().await?;
            let health = super::bootstrap::wait(paths, None).await?;
            println!(
                "{}",
                json!({"installed":true,"descriptor":registration.file,"health":health})
            );
            eprintln!(
                "Hoista will start at login and restart after exit. An already-running daemon keeps its current work; the service takes over when it exits. This user service needs a logged-in session (or Linux user lingering). Logs remain in your private state directory."
            );
        }
        "restart" => {
            hoista_server::infrastructure::tool_paths::read(paths)?;
            if !registration.same_file()? {
                bail!("Install the user service before restarting it: hoist service install.");
            }
            let client = hoista_server::client::LocalClient::new(paths)?;
            if client.health().await.is_ok() {
                // Install (or reuse) the service first, then ask the running daemon to
                // hand over. The process holding accepted work is never killed.
                registration.install().await?;
                let status = super::bootstrap::restart(paths, true).await?;
                println!("{}", json!({"restarted":true,"health":status}));
            } else {
                // A daemon that doesn't answer may still be finishing accepted work:
                // start the service again without stopping that process.
                registration.install().await?;
                let health = super::bootstrap::wait(paths, None).await?;
                println!(
                    "{}",
                    json!({"restarted":false,"service_started_or_reused":true,"health":health})
                );
            }
        }
        "status" => {
            let installed = registration.same_file()?;
            let loaded = registration.loaded().await?;
            let health = hoista_server::client::LocalClient::new(paths)?
                .health()
                .await
                .ok();
            println!(
                "{}",
                json!({
                    "installed": installed,
                    "registered_or_active": loaded,
                    "descriptor": registration.file,
                    "health": health,
                })
            );
        }
        "uninstall" => {
            registration.uninstall().await?;
            println!("{}", json!({"installed":false}));
            eprintln!(
                "Automatic startup removed. A daemon started separately may still be running; project data and credentials are retained."
            );
        }
        _ => bail!("Use hoist service install, restart, status or uninstall."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptors_escape_paths_and_scope_identity_to_state() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let root_path = home.join("state with & < > ' \" $ % spaces");
        let root = root_path.as_path();
        let daemon_path = home.join("hoistd");
        let daemon = daemon_path.as_path();
        let mac = registration(Platform::Mac, home, home, daemon, root).unwrap();
        assert!(mac.content.contains("&amp; &lt; &gt; &apos; &quot;"));
        assert!(mac.content.contains("<key>KeepAlive</key><true/>"));
        assert!(mac.content.contains("<string>--supervised</string>"));
        let linux = registration(Platform::Linux, home, home, daemon, root).unwrap();
        assert!(linux.content.contains("\\\" $$ %% spaces\""));
        assert!(linux.content.contains("Restart=always"));
        assert!(linux.content.contains(" --supervised\nRestart=always"));
        assert_eq!(mac.name, linux.name);
        assert_ne!(
            mac.name,
            registration(Platform::Mac, home, home, daemon, &home.join("different"))
                .unwrap()
                .name
        );
        assert!(registration(Platform::Mac, home, home, daemon, &home.join("bad\npath")).is_err());
    }

    #[test]
    fn registration_is_idempotent_and_never_overwrites_different_files() {
        let directory = tempfile::tempdir().unwrap();
        let item = registration(
            Platform::Mac,
            directory.path(),
            directory.path(),
            &directory.path().join("hoistd"),
            &directory.path().join("state"),
        )
        .unwrap();
        item.save().unwrap();
        item.save().unwrap();
        std::fs::write(&item.file, "unrelated").unwrap();
        assert!(item.save().is_err());
        assert_eq!(std::fs::read_to_string(&item.file).unwrap(), "unrelated");
    }
}
