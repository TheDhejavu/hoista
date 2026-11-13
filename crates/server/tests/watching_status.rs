//! The monitor reads every watched resource's status from its provider, also
//! on providers that report no usage (a Fly app says how it is by its machines).

use hoista_server::{
    Provider,
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::{CatalogRepository, EngineError},
            providers::{Observed, ProviderAdapter, RemoteResource},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
};
use std::sync::Arc;

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

/// A provider like Fly: no usage, but each app's status on request.
struct LiveApps;

#[async_trait::async_trait]
impl ProviderAdapter for LiveApps {
    fn provider(&self) -> Provider {
        Provider::Fly
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            discover: true,
            usage: false,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }

    async fn observe_resource(
        &self,
        _: &Secret,
        _: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        Ok(Observed {
            status: ResourceStatus::Live,
            url: Some(format!("https://{}.fly.dev", resource.provider_ref)),
            kind: None,
        })
    }
}

#[tokio::test]
async fn a_live_app_on_a_provider_without_usage_stops_saying_deploying() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(LocalInspector),
        vec![Arc::new(LiveApps)],
        Arc::new(LocalBuilder::new(paths.root())),
    ));
    let account = engine
        .accounts()
        .connect_account(
            Provider::Fly,
            "Sam".into(),
            "personal".into(),
            Secret::new("t".into()),
        )
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "hello-fly".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .expect("project");
    for (id, status) in [
        ("deploying", ResourceStatus::Deploying),
        ("unknown", ResourceStatus::Unknown),
    ] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: Some(project.id.clone()),
                kind: ResourceKind::Service,
                provider: Provider::Fly,
                account_id: account.id.clone(),
                provider_ref: format!("app-{id}"),
                name: format!("app-{id}"),
                region: None,
                status,
                mode: Mode::Watching,
                url: None,
                group: None,
            })
            .await
            .expect("resource");
    }
    let stop = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(engine.clone().run_monitor(stop.clone()));
    let settled = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let a = store
                .resource("deploying")
                .await
                .expect("read")
                .expect("there");
            let b = store
                .resource("unknown")
                .await
                .expect("read")
                .expect("there");
            if a.status == ResourceStatus::Live && b.status == ResourceStatus::Live {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    stop.cancel();
    handle.await.expect("monitor").expect("monitor ran");
    assert!(settled.is_ok(), "Fly apps stayed deploying or unknown");
    store.close().await;
}
