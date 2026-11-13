//! Reading and saving encrypted transfer files, with a size limit. Saving never
//! overwrites an existing file. These calls block, so run them off the async
//! runtime.

use super::transfer_crypto::{EncryptedTransfer, MAX_ENVELOPE};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::fs::File;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Windows: open a link itself instead of what it points to.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// Windows: the file is a link (a reparse point).
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// What was saved: where, how big, and its hash. Nothing from inside the
/// encrypted transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferFileReceipt {
    /// Where it was saved.
    pub path: PathBuf,
    /// The file's size in bytes.
    pub bytes: u64,
    /// The SHA-256 of the file.
    pub sha256: String,
}

/// Why a transfer file couldn't be read or saved. The messages never include
/// what's in the file.
#[derive(Debug, thiserror::Error)]
pub enum TransferFileError {
    /// Not a full path, or not an ordinary file.
    #[error("Choose an absolute path to a regular Hoista transfer file.")]
    InvalidPath,
    /// Bigger than any transfer Hoista makes.
    #[error("The encrypted transfer exceeds the supported size limit.")]
    TooLarge,
    /// Something is already there. It's left alone, links included.
    #[error("The destination already exists. Choose a new transfer filename.")]
    Exists,
    /// The file or its folder couldn't be used. Nothing was saved.
    #[error("Cannot access the transfer file. Check its directory and available disk space.")]
    Io,
    /// The file was saved, but the folder couldn't be synced, so it may not
    /// survive a crash.
    #[error(
        "The transfer was published, but durable storage could not be confirmed. Inspect the saved file before retrying."
    )]
    DurabilityUncertain(TransferFileReceipt),
}

/// Saves the encrypted transfer at `path`, which must not exist yet. It's
/// written to a temporary file beside it first, so the file appears whole or
/// not at all. On Unix only this account can read it; on Windows it takes the
/// folder's permissions, so the folder must be the person's own.
///
/// # Errors
/// Fails for a relative path, a parent that is a link or not a folder, an
/// existing file, or a write error. When only the final sync fails, the
/// receipt comes back in [`TransferFileError::DurabilityUncertain`], because
/// the file was written.
pub fn save(
    path: &Path,
    encrypted: &EncryptedTransfer,
) -> Result<TransferFileReceipt, TransferFileError> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(TransferFileError::InvalidPath);
    }
    let parent = path.parent().ok_or(TransferFileError::InvalidPath)?;
    let metadata = std::fs::symlink_metadata(parent).map_err(|_| TransferFileError::Io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(TransferFileError::InvalidPath);
    }
    match std::fs::symlink_metadata(path) {
        Ok(_) => return Err(TransferFileError::Exists),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(TransferFileError::Io),
    }
    let bytes = encrypted.as_bytes();
    let receipt = TransferFileReceipt {
        path: path.to_owned(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    };
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| TransferFileError::Io)?;
    temporary
        .write_all(bytes)
        .map_err(|_| TransferFileError::Io)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| TransferFileError::Io)?;
    temporary.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            TransferFileError::Exists
        } else {
            TransferFileError::Io
        }
    })?;
    #[cfg(unix)]
    if File::open(parent)
        .and_then(|directory| directory.sync_all())
        .is_err()
    {
        return Err(TransferFileError::DurabilityUncertain(receipt));
    }
    Ok(receipt)
}

/// Reads a transfer file. The bytes mean nothing until `TransferCipher` has
/// opened them; the file's name or extension proves nothing.
///
/// # Errors
/// Fails for a relative path, a link, anything but an ordinary file, a file
/// over the size limit, or a read error.
pub fn read(path: &Path) -> Result<Vec<u8>, TransferFileError> {
    if !path.is_absolute() {
        return Err(TransferFileError::InvalidPath);
    }
    let before = std::fs::symlink_metadata(path).map_err(|_| TransferFileError::Io)?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(TransferFileError::InvalidPath);
    }
    if before.len() > MAX_ENVELOPE as u64 {
        return Err(TransferFileError::TooLarge);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Non-blocking, so a pipe swapped in after the check above can't hang
        // the open before the check below catches it.
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|_| TransferFileError::Io)?;
    let metadata = file.metadata().map_err(|_| TransferFileError::Io)?;
    if !metadata.is_file() {
        return Err(TransferFileError::InvalidPath);
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(TransferFileError::InvalidPath);
        }
    }
    if metadata.len() > MAX_ENVELOPE as u64 {
        return Err(TransferFileError::TooLarge);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(metadata.len() as usize)
        .map_err(|_| TransferFileError::Io)?;
    file.take(MAX_ENVELOPE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| TransferFileError::Io)?;
    if bytes.len() > MAX_ENVELOPE {
        return Err(TransferFileError::TooLarge);
    }
    Ok(bytes)
}
