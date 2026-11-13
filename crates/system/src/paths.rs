//! Where Hoista keeps its files (`~/.hoista`, or `~/.hoista-dev` for a development build, unless told otherwise), and the
//! lock that lets only one engine at a time use them.

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

/// The state folder and the paths inside it. Provider tokens are never here.
#[derive(Clone, Debug)]
pub struct StatePaths {
    root: PathBuf,
}

impl StatePaths {
    /// The given folder, or this channel's folder in the home folder: `~/.hoista`
    /// for a release, `~/.hoista-dev` for a development build.
    ///
    /// # Errors
    /// Fails when no folder is given and the home folder can't be found.
    pub fn new(root: Option<PathBuf>) -> io::Result<Self> {
        let root = match root {
            Some(root) => root,
            None => dirs::home_dir()
                .ok_or_else(|| {
                    io::Error::other("Cannot find your home directory; pass --state-dir.")
                })?
                .join(crate::channel::current().state_folder_name()),
        };
        Ok(Self { root })
    }

    /// The state folder. Only this user can open it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The SQLite database.
    pub fn database(&self) -> PathBuf {
        self.root.join("hoista.db")
    }

    /// The socket the daemon listens on.
    pub fn socket(&self) -> PathBuf {
        self.root.join("hoistd.sock")
    }

    /// The folder of daily logs.
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Creates the state and log folders, readable only by this user.
    ///
    /// # Errors
    /// Fails when either is a symbolic link or can't be created.
    pub fn prepare(&self) -> io::Result<()> {
        private_directory(&self.root)?;
        private_directory(&self.logs())
    }

    /// Takes the lock that makes this the only engine using the folder: the
    /// daemon, or the CLI working without one. Keep the file open for as long
    /// as the state is in use.
    ///
    /// # Errors
    /// `WouldBlock` when another engine holds it, or a file system error.
    pub fn lock(&self) -> io::Result<File> {
        self.prepare()?;
        let path = self.root.join("hoistd.lock");
        reject_symlink(&path)?;
        let file = private_file(&path)?;
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::WouldBlock,
                "Hoista is already running for this state directory.",
            ),
            std::fs::TryLockError::Error(error) => error,
        })?;
        Ok(file)
    }
}

/// Creates `path` and its parents so only the current user can list or change it.
///
/// # Errors
/// Returns an error when `path` is a symbolic link or the folder can't be created.
pub fn private_directory(path: &Path) -> io::Result<()> {
    reject_symlink(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        // Nobody else may list or change it. Passing through stays allowed
        // where Hoista set it, so its credential keeper can reach its folder.
        let mode = fs::metadata(path)?.permissions().mode() & 0o011;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700 | mode))?;
    }

    #[cfg(not(unix))]
    fs::create_dir_all(path)?;
    Ok(())
}

/// Refuses a path that is a symbolic link; a missing path is fine.
///
/// # Errors
/// Returns an error when `path` is a symbolic link or can't be inspected.
pub fn reject_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::other(
            "Hoista state paths must not be symbolic links.",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Opens or creates a regular file that only the current user can read.
///
/// # Errors
/// Returns an error when `path` exists but isn't a regular file, or can't be opened.
pub fn private_file(path: &Path) -> io::Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(io::Error::other("Expected a regular Hoista state file."));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}
