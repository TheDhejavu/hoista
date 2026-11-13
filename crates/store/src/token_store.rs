//! Where Hoista keeps secrets: a [`StoredVault`] is the place that holds
//! entries (the keychain, or a sealed file on a remote) plus a [`TokenStore`]
//! for provider tokens. A [`KeySource`] supplies the key that seals the file.
//!
//! An unknown key source or token store stops Hoista at start; no fallback.

use crate::{
    application::ports::{catalog::EngineError, vault::TokenVault},
    domain::{
        Secret,
        model::{VaultInfo, VaultPlace},
    },
};
use std::sync::Arc;

/// Chooses the key source: `HOISTA_KEY_SOURCE`, `host-secret` when unset.
pub const KEY_SOURCE_VARIABLE: &str = "HOISTA_KEY_SOURCE";
/// Chooses the token store: `HOISTA_TOKEN_STORE`, `built-in` when unset.
pub const TOKEN_STORE_VARIABLE: &str = "HOISTA_TOKEN_STORE";
/// The provider secret holding the sealing key.
pub const SEALED_KEY_VARIABLE: &str = "HOISTA_SEALED_KEY";

const HOST_SECRET: &str = "host-secret";
const BUILT_IN: &str = "built-in";

/// Where the sealing key comes from.
#[async_trait::async_trait]
pub trait KeySource: Send + Sync {
    /// Its name, as `HOISTA_KEY_SOURCE` spells it.
    fn name(&self) -> &'static str;

    /// The key that opens the sealed file, asked for once at start.
    ///
    /// # Errors
    /// Reports a key that's missing or can't be had.
    async fn sealing_key(&self) -> Result<Secret, EngineError>;
}

/// Where provider tokens live.
#[async_trait::async_trait]
pub trait TokenStore: Send + Sync {
    /// Its name, as `HOISTA_TOKEN_STORE` spells it.
    fn name(&self) -> &'static str;

    /// Checks it can be used, once at start.
    ///
    /// # Errors
    /// Reports a store that can't be reached or won't take writes.
    async fn check(&self) -> Result<(), EngineError>;

    /// Takes the token for vault entry `key` and returns what the sealed
    /// file should keep: the token itself, or a reference to where it went.
    /// `previous` is what was kept for `key` before, so a store can replace
    /// its own copy.
    ///
    /// # Errors
    /// Reports a store that refused it; nothing is kept then.
    async fn put(
        &self,
        key: &str,
        token: Secret,
        previous: Option<Secret>,
    ) -> Result<Secret, EngineError>;

    /// The token from what the sealed file kept.
    ///
    /// # Errors
    /// Reports a token the store can't give back.
    async fn get(&self, kept: Secret) -> Result<Secret, EngineError>;

    /// Removes the store's copy when an account is removed.
    ///
    /// # Errors
    /// Reports a store that refused it.
    async fn remove(&self, key: &str, kept: Secret) -> Result<(), EngineError>;
}

/// Vault keys that hold provider tokens: connected accounts, and tokens waiting
/// for the person to pick an account.
#[must_use]
pub fn provider_token(key: &str) -> bool {
    key.starts_with("account/") || key.starts_with("pending/")
}

/// A vault: entries kept in `place`, provider tokens going through `store`.
pub struct StoredVault {
    place: Arc<dyn TokenVault>,
    store: Arc<dyn TokenStore>,
    kept_in: VaultPlace,
}

impl StoredVault {
    /// Provider tokens through `store`, everything kept in `place`.
    #[must_use]
    pub fn new(
        place: Arc<dyn TokenVault>,
        kept_in: VaultPlace,
        store: Arc<dyn TokenStore>,
    ) -> Self {
        Self {
            place,
            store,
            kept_in,
        }
    }

    /// The same place with tokens through another store.
    #[must_use]
    pub fn with_store(&self, store: Arc<dyn TokenStore>) -> Self {
        Self::new(self.place.clone(), self.kept_in, store)
    }

    /// Checks the store can be used, once at start.
    ///
    /// # Errors
    /// Reports a store that can't be reached.
    pub async fn check(&self) -> Result<(), EngineError> {
        self.store.check().await
    }

    /// Whether `key` has an entry, without asking the store for its token.
    ///
    /// # Errors
    /// Reports a place that can't be read.
    pub async fn has(&self, key: &str) -> Result<bool, EngineError> {
        match self.place.read(key).await {
            Ok(_) => Ok(true),
            Err(EngineError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Where tokens are kept, for Settings and `GET /vault`.
    #[must_use]
    pub fn info(&self, key_source: Option<&str>) -> VaultInfo {
        VaultInfo {
            kept_in: self.kept_in,
            token_store: self.store.name().to_owned(),
            key_source: key_source.map(str::to_owned),
            references: true,
        }
    }
}

#[async_trait::async_trait]
impl TokenVault for StoredVault {
    async fn read(&self, key: &str) -> Result<Secret, EngineError> {
        let kept = self.place.read(key).await?;
        if provider_token(key) {
            self.store.get(kept).await
        } else {
            Ok(kept)
        }
    }

    async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
        if !provider_token(key) {
            return self.place.write(key, value).await;
        }
        let previous = self.place.read(key).await.ok();
        let kept = self.store.put(key, value, previous).await?;
        self.place.write(key, kept).await
    }

    async fn delete(&self, key: &str) -> Result<(), EngineError> {
        if provider_token(key)
            && let Ok(kept) = self.place.read(key).await
        {
            self.store.remove(key, kept).await?;
        }
        self.place.delete(key).await
    }

    async fn check(&self) -> Result<(), EngineError> {
        self.store.check().await
    }
}

/// The key from `HOISTA_SEALED_KEY`, a secret variable set where Hoista runs.
pub struct HostSecret;

#[async_trait::async_trait]
impl KeySource for HostSecret {
    fn name(&self) -> &'static str {
        HOST_SECRET
    }

    async fn sealing_key(&self) -> Result<Secret, EngineError> {
        std::env::var(SEALED_KEY_VARIABLE)
            .ok()
            .filter(|key| !key.trim().is_empty())
            .map(|key| Secret::new(key.trim().to_owned()))
            .ok_or(EngineError::Invalid(
                "Set HOISTA_SEALED_KEY to a new key from `hoistd --new-sealed-key`, kept as a secret variable at your provider.",
            ))
    }
}

/// Tokens stay in the sealed file.
pub struct BuiltIn;

#[async_trait::async_trait]
impl TokenStore for BuiltIn {
    fn name(&self) -> &'static str {
        BUILT_IN
    }

    async fn check(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn put(
        &self,
        _key: &str,
        token: Secret,
        _previous: Option<Secret>,
    ) -> Result<Secret, EngineError> {
        Ok(token)
    }

    async fn get(&self, kept: Secret) -> Result<Secret, EngineError> {
        Ok(kept)
    }

    async fn remove(&self, _key: &str, _kept: Secret) -> Result<(), EngineError> {
        Ok(())
    }
}

/// The key source `name` names (`None`: the default).
///
/// # Errors
/// Names one Hoista doesn't have yet.
pub fn key_source(name: Option<&str>) -> Result<Box<dyn KeySource>, EngineError> {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        None | Some(HOST_SECRET) => Ok(Box::new(HostSecret)),
        Some(_) => Err(EngineError::Invalid(
            "HOISTA_KEY_SOURCE can only be host-secret for now. Key services (AWS KMS, Google Cloud KMS, Vault) are planned.",
        )),
    }
}

/// Every token store Hoista has, by name. The vault conformance test runs
/// each of them; add a new one here as well as in [`token_store`].
pub const TOKEN_STORES: &[&str] = &[BUILT_IN];

/// The token store `name` names (`None`: the default).
///
/// # Errors
/// Names one Hoista doesn't have yet.
pub fn token_store(name: Option<&str>) -> Result<Box<dyn TokenStore>, EngineError> {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        None | Some(BUILT_IN) => Ok(Box::new(BuiltIn)),
        Some(_) => Err(EngineError::Invalid(
            "HOISTA_TOKEN_STORE can only be built-in. To keep a token in 1Password, connect the account with an op:// reference.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_built_in_ones_exist() {
        assert_eq!(key_source(None).map(|s| s.name()).ok(), Some("host-secret"));
        assert_eq!(
            key_source(Some(" ")).map(|s| s.name()).ok(),
            Some("host-secret")
        );
        assert!(key_source(Some("aws-kms")).is_err());
        assert_eq!(token_store(None).map(|s| s.name()).ok(), Some("built-in"));
        assert_eq!(
            token_store(Some("built-in")).map(|s| s.name()).ok(),
            Some("built-in")
        );
        assert!(token_store(Some("1password")).is_err());
    }

    /// An in-memory place, standing for the keychain or the sealed file.
    #[derive(Default)]
    struct Place(std::sync::Mutex<std::collections::HashMap<String, String>>);

    #[async_trait::async_trait]
    impl TokenVault for Place {
        async fn read(&self, key: &str) -> Result<Secret, EngineError> {
            self.0
                .lock()
                .unwrap()
                .get(key)
                .map(|value| Secret::new(value.clone()))
                .ok_or(EngineError::NotFound)
        }

        async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
            self.0
                .lock()
                .unwrap()
                .insert(key.into(), value.expose().into());
            Ok(())
        }

        async fn delete(&self, key: &str) -> Result<(), EngineError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    /// A store that keeps tokens elsewhere and leaves a reference.
    #[derive(Default)]
    struct Elsewhere(std::sync::Mutex<std::collections::HashMap<String, String>>);

    #[async_trait::async_trait]
    impl TokenStore for Elsewhere {
        fn name(&self) -> &'static str {
            "elsewhere"
        }

        async fn check(&self) -> Result<(), EngineError> {
            Ok(())
        }

        async fn put(
            &self,
            key: &str,
            token: Secret,
            _previous: Option<Secret>,
        ) -> Result<Secret, EngineError> {
            self.0
                .lock()
                .unwrap()
                .insert(key.into(), token.expose().into());
            Ok(Secret::new(format!("ref:{key}")))
        }

        async fn get(&self, kept: Secret) -> Result<Secret, EngineError> {
            let key = kept.expose().trim_start_matches("ref:").to_owned();
            self.0
                .lock()
                .unwrap()
                .get(&key)
                .map(|value| Secret::new(value.clone()))
                .ok_or(EngineError::NotFound)
        }

        async fn remove(&self, key: &str, _kept: Secret) -> Result<(), EngineError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[tokio::test]
    async fn provider_tokens_go_through_the_store_and_other_entries_stay_put() {
        let place = Arc::new(Place::default());
        let store = Arc::new(Elsewhere::default());
        let vault = StoredVault::new(place.clone(), VaultPlace::Keychain, store.clone());
        vault
            .write("account/a", Secret::new("token".into()))
            .await
            .unwrap();
        vault
            .write("agent/x", Secret::new("agent-key".into()))
            .await
            .unwrap();
        // The place keeps only a reference for the provider token.
        assert_eq!(
            place.read("account/a").await.unwrap().expose(),
            "ref:account/a"
        );
        assert_eq!(place.read("agent/x").await.unwrap().expose(), "agent-key");
        assert_eq!(vault.read("account/a").await.unwrap().expose(), "token");
        assert!(vault.has("account/a").await.unwrap());
        vault.delete("account/a").await.unwrap();
        assert!(store.0.lock().unwrap().is_empty());
        assert!(!vault.has("account/a").await.unwrap());
        assert_eq!(vault.info(None).token_store, "elsewhere");
    }

    #[tokio::test]
    async fn built_in_keeps_the_token_itself() {
        let kept = BuiltIn
            .put("account/a", Secret::new("t".into()), None)
            .await
            .expect("put");
        assert_eq!(kept.expose(), "t");
        assert_eq!(BuiltIn.get(kept).await.expect("get").expose(), "t");
    }

    /// A keychain in memory, standing for the system's: what Hoista on a
    /// computer writes through.
    #[derive(Default)]
    struct MemoryKeychain(std::sync::Mutex<std::collections::HashMap<String, String>>);

    impl crate::application::CredentialStore for MemoryKeychain {
        fn get(&self, reference: &str) -> Result<Secret, crate::application::CredentialError> {
            self.0
                .lock()
                .unwrap()
                .get(reference)
                .map(|value| Secret::new(value.clone()))
                .ok_or(crate::application::CredentialError::Missing)
        }

        fn set(
            &self,
            reference: &str,
            secret: &Secret,
        ) -> Result<(), crate::application::CredentialError> {
            self.0
                .lock()
                .unwrap()
                .insert(reference.into(), secret.expose().into());
            Ok(())
        }

        fn delete(&self, reference: &str) -> Result<(), crate::application::CredentialError> {
            self.0.lock().unwrap().remove(reference);
            Ok(())
        }
    }

    /// What every vault must do, whatever its place and token store: put,
    /// get, replace and remove provider tokens and other entries, and say a
    /// missing entry is missing.
    async fn vault_conforms(vault: &StoredVault, problems: &mut Vec<String>, what: &str) {
        let token = |value: &str| Secret::new(value.into());
        let mut expect = |ok: bool, rule: &str| {
            if !ok {
                problems.push(format!("{what}: {rule}"));
            }
        };
        expect(vault.check().await.is_ok(), "its store can't be used");
        for key in ["account/a", "pending/p", "agent/x"] {
            expect(
                vault.write(key, token("first-value")).await.is_ok(),
                "refused a write",
            );
            expect(
                vault
                    .read(key)
                    .await
                    .is_ok_and(|value| value.expose() == "first-value"),
                "didn't give back what was put",
            );
            expect(
                vault.write(key, token("second-value")).await.is_ok(),
                "refused a replace",
            );
            expect(
                vault
                    .read(key)
                    .await
                    .is_ok_and(|value| value.expose() == "second-value"),
                "didn't give back the replacement",
            );
            expect(
                vault.has(key).await.unwrap_or(false),
                "lost an entry it holds",
            );
            expect(vault.delete(key).await.is_ok(), "refused a remove");
            expect(
                matches!(vault.read(key).await, Err(EngineError::NotFound)),
                "still gave back a removed entry",
            );
            expect(
                !vault.has(key).await.unwrap_or(true),
                "still has a removed entry",
            );
            expect(
                vault.delete(key).await.is_ok(),
                "refused removing what's already gone",
            );
        }
        expect(
            matches!(
                vault.read("account/never").await,
                Err(EngineError::NotFound)
            ),
            "didn't say a missing entry is missing",
        );
    }

    #[tokio::test]
    async fn every_vault_and_token_store_conforms() {
        use crate::{credentials::NativeVault, sealed_store::SealedStore};
        let mut problems = Vec::new();
        for name in TOKEN_STORES {
            let store: Arc<dyn TokenStore> = Arc::from(token_store(Some(name)).unwrap());
            // On a computer: the keychain.
            let keychain = Arc::new(MemoryKeychain::default());
            let vault = StoredVault::new(
                Arc::new(NativeVault::in_store("conformance", keychain.clone())),
                VaultPlace::Keychain,
                store.clone(),
            );
            vault_conforms(&vault, &mut problems, &format!("keychain with {name}")).await;
            // On a remote: the sealed file. Nothing in it is readable without its key.
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("vault.sealed");
            let sealed = SealedStore::new(path.clone(), &SealedStore::new_key()).unwrap();
            let vault = StoredVault::new(
                Arc::new(NativeVault::in_store("conformance", Arc::new(sealed))),
                VaultPlace::SealedFile,
                store,
            );
            vault_conforms(&vault, &mut problems, &format!("sealed file with {name}")).await;
            vault
                .write("account/kept", Secret::new("readable-only-inside".into()))
                .await
                .unwrap();
            let bytes = std::fs::read(&path).unwrap();
            if bytes
                .windows(b"readable-only-inside".len())
                .any(|window| window == b"readable-only-inside")
            {
                problems.push(format!(
                    "sealed file with {name}: a token is readable in the file"
                ));
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    problems.push(format!(
                        "sealed file with {name}: others may read it (mode {mode:o})"
                    ));
                }
            }
            if folder.path().read_dir().unwrap().count() != 1 {
                problems.push(format!("sealed file with {name}: wrote files beside it"));
            }
        }
        assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
    }
}
