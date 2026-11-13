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
    home().join(state_folder_name())
}

/// The state folder's name: `.hoista` for a release, `.hoista-dev` for a
/// development build, so the two never share files. This follows the same
/// rule as `hoista_system::channel`, which the app can't depend on;
/// `HOISTA_CHANNEL=dev` or `stable` overrides it in both.
fn state_folder_name() -> &'static str {
    match std::env::var("HOISTA_CHANNEL").ok().as_deref() {
        Some("dev") => ".hoista-dev",
        Some("stable") => ".hoista",
        _ if cfg!(debug_assertions) => ".hoista-dev",
        _ => ".hoista",
    }
}

/// Where `hoistd` listens.
pub fn engine_socket() -> PathBuf {
    if let Some(path) = std::env::var_os("HOISTA_SOCKET") {
        return PathBuf::from(path);
    }
    if cfg!(windows) {
        return PathBuf::from(r"\\.\pipe\hoistd");
    }
    state_folder().join("hoistd.sock")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}
