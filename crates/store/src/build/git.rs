//! What git says about a project's source: its commit, whether it has
//! changes, and whether any added line carries a secret. Git runs with no
//! credentials, hooks, filters or system and global config, and the diff is
//! only ever held in memory.

use super::{failure, leaked, scanner};
use crate::{
    application::ports::catalog::EngineError, domain::Secret, process::capture, tool_paths,
};
use std::{path::Path, process::Stdio, time::Duration};
use zeroize::Zeroizing;

const QUICK_TIMEOUT: Duration = Duration::from_secs(10);
const SLOW_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FILTER_KEYS_BYTES: u64 = 32 * 1024;
const MAX_FILTER_KEY_BYTES: usize = 256;
const MAX_FILTER_KEYS: usize = 128;
const MAX_DIFF_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STATUS_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REVISION_BYTES: u64 = 128;
const MAX_REMOTE_BYTES: u64 = 4096;
const MAX_REPOSITORY_PART_CHARS: usize = 100;
/// What git exits with when a lookup finds nothing.
const NOTHING_FOUND: i32 = 1;

/// The only variables git gets from Hoista's environment.
const INHERITED: [&str; 8] = [
    "PATH",
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "COMSPEC",
    "PATHEXT",
];

/// The source's commit and whether it has changes. Both `None` outside a
/// git repository; `commit` is also `None` before the first commit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GitContext {
    pub commit: Option<String>,
    pub dirty: Option<bool>,
}

/// Reads the source's git state, refusing it when a staged or unstaged
/// added line carries one of `secrets` or looks like a secret.
pub(super) async fn check(root: &Path, secrets: &[Secret]) -> Result<GitContext, EngineError> {
    inspect(root, Some(secrets)).await
}

/// Reads the source's git state.
pub(crate) async fn observe(root: &Path) -> Result<GitContext, EngineError> {
    inspect(root, None).await
}

/// A full commit id: SHA-1 or SHA-256, in hex.
fn is_commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn inspect(root: &Path, secrets: Option<&[Secret]>) -> Result<GitContext, EngineError> {
    let mut repository = false;
    for ancestor in root.ancestors() {
        match std::fs::symlink_metadata(ancestor.join(".git")) {
            Ok(_) => {
                repository = true;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(failure("Cannot inspect source Git metadata.")),
        }
    }
    if !repository {
        return Ok(GitContext {
            commit: None,
            dirty: None,
        });
    }
    let private = tempfile::tempdir().map_err(|_| EngineError::Store)?;
    let overrides = filter_overrides(root, private.path()).await?;
    if let Some(secrets) = secrets {
        for staged in [true, false] {
            let mut command = command(root, private.path());
            for setting in &overrides {
                command.arg("-c").arg(setting);
            }
            command.args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--no-color",
                "--text",
                "--unified=0",
                "--ignore-submodules=all",
            ]);
            if staged {
                command.arg("--cached");
            }
            command.args(["--", "."]);
            let (status, bytes) = capture(&mut command, MAX_DIFF_BYTES, SLOW_TIMEOUT)
                .await
                .map_err(|_| {
                    failure("Cannot inspect Git changes within the size/time limit. Check Git availability and repository access.")
                })?;
            if !status.success() {
                return Err(failure(
                    "Cannot inspect source Git changes. Repair the repository before deploying.",
                ));
            }
            if !scanner()?
                .scan("git changes", &added_lines(&bytes), secrets)
                .is_empty()
            {
                return Err(leaked());
            }
        }
    }
    let mut revision = command(root, private.path());
    revision.args(["rev-parse", "--verify", "--quiet", "HEAD"]);
    let (status, bytes) = capture(&mut revision, MAX_REVISION_BYTES, QUICK_TIMEOUT)
        .await
        .map_err(|_| failure("Cannot read source revision."))?;
    let commit = if status.success() {
        let value = std::str::from_utf8(&bytes)
            .map_err(|_| failure("Invalid source revision."))?
            .trim();
        if !is_commit_id(value) {
            return Err(failure("Invalid source revision."));
        }
        Some(value.to_ascii_lowercase())
    } else if status.code() == Some(NOTHING_FOUND) {
        None
    } else {
        return Err(failure("Cannot read source revision."));
    };
    let mut state = command(root, private.path());
    for setting in &overrides {
        state.arg("-c").arg(setting);
    }
    state.args([
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--ignore-submodules=all",
        "--",
        ".",
    ]);
    let (status, bytes) = capture(&mut state, MAX_STATUS_BYTES, SLOW_TIMEOUT)
        .await
        .map_err(|_| failure("Cannot inspect source working tree within the size/time limit."))?;
    if !status.success() {
        return Err(failure("Cannot inspect source working tree."));
    }
    Ok(GitContext {
        commit,
        dirty: Some(!bytes.is_empty()),
    })
}

/// `-c` settings that switch off every filter the repository configures.
///
/// A diff or status can run a repository's clean and process filters even
/// with external diffs and text conversion off, so each one is blanked out.
/// Only the setting names are read, never the commands.
async fn filter_overrides(root: &Path, private: &Path) -> Result<Vec<String>, EngineError> {
    let mut config = command(root, private);
    config.args([
        "config",
        "--null",
        "--name-only",
        "--get-regexp",
        r"^filter\..*\.(clean|smudge|process|required)$",
    ]);
    let (status, keys) = capture(&mut config, MAX_FILTER_KEYS_BYTES, QUICK_TIMEOUT)
        .await
        .map_err(|_| failure("Cannot inspect Git filter configuration."))?;
    if !status.success() && status.code() != Some(NOTHING_FOUND) {
        return Err(failure("Cannot inspect Git filter configuration."));
    }
    let mut overrides = Vec::new();
    for key in keys.split(|byte| *byte == 0).filter(|key| !key.is_empty()) {
        let key = std::str::from_utf8(key)
            .map_err(|_| failure("Unsupported Git filter configuration."))?;
        if key.len() > MAX_FILTER_KEY_BYTES
            || key.chars().any(char::is_control)
            || !key.starts_with("filter.")
            || overrides.len() >= MAX_FILTER_KEYS
        {
            return Err(failure("Unsupported Git filter configuration."));
        }
        let value = if key.ends_with(".required") {
            "false"
        } else {
            ""
        };
        overrides.push(format!("{key}={value}"));
    }
    Ok(overrides)
}

/// Where a source folder's code lives on GitHub: its `origin` remote, the
/// branch checked out and the commit, read without credentials or network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitRemote {
    /// `owner/name` on GitHub.
    pub repository: String,
    /// The branch checked out; `None` when no branch is.
    pub branch: Option<String>,
    /// The commit checked out; `None` before the first commit.
    pub commit: Option<String>,
}

/// The folder's GitHub remote, or None when it isn't a clone of a GitHub repository.
pub(crate) async fn github_remote(root: &Path) -> Option<GitRemote> {
    if !root.is_dir() {
        return None;
    }
    let private = tempfile::tempdir().ok()?;
    let read = |args: &'static [&'static str]| {
        let mut command = command(root, private.path());
        command.args(args);
        command
    };
    let text = |mut command: tokio::process::Command| async move {
        let (status, bytes) = capture(&mut command, MAX_REMOTE_BYTES, REMOTE_TIMEOUT)
            .await
            .ok()?;
        status
            .success()
            .then(|| String::from_utf8(bytes.to_vec()).ok())
            .flatten()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let url = text(read(&["config", "--get", "remote.origin.url"])).await?;
    let repository = github_repository(&url)?;
    let branch = text(read(&["rev-parse", "--abbrev-ref", "HEAD"]))
        .await
        .filter(|branch| branch != "HEAD");
    let commit = text(read(&["rev-parse", "--verify", "--quiet", "HEAD"]))
        .await
        .filter(|commit| is_commit_id(commit));
    Some(GitRemote {
        repository,
        branch,
        commit,
    })
}

/// `owner/name` from a GitHub remote URL in any of its forms; never keeps credentials.
pub(crate) fn github_repository(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| {
            let after = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))?;
            // Drop any user:token@ before the host.
            let host = after.rsplit_once('@').map_or(after, |(_, host)| host);
            host.strip_prefix("github.com/")
        })?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let ok = |part: &str| {
        !part.is_empty()
            && part.len() <= MAX_REPOSITORY_PART_CHARS
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (ok(owner) && ok(name) && !name.contains('/')).then(|| format!("{owner}/{name}"))
}

/// A git command in `root`, with hooks and global config pointed at the
/// empty folder `private`.
fn command(root: &Path, private: &Path) -> tokio::process::Command {
    let mut command = tool_paths::command("git");
    command
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "submodule.recurse=false",
            "-c",
            "protocol.file.allow=never",
            "-c",
        ])
        .arg(format!("core.hooksPath={}", private.display()))
        .current_dir(root)
        .env_clear()
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    for key in INHERITED {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", private.join("empty-config"));
    command
}

/// The lines a diff adds, without the leading `+` and without file headers.
fn added_lines(diff: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut result = Zeroizing::new(Vec::new());
    let mut hunk = false;
    for line in diff.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"diff --git ") {
            hunk = false;
        } else if line.starts_with(b"@@ ") {
            hunk = true;
        } else if hunk && line.starts_with(b"+") {
            result.extend_from_slice(&line[1..]);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additions_preserve_multiline_values_and_ignore_removals_and_headers() {
        let patch = b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1,3 @@\n-old-secret\n+first\n+++content\n+third\ndiff --git a/b b/b\n+++ b/b\n";
        assert_eq!(&added_lines(patch)[..], b"first\n++content\nthird\n");
    }
}

#[cfg(test)]
mod remote_tests {
    use super::github_repository;

    #[test]
    fn reads_github_remotes_in_every_form_and_drops_credentials() {
        for url in [
            "git@github.com:TheOwner/sentinel.git",
            "https://github.com/TheOwner/sentinel",
            "https://github.com/TheOwner/sentinel.git/",
            "https://x-access-token:abc@github.com/TheOwner/sentinel.git",
            "ssh://git@github.com/TheOwner/sentinel.git",
        ] {
            assert_eq!(
                github_repository(url).as_deref(),
                Some("TheOwner/sentinel"),
                "{url}"
            );
        }
        assert_eq!(github_repository("https://gitlab.com/a/b.git"), None);
        assert_eq!(github_repository("https://github.com/only-owner"), None);
    }
}
