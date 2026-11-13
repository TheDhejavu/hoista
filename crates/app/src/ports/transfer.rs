//! Sealing a transfer with a passphrase, and the encrypted file it's kept in.
//! How it's encrypted and written is the implementation's business.

use crate::domain::Secret;
use crate::ports::catalog::EngineError;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// A sealed transfer saved to a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedTransfer {
    /// Where it was saved.
    pub path: PathBuf,
    /// The encrypted file's size.
    pub bytes: u64,
    /// SHA-256 of the encrypted file.
    pub sha256: String,
    /// False when the file was written but syncing it to disk failed, so it
    /// may not survive a crash.
    pub durable: bool,
}

/// Seals and opens transfers.
#[async_trait::async_trait]
pub trait TransferSeal: Send + Sync {
    /// The largest sealed transfer accepted, in bytes.
    fn max_sealed(&self) -> usize;

    /// Seals `plaintext` with `passphrase` and saves it at `path`, which must not
    /// exist yet.
    ///
    /// # Errors
    /// Reports a bad passphrase, a payload too large, or a file that can't be written.
    async fn seal_to_file(
        &self,
        path: &Path,
        passphrase: Secret,
        plaintext: Zeroizing<Vec<u8>>,
    ) -> Result<SavedTransfer, EngineError>;

    /// Reads the sealed transfer at `path` and opens it.
    ///
    /// # Errors
    /// Reports a missing or unreadable file, a wrong passphrase or changed data.
    async fn open_file(
        &self,
        path: &Path,
        passphrase: Secret,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError>;

    /// Opens a sealed transfer already in memory.
    ///
    /// # Errors
    /// Reports a wrong passphrase or changed data.
    async fn open(
        &self,
        sealed: Vec<u8>,
        passphrase: Secret,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError>;
}
