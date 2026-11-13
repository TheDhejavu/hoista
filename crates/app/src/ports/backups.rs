//! Where backups of Hoista's records are kept. On a computer or a server, that's
//! a folder beside Hoista's state.

use crate::ports::catalog::EngineError;

/// One saved backup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema, ts_rs::TS)]
pub struct BackupFile {
    /// Its name, such as `backup-20261104T150211Z.json`.
    pub name: String,
    /// RFC 3339 UTC: when it was saved.
    pub at: String,
    /// Its size.
    pub bytes: u64,
}

/// Keeps backup files. Calls may block on the file system; callers run them
/// off the async runtime.
pub trait BackupFolder: Send + Sync {
    /// Saved backups, newest first.
    ///
    /// # Errors
    /// Reports a folder that can't be read.
    fn list(&self) -> Result<Vec<BackupFile>, EngineError>;

    /// Saves `bytes` as a new backup named for this moment, whole or not at all,
    /// readable only by this account.
    ///
    /// # Errors
    /// Reports a folder that can't be written.
    fn save(&self, bytes: &[u8]) -> Result<BackupFile, EngineError>;

    /// Reads a saved backup.
    ///
    /// # Errors
    /// Rejects a name that isn't a backup's, and reports a missing file.
    fn read(&self, name: &str) -> Result<Vec<u8>, EngineError>;

    /// Removes a saved backup; a missing one is fine.
    fn remove(&self, name: &str);
}
