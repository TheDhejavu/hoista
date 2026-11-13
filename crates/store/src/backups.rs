//! The folder Hoista keeps backups of its records in (never credentials),
//! beside its state. Which backups to keep, and when, is decided in `hoista-app`.

use crate::{
    application::ports::{
        backups::{BackupFile, BackupFolder},
        catalog::EngineError,
    },
    paths::StatePaths,
};
use std::path::{Path, PathBuf};

const PREFIX: &str = "backup-";
const SUFFIX: &str = ".json";

/// Backups as files in the state folder's `backups`.
pub struct LocalBackups {
    folder: PathBuf,
}

impl LocalBackups {
    /// The `backups` folder beside Hoista's state.
    #[must_use]
    pub fn new(paths: &StatePaths) -> Self {
        Self {
            folder: paths.root().join("backups"),
        }
    }
}

impl BackupFolder for LocalBackups {
    fn list(&self) -> Result<Vec<BackupFile>, EngineError> {
        let entries = match std::fs::read_dir(&self.folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(EngineError::Store),
        };
        let mut files: Vec<BackupFile> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let at = saved_at(&name)?;
                let metadata = entry.metadata().ok().filter(std::fs::Metadata::is_file)?;
                Some(BackupFile {
                    name,
                    at,
                    bytes: metadata.len(),
                })
            })
            .collect();
        files.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(files)
    }

    fn save(&self, bytes: &[u8]) -> Result<BackupFile, EngineError> {
        let at = chrono::Utc::now();
        let name = format!("{PREFIX}{}{SUFFIX}", at.format("%Y%m%dT%H%M%SZ"));
        let size = write_whole(&self.folder, &name, bytes)?;
        Ok(BackupFile {
            name,
            at: at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            bytes: size,
        })
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, EngineError> {
        if saved_at(name).is_none() {
            return Err(EngineError::Invalid(
                "Use a backup's name from hoist backups.",
            ));
        }
        std::fs::read(self.folder.join(name)).map_err(|_| EngineError::NotFound)
    }

    fn remove(&self, name: &str) {
        if saved_at(name).is_some() {
            let _ = std::fs::remove_file(self.folder.join(name));
        }
    }
}

/// When a backup was saved, from its name; `None` for any other file.
fn saved_at(name: &str) -> Option<String> {
    let stamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ")
        .ok()
        .map(|at| {
            at.and_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
}

/// Writes `bytes` as `folder/name` whole or not at all, readable only by this account.
fn write_whole(folder: &Path, name: &str, bytes: &[u8]) -> Result<u64, EngineError> {
    use std::io::Write as _;
    std::fs::create_dir_all(folder).map_err(|_| EngineError::Store)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700));
    }
    let partial = folder.join(format!(".{name}.partial"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&partial).map_err(|_| EngineError::Store)?;
    file.write_all(bytes).map_err(|_| EngineError::Store)?;
    file.sync_all().map_err(|_| EngineError::Store)?;
    std::fs::rename(&partial, folder.join(name)).map_err(|_| EngineError::Store)?;
    Ok(bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_backup_names_are_backups() {
        assert_eq!(
            saved_at("backup-20261104T150211Z.json").as_deref(),
            Some("2026-11-04T15:02:11Z")
        );
        for other in [
            "backup-../../etc.json",
            "backup-2026.json",
            "notes.json",
            ".backup-20261104T150211Z.json.partial",
            "backup-20261104T150211Z.json/..",
        ] {
            assert!(saved_at(other).is_none(), "{other}");
        }
    }
}
