//! Runs child processes with a cleared environment, bounded input and a
//! deadline. On Unix the whole process group is cleaned up afterwards.

use hoista_domain::Secret;
use std::{io, process::ExitStatus, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
};

/// Why running a program failed.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// The program could not be started, for example because it isn't installed.
    #[error("The process could not be started.")]
    Spawn(#[source] io::Error),
    /// Reading from or writing to the program failed.
    #[error("Reading from or writing to the program failed.")]
    Io(#[source] io::Error),
    /// The program ran longer than its time limit and was stopped.
    #[error("The process exceeded its deadline.")]
    Timeout,
    /// The program was stopped but Hoista could not confirm it exited.
    #[error("The program was stopped, but it couldn't be confirmed to have exited.")]
    Cleanup,
}

/// Variables a child process inherits from the daemon; everything else is cleared.
///
/// Besides the platform basics, this keeps the network settings that package
/// managers, `git` and `flyctl` need behind a proxy or a private certificate
/// authority, so they reach the network the way the user's own terminal does.
/// Provider tokens and other variables never pass through; callers add the
/// ones a command needs explicitly.
const INHERITED_ENV: [&str; 20] = [
    "PATH",
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "COMSPEC",
    "PATHEXT",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "GIT_SSL_CAINFO",
    "CURL_CA_BUNDLE",
    "NPM_CONFIG_USERCONFIG",
];

/// Copies the allowed variables from the daemon's environment onto a command
/// whose environment was cleared.
pub fn inherit_environment(command: &mut Command) {
    for (key, value) in inherited_environment(|key| std::env::var_os(key)) {
        command.env(key, value);
    }
}

fn inherited_environment(
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(&'static str, std::ffi::OsString)> {
    INHERITED_ENV
        .into_iter()
        .filter_map(|key| lookup(key).map(|value| (key, value)))
        .collect()
}

/// A started program and, on Unix, its process group, which is stopped when
/// this is dropped.
struct OwnedChild {
    child: Child,
    #[cfg(unix)]
    group: Option<nix::unistd::Pid>,
}

impl OwnedChild {
    fn stop_group(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        if let Some(group) = self.group {
            match nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => self.group = None,
                Err(error) => return Err(io::Error::from_raw_os_error(error as i32)),
            }
        }
        Ok(())
    }

    async fn terminate(&mut self) -> Result<(), ProcessError> {
        let group = self.stop_group();
        let kill = self.child.start_kill();
        let reaped = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
        if group.is_err() || kill.is_err() || !matches!(reaped, Ok(Ok(_))) {
            return Err(ProcessError::Cleanup);
        }
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.stop_group().is_err() {
            tracing::warn!(code = "process.group_cleanup_failed");
        }
        // Child's kill_on_drop also covers cancellation and Windows direct children.
        // Tokio owns best-effort reaping when an outer future is unexpectedly dropped.
    }
}

/// Runs `command` to completion within `limit`, writing `input` to its stdin
/// when stdin is piped. The caller sets up the streams and environment; the
/// saved tool folders are added to PATH, and the whole process group is
/// stopped (on Unix) if the program runs too long.
///
/// # Errors
/// Returns a [`ProcessError`] when the program can't start, fails while running,
/// or exceeds `limit`.
pub async fn run(
    command: &mut Command,
    input: Option<&Secret>,
    limit: Duration,
) -> Result<ExitStatus, ProcessError> {
    run_inner(command, input, None, None, limit).await
}

/// Runs `command`, streaming its stdout into `output`, and stops it if it
/// writes more than `max_bytes` or runs longer than `limit`.
///
/// # Errors
/// Returns a [`ProcessError`] when the program can't start, fails while running,
/// writes too much or runs too long.
pub async fn run_to_file(
    command: &mut Command,
    output: &mut tokio::fs::File,
    max_bytes: u64,
    limit: Duration,
) -> Result<ExitStatus, ProcessError> {
    command.stdout(std::process::Stdio::piped());
    run_inner(
        command,
        None,
        Some((Output::File(output), max_bytes)),
        None,
        limit,
    )
    .await
}

enum Output<'a> {
    File(&'a mut tokio::fs::File),
    Memory(&'a mut zeroize::Zeroizing<Vec<u8>>),
}

/// Runs `command` and returns its stdout, in memory that's wiped when dropped.
/// Stops it if it writes more than `max_bytes` or runs longer than `limit`.
///
/// # Errors
/// Returns a [`ProcessError`] when the program can't start, fails while running,
/// writes too much or runs too long.
pub async fn capture(
    command: &mut Command,
    max_bytes: u64,
    limit: Duration,
) -> Result<(ExitStatus, zeroize::Zeroizing<Vec<u8>>), ProcessError> {
    command.stdout(std::process::Stdio::piped());
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    let status = run_inner(
        command,
        None,
        Some((Output::Memory(&mut bytes), max_bytes)),
        None,
        limit,
    )
    .await?;
    Ok((status, bytes))
}

/// Captures bounded stdout and the first `max_bytes` of stderr in zeroizing
/// memory, for tools whose messages explain a failure. Stdout beyond the
/// bound stops the process; stderr beyond it is dropped.
///
/// # Errors
/// Returns a [`ProcessError`] when the program can't start, fails while running,
/// writes too much or runs too long.
pub async fn capture_all(
    command: &mut Command,
    input: Option<&Secret>,
    max_bytes: u64,
    limit: Duration,
) -> Result<
    (
        ExitStatus,
        zeroize::Zeroizing<Vec<u8>>,
        zeroize::Zeroizing<Vec<u8>>,
    ),
    ProcessError,
> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    let mut errors = zeroize::Zeroizing::new(Vec::new());
    let status = run_inner(
        command,
        input,
        Some((Output::Memory(&mut bytes), max_bytes)),
        Some((&mut errors, max_bytes)),
        limit,
    )
    .await?;
    Ok((status, bytes, errors))
}

async fn run_inner(
    command: &mut Command,
    input: Option<&Secret>,
    output: Option<(Output<'_>, u64)>,
    errors: Option<(&mut zeroize::Zeroizing<Vec<u8>>, u64)>,
    limit: Duration,
) -> Result<ExitStatus, ProcessError> {
    crate::tool_paths::apply(command);
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(ProcessError::Spawn)?;
    let mut owned = OwnedChild {
        #[cfg(unix)]
        group: child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .map(nix::unistd::Pid::from_raw),
        child,
    };
    let operation = async {
        if let Some(mut stdin) = owned.child.stdin.take() {
            if let Some(input) = input {
                stdin.write_all(input.expose().as_bytes()).await?;
            }
            stdin.shutdown().await?;
        }
        if let Some((mut output, max_bytes)) = output {
            let mut stdout = owned
                .child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("Missing output pipe"))?;
            let copy = async {
                let mut buffer = zeroize::Zeroizing::new([0_u8; 16 * 1024]);
                let mut written = 0_u64;
                loop {
                    let count = stdout.read(&mut buffer[..]).await?;
                    if count == 0 {
                        break;
                    }
                    written = written
                        .checked_add(count as u64)
                        .ok_or_else(|| io::Error::other("Output limit"))?;
                    if written > max_bytes {
                        return Err(io::Error::other("Output limit"));
                    }
                    match &mut output {
                        Output::File(file) => file.write_all(&buffer[..count]).await?,
                        Output::Memory(bytes) => bytes.extend_from_slice(&buffer[..count]),
                    }
                }
                match output {
                    Output::File(file) => file.flush().await,
                    Output::Memory(_) => Ok(()),
                }
            };
            let stderr = owned.child.stderr.take();
            let drain = async {
                if let (Some(mut stderr), Some((errors, max_bytes))) = (stderr, errors) {
                    let mut buffer = zeroize::Zeroizing::new([0_u8; 8 * 1024]);
                    loop {
                        let count = stderr.read(&mut buffer[..]).await?;
                        if count == 0 {
                            break;
                        }
                        let room = usize::try_from(max_bytes)
                            .unwrap_or(usize::MAX)
                            .saturating_sub(errors.len());
                        errors.extend_from_slice(&buffer[..count.min(room)]);
                    }
                }
                Ok::<(), io::Error>(())
            };
            let (_, _, status) = tokio::try_join!(copy, drain, owned.child.wait())?;
            Ok(status)
        } else {
            owned.child.wait().await
        }
    };
    let result = tokio::time::timeout(limit, operation).await;
    match result {
        Ok(Ok(status)) => {
            // Build tools may leave background helpers behind even after a successful exit.
            owned.stop_group().map_err(ProcessError::Io)?;
            Ok(status)
        }
        Ok(Err(error)) => {
            owned.terminate().await?;
            Err(ProcessError::Io(error))
        }
        Err(_) => {
            owned.terminate().await?;
            Err(ProcessError::Timeout)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{path::Path, process::Stdio};
    fn shell(directory: &Path, script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    #[tokio::test]
    async fn memory_capture_is_bounded_and_returns_only_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = shell(dir.path(), "printf captured; printf diagnostic >&2");
        let (status, bytes) = capture(&mut command, 8, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(status.success());
        assert_eq!(&bytes[..], b"captured");
        let mut command = shell(dir.path(), "printf oversized");
        assert!(
            capture(&mut command, 2, Duration::from_secs(2))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn capture_all_keeps_both_streams_and_bounds_messages() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = shell(dir.path(), "printf out; printf 'long message' >&2; exit 3");
        let (status, out, errors) = capture_all(&mut command, None, 4, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(status.code(), Some(3));
        assert_eq!(&out[..], b"out");
        assert_eq!(&errors[..], b"long");
    }

    async fn group(directory: &Path) -> nix::unistd::Pid {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(value) = tokio::fs::read_to_string(directory.join("group")).await
                    && let Ok(pid) = value.trim().parse::<i32>()
                {
                    return nix::unistd::Pid::from_raw(pid);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("process group started")
    }

    async fn gone(group: nix::unistd::Pid) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if nix::sys::signal::killpg(group, None) == Err(nix::errno::Errno::ESRCH) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("process group terminated");
    }

    #[tokio::test]
    async fn deadlines_and_cancellation_terminate_child_groups() {
        for cancel in [false, true] {
            let directory = tempfile::tempdir().expect("directory");
            let mut command = shell(directory.path(), "sleep 30 & echo $$ > group; wait");
            let task =
                tokio::spawn(
                    async move { run(&mut command, None, Duration::from_millis(500)).await },
                );
            let pid = group(directory.path()).await;
            if cancel {
                task.abort();
                assert!(task.await.expect_err("cancelled").is_cancelled());
            } else {
                assert!(matches!(
                    task.await.expect("task"),
                    Err(ProcessError::Timeout)
                ));
            }
            gone(pid).await;
        }
    }

    #[tokio::test]
    async fn successful_exit_closes_stdin_and_cleans_background_helpers() {
        let directory = tempfile::tempdir().expect("directory");
        let mut command = shell(
            directory.path(),
            "read value; test \"$value\" = fixture || exit 7; sleep 30 & echo $$ > group; exit 0",
        );
        let status = run(
            &mut command,
            Some(&Secret::new("fixture\n".into())),
            Duration::from_secs(5),
        )
        .await
        .expect("run");
        assert!(status.success());
        gone(group(directory.path()).await).await;
    }
}

#[cfg(test)]
mod environment_tests {
    use super::inherited_environment;

    #[test]
    fn children_inherit_proxy_and_certificate_settings_but_not_other_variables() {
        let env = inherited_environment(|key| match key {
            "PATH" => Some("/usr/bin".into()),
            "HTTPS_PROXY" => Some("http://proxy.internal:3128".into()),
            "NODE_EXTRA_CA_CERTS" => Some("/etc/ssl/corp.pem".into()),
            "AWS_SECRET_ACCESS_KEY" | "CLOUDFLARE_API_TOKEN" | "FLY_API_TOKEN" => {
                Some("secret".into())
            }
            _ => None,
        });
        let keys: Vec<&str> = env.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, ["PATH", "HTTPS_PROXY", "NODE_EXTRA_CA_CERTS"]);
    }
}
