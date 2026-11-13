//! The computer's own owner prompt: Touch ID or the account password on
//! macOS, Windows Hello on Windows, and polkit's password prompt on Linux.
//! Shown by `hoistd` itself, so an agent's key or a script can't answer it.

use crate::application::ports::{
    catalog::EngineError,
    owner_check::{OwnerCheck, declined, unavailable},
};
use std::time::Duration;

/// How long the prompt waits for the person; shorter than the coding-tool
/// bridge's one-minute wait for Hoista's answer.
const WAIT: Duration = Duration::from_secs(50);

/// The owner prompt of the computer Hoista runs on.
pub struct SystemOwnerCheck;

#[async_trait::async_trait]
impl OwnerCheck for SystemOwnerCheck {
    async fn confirm(&self, reason: &str) -> Result<(), EngineError> {
        // Test builds only: lets the engine's own end-to-end tests answer.
        // Compiled out of release builds.
        #[cfg(debug_assertions)]
        if let Ok(answer) = std::env::var("HOISTA_TEST_OWNER_CHECK") {
            return if answer == "allow" {
                Ok(())
            } else {
                Err(declined())
            };
        }
        let reason = reason.to_owned();
        tokio::task::spawn_blocking(move || platform::confirm(&reason))
            .await
            .map_err(|_| unavailable())?
    }
}

/// macOS (Touch ID, Apple Watch or the account password) and Windows
/// (Windows Hello, or the account password), through `robius-authentication`,
/// which keeps the platform calls out of Hoista's own code.
#[cfg(any(target_os = "macos", windows))]
mod platform {
    use super::{WAIT, declined, unavailable};
    use crate::application::ports::catalog::EngineError;
    use robius_authentication::{
        AndroidText, BiometricStrength, Context, PolicyBuilder, Text, WindowsText,
    };

    pub(super) fn confirm(reason: &str) -> Result<(), EngineError> {
        let policy = PolicyBuilder::new()
            .biometrics(Some(BiometricStrength::Strong))
            .password(true)
            .companion(true)
            .build()
            .ok_or_else(unavailable)?;
        let text = Text {
            android: AndroidText {
                title: "Hoista",
                subtitle: None,
                description: None,
            },
            // macOS shows: "hoistd" is trying to <reason>.
            apple: reason,
            windows: WindowsText::new_truncated("Hoista", reason),
        };
        let (send, answer) = std::sync::mpsc::channel::<bool>();
        // Kept until the answer arrives, so the prompt isn't cancelled.
        let context = Context::new(());
        context
            .authenticate(text, &policy, move |result| {
                let _ = send.send(result.is_ok());
            })
            .map_err(|_| unavailable())?;
        let confirmed = answer.recv_timeout(WAIT).unwrap_or(false);
        drop(context);
        if confirmed { Ok(()) } else { Err(declined()) }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::{WAIT, declined, unavailable};
    use crate::application::ports::catalog::EngineError;
    use std::process::{Command, Stdio};

    /// Asks polkit, which shows the desktop's password prompt. Without a
    /// polkit agent in the session there is nothing to ask, so nothing is
    /// allowed.
    pub(super) fn confirm(_reason: &str) -> Result<(), EngineError> {
        let mut child = Command::new("pkcheck")
            .args([
                "--action-id",
                "org.freedesktop.policykit.exec",
                "--process",
                &std::process::id().to_string(),
                "--allow-user-interaction",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| unavailable())?;
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                // 2: not authorized without asking, 3: the prompt was
                // dismissed, 4: not authorized. Anything else means polkit
                // couldn't be reached, so there was nobody to ask.
                Ok(Some(status)) if matches!(status.code(), Some(2..=4)) => {
                    return Err(declined());
                }
                Ok(Some(_)) => return Err(unavailable()),
                Ok(None) if started.elapsed() < WAIT => {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                _ => {
                    let _ = child.kill();
                    return Err(declined());
                }
            }
        }
    }
}
