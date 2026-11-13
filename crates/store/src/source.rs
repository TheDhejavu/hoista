//! Where a project's source is on this computer: its folder, or a fresh
//! checkout of one exact GitHub commit. Checkouts run git directly, with no
//! shell, hooks or submodules.

use super::{
    process::{self, ProcessError},
    tool_paths,
};
use crate::{application::ports::catalog::EngineError, domain::model::Source};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

/// A full commit SHA, in hex. Branches and tags can move, so only these are used.
const COMMIT_SHA_CHARS: usize = 40;
const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(120);

/// The source folder. A GitHub checkout is deleted when this is dropped.
pub struct ResolvedSource {
    root: PathBuf,
    _checkout: Option<tempfile::TempDir>,
}

impl ResolvedSource {
    /// The folder, valid while this value lives.
    pub fn path(&self) -> &Path {
        &self.root
    }
}

/// Finds the source on this computer, checking it out first when it's on
/// GitHub.
///
/// # Errors
/// Fails for a project with no source, a relative or missing folder, a
/// repository name that isn't `owner/name`, anything but a full commit SHA,
/// or when git is missing or the checkout fails.
pub async fn resolve(source: &Source) -> Result<ResolvedSource, EngineError> {
    match source {
        Source::Unlinked => Err(EngineError::Invalid(
            "Link a local folder or a GitHub commit before deploying this imported project.",
        )),
        Source::Folder { path } => {
            if !Path::new(path).is_absolute() {
                return Err(EngineError::Invalid(
                    "Choose an absolute source folder path.",
                ));
            }
            Ok(ResolvedSource {
                root: std::fs::canonicalize(path)
                    .map_err(|_| EngineError::Invalid("The source folder is not available."))?,
                _checkout: None,
            })
        }
        Source::Github { repository, commit } => {
            let pieces = repository.split('/').collect::<Vec<_>>();
            if pieces.len() != 2
                || pieces.iter().any(|piece| {
                    piece.is_empty()
                        || *piece == "."
                        || *piece == ".."
                        || !piece
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                })
                || commit.len() != COMMIT_SHA_CHARS
                || !commit.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(EngineError::Invalid(
                    "Use a GitHub owner/repository and an exact 40-character commit SHA.",
                ));
            }
            let temporary = tempfile::tempdir().map_err(|_| EngineError::Store)?;
            let root = temporary.path().join("checkout");
            std::fs::create_dir(&root).map_err(|_| EngineError::Store)?;
            let hooks = temporary.path().join("empty-hooks");
            std::fs::create_dir(&hooks).map_err(|_| EngineError::Store)?;
            git(&root, &hooks, &["init", "--quiet"]).await?;
            git(
                &root,
                &hooks,
                &[
                    "remote",
                    "add",
                    "origin",
                    &format!("https://github.com/{repository}.git"),
                ],
            )
            .await?;
            git(
                &root,
                &hooks,
                &[
                    "fetch",
                    "--quiet",
                    "--depth=1",
                    "--no-tags",
                    "origin",
                    commit,
                ],
            )
            .await?;
            git(&root, &hooks, &["checkout", "--quiet", "--detach", commit]).await?;
            Ok(ResolvedSource {
                root,
                _checkout: Some(temporary),
            })
        }
    }
}

async fn git(root: &Path, hooks: &Path, args: &[&str]) -> Result<(), EngineError> {
    let mut command = tool_paths::command("git");
    command
        .args([
            "-c",
            "protocol.file.allow=never",
            "-c",
            "submodule.recurse=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
        ])
        .arg(format!("core.hooksPath={}", hooks.display()))
        .args(args)
        .current_dir(root)
        .env_clear()
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    process::inherit_environment(&mut command);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", hooks.join("empty-config"));
    let status = process::run(&mut command, None, CHECKOUT_TIMEOUT)
        .await
        .map_err(|error| match error {
            ProcessError::Spawn(_) => {
                EngineError::Invalid("Install git to deploy from GitHub. If it's installed, run hoist tools configure and restart Hoista.")
            }
            ProcessError::Timeout => {
                EngineError::Invalid("The GitHub checkout took too long and was stopped.")
            }
            ProcessError::Cleanup => {
                EngineError::Invalid("Git was stopped, but Hoista couldn't confirm it exited.")
            }
            ProcessError::Io(_) => EngineError::Invalid("Could not fetch the GitHub commit."),
        })?;
    if !status.success() {
        return Err(EngineError::Invalid(
            "Could not fetch that GitHub commit. Check the repository, the commit SHA and your GitHub sign-in.",
        ));
    }
    Ok(())
}
