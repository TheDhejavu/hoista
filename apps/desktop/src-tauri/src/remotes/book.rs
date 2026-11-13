use crate::error::Result;
use crate::files;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One remote in the list. Its key isn't here; that's in the keychain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Remote {
    pub id: String,
    pub name: String,
    pub url: String,
    pub added_at: String,
}

/// The list of remotes, in Hoista's state folder beside the engine's own
/// files, and which Hoista the app showed last, in the app's config folder.
pub struct Book {
    remotes: PathBuf,
    shown: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct RemotesFile {
    remotes: Vec<Remote>,
}

#[derive(Serialize, Deserialize)]
struct ShownFile {
    showing: String,
}

pub const THIS_COMPUTER: &str = "this_computer";

impl Book {
    pub fn new(state_folder: PathBuf, config_folder: PathBuf) -> Self {
        Self {
            remotes: state_folder.join("remotes.json"),
            shown: config_folder.join("server-view.json"),
        }
    }

    /// The remotes, or none if the file is missing or damaged.
    pub fn list(&self) -> Vec<Remote> {
        files::read(&self.remotes)
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str::<RemotesFile>(&text).ok())
            .unwrap_or_default()
            .remotes
    }

    pub fn save(&self, remotes: &[Remote]) -> Result<()> {
        let file = RemotesFile {
            remotes: remotes.to_vec(),
        };
        files::write_private(&self.remotes, &serde_json::to_string_pretty(&file)?)
    }

    pub fn last_shown(&self) -> String {
        files::read(&self.shown)
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str::<ShownFile>(&text).ok())
            .map_or_else(|| THIS_COMPUTER.to_owned(), |file| file.showing)
    }

    pub fn remember_shown(&self, showing: &str) -> Result<()> {
        let file = ShownFile {
            showing: showing.to_owned(),
        };
        files::write_private(&self.shown, &serde_json::to_string(&file)?)
    }
}
