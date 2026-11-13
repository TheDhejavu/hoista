//! Confirms it's the person, not an agent or a script, before Hoista lets
//! an agent connect or allows something an agent asked for. The check runs
//! in `hoistd` and uses the computer's own owner prompt (Touch ID or the
//! account password on macOS), which an agent's key can't answer.

use crate::ports::catalog::EngineError;

/// Asks the computer's owner to confirm, showing `reason`.
#[async_trait::async_trait]
pub trait OwnerCheck: Send + Sync {
    /// Returns once the owner confirmed.
    ///
    /// # Errors
    /// Reports a refusal, a timeout, or a computer that can't ask.
    async fn confirm(&self, reason: &str) -> Result<(), EngineError>;
}

/// The person said no or let the prompt time out.
#[must_use]
pub fn declined() -> EngineError {
    EngineError::Conflict("You didn't confirm it's you, so nothing changed.")
}

/// This computer has no owner prompt Hoista can use.
#[must_use]
pub fn unavailable() -> EngineError {
    EngineError::Conflict(
        "Hoista couldn't ask you to confirm it's you on this computer (Touch ID, your password or Windows Hello), so nothing changed.",
    )
}

/// The check until a real one is set where the process starts: it refuses
/// everything, so nothing is ever allowed without one.
pub struct NoOwnerCheck;

#[async_trait::async_trait]
impl OwnerCheck for NoOwnerCheck {
    async fn confirm(&self, _reason: &str) -> Result<(), EngineError> {
        Err(unavailable())
    }
}

/// A check with a fixed answer, for tests and fixtures that wire their own
/// engine. `hoistd` never uses it.
pub struct FixedOwnerCheck(pub bool);

#[async_trait::async_trait]
impl OwnerCheck for FixedOwnerCheck {
    async fn confirm(&self, _reason: &str) -> Result<(), EngineError> {
        if self.0 { Ok(()) } else { Err(declined()) }
    }
}

tokio::task_local! {
    /// Set while handling a request from the owner who signed in with a
    /// passkey moments ago (Hoista on a server).
    static SIGNED_IN: bool;
}

/// Hoista on a server: the owner has proved it's them with a passkey just
/// before the request that is being handled. There is no prompt to show on
/// a server, so anything else is refused.
pub struct PasskeyCheck;

#[async_trait::async_trait]
impl OwnerCheck for PasskeyCheck {
    async fn confirm(&self, _reason: &str) -> Result<(), EngineError> {
        if SIGNED_IN.try_with(|signed_in| *signed_in).unwrap_or(false) {
            Ok(())
        } else {
            Err(EngineError::Conflict(
                "Sign in to Hoista with your passkey, then try again. Nothing changed.",
            ))
        }
    }
}

/// Runs `work` as the owner who just signed in with a passkey.
pub async fn as_signed_in_owner<F: std::future::Future>(work: F) -> F::Output {
    SIGNED_IN.scope(true, work).await
}
