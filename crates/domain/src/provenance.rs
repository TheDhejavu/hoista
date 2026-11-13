//! What a deploy was built from: the source as Hoista copied it, whatever the
//! provider calls the release.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The exact files a build used, and the Git state around them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct SourceProvenance {
    /// The commit at `HEAD`, or `null` for a folder outside Git or a repository with no commits.
    pub git_commit: Option<String>,
    /// Whether the project's folder had changes not yet committed. `null` outside Git.
    pub git_dirty: Option<bool>,
    /// SHA-256 of the list of copied files (paths, contents and whether each is executable).
    /// Not a Git tree hash.
    pub snapshot_sha256: String,
    /// Which version of that list's format the hash was taken over.
    pub snapshot_format: String,
    /// How many files were copied. `.env` and generated files aren't copied or counted.
    pub snapshot_files: u32,
}

/// A first look at the source, taken before the files are copied. The copy
/// may differ if the folder changes in between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceObservation {
    /// A Git repository on this computer. No paths, changes or values are kept.
    LocalGit {
        /// The commit at `HEAD`, or `null` when there are no commits yet.
        commit: Option<String>,
        /// Whether the project's folder had uncommitted changes.
        dirty: bool,
    },
    /// A folder on this computer, outside Git.
    LocalFolder,
    /// Source somewhere else, such as a GitHub repository. The deploy records which commit was asked for.
    RemoteSource,
    /// The source couldn't be looked at in time, or this kind can't be.
    Unavailable,
}
