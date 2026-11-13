//! Isolating a project's build code (install and build scripts, Prisma) from
//! the rest of the computer: `bwrap` on Linux, `sandbox-exec` on macOS.
//!
//! Without either there is no isolation. This module never falls back to
//! running unisolated by itself; the engine decides whether to allow it.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
/// Where Linux distributions install bubblewrap.
const BWRAP_PATHS: [&str; 3] = ["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"];

/// How builds are kept apart on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// Bubblewrap at this path.
    Bubblewrap(PathBuf),
    /// The macOS sandbox.
    MacSandbox,
    /// None: why, and what to do about it.
    Unavailable(&'static str),
}

impl Isolation {
    /// Plain name for the person.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Bubblewrap(_) => "bubblewrap",
            Self::MacSandbox => "the macOS sandbox",
            Self::Unavailable(_) => "none",
        }
    }
}

/// What this computer offers, checked once.
pub fn isolation() -> &'static Isolation {
    static FOUND: OnceLock<Isolation> = OnceLock::new();
    FOUND.get_or_init(detect)
}

fn detect() -> Isolation {
    if cfg!(target_os = "macos") {
        return if Path::new(SANDBOX_EXEC).is_file() {
            Isolation::MacSandbox
        } else {
            Isolation::Unavailable("This Mac has no sandbox-exec, so builds can't be isolated.")
        };
    }
    if cfg!(target_os = "linux") {
        let Some(bwrap) = BWRAP_PATHS
            .iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
        else {
            return Isolation::Unavailable(
                "Install bubblewrap (the bwrap command) so Hoista can build projects in isolation.",
            );
        };
        // Some systems turn off the user namespaces bubblewrap needs.
        let works = std::process::Command::new(&bwrap)
            .args([
                "--ro-bind",
                "/",
                "/",
                "--unshare-user",
                "--unshare-pid",
                "--die-with-parent",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "true",
            ])
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        return if works {
            Isolation::Bubblewrap(bwrap)
        } else {
            Isolation::Unavailable(
                "bubblewrap can't start here (user namespaces may be turned off), so builds can't be isolated.",
            )
        };
    }
    Isolation::Unavailable("Hoista can't build projects in isolation on this system yet.")
}

/// What an isolated build may see.
pub struct Fence<'a> {
    /// The build's own copy of the source; read and write.
    pub workspace: &'a Path,
    /// Package caches, kept between builds; read and write.
    pub cache: &'a Path,
    /// Hoista's state folder: never visible.
    pub state: &'a Path,
    /// The home folder: hidden, apart from the tools below.
    pub home: Option<&'a Path>,
    /// Folders with the tools the build runs (from `PATH`); read only.
    pub tools: Vec<PathBuf>,
}

impl Fence<'_> {
    /// Where `PATH`'s tools live, for the ones inside the home folder: a
    /// `bin` folder brings its install folder (`~/.nvm/versions/node/v22`).
    #[must_use]
    pub fn tool_folders(path: Option<&OsString>, home: Option<&Path>) -> Vec<PathBuf> {
        let Some(path) = path else {
            return Vec::new();
        };
        let mut folders: Vec<PathBuf> = std::env::split_paths(path)
            .filter(|entry| entry.is_absolute())
            .filter(|entry| home.is_some_and(|home| entry.starts_with(home)))
            .map(|entry| match entry.file_name() {
                Some(name) if name == "bin" => {
                    entry.parent().map_or(entry.clone(), Path::to_path_buf)
                }
                _ => entry,
            })
            .collect();
        folders.sort();
        folders.dedup();
        folders
    }
}

/// `program args`, run inside `fence` with `how`. The caller sets the
/// environment, the working folder and output handling on what's returned.
#[must_use]
pub fn fenced(
    how: &Isolation,
    program: &std::ffi::OsStr,
    args: &[&str],
    fence: &Fence<'_>,
) -> Option<tokio::process::Command> {
    match how {
        Isolation::Bubblewrap(bwrap) => {
            let mut command = tokio::process::Command::new(bwrap);
            command.args(bubblewrap_args(fence));
            command.arg("--").arg(program).args(args);
            Some(command)
        }
        Isolation::MacSandbox => {
            let mut command = tokio::process::Command::new(SANDBOX_EXEC);
            let (profile, params) = mac_profile(fence);
            command.arg("-p").arg(profile);
            for (name, value) in params {
                command.arg("-D").arg(format!("{name}={}", value.display()));
            }
            command.arg(program).args(args);
            Some(command)
        }
        Isolation::Unavailable(_) => None,
    }
}

fn bubblewrap_args(fence: &Fence<'_>) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |items: &[&std::ffi::OsStr]| {
        args.extend(items.iter().map(|item| (*item).to_owned()));
    };
    push(&["--ro-bind".as_ref(), "/".as_ref(), "/".as_ref()]);
    push(&["--dev".as_ref(), "/dev".as_ref()]);
    push(&["--proc".as_ref(), "/proc".as_ref()]);
    push(&["--tmpfs".as_ref(), "/tmp".as_ref()]);
    // No session bus (the keyring lives behind it), no other user's sockets.
    push(&["--tmpfs".as_ref(), "/run".as_ref()]);
    // Name lookups on systems where resolv.conf points into /run.
    push(&[
        "--ro-bind-try".as_ref(),
        "/run/systemd/resolve".as_ref(),
        "/run/systemd/resolve".as_ref(),
    ]);
    if let Some(home) = fence.home {
        push(&["--tmpfs".as_ref(), home.as_os_str()]);
    }
    push(&["--tmpfs".as_ref(), fence.state.as_os_str()]);
    for tool in &fence.tools {
        push(&["--ro-bind-try".as_ref(), tool.as_os_str(), tool.as_os_str()]);
    }
    push(&[
        "--bind".as_ref(),
        fence.cache.as_os_str(),
        fence.cache.as_os_str(),
    ]);
    push(&[
        "--bind".as_ref(),
        fence.workspace.as_os_str(),
        fence.workspace.as_os_str(),
    ]);
    push(&["--chdir".as_ref(), fence.workspace.as_os_str()]);
    for flag in [
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--die-with-parent",
        "--new-session",
    ] {
        push(&[flag.as_ref()]);
    }
    args
}

/// The macOS sandbox profile, with every path passed as a parameter so no
/// path can change the profile itself. Later rules win.
fn mac_profile(fence: &Fence<'_>) -> (String, Vec<(String, PathBuf)>) {
    let mut params = vec![
        ("WORKSPACE".to_owned(), fence.workspace.to_path_buf()),
        ("CACHE".to_owned(), fence.cache.to_path_buf()),
        ("STATE".to_owned(), fence.state.to_path_buf()),
    ];
    let mut profile = String::from(
        "(version 1)\n(allow default)\n\
         ; The keychain's services: no reading saved passwords or tokens.\n\
         (deny mach-lookup (global-name \"com.apple.SecurityServer\") (global-name \"com.apple.securityd\") (global-name \"com.apple.secd\") (global-name \"com.apple.security.agent\") (global-name \"com.apple.security.authhost\"))\n",
    );
    if let Some(home) = fence.home {
        params.push(("HOME".to_owned(), home.to_path_buf()));
        profile.push_str("(deny file-read* file-write* (subpath (param \"HOME\")))\n");
    }
    profile.push_str("(deny file-read* file-write* (subpath (param \"STATE\")))\n");
    for (index, tool) in fence.tools.iter().enumerate() {
        let name = format!("TOOL{index}");
        profile.push_str(&format!(
            "(allow file-read* (subpath (param \"{name}\")))\n"
        ));
        params.push((name, tool.clone()));
    }
    profile.push_str("(allow file-read* file-write* (subpath (param \"CACHE\")) (subpath (param \"WORKSPACE\")))\n");
    // Tools resolve paths by looking up each parent folder: details only
    // (that a folder exists, its permissions), never what's in a file.
    profile.push_str("(allow file-read-metadata)\n");
    (profile, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fence<'a>(root: &'a Path, home: &'a Path) -> Fence<'a> {
        Fence {
            workspace: root,
            cache: root,
            state: root,
            home: Some(home),
            tools: vec![home.join(".nvm/versions/node/v22")],
        }
    }

    // The paths in this test are Unix paths.
    #[cfg(unix)]
    #[test]
    fn tools_inside_home_bring_their_install_folder() {
        let home = Path::new("/home/sam");
        let path: OsString = "/usr/bin:/home/sam/.nvm/versions/node/v22/bin:/home/sam/.bun/bin:relative:/home/sam/bin".into();
        assert_eq!(
            Fence::tool_folders(Some(&path), Some(home)),
            vec![
                PathBuf::from("/home/sam"),
                PathBuf::from("/home/sam/.bun"),
                PathBuf::from("/home/sam/.nvm/versions/node/v22"),
            ]
        );
        assert!(Fence::tool_folders(None, Some(home)).is_empty());
    }

    #[test]
    fn the_mac_profile_takes_paths_only_as_parameters() {
        let odd = Path::new("/tmp/a\"b) (allow default");
        let (profile, params) = mac_profile(&fence(odd, Path::new("/Users/sam")));
        assert!(
            !profile.contains("a\\\"b"),
            "no path is written into the profile"
        );
        assert!(!profile.contains("/Users/sam"));
        assert!(profile.contains("com.apple.SecurityServer"));
        assert!(
            params
                .iter()
                .any(|(name, value)| name == "WORKSPACE" && value == odd)
        );
        // Allowing the workspace comes after denying home and state, so it wins.
        let deny = profile.find("(param \"STATE\")").expect("state");
        let allow = profile
            .find("(subpath (param \"WORKSPACE\"))")
            .expect("workspace");
        assert!(allow > deny);
    }

    #[test]
    fn bubblewrap_hides_home_state_and_the_session_bus() {
        let args: Vec<String> =
            bubblewrap_args(&fence(Path::new("/data/w"), Path::new("/home/sam")))
                .into_iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
        let joined = args.join(" ");
        assert!(joined.contains("--tmpfs /home/sam"));
        assert!(joined.contains("--tmpfs /run"));
        assert!(joined.contains("--tmpfs /data/w"));
        assert!(joined.contains("--bind /data/w /data/w"));
        assert!(joined.contains("--unshare-user"));
        // The workspace is mounted after state and home are hidden.
        let hidden = joined.find("--tmpfs /data/w").expect("hidden");
        let shown = joined.find("--bind /data/w /data/w").expect("shown");
        assert!(shown > hidden);
    }

    /// Runs real code in the fence: it builds in its workspace, but can't
    /// see the home folder, Hoista's state or anything it hides.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn build_code_sees_only_its_workspace() {
        let how = isolation();
        if matches!(how, Isolation::Unavailable(_)) {
            eprintln!("bubblewrap isn't available here; skipping");
            return;
        }
        let temp = tempfile::tempdir().expect("temp");
        let home = temp.path().join("home");
        let state = home.join(".hoista");
        let workspace = state.join("builds/b1");
        let cache = state.join("build-cache");
        for folder in [&workspace, &cache] {
            std::fs::create_dir_all(folder).expect("folder");
        }
        std::fs::write(home.join("secret.txt"), "home-secret").expect("secret");
        std::fs::write(state.join("vault.sealed"), "state-secret").expect("state");
        std::fs::create_dir_all(state.join("builds/other")).expect("other");
        std::fs::write(state.join("builds/other/env"), "other-build").expect("other");
        let fence = Fence {
            workspace: &workspace,
            cache: &cache,
            state: &state,
            home: Some(&home),
            tools: Vec::new(),
        };
        let script = format!(
            "cat {home}/secret.txt {state}/vault.sealed {state}/builds/other/env 2>/dev/null; \
             echo built > {workspace}/out; echo cached > {cache}/c; \
             touch {home}/planted 2>/dev/null; ls /run/user 2>/dev/null; true",
            home = home.display(),
            state = state.display(),
            workspace = workspace.display(),
            cache = cache.display(),
        );
        let mut command = fenced(how, "sh".as_ref(), &["-c", &script], &fence).expect("fenced");
        let output = command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .await
            .expect("ran");
        let seen = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!seen.contains("home-secret"), "{seen}");
        assert!(!seen.contains("state-secret"), "{seen}");
        assert!(!seen.contains("other-build"), "{seen}");
        assert_eq!(
            std::fs::read_to_string(workspace.join("out")).expect("out"),
            "built\n"
        );
        assert_eq!(
            std::fs::read_to_string(cache.join("c")).expect("cache"),
            "cached\n"
        );
        assert!(
            !home.join("planted").exists(),
            "nothing lands in the real home folder"
        );
        assert_eq!(
            std::fs::read_to_string(home.join("secret.txt")).expect("kept"),
            "home-secret"
        );
    }
}
