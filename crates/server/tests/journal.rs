//! The operation journal: changes to a provider are written down before they're
//! sent, and one that may already have happened is never sent again blindly.

use hoista_server::{
    Provider,
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::EngineError,
            providers::{ProviderAdapter, ProviderCreate},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
};
use std::sync::{
    Arc, Mutex,
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

/// A provider whose answers the test picks, counting what reaches it.
#[derive(Default)]
struct TestProvider {
    calls: AtomicUsize,
    /// The next answers; `None` is a success.
    answers: Mutex<Vec<Option<&'static str>>>,
    /// Never answers a create, like a provider that hangs until Hoista stops.
    hang: std::sync::atomic::AtomicBool,
}

impl TestProvider {
    fn answer(&self) -> Result<(), EngineError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let next = self
            .answers
            .lock()
            .expect("provider fixture")
            .pop()
            .flatten();
        match next {
            None => Ok(()),
            Some(code) => Err(EngineError::Provider {
                evidence: None,
                code: code.into(),
                message: "The provider could not be reached.".into(),
            }),
        }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for TestProvider {
    fn provider(&self) -> Provider {
        Provider::Railway
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            deploy: false,
            logs: false,
            metrics: false,
            usage: false,
            rollback: false,
            pause: false,
            domains: false,
            secrets: false,
            discover: false,
            server: false,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(
        &self,
        _: &Secret,
        _: &str,
    ) -> Result<Vec<hoista_server::application::ports::providers::RemoteResource>, EngineError>
    {
        Ok(vec![])
    }

    async fn provider_create(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: &ProviderCreate<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        if self.hang.load(Ordering::SeqCst) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
        self.answer()
            .map(|()| (vec!["svc".into()], "Created".into()))
    }

    async fn set_provider_variable(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: &str,
        _: &Secret,
    ) -> Result<(), EngineError> {
        self.answer()
    }
}

fn service(name: &str) -> ProviderCreate<'_> {
    ProviderCreate::Service {
        name,
        repo: None,
        branch: None,
        image: Some("nginx@sha256:0"),
    }
}

#[tokio::test]
async fn a_change_that_may_have_happened_is_not_sent_again_until_the_person_checks() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(LocalInspector),
        vec![provider.clone()],
        Arc::new(LocalBuilder::new(paths.root())),
    );
    let adapter = engine.adapter(Provider::Railway).expect("adapter").clone();
    let token = Secret::new("t".into());

    // The connection drops mid-create: Hoista can't know whether it happened.
    provider
        .answers
        .lock()
        .expect("provider fixture")
        .push(Some("provider.unreachable"));
    assert!(
        adapter
            .provider_create(&token, "acct", "p1/s1", &service("web"))
            .await
            .is_err()
    );
    let page = engine.observe().operations(None).await.expect("operations");
    let first = &page.items[0];
    assert_eq!(first.state, OperationState::Unclear);
    assert_eq!(first.kind, "create");
    assert_eq!(first.target, "service web next to p1/s1");
    assert!(first.repeats);
    assert!(first.key.is_empty(), "the digest stays inside Hoista");

    // The identical create waits, and nothing reaches the provider.
    let again = adapter
        .provider_create(&token, "acct", "p1/s1", &service("web"))
        .await;
    assert!(
        matches!(&again, Err(EngineError::Provider { code, message, .. })
            if code == "operation.unclear" && message.contains(&first.id)),
        "{again:?}"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    // Something else still goes.
    adapter
        .provider_create(&token, "acct", "p1/s1", &service("worker"))
        .await
        .expect("another service");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);

    // Only the person can say what happened.
    let agent = Actor::Agent { id: "a".into() };
    assert!(
        engine
            .observe()
            .check_operation(&agent, &first.id, false)
            .await
            .is_err()
    );
    let checked = engine
        .observe()
        .check_operation(&Actor::Person, &first.id, false)
        .await
        .expect("checked");
    assert_eq!(checked.state, OperationState::DidNotHappen);
    assert!(
        engine
            .observe()
            .check_operation(&Actor::Person, &first.id, true)
            .await
            .is_err(),
        "an answered change stays answered"
    );
    adapter
        .provider_create(&token, "acct", "p1/s1", &service("web"))
        .await
        .expect("sent again once checked");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 3);

    // A change that's safe to repeat is written down but never waits, and
    // its value isn't kept.
    provider
        .answers
        .lock()
        .expect("provider fixture")
        .push(Some("provider.unreachable"));
    let value = Secret::new("very-secret-value-123".into());
    assert!(
        adapter
            .set_provider_variable(&token, "acct", "p1/s1", "API_KEY", &value)
            .await
            .is_err()
    );
    adapter
        .set_provider_variable(&token, "acct", "p1/s1", "API_KEY", &value)
        .await
        .expect("set again");
    let page = engine.observe().operations(None).await.expect("operations");
    let set: Vec<&Operation> = page
        .items
        .iter()
        .filter(|item| item.kind == "set_variable")
        .collect();
    assert_eq!(set.len(), 2);
    assert!(set.iter().any(|item| item.state == OperationState::Unclear));
    assert!(set.iter().any(|item| item.state == OperationState::Done));
    assert_eq!(set[0].target, "API_KEY on p1/s1");
    let mut database = std::fs::read(paths.database()).expect("database");
    database.extend(std::fs::read(paths.root().join("hoista.db-wal")).unwrap_or_default());
    assert!(String::from_utf8_lossy(&database).contains("API_KEY on p1/s1"));
    assert!(
        !String::from_utf8_lossy(&database).contains("very-secret-value-123"),
        "no value is kept"
    );
    store.close().await;
}

#[tokio::test]
async fn a_change_cut_off_by_a_stop_counts_as_unclear() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let token = Secret::new("t".into());
    let build = || {
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault),
            Arc::new(LocalInspector),
            vec![provider.clone()],
            Arc::new(LocalBuilder::new(paths.root())),
        )
    };
    // The first engine starts a create and stops before the provider answers.
    provider.hang.store(true, Ordering::SeqCst);
    let before = Arc::new(build());
    let running = {
        let engine = before.clone();
        let token = Secret::new("t".into());
        tokio::spawn(async move {
            let adapter = engine.adapter(Provider::Railway).expect("adapter").clone();
            let _ = adapter
                .provider_create(&token, "acct", "p1/s1", &service("web"))
                .await;
        })
    };
    while provider.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    // While it's being sent, the same create is refused as already running.
    let same = before
        .adapter(Provider::Railway)
        .expect("adapter")
        .provider_create(&token, "acct", "p1/s1", &service("web"))
        .await;
    assert!(matches!(same, Err(EngineError::Conflict(_))), "{same:?}");
    running.abort();
    let _ = running.await;
    drop(before);
    provider.hang.store(false, Ordering::SeqCst);

    // Hoista started again sees it as unclear and won't repeat it.
    let after = build();
    let page = after.observe().operations(None).await.expect("operations");
    let cut = &page.items[0];
    assert_eq!(cut.state, OperationState::Unclear);
    let adapter = after.adapter(Provider::Railway).expect("adapter").clone();
    assert!(
        adapter
            .provider_create(&token, "acct", "p1/s1", &service("web"))
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    after
        .observe()
        .check_operation(&Actor::Person, &cut.id, true)
        .await
        .expect("checked");
    adapter
        .provider_create(&token, "acct", "p1/s1", &service("web"))
        .await
        .expect("sent once checked");
    store.close().await;
}

#[tokio::test]
async fn a_command_that_failed_with_a_reason_is_failed_and_can_be_sent_again() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(LocalInspector),
        vec![provider.clone()],
        Arc::new(LocalBuilder::new(paths.root())),
    );
    let adapter = engine.adapter(Provider::Railway).expect("adapter").clone();
    let token = Secret::new("t".into());

    // The command ran to the end and said why it failed: nothing happened.
    provider
        .answers
        .lock()
        .expect("provider fixture")
        .push(Some("provider.command_failed"));
    assert!(
        adapter
            .provider_create(&token, "acct", "p1/s1", &service("web"))
            .await
            .is_err()
    );
    let page = engine.observe().operations(None).await.expect("operations");
    assert_eq!(page.items[0].state, OperationState::Failed);

    // The same change goes out again without anyone checking first.
    adapter
        .provider_create(&token, "acct", "p1/s1", &service("web"))
        .await
        .expect("sent again");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}
