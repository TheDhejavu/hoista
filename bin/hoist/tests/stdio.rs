//! Agent protocol and permission tests, using a real migrated SQLite store and
//! an in-memory vault.

use hoista_server::{
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::{CatalogRepository, EngineError},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);
#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, key: &str) -> Result<Secret, EngineError> {
        self.0
            .lock()
            .expect("MCP fixture")
            .get(key)
            .map(|s| Secret::new(s.clone()))
            .ok_or(EngineError::NotFound)
    }

    async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
        self.0
            .lock()
            .expect("MCP fixture")
            .insert(key.into(), value.expose().into());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), EngineError> {
        self.0.lock().expect("MCP fixture").remove(key);
        Ok(())
    }
}

async fn setup() -> (tempfile::TempDir, SqliteStore, Engine) {
    let temp = tempfile::tempdir().expect("MCP fixture");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("MCP fixture");
    let store = SqliteStore::open(&paths).await.expect("MCP fixture");
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault::default()),
        Arc::new(LocalInspector),
        vec![],
        Arc::new(LocalBuilder::new(paths.root())),
    )
    .with_owner_check(Arc::new(
        hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
    ));
    (temp, store, engine)
}

#[tokio::test]
async fn stdio_over_native_ipc_requires_initialization_and_owner_approval() {
    use hoista_server::{client::LocalClient, server};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (temp, store, engine) = setup().await;
    let paths = StatePaths::new(Some(temp.path().join("state"))).unwrap();
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "stdio-project".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .unwrap();
    let engine = Arc::new(engine);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server_task = {
        let paths = paths.clone();
        let store = store.clone();
        let engine = engine.clone();
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
    async fn cli(paths: &StatePaths, args: &[&str]) -> Value {
        let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_hoist"))
            .args(args)
            .args(["--json", "--yes", "--state-dir"])
            .arg(paths.root())
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    }
    assert_eq!(cli(&paths, &["accounts"]).await, json!([]));
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_hoist"));
    command
        .args(["mcp", "--name", "Scripted stdio", "--state-dir"])
        .env("HOISTA_NO_BROWSER", "1")
        .arg(paths.root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    // Read the bridge's stderr as it goes, so a full pipe never stalls it.
    let mut errors = child.stderr.take().unwrap();
    let stderr_task = tokio::spawn(async move {
        let mut text = String::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = tokio::io::AsyncReadExt::read(&mut errors, &mut buf)
                .await
                .unwrap();
            if n == 0 {
                break;
            }
            text.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        text
    });
    let mut write = child.stdin.take().unwrap();
    let mut read = tokio::io::BufReader::new(child.stdout.take().unwrap());
    let mut transcript = String::new();
    async fn exchange<W: tokio::io::AsyncWrite + Unpin, R: tokio::io::AsyncBufRead + Unpin>(
        write: &mut W,
        read: &mut R,
        value: Value,
        transcript: &mut String,
    ) -> Value {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        write.write_all(&bytes).await.unwrap();
        write.flush().await.unwrap();
        let mut line = String::new();
        let count = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            read.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(count > 0);
        let response: Value =
            serde_json::from_str(&line).expect("Only JSON-RPC may reach stdio stdout");
        assert_eq!(response["jsonrpc"], "2.0");
        transcript.push_str(&line);
        response
    }
    let blocked = exchange(
        &mut write,
        &mut read,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        &mut transcript,
    )
    .await;
    assert_eq!(blocked["error"]["code"], -32000);
    // The session starts at once, before the person has answered.
    let initialized = exchange(
        &mut write,
        &mut read,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {"elicitation": {}},
                "clientInfo": {"name": "fixture", "version": "1"},
            },
        }),
        &mut transcript,
    )
    .await;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), read.fill_buf())
            .await
            .is_err()
    );
    // Connecting asks straight away: the request is there before any tool call.
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let requests = owner.get_json("/agents/connection-requests").await.unwrap();
            if let Some(found) = requests["items"]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["name"] == "Scripted stdio"))
            {
                break found.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    let Ok(request) = waiting else {
        child.kill().await.unwrap();
        let log = stderr_task.await.unwrap();
        panic!("the bridge never asked to connect. Its stderr:\n{log}");
    };
    assert_eq!(request["code"].as_str().unwrap().len(), 7);
    owner
        .post_json(
            &format!(
                "/agents/connection-requests/{}",
                request["id"].as_str().unwrap()
            ),
            &json!({"allow": true, "scopes": ["apps:read", "apps:write"]}),
        )
        .await
        .unwrap();
    let tools = exchange(
        &mut write,
        &mut read,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
        &mut transcript,
    )
    .await;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "delete_project")
    );
    // A gated call asks the person in the client itself (MCP elicitation);
    // the model never sees the question or answers it.
    let question = exchange(
        &mut write,
        &mut read,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "delete_project", "arguments": {"id": project.id}},
        }),
        &mut transcript,
    )
    .await;
    assert_eq!(question["method"], "elicitation/create");
    assert!(
        question["params"]["message"]
            .as_str()
            .unwrap()
            .contains(&project.name)
    );
    // A wrong name deletes nothing.
    let wrong = exchange(&mut write,&mut read,json!({"jsonrpc":"2.0","id":question["id"],"result":{"action":"accept","content":{"name":"not-it"}}}),&mut transcript).await;
    assert_eq!(wrong["id"], 4);
    assert!(
        wrong["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("doesn't match")
    );
    assert!(store.project(&project.id).await.unwrap().is_some());
    // Declining deletes nothing either.
    let question = exchange(
        &mut write,
        &mut read,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "delete_project", "arguments": {"id": project.id}},
        }),
        &mut transcript,
    )
    .await;
    let declined = exchange(
        &mut write,
        &mut read,
        json!({"jsonrpc":"2.0","id":question["id"],"result":{"action":"decline"}}),
        &mut transcript,
    )
    .await;
    assert!(
        declined["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("said no")
    );
    assert!(store.project(&project.id).await.unwrap().is_some());
    // Typing the name allows it, and the same call finishes.
    let question = exchange(
        &mut write,
        &mut read,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "delete_project", "arguments": {"id": project.id}},
        }),
        &mut transcript,
    )
    .await;
    let deleted = exchange(&mut write,&mut read,json!({"jsonrpc":"2.0","id":question["id"],"result":{"action":"accept","content":{"name":project.name}}}),&mut transcript).await;
    assert_eq!(deleted["id"], 6);
    assert_eq!(deleted["result"]["isError"], false, "{deleted}");
    assert!(store.project(&project.id).await.unwrap().is_none());
    write.write_all(b"broken json\n").await.unwrap();
    let mut invalid = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        read.read_line(&mut invalid),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&invalid).unwrap()["error"]["code"],
        -32700
    );
    transcript.push_str(&invalid);
    let agents = store.agents(None).await.unwrap();
    assert_eq!(agents.items.len(), 1);
    let token = engine
        .vault
        .read(&format!("agent/{}", agents.items[0].id))
        .await
        .unwrap();
    assert!(!transcript.contains(token.expose()));
    write.shutdown().await.unwrap();
    drop(write);
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // The waiting state goes to stderr, never stdout, and never the credential.
    let stderr = stderr_task.await.unwrap();
    assert!(stderr.contains("is asking to connect"));
    assert!(!stderr.contains(token.expose()));
    assert_eq!(cli(&paths, &["agents"]).await.as_array().unwrap().len(), 1);
    cli(
        &paths,
        &["agents", "access", &agents.items[0].id, "--access", "view"],
    )
    .await;
    // A later session is never connected by its name alone. With a keychain
    // it reuses the credential it saved; without one (as here) it asks again,
    // and allowing that makes a fresh identity.
    async fn allow_named(owner: &LocalClient, name: &str) {
        let wait = async {
            loop {
                let requests = owner.get_json("/agents/connection-requests").await.unwrap();
                if let Some(found) = requests["items"]
                    .as_array()
                    .and_then(|items| items.iter().find(|item| item["name"] == name))
                {
                    owner
                        .post_json(
                            &format!(
                                "/agents/connection-requests/{}",
                                found["id"].as_str().unwrap()
                            ),
                            &json!({"allow": true}),
                        )
                        .await
                        .unwrap();
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(20), wait)
            .await
            .expect(
                "no connection request appeared. On a machine with a keychain the session \
                 reuses its saved credential and never asks again; this test expects none",
            );
    }
    let mut resumed = command.spawn().unwrap();
    let mut resumed_input = resumed.stdin.take().unwrap();
    let mut resumed_output = tokio::io::BufReader::new(resumed.stdout.take().unwrap());
    let response = exchange(
        &mut resumed_input,
        &mut resumed_output,
        json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "initialize",
            "params": {"clientInfo": {"name": "Ignored by explicit label"}},
        }),
        &mut transcript,
    )
    .await;
    assert!(response.get("result").is_some());
    allow_named(&owner, "Scripted stdio").await;
    let access = exchange(
        &mut resumed_input,
        &mut resumed_output,
        json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {"name": "get_my_access", "arguments": {}},
        }),
        &mut transcript,
    )
    .await;
    assert_eq!(access["result"]["isError"], false);
    let current = store.agents(None).await.unwrap();
    assert_eq!(current.items.len(), 2);
    // The first identity keeps the access the person chose.
    let first = current
        .items
        .iter()
        .find(|agent| agent.id == agents.items[0].id)
        .unwrap();
    assert_eq!(first.scopes, vec!["apps:read"]);
    drop(resumed_input);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), resumed.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    cli(&paths, &["agents", "revoke", &agents.items[0].id]).await;
    let reconnected = cli(&paths, &["agents", "rotate", &agents.items[0].id]).await;
    assert_eq!(reconnected["scopes"], json!(["apps:read"]));
    assert!(reconnected.get("credential").is_none());
    assert!(
        engine
            .agents()
            .authenticate_agent(token.expose())
            .await
            .is_err()
    );

    // With no explicit label, standard MCP clientInfo names the request.
    for name in ["Client A", "Client B"] {
        let mut process = tokio::process::Command::new(env!("CARGO_BIN_EXE_hoist"));
        process
            .args(["mcp", "--state-dir"])
            .arg(paths.root())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut process = process.spawn().unwrap();
        let mut input = process.stdin.take().unwrap();
        let mut output = tokio::io::BufReader::new(process.stdout.take().unwrap());
        let result = exchange(
            &mut input,
            &mut output,
            json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "initialize",
                "params": {"clientInfo": {"name": name}},
            }),
            &mut transcript,
        )
        .await;
        assert!(result.get("result").is_some());
        allow_named(&owner, name).await;
        drop(input);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(3), process.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
    let identities = store.agents(None).await.unwrap();
    assert_eq!(identities.items.len(), 4);
    assert!(
        identities
            .items
            .iter()
            .any(|agent| agent.name == "Client A")
    );
    assert!(
        identities
            .items
            .iter()
            .any(|agent| agent.name == "Client B")
    );
    // A frame beyond the documented limit must fail without requiring a newline
    // or echoing any submitted bytes into the protocol/error output.
    let mut oversized = command.spawn().unwrap();
    let mut input = oversized.stdin.take().unwrap();
    input.write_all(&vec![b'x'; 128 * 1024 + 1]).await.unwrap();
    drop(input);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        oversized.wait_with_output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("MCP message exceeds 128 KiB"));
    assert_eq!(store.agents(None).await.unwrap().items.len(), 4);
    // Global pause blocks existing credentials without changing their scopes or
    // revocation status; resume never restores a revoked connection.
    let current_token = engine
        .vault
        .read(&format!("agent/{}", agents.items[0].id))
        .await
        .unwrap();
    assert!(
        engine
            .agents()
            .authenticate_agent(current_token.expose())
            .await
            .is_ok()
    );
    let before_pause = store.agents(None).await.unwrap().items;
    assert_eq!(
        cli(&paths, &["agents", "pause"]).await["agents_enabled"],
        false
    );
    assert!(matches!(
        engine
            .agents()
            .authenticate_agent(current_token.expose())
            .await,
        Err(EngineError::AgentsDisabled)
    ));
    assert_eq!(store.agents(None).await.unwrap().items, before_pause);
    cli(&paths, &["agents", "revoke", &agents.items[0].id]).await;
    assert_eq!(
        cli(&paths, &["agents", "resume"]).await["agents_enabled"],
        true
    );
    assert!(
        engine
            .agents()
            .authenticate_agent(current_token.expose())
            .await
            .is_err()
    );
    assert!(
        store
            .agent(&agents.items[0].id)
            .await
            .unwrap()
            .unwrap()
            .revoked
    );
    // CLI disconnection removes only credentials; it never calls a provider.
    let account = Account {
        id: "cli-account-fixture".into(),
        provider: hoista_server::Provider::Cloudflare,
        label: "Fixture".into(),
        external_id: "remote-fixture".into(),
        scopes: vec!["apps:read".into()],
        status: AccountStatus::Connected,
        connected_at: "2026-11-02T00:00:00Z".into(),
        last_discovery: None,
        credential: None,
        in_1password: false,
    };
    store.put_account(&account).await.unwrap();
    engine
        .vault
        .write(
            "account/cli-account-fixture",
            Secret::new("fixture-token".into()),
        )
        .await
        .unwrap();
    let disconnected = cli(&paths, &["accounts", "remove", &account.id]).await;
    assert_eq!(disconnected["removed"], true);
    assert!(
        engine
            .vault
            .read("account/cli-account-fixture")
            .await
            .is_err()
    );
    let saved = store.account(&account.id).await.unwrap().unwrap();
    assert_eq!(saved.status, AccountStatus::Disconnected);
    assert_eq!(saved.external_id, "remote-fixture");
    shutdown.cancel();
    server_task.await.unwrap().unwrap();
    engine.observe().stop_logs().await;
    store.close().await;
}
