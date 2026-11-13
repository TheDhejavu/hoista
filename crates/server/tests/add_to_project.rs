//! Adding pieces at providers to a project: what the provider makes joins the
//! project, and the project keeps its mode, so a project an agent made to
//! deploy to stays one Hoista can deploy to.

use hoista_server::{
    Provider,
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::{CatalogRepository, EngineError},
            providers::{ProviderAdapter, ProviderNew, RemoteResource},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
};
use std::sync::{Arc, Mutex};

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

/// A provider that makes Postgres databases, and finds what it made.
#[derive(Default)]
struct Databases {
    made: Mutex<Vec<String>>,
    /// The first create gets no answer from the provider, and isn't made.
    first_unanswered: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl ProviderAdapter for Databases {
    fn provider(&self) -> Provider {
        Provider::Neon
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            discover: true,
            new_database: true,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(self
            .made
            .lock()
            .expect("fixture")
            .iter()
            .map(|name| RemoteResource {
                reference: format!("db-{name}"),
                name: name.clone(),
                kind: ResourceKind::Database,
                region: None,
                status: ResourceStatus::Live,
                url: None,
                group: None,
            })
            .collect())
    }

    async fn provider_add(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: Option<&str>,
        what: &ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let ProviderNew::Database { name, .. } = *what else {
            return Err(hoista_server::application::ports::providers::unsupported());
        };
        if self
            .first_unanswered
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.timeout".into(),
                message: "No answer.".into(),
            });
        }
        self.made.lock().expect("fixture").push(name.to_owned());
        Ok((vec![format!("db-{name}")], format!("Made {name}")))
    }
}

async fn setup() -> (tempfile::TempDir, Engine) {
    setup_with(Databases::default()).await
}

async fn setup_with(host: Databases) -> (tempfile::TempDir, Engine) {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(LocalInspector),
        vec![Arc::new(host)],
        Arc::new(LocalBuilder::new(paths.root())),
    );
    store
        .put_account(&Account {
            last_discovery: None,
            id: "neon".into(),
            provider: Provider::Neon,
            label: "fixture".into(),
            external_id: "org".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-07T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    (temp, engine)
}

#[tokio::test]
async fn a_database_joins_the_project_and_a_managed_project_stays_managed() {
    let (_temp, engine) = setup().await;
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .expect("project");
    let (summary, added) = engine
        .projects()
        .add_to_project(
            &Actor::Person,
            &project.id,
            "neon",
            ProviderNew::Database {
                name: "sentinel-db",
                engine: "postgres",
                region: None,
            },
        )
        .await
        .expect("added");
    assert_eq!(summary, "Made sentinel-db.");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(added[0].kind, ResourceKind::Database);
    let after = engine
        .repository
        .project(&project.id)
        .await
        .expect("read")
        .expect("project");
    assert_eq!(after.mode, Mode::Managed);
}

#[tokio::test]
async fn a_provider_that_doesnt_make_this_kind_is_refused_before_anything_is_sent() {
    let (_temp, engine) = setup().await;
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .expect("project");
    let refused = engine
        .projects()
        .add_to_project(
            &Actor::Person,
            &project.id,
            "neon",
            ProviderNew::Bucket { name: "files" },
        )
        .await
        .expect_err("refused");
    assert!(matches!(refused, EngineError::Conflict(_)));
}

#[tokio::test]
async fn a_retry_after_no_answer_goes_ahead_when_the_provider_doesnt_have_it() {
    let host = Databases::default();
    host.first_unanswered
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (_temp, engine) = setup_with(host).await;
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .expect("project");
    let what = ProviderNew::Database {
        name: "sentinel-db",
        engine: "postgres",
        region: None,
    };
    engine
        .projects()
        .add_to_project(&Actor::Person, &project.id, "neon", what)
        .await
        .expect_err("no answer the first time");
    // Hoista looks at the provider, finds nothing, and sends it again.
    let (_, added) = engine
        .projects()
        .add_to_project(&Actor::Person, &project.id, "neon", what)
        .await
        .expect("made on the retry");
    assert_eq!(added.len(), 1);
    let operations = engine
        .observe()
        .operations(None)
        .await
        .expect("journal")
        .items;
    assert!(
        operations
            .iter()
            .any(|operation| operation.state == OperationState::DidNotHappen),
        "the unanswered try is settled as not happened"
    );
}

#[tokio::test]
async fn a_resource_in_the_wrong_project_moves_and_the_empty_project_goes() {
    let (_temp, engine) = setup().await;
    let sentinel = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .expect("project");
    let stray = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel-llm".into(),
            Source::Unlinked,
            Mode::Watching,
        )
        .await
        .expect("project");
    let (_, added) = engine
        .projects()
        .add_to_project(
            &Actor::Person,
            &stray.id,
            "neon",
            ProviderNew::Database {
                name: "llm-db",
                engine: "postgres",
                region: None,
            },
        )
        .await
        .expect("added");
    let ids: Vec<String> = added.iter().map(|resource| resource.id.clone()).collect();
    let (moved, removed) = engine
        .projects()
        .move_resources(&Actor::Person, &ids, &sentinel.id)
        .await
        .expect("moved");
    assert_eq!(moved.len(), 1);
    assert_eq!(removed, ["sentinel-llm"]);
    let resource = engine
        .repository
        .resource(&ids[0])
        .await
        .expect("read")
        .expect("resource");
    assert_eq!(resource.project_id.as_deref(), Some(sentinel.id.as_str()));
    assert!(
        engine
            .repository
            .project(&stray.id)
            .await
            .expect("read")
            .is_none()
    );
    let after = engine
        .repository
        .project(&sentinel.id)
        .await
        .expect("read")
        .expect("project");
    assert_eq!(after.mode, Mode::Managed);
}
