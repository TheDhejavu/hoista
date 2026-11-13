//! Where the demo keeps its account tokens: a file in the demo's own state
//! folder. The tokens are made up and reach nothing, so they never go near the
//! system keychain or 1Password.

use hoista_server::application::ports::{catalog::EngineError, vault::TokenVault};
use hoista_server::domain::Secret;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// What `demo-vault.json` holds: entry name → value.
type Entries = BTreeMap<String, String>;

pub struct DemoVault {
    file: PathBuf,
    entries: Mutex<Entries>,
}

impl DemoVault {
    /// The vault kept in `folder`, with whatever an earlier run saved.
    pub fn open(folder: &Path) -> Self {
        let file = folder.join("demo-vault.json");
        let entries = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            file,
            entries: Mutex::new(entries),
        }
    }

    fn change(&self, edit: impl FnOnce(&mut Entries)) -> Result<(), EngineError> {
        let mut entries = self.entries.lock().map_err(|_| EngineError::Store)?;
        edit(&mut entries);
        let text = serde_json::to_vec_pretty(&*entries).map_err(|_| EngineError::Store)?;
        std::fs::write(&self.file, text).map_err(|_| EngineError::Store)
    }
}

#[async_trait::async_trait]
impl TokenVault for DemoVault {
    async fn read(&self, reference: &str) -> Result<Secret, EngineError> {
        let entries = self.entries.lock().map_err(|_| EngineError::Store)?;
        entries
            .get(reference)
            .map(|value| Secret::new(value.clone()))
            .ok_or(EngineError::NotFound)
    }

    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError> {
        self.change(|entries| {
            entries.insert(reference.to_owned(), value.expose().to_owned());
        })
    }

    async fn delete(&self, reference: &str) -> Result<(), EngineError> {
        self.change(|entries| {
            entries.remove(reference);
        })
    }
}
