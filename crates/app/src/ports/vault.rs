//! Where account tokens and keys are stored: the keychain, 1Password, a sealed file,
//! or the credential keeper on a server.

use crate::{domain::Secret, ports::catalog::EngineError};

/// Where account tokens and keys are stored. A vault never quietly writes a
/// token to an ordinary file instead.
#[async_trait::async_trait]
pub trait TokenVault: Send + Sync {
    /// Reads one entry, when a provider call needs it.
    ///
    /// # Errors
    /// [`EngineError::NotFound`] when there's no such entry; otherwise the
    /// vault couldn't be read.
    async fn read(&self, reference: &str) -> Result<Secret, EngineError>;

    /// Sets one entry.
    ///
    /// # Errors
    /// Fails when the vault can't be written.
    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError>;

    /// Removes one entry. One that's already gone is fine.
    ///
    /// # Errors
    /// Fails when the vault can't be written.
    async fn delete(&self, reference: &str) -> Result<(), EngineError>;

    /// The usable value for a token as the person gave it: a password-manager
    /// reference (`op://…`) is read from the manager; anything else is itself.
    ///
    /// # Errors
    /// Reports a reference the manager can't read.
    async fn resolve(&self, value: &Secret) -> Result<Secret, EngineError> {
        Ok(Secret::new(value.expose().to_owned()))
    }

    /// Checks the vault can be used, once at start: its token store can be
    /// reached and will take writes.
    ///
    /// # Errors
    /// Reports a store that can't be used.
    async fn check(&self) -> Result<(), EngineError> {
        Ok(())
    }
}
