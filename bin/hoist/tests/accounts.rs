//! Finding which account a token is for crosses the real CLI and IPC
//! boundary without saving the token or showing it.

use hoista_server::{
    Provider,
    application::{
        StateStore,
        engine::Engine,
        ports::{
            catalog::{CatalogRepository, EngineError},
            providers::{ProviderAdapter, RemoteResource},
            vault::TokenVault,
        },
    },
    client::LocalClient,
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
    server,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::AsyncWriteExt;
struct NoVault;
#[async_trait::async_trait]
impl TokenVault for NoVault {
    async fn read(&self, _: &str) -> Result<Secret, EngineError> {
        panic!("lookup must not read saved credentials")
    }

    async fn write(&self, _: &str, _: Secret) -> Result<(), EngineError> {
        panic!("lookup must not save credentials")
    }

    async fn delete(&self, _: &str) -> Result<(), EngineError> {
        panic!("lookup must not delete credentials")
    }
}

struct Accounts(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl ProviderAdapter for Accounts {
    fn provider(&self) -> Provider {
        Provider::Cloudflare
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
            discover: true,
            server: false,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        panic!("lookup must not connect")
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        panic!("lookup must not discover resources")
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let choice = AccountChoice {
            external_id: "first-id".into(),
            name: "Personal account".into(),
        };
        Ok(match token.expose() {
            "fixture-lookup-token" => vec![
                choice,
                AccountChoice {
                    external_id: "second-id".into(),
                    name: "Team account".into(),
                },
            ],
            "error-lookup-token" => {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "raw-code".into(),
                    message: token.expose().into(),
                });
            }
            "echo-lookup-token" => vec![AccountChoice {
                name: token.expose().into(),
                ..choice
            }],
            "escape-lookup-token" => vec![AccountChoice {
                name: "\x1b[31mUnsafe".into(),
                ..choice
            }],
            "duplicate-lookup-token" => vec![choice.clone(), choice],
            "oversized-lookup-token" => vec![AccountChoice {
                external_id: "a".repeat(257),
                ..choice
            }],
            "empty-id-lookup-token" => vec![AccountChoice {
                external_id: " ".into(),
                ..choice
            }],
            _ => vec![],
        })
    }
}

#[tokio::test]
async fn lookup_is_ephemeral_bounded_and_does_not_echo_provider_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(temp.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(NoVault),
        Arc::new(LocalInspector),
        vec![Arc::new(Accounts(calls.clone()))],
        Arc::new(LocalBuilder::new(paths.root())),
    ));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let daemon = {
        let paths = paths.clone();
        let store = store.clone();
        let stop = shutdown.clone();
        tokio::spawn(async move { server::serve(&paths, store, stop, engine).await })
    };
    let owner = LocalClient::new(&paths).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while owner.health().await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Without a terminal, `accounts add` stops and names the flag to pass
    // when it can't tell the account; it never saves or echoes the token.
    for (token, said) in [
        ("fixture-lookup-token", "second-id"),
        ("error-lookup-token", "Pass --account"),
        ("echo-lookup-token", "Pass --account"),
    ] {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_hoist"))
            .args([
                "accounts",
                "add",
                "cloudflare",
                "--token-stdin",
                "--no-input",
                "--state-dir",
            ])
            .arg(paths.root())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{token}\n").as_bytes())
            .await
            .unwrap();
        let output =
            tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains(said), "{stderr}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains(token));
        assert!(!stderr.contains(token));
    }
    for token in [
        "escape-lookup-token",
        "duplicate-lookup-token",
        "oversized-lookup-token",
        "empty-id-lookup-token",
    ] {
        let error = owner
            .post_json(
                "/accounts/lookup",
                &json!({"provider":"cloudflare","token":token}),
            )
            .await
            .unwrap_err();
        assert!(!error.to_string().contains(token));
        assert!(error.to_string().contains("explicit account ID"));
    }
    let count = calls.load(Ordering::SeqCst);
    for token in ["".into(), "a\nb".into(), "a".repeat(16385)] {
        assert!(
            owner
                .post_json(
                    "/accounts/lookup",
                    &json!({"provider":"cloudflare","token":token})
                )
                .await
                .is_err()
        );
    }
    assert!(owner.post_json("/accounts/lookup", &json!({"provider":"cloudflare","token":"fixture-lookup-token","url":"https://example.com"})).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), count);
    assert!(store.accounts(None).await.unwrap().items.is_empty());
    assert!(store.events_after(0).await.unwrap().is_empty());
    shutdown.cancel();
    daemon.await.unwrap().unwrap();
    store.close().await;
}
