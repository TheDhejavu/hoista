//! An account connected a moment ago is read before anyone is told what's on
//! it, rather than looking empty until the background monitor gets to it.

use hoista_server::{
    Provider,
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::EngineError,
            providers::{ProviderAdapter, RemoteResource},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Vault;
#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, _: &str) -> Result<Secret, EngineError> {
        Ok(Secret::new("fixture-token".into()))
    }

    async fn write(&self, _: &str, _: Secret) -> Result<(), EngineError> {
        Ok(())
    }

    async fn delete(&self, _: &str) -> Result<(), EngineError> {
        Ok(())
    }
}

/// A provider with one Worker on it, counting how often it's read.
#[derive(Default)]
struct OneWorker {
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderAdapter for OneWorker {
    fn provider(&self) -> Provider {
        Provider::Cloudflare
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            discover: true,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![RemoteResource {
            reference: "worker/sentinel-edge".into(),
            name: "sentinel-edge".into(),
            kind: ResourceKind::Web,
            region: None,
            status: ResourceStatus::Live,
            url: None,
            group: None,
        }])
    }
}

#[tokio::test]
async fn what_is_on_a_new_account_is_known_before_it_is_listed() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(OneWorker::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(LocalInspector),
        vec![provider.clone()],
        Arc::new(LocalBuilder::new(paths.root())),
    );
    engine
        .accounts()
        .connect_account(
            Provider::Cloudflare,
            "Sam".into(),
            "acct".into(),
            Secret::new("fixture-token".into()),
        )
        .await
        .expect("connected");
    // Nothing has read it yet; asking what's there reads it first.
    let found = engine
        .accounts()
        .import_candidates(&Actor::Person, false, None)
        .await
        .expect("found");
    assert_eq!(found.items.len(), 1, "{found:?}");
    assert_eq!(found.items[0].name, "sentinel-edge");
    // Suggestions and later lists don't read it again: the monitor does.
    engine
        .accounts()
        .import_suggestions(&Actor::Person, None)
        .await
        .expect("suggestions");
    engine
        .accounts()
        .import_candidates(&Actor::Person, false, None)
        .await
        .expect("found");
    assert_eq!(provider.reads.load(Ordering::SeqCst), 1);
    store.close().await;
}
