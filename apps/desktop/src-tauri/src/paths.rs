use std::path::PathBuf;

/// Hoista's state folder, shared with `hoist` and `hoistd`. A test engine
/// started with `HOISTA_SOCKET=/some/dir/hoistd.sock` keeps its state beside
/// its socket, so the app follows the socket there.
pub fn state_folder() -> PathBuf {
    if let Some(socket) = std::env::var_os("HOISTA_SOCKET").map(PathBuf::from)
        && !cfg!(windows)
        && let Some(folder) = socket.parent()
    {
        return folder.to_path_buf();
    }
    home().join(".hoista")
}

/// Where `hoistd` listens.
pub fn engine_socket() -> PathBuf {
    if let Some(path) = std::env::var_os("HOISTA_SOCKET") {
        return PathBuf::from(path);
    }
    if cfg!(windows) {
        return PathBuf::from(r"\\.\pipe\hoistd");
    }
    home().join(".hoista").join("hoistd.sock")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}
