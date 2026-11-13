//! Credential boundary tests never use the developer's OS keychain by default.

use hoista_server::{
    application::{CredentialError, CredentialStore, check_keychain},
    domain::Secret,
    infrastructure::{paths::StatePaths, store::SqliteStore},
};
use std::{cell::RefCell, collections::HashMap};

#[derive(Default)]
struct FakeKeychain {
    entries: RefCell<HashMap<String, Secret>>,
    fail_read: bool,
}

impl CredentialStore for FakeKeychain {
    fn get(&self, reference: &str) -> Result<Secret, CredentialError> {
        if self.fail_read {
            return Err(CredentialError::Unavailable);
        }
        self.entries
            .borrow()
            .get(reference)
            .map(|secret| Secret::new(secret.expose().into()))
            .ok_or(CredentialError::Missing)
    }

    fn set(&self, reference: &str, secret: &Secret) -> Result<(), CredentialError> {
        self.entries
            .borrow_mut()
            .insert(reference.into(), Secret::new(secret.expose().into()));
        Ok(())
    }

    fn delete(&self, reference: &str) -> Result<(), CredentialError> {
        self.entries.borrow_mut().remove(reference);
        Ok(())
    }
}

#[test]
fn doctor_cleans_up_its_probe_even_when_read_access_fails() {
    let keychain = FakeKeychain::default();
    keychain
        .set("account/test", &Secret::new("keep-this-token".into()))
        .unwrap();
    check_keychain(&keychain).unwrap();
    assert_eq!(keychain.entries.borrow().len(), 1);
    let failing = FakeKeychain {
        fail_read: true,
        ..FakeKeychain::default()
    };
    assert_eq!(check_keychain(&failing), Err(CredentialError::Unavailable));
    assert!(failing.entries.borrow().is_empty());
}

#[tokio::test]
async fn deleting_state_does_not_delete_credential_entries() {
    let directory = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(directory.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let keychain = FakeKeychain::default();
    keychain
        .set("account/test", &Secret::new("keep-this-token".into()))
        .unwrap();
    store.close().await;
    drop(store);
    std::fs::remove_dir_all(paths.root()).unwrap();
    assert_eq!(
        keychain.get("account/test").unwrap().expose(),
        "keep-this-token"
    );
    keychain.delete("account/test").unwrap();
    assert_eq!(
        keychain.get("account/test").unwrap_err(),
        CredentialError::Missing
    );
}

#[test]
#[ignore = "requires interactive native keychain access; creates and deletes only a unique doctor probe"]
fn native_keychain_round_trip() {
    check_keychain(&hoista_server::infrastructure::credentials::SystemKeychain).unwrap();
}

/// Refuses every read, like a keychain prompt that is denied.
#[derive(Default)]
struct RefusingKeychain {
    reads: std::sync::atomic::AtomicUsize,
}

impl CredentialStore for RefusingKeychain {
    fn get(&self, _: &str) -> Result<Secret, CredentialError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(CredentialError::Unavailable)
    }

    fn set(&self, _: &str, _: &Secret) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }

    fn delete(&self, _: &str) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

#[tokio::test]
async fn a_refused_keychain_read_is_not_retried_at_once() {
    use hoista_server::{
        application::ports::vault::TokenVault, infrastructure::credentials::NativeVault,
    };
    let keychain = std::sync::Arc::new(RefusingKeychain::default());
    let vault = NativeVault::in_store("engine", keychain.clone());
    for _ in 0..20 {
        assert!(vault.read("agent/one").await.is_err());
    }
    assert_eq!(
        keychain.reads.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one refusal must not become a prompt per request"
    );
}
