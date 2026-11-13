//! Credential-free, least-privilege Task Scheduler registration for the current user.

use super::{Platform, Registration, path_text, xml};
use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Stdio};

/// Windows caps a command line at 32,767 characters; this stays under it.
const MAX_COMMAND_CHARS: usize = 30_000;
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const MAX_OUTPUT_BYTES: usize = 8192;
const MAX_SID_BYTES: usize = 256;

// CommandLineToArgvW / CRT quoting: preserve spaces, quotes and trailing backslashes.
fn argument(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        if character == '"' {
            result.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            result.extend(std::iter::repeat_n('\\', slashes));
        }
        result.push(character);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}

fn normal_path(value: &str) -> String {
    if let Some(value) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{value}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(value).to_owned()
    }
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn encode(script: &str) -> anyhow::Result<String> {
    let encoded = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    if encoded.len() > MAX_COMMAND_CHARS {
        bail!(
            "The service command exceeds Windows' command-line limit. Use shorter installation and state paths."
        );
    }
    Ok(encoded)
}

async fn powershell(script: &str) -> anyhow::Result<String> {
    let system = std::env::var_os("SystemRoot").context("Windows SystemRoot is unavailable")?;
    let program =
        std::path::PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    if !program.is_absolute() || !program.is_file() {
        bail!("Windows PowerShell is unavailable.");
    }
    let mut command = tokio::process::Command::new(program);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encode(script)?,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
        .await
        .context("Windows Task Scheduler did not respond within 20 seconds")??;
    if !output.status.success() {
        bail!(
            "Windows Task Scheduler rejected the request or an existing task differs from this installation. Run from your own login session and review the Hoista task before retrying. No different task is overwritten."
        );
    }
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        bail!("Unexpected Windows service response.");
    }
    Ok(String::from_utf8(output.stdout)
        .context("Invalid Windows service response")?
        .trim()
        .to_owned())
}

fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid.len() <= MAX_SID_BYTES
        && sid[4..].split('-').all(|resource| {
            !resource.is_empty() && resource.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn descriptor(name: &str, daemon: &str, root: &str, sid: &str) -> anyhow::Result<String> {
    if !valid_sid(sid)
        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || daemon.chars().chain(root.chars()).any(char::is_control)
    {
        bail!("Invalid Windows service identity or paths.");
    }
    let command = xml(&normal_path(daemon));
    let arguments = xml(&format!(
        "--state-dir {} --supervised",
        argument(&normal_path(root))
    ));
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<RegistrationInfo><Description>Hoista per-user daemon {name}</Description></RegistrationInfo>
<Triggers>
<LogonTrigger id="HoistaLogin"><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger>
<TimeTrigger id="HoistaRetry"><Repetition><Interval>PT1M</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>true</Enabled></TimeTrigger>
</Triggers>
<Principals><Principal id="HoistaUser"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><AllowHardTerminate>true</AllowHardTerminate><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled><Hidden>false</Hidden><RunOnlyIfIdle>false</RunOnlyIfIdle><WakeToRun>false</WakeToRun><ExecutionTimeLimit>PT0S</ExecutionTimeLimit></Settings>
<Actions Context="HoistaUser"><Exec><Command>{command}</Command><Arguments>{arguments}</Arguments></Exec></Actions>
</Task>
"#
    ))
}

pub(super) async fn registration(daemon: &Path, root: &Path) -> anyhow::Result<Registration> {
    let sid = powershell("[Security.Principal.WindowsIdentity]::GetCurrent().User.Value").await?;
    let state = path_text(root)?;
    let digest = format!("{:x}", Sha256::digest(format!("{sid}\n{state}").as_bytes()));
    let name = format!("Hoista-{}", &digest[..24]);
    Ok(Registration {
        platform: Platform::Windows,
        file: root.join("service-task.xml"),
        content: descriptor(&name, path_text(daemon)?, state, &sid)?,
        name,
    })
}

fn script(name: &str, content: &str, action: &str) -> anyhow::Result<String> {
    if !matches!(action, "install" | "status" | "uninstall") {
        bail!("Invalid service action.");
    }
    Ok(format!(
        "$HoistaTaskName={}\n$HoistaXml={}\n$HoistaAction={}\n{}",
        literal(name),
        literal(content),
        literal(action),
        include_str!("windows.ps1")
    ))
}

pub(super) async fn manager(registration: &Registration, action: &str) -> anyhow::Result<bool> {
    let result = powershell(&script(&registration.name, &registration.content, action)?).await?;
    let value: serde_json::Value =
        serde_json::from_str(&result).context("Invalid Windows task response")?;
    value["registered"]
        .as_bool()
        .context("Missing Windows task registration status")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Exports an offline Task Scheduler fixture to an explicitly selected directory"]
    fn export_windows_service_fixture() {
        use std::io::Write;
        let directory = std::path::PathBuf::from(
            std::env::var_os("HOISTA_SERVICE_FIXTURE_DIR")
                .expect("Set HOISTA_SERVICE_FIXTURE_DIR to an existing disposable directory"),
        );
        let content = descriptor(
            "Hoista-Fixture",
            r"C:\O'Brien & tools\hoistd.exe",
            r"C:\state with $ and %\",
            "S-1-5-21-123",
        )
        .unwrap();
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("task.xml"))
            .unwrap();
        output.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn windows_quoting_and_task_policy_preserve_paths_without_credentials() {
        assert_eq!(
            argument(r"C:\state with spaces\"),
            "\"C:\\state with spaces\\\\\""
        );
        assert_eq!(argument("a\\\"b"), "\"a\\\\\\\"b\"");
        assert_eq!(normal_path(r"\\?\C:\bin\hoistd.exe"), r"C:\bin\hoistd.exe");
        assert_eq!(normal_path(r"\\?\UNC\provider\share"), r"\\provider\share");
        let content = descriptor(
            "Hoista-abc",
            r"C:\O'Brien & tools\hoistd.exe",
            r"C:\state with $ and %\",
            "S-1-5-21-123",
        )
        .unwrap();
        for expected in [
            "InteractiveToken",
            "LeastPrivilege",
            "<Interval>PT1M</Interval>",
            "<ExecutionTimeLimit>PT0S",
            "IgnoreNew",
            "O&apos;Brien &amp; tools",
            "<Hidden>false",
            "<RunOnlyIfNetworkAvailable>false",
            " --supervised</Arguments>",
        ] {
            assert!(content.contains(expected), "{expected}");
        }
        assert!(!content.contains("<Duration>"));
        assert!(!content.contains("<Password>"));
        let command = script("Hoista-abc", &content, "install").unwrap();
        assert!(encode(&command).is_ok());
        let bytes = STANDARD.decode(encode(&command).unwrap()).unwrap();
        let decoded = String::from_utf16(
            &bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(decoded, command);
        assert_eq!(literal("O'Brien $(command)"), "'O''Brien $(command)'");
        assert!(descriptor("Hoista-abc", "x", "y", "S-1-5; command").is_err());
        assert!(script("Hoista-abc", &content, "invalid").is_err());
        assert!(script("Hoista-abc", &content, "restart").is_err());
        assert!(encode(&"x".repeat(30_000)).is_err());
    }
}
