//! The folders Hoista looks for programs in (node, git, flyctl and so on).
//!
//! A daemon started at login gets a much shorter PATH than a terminal, so
//! `hoist tools configure` saves the PATH folders of the terminal it runs in,
//! and the daemon loads them once at start. Only folders are saved, never any
//! other variable.

use crate::paths::StatePaths;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};

static ACTIVE: OnceLock<Option<ToolPaths>> = OnceLock::new();

const FILE: &str = "tool-paths.json";
const FORMAT_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_DIRECTORIES: usize = 128;
const MAX_DIRECTORY_CHARS: usize = 4096;

/// The saved folders, from `tool-paths.json` in the state folder. They belong
/// to this computer, so backups leave them out.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPaths {
    /// Always [`FORMAT_VERSION`].
    pub version: u32,
    /// Absolute folders, searched in this order.
    pub directories: Vec<PathBuf>,
}

impl ToolPaths {
    fn validate(&self) -> io::Result<()> {
        if self.version != FORMAT_VERSION
            || self.directories.is_empty()
            || self.directories.len() > MAX_DIRECTORIES
            || self.directories.iter().any(|path| {
                !path.is_absolute()
                    || path.to_str().is_none_or(|text| {
                        text.len() > MAX_DIRECTORY_CHARS || text.chars().any(char::is_control)
                    })
            })
            || std::env::join_paths(&self.directories).is_err()
        {
            return Err(io::Error::other(
                "Invalid tool directories. Run hoist tools configure from a terminal with your deployment tools installed.",
            ));
        }
        Ok(())
    }

    fn capture(value: &OsStr) -> io::Result<Self> {
        if value.len() as u64 > MAX_FILE_BYTES {
            return Err(io::Error::other("PATH is longer than 64 KiB."));
        }
        let mut seen = HashSet::new();
        let directories = std::env::split_paths(value)
            .filter(|path| path.is_absolute())
            .map(|path| stable_directory(&path))
            .filter(|path| seen.insert(path.clone()))
            .collect();
        let result = Self {
            version: FORMAT_VERSION,
            directories,
        };
        result.validate()?;
        Ok(result)
    }

    fn environment(&self) -> OsString {
        // validate() already joined these, so no folder holds a PATH separator.
        std::env::join_paths(&self.directories).expect("validated executable search paths")
    }

    /// Where the program `name` is in these folders, without running it. A link
    /// to it (as version managers make) counts.
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        if name.is_empty()
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
            || Path::new(name).file_name() != Some(OsStr::new(name))
        {
            return None;
        }
        self.directories
            .iter()
            .flat_map(|directory| {
                let base = directory.join(name);
                #[cfg(windows)]
                let candidates = vec![
                    base.with_extension("exe"),
                    base.with_extension("cmd"),
                    base.with_extension("bat"),
                    base.with_extension("com"),
                    base,
                ];
                #[cfg(not(windows))]
                let candidates = vec![base];
                candidates
            })
            .find(|candidate| {
                let Ok(metadata) = candidate.metadata() else {
                    return false;
                };
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                }

                #[cfg(not(unix))]
                {
                    metadata.is_file()
                }
            })
    }

    /// Where each program Hoista may run is, without running any of them.
    pub fn executables(&self) -> std::collections::BTreeMap<&'static str, Option<PathBuf>> {
        [
            "node",
            "npm",
            "pnpm",
            "yarn",
            "bun",
            "git",
            "flyctl",
            "pg_dump",
            "pg_restore",
        ]
        .into_iter()
        .map(|name| (name, self.find(name)))
        .collect()
    }
}

fn stable_directory(path: &Path) -> PathBuf {
    // A version manager's per-terminal folder disappears when the terminal
    // closes, so save where an existing folder really is. A folder that doesn't
    // exist yet is kept as given, so a tool installed there later is found.
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    #[cfg(windows)]
    if let Some(value) = resolved.to_str() {
        if let Some(value) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{value}"));
        }
        if let Some(value) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(value);
        }
    }
    resolved
}

/// Reads the saved folders. `None` when there are none, and Hoista uses its
/// own PATH.
///
/// # Errors
/// Fails when the file is a link, too large, unreadable or not valid.
pub fn read(paths: &StatePaths) -> io::Result<Option<ToolPaths>> {
    let file = paths.root().join(FILE);
    match std::fs::symlink_metadata(&file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES => {
            return Err(io::Error::other(
                "Tool configuration must be a regular file under 64 KiB.",
            ));
        }
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    std::fs::File::open(file)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other(
            "The tool folders file is larger than 64 KiB.",
        ));
    }
    let value: ToolPaths = serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::other("Tool configuration is invalid. Run hoist tools configure to replace it.")
    })?;
    value.validate()?;
    Ok(Some(value))
}

/// Saves the folders on this process's PATH, unless some are saved already
/// and `replace` is false. The file is replaced whole and only this user can
/// read it.
///
/// # Errors
/// Fails when PATH is missing or not valid, or the file can't be written.
pub fn configure(paths: &StatePaths, replace: bool) -> io::Result<ToolPaths> {
    if !replace && let Some(value) = read(paths)? {
        return Ok(value);
    }
    let value = ToolPaths::capture(
        &std::env::var_os("PATH").ok_or_else(|| io::Error::other("PATH is unavailable."))?,
    )?;
    paths.prepare()?;
    let target = paths.root().join(FILE);
    if let Ok(metadata) = std::fs::symlink_metadata(&target)
        && !metadata.is_file()
    {
        return Err(io::Error::other(
            "Refusing to replace a non-regular tool configuration.",
        ));
    }
    let bytes = serde_json::to_vec_pretty(&value)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other(
            "The tool folders file is larger than 64 KiB.",
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(paths.root())?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    if replace {
        temporary.persist(&target).map_err(|error| error.error)?;
    } else {
        match temporary.persist_noclobber(&target) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                return read(paths)?
                    .ok_or_else(|| io::Error::other("Tool configuration changed during setup."));
            }
            Err(error) => return Err(error.error),
        }
    }

    #[cfg(unix)]
    std::fs::File::open(paths.root())?.sync_all()?;
    Ok(value)
}

/// Loads the saved folders, once, before the daemon starts any work. The
/// process's own environment isn't changed.
///
/// # Errors
/// Fails when the saved file isn't valid, or when called twice.
pub fn initialize(paths: &StatePaths) -> io::Result<()> {
    ACTIVE
        .set(read(paths)?)
        .map_err(|_| io::Error::other("Tool lookup is already initialized."))
}

/// Where the program `name` is on this computer, using the saved tool folders when set.
#[must_use]
pub fn executable(name: &str) -> Option<PathBuf> {
    if let Some(active) = ACTIVE.get().and_then(Option::as_ref) {
        active.find(name)
    } else {
        ToolPaths::capture(&std::env::var_os("PATH")?)
            .ok()?
            .find(name)
    }
}

/// A command for `program`, resolved through the saved tool folders when set.
#[must_use]
pub fn command(program: &str) -> tokio::process::Command {
    if let Some(active) = ACTIVE.get().and_then(Option::as_ref) {
        // A tool missing from the saved folders isn't looked for on the daemon's
        // own PATH: the person chose these folders.
        let program = active
            .find(program)
            .unwrap_or_else(|| active.directories[0].join(program));
        tokio::process::Command::new(program)
    } else {
        tokio::process::Command::new(program)
    }
}

/// The saved tool folders as a PATH value, when `hoist tools configure` saved them.
pub fn search_path() -> Option<OsString> {
    ACTIVE
        .get()
        .and_then(Option::as_ref)
        .map(ToolPaths::environment)
}

/// Gives `command` the saved tool folders as its PATH, when they are set.
pub fn apply(command: &mut tokio::process::Command) {
    if let Some(active) = ACTIVE.get().and_then(Option::as_ref) {
        command.env("PATH", active.environment());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_excludes_relative_entries_and_configuration_is_atomic_and_private() {
        let temp = tempfile::tempdir().unwrap();
        let joined = std::env::join_paths([
            Path::new(""),
            Path::new("relative"),
            temp.path(),
            temp.path(),
        ])
        .unwrap();
        let paths = ToolPaths::capture(&joined).unwrap();
        assert_eq!(paths.directories, vec![stable_directory(temp.path())]);
        #[cfg(unix)]
        {
            let alias = temp.path().join("session-alias");
            std::os::unix::fs::symlink(temp.path(), &alias).unwrap();
            let captured =
                ToolPaths::capture(&std::env::join_paths([alias.as_path(), temp.path()]).unwrap())
                    .unwrap();
            assert_eq!(captured.directories, vec![stable_directory(temp.path())]);
            std::fs::remove_file(alias).unwrap();
            assert!(captured.directories[0].is_dir());
        }
        let state = StatePaths::new(Some(temp.path().join("state"))).unwrap();
        let initial = configure(&state, false).unwrap();
        assert_eq!(
            configure(&state, false).unwrap().directories,
            initial.directories
        );
        assert_eq!(
            read(&state).unwrap().unwrap().directories,
            initial.directories
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(state.root().join(FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::write(state.root().join(FILE), b"invalid").unwrap();
        assert!(read(&state).is_err());
        assert!(configure(&state, true).is_ok());
        let malformed = ToolPaths {
            version: 2,
            directories: vec![temp.path().into()],
        };
        assert!(malformed.validate().is_err());
        assert!(ToolPaths::capture(OsStr::new("relative")).is_err());
    }

    #[test]
    fn executable_lookup_obeys_directory_order_and_rejects_path_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("tools with spaces");
        std::fs::create_dir(&directory).unwrap();
        let binary = directory.join(if cfg!(windows) {
            "fixture-tool.exe"
        } else {
            "fixture-tool"
        });
        std::fs::write(&binary, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let paths = ToolPaths {
            version: 1,
            directories: vec![temp.path().join("missing"), directory],
        };
        assert_eq!(paths.find("fixture-tool"), Some(binary.clone()));
        assert!(paths.find("missing-tool").is_none());
        assert!(paths.find("../fixture-tool").is_none());
        assert!(paths.find(binary.to_str().unwrap()).is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(paths.find("fixture-tool").is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_configuration_is_never_followed_or_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let state = StatePaths::new(Some(temp.path().join("state"))).unwrap();
        state.prepare().unwrap();
        let outside = temp.path().join("outside");
        std::fs::write(&outside, b"unrelated").unwrap();
        std::os::unix::fs::symlink(&outside, state.root().join(FILE)).unwrap();
        assert!(read(&state).is_err());
        assert!(configure(&state, true).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"unrelated");
    }
}
