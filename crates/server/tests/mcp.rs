//! MCP and agent permission tests, on a real migrated SQLite store and an
//! in-memory vault.

use hoista_server::{
    application::{
        engine::{Actor, Engine, EnrollmentStatus},
        ports::{
            catalog::{CatalogRepository, EngineError},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
    mcp,
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

/// Remote agents reach Hoista on a server at this address.
const SERVER: &str = "https://hoista.example.com";
async fn setup() -> (tempfile::TempDir, SqliteStore, Engine) {
    setup_with(vec![]).await
}

/// An engine with these provider adapters.
async fn setup_with(
    adapters: Vec<Arc<dyn hoista_server::application::ports::providers::ProviderAdapter>>,
) -> (tempfile::TempDir, SqliteStore, Engine) {
    let temp = tempfile::tempdir().expect("MCP fixture");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("MCP fixture");
    let store = SqliteStore::open(&paths).await.expect("MCP fixture");
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault::default()),
        Arc::new(LocalInspector),
        adapters,
        Arc::new(LocalBuilder::new(paths.root())),
    )
    .with_owner_check(Arc::new(
        hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
    ));
    (temp, store, engine)
}

async fn call(engine: &Engine, actor: &Actor, name: &str, args: Value) -> Value {
    mcp::dispatch(
        engine,
        actor,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        }),
    )
    .await
    .expect("MCP fixture")
}

#[tokio::test]
async fn enrollment_rejects_forgery_revocation_and_expiry() {
    let (_temp, store, engine) = setup().await;
    let (mut agent, token) = engine
        .agents()
        .register_agent(
            "Test".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("MCP fixture");
    assert!(
        engine
            .agents()
            .authenticate_agent(token.expose())
            .await
            .is_ok()
    );
    assert!(
        engine
            .agents()
            .authenticate_agent(&format!("{}.wrong", agent.id))
            .await
            .is_err()
    );
    agent.revoked = true;
    store.put_agent(&agent).await.expect("MCP fixture");
    assert!(
        engine
            .agents()
            .authenticate_agent(token.expose())
            .await
            .is_err()
    );
    agent.revoked = false;
    agent.last_seen = "2020-01-01T00:00:00.000Z".into();
    store.put_agent(&agent).await.expect("MCP fixture");
    assert!(
        engine
            .agents()
            .authenticate_agent(token.expose())
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn tools_enforce_scopes_strict_inputs_and_never_echo_secret_arguments() {
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Read only".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("MCP fixture");
    let actor = Actor::Agent { id: agent.id };
    assert_eq!(
        call(&engine, &actor, "list_projects", json!({})).await["result"]["isError"],
        false
    );
    let secret = "not-for-output-or-audit";
    let response = call(
        &engine,
        &actor,
        "set_secret",
        json!({"project_id":"a","resource_id":"p","name":"SECRET","value":secret}),
    )
    .await;
    assert_eq!(response["result"]["isError"], true);
    assert!(!response.to_string().contains(secret));
    let failure: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("tool failure text"),
    )
    .expect("tool failure JSON");
    assert!(failure["recovery"]["next_step"].is_string());
    assert_eq!(
        call(&engine, &actor, "list_projects", json!({"actor":"person"})).await["result"]["isError"],
        true
    );
    use hoista_server::application::StateStore;
    assert!(!format!("{:?}", store.events_after(0).await.expect("MCP fixture")).contains(secret));
    assert!(
        mcp::dispatch(
            &engine,
            &actor,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .await
        .is_none()
    );
    assert_eq!(
        mcp::dispatch(&engine, &actor, json!([]))
            .await
            .expect("MCP fixture")["error"]["code"],
        -32600
    );
    store.close().await;
}

#[tokio::test]
async fn gated_tool_is_pending_until_exact_approval_and_cannot_be_reused() {
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Writer".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("MCP fixture");
    let actor = Actor::Agent { id: agent.id };
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "empty-project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("MCP fixture");
    let pending = call(&engine, &actor, "delete_project", json!({"id":project.id})).await;
    let approval = pending["result"]["structuredContent"]["approval_id"]
        .as_str()
        .expect("MCP fixture");
    assert!(
        store
            .project(&project.id)
            .await
            .expect("MCP fixture")
            .is_some()
    );
    let blocked = call(
        &engine,
        &actor,
        "delete_project",
        json!({"id":project.id,"approval_id":approval}),
    )
    .await;
    assert_eq!(blocked["result"]["isError"], true);
    store
        .decide_approval(approval, true, &hoista_server::application::engine::now())
        .await
        .expect("MCP fixture");
    let deleted = call(
        &engine,
        &actor,
        "delete_project",
        json!({"id":project.id,"approval_id":approval}),
    )
    .await;
    assert_eq!(deleted["result"]["isError"], false);
    assert!(
        store
            .project(&project.id)
            .await
            .expect("MCP fixture")
            .is_none()
    );
    assert_eq!(
        store
            .approval(approval)
            .await
            .expect("MCP fixture")
            .expect("MCP fixture")
            .decision,
        ApprovalDecision::Consumed
    );
    assert_eq!(
        call(
            &engine,
            &actor,
            "delete_project",
            json!({"id":project.id,"approval_id":approval})
        )
        .await["result"]["isError"],
        true
    );
    store.close().await;
}

#[tokio::test]
async fn oauth_codes_require_approval_pkce_exact_redirect_and_audience_and_are_single_use() {
    use base64::Engine as _;
    use hoista_server::application::engine::AuthorizationRequest;
    use sha2::{Digest, Sha256};
    let (_temp, store, engine) = setup().await;
    let engine = engine.with_public_url(SERVER.into());
    assert!(
        engine
            .accounts()
            .oauth_register(
                "Bad client".into(),
                vec!["https://example.com/callback#fragment".into()]
            )
            .await
            .is_err()
    );
    let client = engine
        .accounts()
        .oauth_register(
            "Chat fixture".into(),
            vec!["https://example.com/callback".into()],
        )
        .await
        .expect("client");
    let verifier = "v".repeat(43);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let resource = format!("{SERVER}/mcp");
    let (pending, code) = engine
        .accounts()
        .oauth_authorize(AuthorizationRequest {
            client_id: client.id.clone(),
            redirect_uri: client.redirect_uris[0].clone(),
            state: "client-state".into(),
            challenge,
            resource: resource.clone(),
            scopes: vec!["apps:read".into()],
        })
        .await
        .expect("authorization");
    assert!(
        engine
            .vault
            .read(&format!("agent/{}", pending.agent_id))
            .await
            .is_err(),
        "unapproved clients do not allocate a vault credential"
    );
    assert!(
        engine
            .accounts()
            .oauth_token(
                code.expose(),
                &client.id,
                &client.redirect_uris[0],
                &verifier,
                &resource
            )
            .await
            .is_err()
    );
    store
        .decide_approval(
            &pending.approval_id,
            true,
            &hoista_server::application::engine::now(),
        )
        .await
        .expect("approval");
    for (id, redirect, verify, audience) in [
        (
            "other",
            client.redirect_uris[0].as_str(),
            verifier.as_str(),
            resource.as_str(),
        ),
        (
            client.id.as_str(),
            "https://attacker.example/callback",
            verifier.as_str(),
            resource.as_str(),
        ),
        (
            client.id.as_str(),
            client.redirect_uris[0].as_str(),
            "wrong-verifier",
            resource.as_str(),
        ),
        (
            client.id.as_str(),
            client.redirect_uris[0].as_str(),
            verifier.as_str(),
            "https://other.example/mcp",
        ),
    ] {
        assert!(
            engine
                .accounts()
                .oauth_token(code.expose(), id, redirect, verify, audience)
                .await
                .is_err()
        );
    }
    let token = engine
        .accounts()
        .oauth_token(
            code.expose(),
            &client.id,
            &client.redirect_uris[0],
            &verifier,
            &resource,
        )
        .await
        .expect("grant");
    assert!(
        engine
            .agents()
            .authenticate_agent(token.expose())
            .await
            .is_ok()
    );
    assert!(
        engine
            .accounts()
            .oauth_token(
                code.expose(),
                &client.id,
                &client.redirect_uris[0],
                &verifier,
                &resource
            )
            .await
            .is_err()
    );
    assert!(
        !serde_json::to_string(&engine.data().export().await.expect("backup"))
            .expect("JSON")
            .contains(token.expose())
    );
    store.close().await;
}

#[tokio::test]
async fn metadata_restore_is_atomic_disconnected_and_read_only() {
    let (_source_dir, source, engine) = setup().await;
    engine
        .projects()
        .create_project(
            &Actor::Person,
            "portable".into(),
            Source::Folder {
                path: "/source".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let backup = engine.data().export().await.expect("backup");
    let (_target_dir, target, destination) = setup().await;
    destination
        .data()
        .restore(backup.clone())
        .await
        .expect("restore");
    assert_eq!(
        target.projects(None).await.expect("projects").items[0].mode,
        Mode::Watching
    );
    assert!(destination.data().restore(backup.clone()).await.is_err());
    let (_empty_dir, empty, receiver) = setup().await;
    let mut broken = backup;
    broken.resources.push(Resource {
        id: "bad".into(),
        project_id: None,
        kind: ResourceKind::Web,
        provider: hoista_server::Provider::Cloudflare,
        account_id: "missing".into(),
        provider_ref: "worker:bad".into(),
        name: "bad".into(),
        region: None,
        status: ResourceStatus::Live,
        mode: Mode::Managed,
        url: None,
        group: None,
    });
    assert!(receiver.data().restore(broken).await.is_err());
    assert!(
        empty
            .projects(None)
            .await
            .expect("empty projects")
            .items
            .is_empty()
    );
    source.close().await;
    target.close().await;
    empty.close().await;
}

#[tokio::test]
async fn removing_token_keeps_provider_resources_and_disables_stale_credentials() {
    let (_temp, store, engine) = setup().await;
    let account = Account {
        last_discovery: None,
        id: "cloud".into(),
        provider: hoista_server::Provider::Cloudflare,
        label: "Personal".into(),
        external_id: "remote".into(),
        scopes: vec!["write".into()],
        status: AccountStatus::Connected,
        connected_at: "2026-11-02T00:00:00Z".into(),
        credential: None,
        in_1password: false,
    };
    store.put_account(&account).await.expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "kept-project".into(),
            Source::Folder {
                path: "/tmp/app".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let resource = Resource {
        id: "web".into(),
        project_id: Some(project.id.clone()),
        kind: ResourceKind::Web,
        provider: account.provider,
        account_id: account.id.clone(),
        provider_ref: "worker:kept".into(),
        name: "kept-project".into(),
        region: None,
        status: ResourceStatus::Live,
        mode: Mode::Managed,
        url: Some("https://kept.example.workers.dev".into()),
        group: None,
    };
    store.put_resource(&resource).await.expect("resource");
    engine
        .vault
        .write("account/cloud", Secret::new("fixture-token".into()))
        .await
        .expect("token");
    assert!(engine.accounts().account_token("cloud").await.is_ok());
    // No provider adapters are installed: success also proves no remote mutation is needed.
    engine
        .accounts()
        .disconnect_account("cloud")
        .await
        .expect("disconnect");
    assert!(engine.vault.read("account/cloud").await.is_err());
    assert_eq!(
        store.resource("web").await.expect("resource"),
        Some(resource)
    );
    assert_eq!(
        store.project(&project.id).await.expect("project"),
        Some(project)
    );
    let disconnected = store
        .account("cloud")
        .await
        .expect("account")
        .expect("retained");
    assert_eq!(disconnected.status, AccountStatus::Disconnected);
    assert!(disconnected.scopes.is_empty());
    // Even if a credential remains/reappears in the keychain, catalog revocation wins.
    engine
        .vault
        .write("account/cloud", Secret::new("stale-token".into()))
        .await
        .expect("token");
    assert!(engine.accounts().account_token("cloud").await.is_err());
    engine
        .accounts()
        .disconnect_account("cloud")
        .await
        .expect("safe retry");
    assert!(engine.vault.read("account/cloud").await.is_err());
    store.close().await;
}

#[tokio::test]
async fn queued_cancellation_survives_restart_and_stops_before_source_or_provider_access() {
    let (_temp, store, engine) = setup().await;
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "cancelled-project".into(),
            Source::Folder {
                path: "/nonexistent-source".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let queued = store
        .enqueue(&Deploy {
            title: None,
            channel: DeployChannel::Desktop,
            started_by_name: None,
            environment_keys: vec![],
            failure: None,
            release_version: None,
            provenance: None,
            source_observation: None,
            cancel_requested: false,
            plan_id: "nonexistent-plan".into(),
            releases: vec![],
            id: "cancel-before-start".into(),
            project_id: project.id,
            version: 0,
            source: project.source,
            started_by: "person".into(),
            status: DeployStatus::Queued,
            steps: vec![DeployStep {
                name: "detect".into(),
                status: StepStatus::Pending,
                message: None,
            }],
            url: None,
            started_at: "2026-11-02T00:00:00Z".into(),
            finished_at: None,
            duration_ms: None,
            rollback_of: None,
        })
        .await
        .expect("queue");
    engine
        .deploys()
        .cancel_deploy(&Actor::Person, &queued.id)
        .await
        .expect("cancel");
    assert!(store.queued().await.expect("queue")[0].cancel_requested);
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(Arc::new(engine).run_deployments(stop.clone()));
    let finished = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let value = store
                .deploy(&queued.id)
                .await
                .expect("deploy")
                .expect("exists");
            if value.status == DeployStatus::Failed {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    stop.cancel();
    worker
        .await
        .expect("joined worker")
        .expect("worker shutdown");
    let finished = finished.expect("cancellation completed");
    assert!(finished.cancel_requested);
    assert!(finished.releases.is_empty());
    let failure = finished.failure.as_ref().expect("cancellation evidence");
    assert_eq!(failure.code, "deploy.cancelled");
    assert!(failure.message.contains("cancelled at a safe boundary"));
    assert!(failure.steps.is_empty());
    assert!(!failure.release_started);
    assert!(finished.steps[0].message.is_none());
    store.close().await;
}

#[tokio::test]
async fn unapproved_oauth_requests_are_bounded_and_expired_identities_are_pruned() {
    use base64::Engine as _;
    use hoista_server::application::engine::AuthorizationRequest;
    use sha2::{Digest, Sha256};
    let (_temp, store, engine) = setup().await;
    let engine = engine.with_public_url(SERVER.into());
    let client = engine
        .accounts()
        .oauth_register(
            "Unverified client".into(),
            vec!["https://client.example/callback".into()],
        )
        .await
        .expect("client");
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest("a".repeat(43).as_bytes()));
    let request = || AuthorizationRequest {
        client_id: client.id.clone(),
        redirect_uri: client.redirect_uris[0].clone(),
        state: "state".into(),
        challenge: challenge.clone(),
        resource: format!("{SERVER}/mcp"),
        scopes: vec!["apps:read".into()],
    };
    let mut last = None;
    for _ in 0..100 {
        let (pending, _) = engine
            .accounts()
            .oauth_authorize(request())
            .await
            .expect("within limit");
        assert!(
            engine
                .vault
                .read(&format!("agent/{}", pending.agent_id))
                .await
                .is_err()
        );
        last = Some(pending);
    }
    assert!(engine.accounts().oauth_authorize(request()).await.is_err());
    assert_eq!(store.agents(None).await.expect("agents").items.len(), 100);
    assert!(
        store
            .agents(None)
            .await
            .expect("agents")
            .next_cursor
            .is_none()
    );
    let mut expired = last.expect("pending");
    expired.expires_at = "2020-01-01T00:00:00Z".into();
    store
        .put_oauth_pending(&expired)
        .await
        .expect("expire fixture");
    engine
        .accounts()
        .oauth_authorize(request())
        .await
        .expect("expired slot reclaimed");
    assert!(
        store
            .agent(&expired.agent_id)
            .await
            .expect("agent read")
            .is_none()
    );
    assert!(
        store
            .approval(&expired.approval_id)
            .await
            .expect("approval read")
            .is_none()
    );
    assert_eq!(store.agents(None).await.expect("agents").items.len(), 100);
    store.close().await;
}

#[tokio::test]
async fn permission_edits_preserve_credentials_and_concurrent_activity_cannot_undo_revocation() {
    let (_temp, store, engine) = setup().await;
    let engine = Arc::new(engine);
    let (agent, token) = engine
        .agents()
        .register_agent(
            "Editable".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("register");
    let actor = engine
        .agents()
        .authenticate_agent(token.expose())
        .await
        .expect("authenticate");
    let edited = engine
        .agents()
        .set_agent_scopes(&Actor::Person, &agent.id, vec!["apps:read".into()])
        .await
        .expect("narrow permissions");
    assert_eq!(
        edited.last_seen, agent.last_seen,
        "editing must not extend inactivity expiry"
    );
    assert!(
        engine
            .agents()
            .authenticate_agent(token.expose())
            .await
            .is_ok(),
        "credential remains valid"
    );
    assert!(engine.authorize(&actor, "apps:write").await.is_err());
    assert!(
        engine
            .agents()
            .set_agent_scopes(&actor, &agent.id, vec!["apps:write".into()])
            .await
            .is_err(),
        "agents cannot expand their own access"
    );
    for scopes in [
        vec![],
        vec!["apps:read".into(), "apps:read".into()],
        vec!["admin".into()],
    ] {
        assert!(
            engine
                .agents()
                .set_agent_scopes(&Actor::Person, &agent.id, scopes)
                .await
                .is_err()
        );
    }
    let barrier = Arc::new(tokio::sync::Barrier::new(21));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..20 {
        let engine = engine.clone();
        let barrier = barrier.clone();
        let id = agent.id.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            engine.authorize(&Actor::Agent { id }, "apps:read").await
        });
    }
    barrier.wait().await;
    store
        .edit_agent("person", &agent.id, None, true)
        .await
        .expect("revoke");
    while let Some(result) = tasks.join_next().await {
        let _authorized_before_revocation = result.expect("activity task");
    }
    let saved = store
        .agent(&agent.id)
        .await
        .expect("agent")
        .expect("exists");
    assert!(saved.revoked, "activity cannot undo revocation");
    assert_eq!(saved.scopes, vec!["apps:read"]);
    assert!(
        engine
            .agents()
            .authenticate_agent(token.expose())
            .await
            .is_err()
    );
    assert!(
        engine
            .agents()
            .set_agent_scopes(&Actor::Person, &agent.id, vec!["apps:write".into()])
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn pausing_agent_access_blocks_requests_and_enrollment_without_revoking_credentials() {
    let (_temp, store, engine) = setup().await;
    let engine = Arc::new(engine);
    use tower::ServiceExt;
    let (agent, token) = engine
        .agents()
        .register_agent(
            "Pause fixture".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("register");
    let actor = engine
        .agents()
        .authenticate_agent(token.expose())
        .await
        .expect("actor");
    assert!(
        engine
            .settings()
            .update_settings(
                &actor,
                SettingsPatch {
                    agents_enabled: Some(false),
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    engine
        .settings()
        .update_settings(
            &Actor::Person,
            SettingsPatch {
                agents_enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("pause");
    assert!(matches!(
        engine.agents().authenticate_agent(token.expose()).await,
        Err(EngineError::AgentsDisabled)
    ));
    assert!(matches!(
        engine.authorize(&actor, "apps:read").await,
        Err(EngineError::AgentsDisabled)
    ));
    assert!(matches!(
        store
            .authorize_agent(&agent.id, "apps:read", &chrono::Utc::now().to_rfc3339(), 30)
            .await,
        Err(EngineError::AgentsDisabled)
    ));
    assert!(matches!(
        engine
            .agents()
            .register_agent(
                "Blocked".into(),
                vec!["apps:read".into()],
                AgentTransport::Stdio
            )
            .await,
        Err(EngineError::AgentsDisabled)
    ));
    assert!(matches!(
        engine
            .accounts()
            .oauth_register(
                "Blocked".into(),
                vec!["https://example.com/callback".into()]
            )
            .await,
        Err(EngineError::AgentsDisabled)
    ));
    assert!(matches!(
        engine
            .accounts()
            .oauth_token(
                "code",
                "client",
                "https://example.com/callback",
                "verifier",
                "resource"
            )
            .await,
        Err(EngineError::AgentsDisabled)
    ));
    let response = mcp::router(engine.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("authorization", format!("Bearer {}", token.expose()))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .expect("body");
    let error: Value = serde_json::from_slice(&bytes).expect("error");
    assert_eq!(error["code"], "agent.disabled");
    assert_eq!(error["recovery"]["retry"], "owner_decision");
    engine
        .authorize(&Actor::Person, "apps:write")
        .await
        .expect("owner retains access");
    assert!(
        !store
            .agent(&agent.id)
            .await
            .expect("agent")
            .expect("exists")
            .revoked
    );
    engine
        .settings()
        .update_settings(
            &Actor::Person,
            SettingsPatch {
                agents_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("resume");
    engine
        .agents()
        .authenticate_agent(token.expose())
        .await
        .expect("same credential");
    engine
        .authorize(&actor, "apps:read")
        .await
        .expect("access restored");
    store.close().await;
}

#[tokio::test]
async fn create_project_tool_requires_explicit_creation_scope() {
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Deployer".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("agent");
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let args = json!({"name":"new-project","source":{"kind":"folder","path":"/fixture"}});
    assert_eq!(
        call(&engine, &actor, "create_project", args.clone()).await["result"]["isError"],
        true
    );
    assert!(
        store
            .projects(None)
            .await
            .expect("projects")
            .items
            .is_empty()
    );
    engine
        .agents()
        .set_agent_scopes(
            &Actor::Person,
            &agent.id,
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
        )
        .await
        .expect("grant creation");
    assert_eq!(
        call(&engine, &actor, "create_project", args).await["result"]["isError"],
        false
    );
    assert_eq!(store.projects(None).await.expect("projects").items.len(), 1);
    assert!(mcp::tools().iter().any(|tool| tool["name"] == "add_domain"));
    store.close().await;
}

#[tokio::test]
async fn deletion_approvals_bind_reviewed_impact_and_reject_later_changes() {
    use hoista_server::domain::deletion::DeletionConfirmation;
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Writer".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent { id: agent.id };
    let mut project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "before".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .unwrap();
    let request = engine
        .request_approval(
            &actor,
            GatedAction::DeleteProject,
            project.id.clone(),
            "Remove this project".into(),
        )
        .await
        .unwrap();
    let snapshot = request.deletion.as_ref().unwrap();
    let confirmation = DeletionConfirmation {
        backup_databases: true,
        confirmation_name: snapshot.confirmation_name.clone(),
        fingerprint: snapshot.fingerprint.clone(),
    };
    assert!(
        engine
            .projects()
            .decide_approval(&request.id, true, None)
            .await
            .is_err()
    );
    engine
        .projects()
        .decide_approval(&request.id, true, Some(&confirmation))
        .await
        .unwrap();
    project.name = "after".into();
    store.put_project(&project).await.unwrap();
    assert!(
        engine
            .deploys()
            .delete_project(&actor, &project.id, Some(&request.id), None)
            .await
            .is_err()
    );
    assert_eq!(
        store.approval(&request.id).await.unwrap().unwrap().decision,
        ApprovalDecision::Allowed
    );
    assert!(store.project(&project.id).await.unwrap().is_some());
    let replacement = engine
        .request_approval(
            &actor,
            GatedAction::DeleteProject,
            project.id.clone(),
            "Remove reviewed project".into(),
        )
        .await
        .unwrap();
    let snapshot = replacement.deletion.as_ref().unwrap();
    let confirmation = DeletionConfirmation {
        backup_databases: true,
        confirmation_name: snapshot.confirmation_name.clone(),
        fingerprint: snapshot.fingerprint.clone(),
    };
    engine
        .projects()
        .decide_approval(&replacement.id, true, Some(&confirmation))
        .await
        .unwrap();
    engine
        .deploys()
        .delete_project(&actor, &project.id, Some(&replacement.id), None)
        .await
        .unwrap();
    assert!(store.project(&project.id).await.unwrap().is_none());
    // Legacy JSON still reads, but cannot represent a reviewed destructive grant.
    let mut json = serde_json::to_value(&replacement).unwrap();
    json.as_object_mut().unwrap().remove("deletion");
    assert!(
        serde_json::from_value::<Approval>(json)
            .unwrap()
            .deletion
            .is_none()
    );
}

#[tokio::test]
async fn desktop_deletion_requires_typed_current_preview_and_exposes_impact() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let (_temp, store, engine) = setup().await;
    let mut project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "typed-name".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .unwrap();
    let router = hoista_server::catalog::router(Arc::new(engine));
    let path = format!("/projects/{}", project.id);
    let preview = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{path}/deletion-preview"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let preview: Value =
        serde_json::from_slice(&to_bytes(preview.into_body(), 128 * 1024).await.unwrap()).unwrap();
    assert_eq!(preview["resources"], json!([]));
    assert_eq!(preview["database_export_available"], false);
    let payload = json!({"confirmation_name":"typed-name","fingerprint":preview["fingerprint"]});
    for body in [
        json!({}),
        json!({"confirmation_name":"wrong","fingerprint":preview["fingerprint"]}),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(matches!(
            response.status(),
            StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY
        ));
    }
    project.name = "changed-name".into();
    store.put_project(&project).await.unwrap();
    let stale = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&path)
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let preview = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{path}/deletion-preview"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let preview: Value =
        serde_json::from_slice(&to_bytes(preview.into_body(), 128 * 1024).await.unwrap()).unwrap();
    let payload = json!({"confirmation_name":preview["confirmation_name"],"fingerprint":preview["fingerprint"]});
    let deleted = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&path)
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
    assert!(store.project(&project.id).await.unwrap().is_none());
}

#[tokio::test]
async fn desktop_import_defaults_to_watching_and_links_source_without_provider_calls() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let (temp, store, engine) = setup().await;
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: hoista_server::Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    store
        .put_resource(&Resource {
            id: "resource".into(),
            project_id: None,
            kind: ResourceKind::Web,
            provider: hoista_server::Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:fixture".into(),
            name: "fixture".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Watching,
            url: None,
            group: None,
        })
        .await
        .unwrap();
    let (agent, _) = engine
        .agents()
        .register_agent(
            "fixture".into(),
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    assert!(matches!(
        engine
            .projects()
            .import_project(
                &Actor::Agent { id: agent.id },
                "fixture".into(),
                None,
                Mode::Managed,
                &["resource".into()]
            )
            .await,
        Err(EngineError::ApprovalRequired)
    ));
    assert!(store.projects(None).await.unwrap().items.is_empty());
    // This engine has no provider adapters: importing and linking must stay local.
    let router = hoista_server::catalog::router(Arc::new(engine));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import/projects")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name":"imported","resource_ids":["resource"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap();
    assert_eq!(result["project"]["source"], json!({"kind":"unlinked"}));
    assert_eq!(result["project"]["mode"], "watching");
    assert_eq!(result["resources"][0]["mode"], "watching");
    let id = result["project"]["id"].as_str().unwrap();
    let path = temp.path().join("source");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(
        path.join("index.html"),
        "<!doctype html><title>fixture</title>",
    )
    .unwrap();
    for (source, expected) in [
        (
            json!({"kind":"folder","path":"."}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"kind":"folder","path":temp.path().join("absent")}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (json!({"kind":"folder","path":path}), StatusCode::OK),
        // A watched project's code can be linked again.
        (json!({"kind":"folder","path":path}), StatusCode::OK),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/projects/{id}/source"))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"source":source}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    assert_eq!(
        store.project(id).await.unwrap().unwrap().mode,
        Mode::Watching
    );
    assert_eq!(
        store.resource("resource").await.unwrap().unwrap().mode,
        Mode::Watching
    );
    store.close().await;
}

#[tokio::test]
async fn desktop_and_mcp_follow_collection_cursors_without_exposing_secret_fingerprints() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let (_temp, store, engine) = setup().await;
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "pages".into(),
            Source::Unlinked,
            Mode::Watching,
        )
        .await
        .unwrap();
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: hoista_server::Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "fixture".into(),
            scopes: vec![],
            status: AccountStatus::Disconnected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    store
        .put_resource(&Resource {
            id: "resource".into(),
            project_id: Some(project.id.clone()),
            kind: ResourceKind::Web,
            provider: hoista_server::Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:fixture".into(),
            name: "fixture".into(),
            region: None,
            status: ResourceStatus::Unknown,
            mode: Mode::Watching,
            url: None,
            group: None,
        })
        .await
        .unwrap();
    let (agent, _) = engine
        .agents()
        .register_agent(
            "fixture".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    for index in 0..105 {
        store
            .put_secret(&SecretMetadata {
                requires_source_sync: false,
                set_by: None,
                managed_by: None,
                project_id: project.id.clone(),
                resource_id: "resource".into(),
                name: format!("KEY_{index:03}"),
                fingerprint: "never-display-this-fingerprint".into(),
                set_at: chrono::Utc::now().to_rfc3339(),
            })
            .await
            .unwrap();
        let day = (chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(index))
            .to_string();
        store
            .put_usage(&Usage {
                resource_id: "resource".into(),
                day,
                requests: Some(1),
                errors: None,
                compute_seconds: None,
                storage_bytes: None,
                egress_bytes: None,
            })
            .await
            .unwrap();
    }
    for index in 0..101 {
        store
            .put_approval(&Approval {
                id: format!("approval-{index:03}"),
                agent_id: agent.id.clone(),
                action: GatedAction::ProviderChange,
                target: project.id.clone(),
                deletion: None,
                reason: "fixture".into(),
                created_at: "2000-01-01T00:00:00Z".into(),
                expires_at: if index < 100 {
                    "2000-01-02T00:00:00Z"
                } else {
                    "2099-01-01T00:00:00Z"
                }
                .into(),
                decision: ApprovalDecision::Pending,
            })
            .await
            .unwrap();
    }
    let engine = Arc::new(engine);
    let router = hoista_server::catalog::router(engine.clone());
    async fn get(router: &axum::Router, path: &str) -> Value {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
    }
    let first = get(&router, &format!("/projects/{}/secrets", project.id)).await;
    assert_eq!(first["items"].as_array().unwrap().len(), 100);
    assert!(!first.to_string().contains("fingerprint"));
    let second = get(
        &router,
        &format!(
            "/projects/{}/secrets?cursor={}",
            project.id,
            first["next_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(second["items"].as_array().unwrap().len(), 5);
    assert!(second["next_cursor"].is_null());
    let first = get(
        &router,
        &format!("/projects/{}/usage?since=2026-01-01", project.id),
    )
    .await;
    assert_eq!(first["items"].as_array().unwrap().len(), 100);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = get(
        &router,
        &format!(
            "/projects/{}/usage?since=2026-01-01&cursor={cursor}",
            project.id
        ),
    )
    .await;
    assert_eq!(second["items"].as_array().unwrap().len(), 5);
    let mcp = call(
        &engine,
        &Actor::Agent {
            id: agent.id.clone(),
        },
        "get_usage",
        json!({"project_id":project.id,"since":"2026-01-01","cursor":cursor}),
    )
    .await;
    assert_eq!(
        mcp["result"]["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let first = get(&router, "/attention").await;
    assert!(first["approvals"].as_array().unwrap().is_empty());
    let second = get(
        &router,
        &format!(
            "/attention?cursor={}",
            first["next_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(second["approvals"].as_array().unwrap().len(), 1);
    assert_eq!(second["approvals"][0]["id"], "approval-100");
    assert!(second["next_cursor"].is_null());
    store.close().await;
}

#[tokio::test]
async fn resource_details_include_relationships_usage_domains_and_resource_specific_history() {
    use axum::{body::Body, http::Request};
    use hoista_server::domain::domains::*;
    use tower::ServiceExt;
    let (_temp, store, engine) = setup().await;
    let engine = Arc::new(engine);
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "detail".into(),
            Source::Unlinked,
            Mode::Watching,
        )
        .await
        .unwrap();
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: hoista_server::Provider::Cloudflare,
            label: "Disconnected fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Disconnected,
            connected_at: "2026-11-01T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    for id in ["web", "database", "bucket", "ungrouped"] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: (id != "ungrouped").then(|| project.id.clone()),
                kind: if id == "database" {
                    ResourceKind::Database
                } else {
                    ResourceKind::Web
                },
                provider: hoista_server::Provider::Cloudflare,
                account_id: "account".into(),
                provider_ref: format!("worker:{id}"),
                name: id.into(),
                region: None,
                status: ResourceStatus::Unknown,
                mode: Mode::Watching,
                url: None,
                group: None,
            })
            .await
            .unwrap();
    }
    store
        .put_link(&Link {
            id: "consumer".into(),
            from_resource: "web".into(),
            to_resource: "database".into(),
            via: "DATABASE_URL".into(),
            derived: true,
            missing: true,
        })
        .await
        .unwrap();
    store
        .put_link(&Link {
            id: "dependency".into(),
            from_resource: "database".into(),
            to_resource: "bucket".into(),
            via: "archive".into(),
            derived: false,
            missing: false,
        })
        .await
        .unwrap();
    for resource_id in ["database", "ungrouped"] {
        for day in ["2026-11-02", "2026-11-01"] {
            store
                .put_usage(&Usage {
                    resource_id: resource_id.into(),
                    day: day.into(),
                    requests: None,
                    errors: None,
                    compute_seconds: None,
                    storage_bytes: Some(123),
                    egress_bytes: None,
                })
                .await
                .unwrap();
        }
    }
    for index in 0..102 {
        let target = if index == 0 { "database" } else { "web" };
        let mut deploy = store
            .enqueue(&Deploy {
                title: None,
                channel: DeployChannel::Desktop,
                started_by_name: None,
                environment_keys: vec![],
                failure: None,
                release_version: None,
                provenance: None,
                source_observation: None,
                cancel_requested: false,
                plan_id: "fixture".into(),
                releases: vec![],
                id: format!("deploy-{index:03}"),
                project_id: project.id.clone(),
                version: 0,
                source: Source::Unlinked,
                started_by: "person".into(),
                status: DeployStatus::Queued,
                steps: vec![],
                url: None,
                started_at: "2026-11-02T00:00:00Z".into(),
                finished_at: None,
                duration_ms: None,
                rollback_of: None,
            })
            .await
            .unwrap();
        deploy.status = if index == 0 {
            DeployStatus::Failed
        } else {
            DeployStatus::Live
        };
        deploy.releases = vec![ResourceRelease {
            resource_id: target.into(),
            name: target.into(),
            release: Release {
                schedules: (index == 101).then(|| vec!["0 * * * *".into()]),
                reference: format!("worker:{target}"),
                version: format!("release-{index}"),
                url: None,
            },
        }];
        store.put_deploy(&deploy).await.unwrap();
    }
    for index in 0..101 {
        store
            .put_custom_domain(&CustomDomain {
                id: format!("domain-{index:03}"),
                project_id: project.id.clone(),
                resource_id: if index == 100 { "database" } else { "web" }.into(),
                hostname: format!("host{index}.example.com"),
                observed: DomainObservation {
                    dns_records: vec![],
                    zone_id: "zone".into(),
                    zone_name: "example.com".into(),
                    nameservers: vec![],
                    registrar: None,
                    dns: DomainReadiness::Unknown,
                    https: DomainReadiness::Unknown,
                    provider_domain_id: None,
                    certificate_id: None,
                    warnings: vec![],
                },
                checked_at: "2026-11-02T00:00:00Z".into(),
            })
            .await
            .unwrap();
    }
    let detail = engine
        .projects()
        .resource_detail(&Actor::Person, "database")
        .await
        .unwrap();
    assert_eq!(detail.used_by.len(), 1);
    assert_eq!(detail.used_by[0].resource.id, "web");
    assert!(detail.used_by[0].link.missing);
    assert_eq!(detail.used_by[0].project.as_ref().unwrap().id, project.id);
    assert_eq!(detail.dependencies.len(), 1);
    assert_eq!(detail.dependencies[0].resource.id, "bucket");
    assert_eq!(detail.domains.len(), 1);
    assert_eq!(detail.domains[0].id, "domain-100");
    assert_eq!(detail.latest_usage.as_ref().unwrap().day, "2026-11-02");
    assert_eq!(detail.account.status, AccountStatus::Disconnected);
    assert!(!detail.relationships_complete);
    let release = detail.latest_recorded_release.unwrap();
    assert_eq!(release.deploy_id, "deploy-000");
    assert_eq!(release.attempt_version, 1);
    assert_eq!(release.status, DeployStatus::Failed);
    let candidate = engine
        .projects()
        .resource_detail(&Actor::Person, "ungrouped")
        .await
        .unwrap();
    assert!(candidate.project.is_none());
    assert!(candidate.latest_recorded_release.is_none());
    assert!(candidate.latest_usage.is_some());
    assert!(candidate.used_by.is_empty());
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Reader".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let map_reply = call(
        &engine,
        &actor,
        "get_project_map",
        json!({"id": project.id}),
    )
    .await;
    assert_eq!(map_reply["result"]["isError"], false);
    let map = &map_reply["result"]["structuredContent"];
    let nodes = map["graph"]["nodes"].as_array().unwrap();
    assert!(nodes.iter().all(|node| node["type"] != "source"));
    let web = nodes
        .iter()
        .find(|node| node["id"] == "resource:web")
        .unwrap();
    assert_eq!(web["schedules"], json!(["0 * * * *"]));
    assert_eq!(
        nodes.iter().filter(|node| node["type"] == "domain").count(),
        101
    );
    let database = nodes
        .iter()
        .find(|node| node["id"] == "resource:database")
        .unwrap();
    assert_eq!(database["latest_usage"]["storage_bytes"], 123);
    assert!(database["latest_usage"]["requests"].is_null());
    assert!(database["schedules"].is_null());
    let edges = map["graph"]["edges"].as_array().unwrap();
    assert_eq!(
        edges.iter().filter(|edge| edge["kind"] == "route").count(),
        101
    );
    assert!(edges.iter().all(|edge| edge["traffic"].is_null()));
    let dependency = edges
        .iter()
        .find(|edge| edge["id"] == "link:consumer")
        .unwrap();
    assert_eq!(dependency["missing"], true);
    assert_eq!(dependency["origin"], "derived");
    assert_eq!(dependency["via"], "DATABASE_URL");
    let map_http = hoista_server::catalog::router(engine.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/projects/{}/map", project.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(map_http.status(), 200);
    let http_map: Value = serde_json::from_slice(
        &axum::body::to_bytes(map_http.into_body(), 1_000_000)
            .await
            .unwrap(),
    )
    .unwrap();
    // The tool answers with the same map, in the agent's words.
    assert_eq!(&http_map, map);
    let mut linked = project.clone();
    linked.source = Source::Folder {
        path: "/fixture".into(),
    };
    store.put_project(&linked).await.unwrap();
    let linked_map = serde_json::to_value(
        engine
            .projects()
            .project_map(&actor, &project.id)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        linked_map["graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "source")
            .count(),
        1
    );
    assert!(
        linked_map["graph"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["kind"] != "deploys")
    );
    store.put_project(&project).await.unwrap();
    let reply = call(&engine, &actor, "get_resource", json!({"id":"database"})).await;
    assert_eq!(reply["result"]["isError"], false);
    engine
        .agents()
        .revoke_agent(&Actor::Person, &agent.id)
        .await
        .unwrap();
    assert!(
        engine
            .projects()
            .resource_detail(&actor, "database")
            .await
            .is_err()
    );
    assert!(
        engine
            .projects()
            .project_map(&actor, &project.id)
            .await
            .is_err()
    );
    let router = hoista_server::catalog::router(engine.clone());
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/resources/database/details")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let value: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 100_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(value["latest_recorded_release"]["deploy_id"], "deploy-000");
    let missing = router
        .oneshot(
            Request::builder()
                .uri("/resources/missing/details")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    // The collection limit is explicit, never an apparently complete partial graph.
    for index in 0..999 {
        store
            .put_link(&Link {
                id: format!("extra-{index:04}"),
                from_resource: "web".into(),
                to_resource: "database".into(),
                via: format!("BINDING_{index}"),
                derived: false,
                missing: false,
            })
            .await
            .unwrap();
    }
    assert!(
        engine
            .projects()
            .resource_detail(&Actor::Person, "database")
            .await
            .is_err()
    );
    engine.observe().stop_logs().await;
    store.close().await;
}

#[tokio::test]
async fn attention_uses_complete_observations_and_latest_attempt_without_mutations() {
    use chrono::{Datelike, Duration, Utc};
    use hoista_server::application::engine::{AttentionNotice, AttentionQuery};
    let (_temp, store, engine) = setup().await;
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "attention".into(),
            Source::Unlinked,
            Mode::Watching,
        )
        .await
        .unwrap();
    store
        .put_account(&Account {
            last_discovery: None,
            id: "a".into(),
            provider: hoista_server::Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Disconnected,
            connected_at: Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    store
        .put_resource(&Resource {
            id: "p".into(),
            project_id: Some(project.id.clone()),
            kind: ResourceKind::Web,
            provider: hoista_server::Provider::Cloudflare,
            account_id: "a".into(),
            provider_ref: "worker:p".into(),
            name: "web".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Watching,
            url: None,
            group: None,
        })
        .await
        .unwrap();
    let mut settings = store.settings().await.unwrap();
    settings.idle_days = 2;
    store.put_settings(&settings).await.unwrap();
    let today = Utc::now().date_naive();
    let mut day = today.with_day(1).unwrap().min(today - Duration::days(2));
    while day <= today {
        store
            .put_usage(&Usage {
                resource_id: "p".into(),
                day: day.to_string(),
                requests: Some(0),
                errors: Some(0),
                compute_seconds: None,
                storage_bytes: None,
                egress_bytes: None,
            })
            .await
            .unwrap();
        day += Duration::days(1);
    }
    let mut deploy = store
        .enqueue(&Deploy {
            title: None,
            channel: DeployChannel::Desktop,
            started_by_name: None,
            environment_keys: vec![],
            failure: None,
            release_version: None,
            provenance: None,
            source_observation: None,
            cancel_requested: false,
            plan_id: "fixture".into(),
            releases: vec![],
            id: "failed".into(),
            project_id: project.id.clone(),
            version: 0,
            source: Source::Unlinked,
            started_by: "person".into(),
            status: DeployStatus::Queued,
            steps: vec![],
            url: None,
            started_at: Utc::now().to_rfc3339(),
            finished_at: None,
            duration_ms: None,
            rollback_of: None,
        })
        .await
        .unwrap();
    deploy.status = DeployStatus::Failed;
    store.put_deploy(&deploy).await.unwrap();
    let feed = engine
        .projects()
        .attention(AttentionQuery::default())
        .await
        .unwrap();
    assert_eq!(feed.notices.len(), 2);
    assert!(feed.notices.iter().any(|n| matches!(n, AttentionNotice::DeploymentFailed { deploy_id, .. } if deploy_id == "failed")));
    assert!(
        feed.notices
            .iter()
            .any(|n| matches!(n, AttentionNotice::IdleResource { days: 2, .. }))
    );
    assert_eq!(
        store.resource("p").await.unwrap().unwrap().status,
        ResourceStatus::Live
    );
    deploy.id = "success".into();
    deploy.status = DeployStatus::Queued;
    let mut deploy = store.enqueue(&deploy).await.unwrap();
    deploy.status = DeployStatus::Live;
    store.put_deploy(&deploy).await.unwrap();
    store
        .put_usage(&Usage {
            resource_id: "p".into(),
            day: (today - Duration::days(1)).to_string(),
            requests: None,
            errors: None,
            compute_seconds: None,
            storage_bytes: None,
            egress_bytes: None,
        })
        .await
        .unwrap();
    let feed = engine
        .projects()
        .attention(AttentionQuery::default())
        .await
        .unwrap();
    assert!(!feed.notices.iter().any(|n| matches!(
        n,
        AttentionNotice::DeploymentFailed { .. } | AttentionNotice::IdleResource { .. }
    )));
    if today.day() > 1 {
        assert!(feed.notices.is_empty());
    }
    store.close().await;
}

#[tokio::test]
async fn plans_carry_no_cost_estimates() {
    // Planning chooses among the providers the engine has adapters for.
    let http = hoista_server::infrastructure::providers::http::ProviderHttp::new().unwrap();
    let (temp, store, engine) =
        setup_with(hoista_server::infrastructure::providers::adapters(&http)).await;
    let source_dir = temp.path().join("project");
    std::fs::create_dir(&source_dir).unwrap();
    std::fs::write(source_dir.join("index.html"), "hello").unwrap();
    let source = Source::Folder {
        path: source_dir.to_string_lossy().into(),
    };
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "priced".into(),
            source.clone(),
            Mode::Watching,
        )
        .await
        .unwrap();
    store
        .put_account(&Account {
            last_discovery: None,
            id: "a".into(),
            provider: hoista_server::Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    let fresh = engine
        .deploys()
        .plan(&Actor::Person, source.clone(), "priced", None, None)
        .await
        .unwrap();
    // A plan says where each resource goes; it carries no cost.
    let plan = serde_json::to_value(&fresh).unwrap();
    assert!(plan.get("estimates").is_none() && plan.get("monthly_estimate").is_none());
    drop(project);
}

#[tokio::test]
async fn import_suggestions_are_read_only_and_require_read_scope() {
    let (_directory, store, engine) = setup().await;
    for (account_id, provider, id, name, kind) in [
        (
            "cloud",
            hoista_server::Provider::Cloudflare,
            "site",
            "moodboard-web",
            ResourceKind::Web,
        ),
        (
            "neon",
            hoista_server::Provider::Neon,
            "database",
            "moodboard-db",
            ResourceKind::Database,
        ),
    ] {
        store
            .put_account(&Account {
                last_discovery: None,
                id: account_id.into(),
                provider,
                label: account_id.into(),
                external_id: account_id.into(),
                scopes: vec![],
                status: AccountStatus::Disconnected,
                connected_at: "2026-11-02T00:00:00Z".into(),
                credential: None,
                in_1password: false,
            })
            .await
            .unwrap();
        store
            .put_resource(&Resource {
                id: id.into(),
                name: name.into(),
                kind,
                provider,
                account_id: account_id.into(),
                project_id: None,
                provider_ref: id.into(),
                region: None,
                status: ResourceStatus::Unknown,
                mode: Mode::Watching,
                url: None,
                group: None,
            })
            .await
            .unwrap();
    }
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Reader".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let response = call(
        &engine,
        &Actor::Agent { id: agent.id },
        "suggest_imports",
        json!({}),
    )
    .await;
    assert_eq!(response["result"]["isError"], false, "{response}");
    let proposals = response["result"]["structuredContent"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0]["label"], "moodboard");
    assert_eq!(proposals[0]["ambiguous"], false);
    assert_eq!(store.projects(None).await.unwrap().items.len(), 0);
    assert!(
        store
            .resources(None)
            .await
            .unwrap()
            .items
            .iter()
            .all(|resource| resource.project_id.is_none() && resource.mode == Mode::Watching)
    );
    let (writer, _) = engine
        .agents()
        .register_agent(
            "Writer".into(),
            vec!["apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let denied = call(
        &engine,
        &Actor::Agent { id: writer.id },
        "suggest_imports",
        json!({}),
    )
    .await;
    assert_eq!(denied["result"]["isError"], true);
    let engine = Arc::new(engine);
    use tower::ServiceExt;
    let response = hoista_server::catalog::router(engine)
        .oneshot(
            axum::http::Request::builder()
                .uri("/import/suggestions")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 32768)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn doctor_account_pages_skip_disconnected_credentials_and_report_missing_entries() {
    use hoista_server::application::engine::AccountDiagnosticStatus as Status;
    let (_temp, store, engine) = setup().await;
    for index in 0..101 {
        store
            .put_account(&Account {
                last_discovery: None,
                id: format!("account-{index:03}"),
                provider: hoista_server::Provider::Cloudflare,
                label: "fixture".into(),
                external_id: format!("remote-{index}"),
                scopes: vec![],
                status: if index == 100 {
                    AccountStatus::Connected
                } else {
                    AccountStatus::Disconnected
                },
                connected_at: "2026-11-02T00:00:00Z".into(),
                credential: None,
                in_1password: false,
            })
            .await
            .unwrap();
    }
    let first = engine
        .observe()
        .diagnose_accounts(&Actor::Person, None)
        .await
        .unwrap();
    assert_eq!(first.items.len(), 100);
    assert!(
        first
            .items
            .iter()
            .all(|item| item.status == Status::Disconnected && item.checked_at.is_none())
    );
    let last = engine
        .observe()
        .diagnose_accounts(&Actor::Person, first.next_cursor.as_deref())
        .await
        .unwrap();
    assert_eq!(last.items.len(), 1);
    assert_eq!(last.items[0].status, Status::CredentialMissing);
    assert!(last.next_cursor.is_none());
    let report = hoista_server::doctor::accounts(
        hoista_server::contracts::DoctorReport {
            network: None,
            accounts: None,
            version: "fixture".into(),
            mode: "fixture".into(),
            checks: vec![],
        },
        &engine,
    )
    .await;
    let network = report.network.as_ref().expect("network observations");
    assert!(
        network
            .allowed_api_hosts
            .iter()
            .any(|host| host == "api.cloudflare.com")
    );
    assert!(!network.limitations.is_empty());
    assert!(network.recent_hosts.len() <= 64);
    assert!(report.accounts.unwrap().next_cursor.is_some());
    assert_eq!(
        report.checks[0].status,
        hoista_server::contracts::CheckStatus::NotChecked
    );
    use tower::ServiceExt;
    let response = hoista_server::catalog::router(Arc::new(engine))
        .oneshot(
            axum::http::Request::builder()
                .uri("/doctor/accounts")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 100_000)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["items"].as_array().unwrap().len(), 100);
    store.close().await;
}

#[tokio::test]
async fn tool_calls_keep_what_was_sent_without_secret_values() {
    use hoista_server::application::StateStore;
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Writer".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("MCP fixture");
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let secret = "do-not-keep-this-7f3a9c";
    call(
        &engine,
        &actor,
        "set_secret",
        json!({"project_id":"missing","resource_id":"p","name":"API_KEY","value":secret}),
    )
    .await;
    let events = store
        .events_before(None, Some(&agent.id), None, None)
        .await
        .expect("events");
    let event = events
        .iter()
        .find(|event| event.kind == "agent.tool")
        .expect("tool call recorded");
    let detail = store
        .event_detail(event.id)
        .await
        .expect("detail")
        .expect("detail kept");
    assert!(!detail.contains(secret), "secret value stored: {detail}");
    let detail: Value = serde_json::from_str(&detail).expect("json");
    assert_eq!(detail["tool"], "set_secret");
    assert_eq!(detail["sent"]["name"], "API_KEY");
    assert_eq!(detail["sent"]["project_id"], "missing");
    assert_eq!(detail["result"]["outcome"], "rejected");
    assert!(detail["result"]["message"].is_string());
}

#[tokio::test]
async fn many_variables_keep_names_but_never_values() {
    use hoista_server::application::StateStore;
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Writer".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("MCP fixture");
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let secret = "plain-text-value-9d2e1";
    call(
        &engine,
        &actor,
        "set_secrets",
        json!({"resource_id":"missing","variables":{"STRIPE_KEY":secret,"MODE":"live"}}),
    )
    .await;
    let events = store
        .events_before(None, Some(&agent.id), None, None)
        .await
        .expect("events");
    let event = events
        .iter()
        .find(|event| event.kind == "agent.tool")
        .expect("recorded");
    let detail = store
        .event_detail(event.id)
        .await
        .expect("detail")
        .expect("kept");
    assert!(!detail.contains(secret), "value stored: {detail}");
    assert!(detail.contains("STRIPE_KEY"));
}

#[test]
fn agents_see_resources_not_parts() {
    let tools = serde_json::to_string(&mcp::tools()).unwrap();
    assert!(!tools.contains("part_id"), "a schema still says part_id");
    assert!(!tools.contains("\"get_part\""));
    assert!(tools.contains("\"get_resource\""));
    assert!(tools.contains("resource_id"));
}

#[tokio::test]
async fn old_tool_names_are_unknown() {
    let (_temp, _store, engine) = setup().await;
    for old in ["get_part", "list_apps"] {
        let answer = call(&engine, &Actor::Person, old, json!({})).await;
        assert!(answer.get("error").is_some(), "{old} should be unknown");
    }
}

async fn two_projects(engine: &Engine) -> (Project, Project) {
    let make = |name: &str| {
        engine.projects().create_project(
            &Actor::Person,
            name.into(),
            Source::Folder {
                path: format!("/fixture/{name}"),
            },
            Mode::Managed,
        )
    };
    (
        make("allowed").await.expect("project fixture"),
        make("other").await.expect("project fixture"),
    )
}

fn failed(response: &Value) -> bool {
    response["result"]["isError"] == true
}

#[tokio::test]
async fn grants_limit_agents_to_chosen_projects_and_filter_lists() {
    let (_temp, store, engine) = setup().await;
    let (allowed, other) = two_projects(&engine).await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Scoped".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    // Every project by default, so identities saved before grants keep working.
    assert!(!failed(
        &call(&engine, &actor, "get_project", json!({"id": other.id})).await
    ));

    let changed = engine
        .agents()
        .set_agent_grant(
            &Actor::Person,
            &agent.id,
            AgentGrantInput {
                projects: Some(vec![allowed.id.clone()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.grant.version, 1);

    assert!(!failed(
        &call(&engine, &actor, "get_project", json!({"id": allowed.id})).await
    ));
    let denied = call(&engine, &actor, "get_project", json!({"id": other.id})).await;
    assert!(failed(&denied));
    assert!(denied.to_string().contains("may not touch that project"));

    // Lists only show its projects.
    let listed = call(&engine, &actor, "list_projects", json!({})).await;
    let text = listed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&allowed.id) && !text.contains(&other.id));

    // Whole-account and project-less tools are refused while limited.
    assert!(failed(&call(&engine, &actor, "get_bills", json!({})).await));
    assert!(failed(
        &call(
            &engine,
            &actor,
            "list_provider_operations",
            json!({"account_id": "acc"})
        )
        .await
    ));

    // The agent can see its own limits.
    let access = call(&engine, &actor, "get_my_access", json!({})).await;
    assert!(!failed(&access));
    assert!(access.to_string().contains("on 1 project"));

    // Only the person can change a grant, and unknown projects are refused.
    assert!(
        engine
            .agents()
            .set_agent_grant(&actor, &agent.id, AgentGrantInput::default())
            .await
            .is_err()
    );
    assert!(
        engine
            .agents()
            .set_agent_grant(
                &Actor::Person,
                &agent.id,
                AgentGrantInput {
                    projects: Some(vec!["no-such-project".into()]),
                    ..Default::default()
                },
            )
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn denied_tools_and_ended_grants_are_refused() {
    let (_temp, store, engine) = setup().await;
    let (allowed, _) = two_projects(&engine).await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Narrow".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    engine
        .agents()
        .set_agent_grant(
            &Actor::Person,
            &agent.id,
            AgentGrantInput {
                deny: vec!["get_project".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // A denied tool is refused even with the scope for it.
    let denied = call(&engine, &actor, "get_project", json!({"id": allowed.id})).await;
    assert!(failed(&denied) && denied.to_string().contains("may not use this tool"));

    // An end date in the past can't be set; one that passes ends access.
    assert!(
        engine
            .agents()
            .set_agent_grant(
                &Actor::Person,
                &agent.id,
                AgentGrantInput {
                    expires_at: Some("2000-01-01T00:00:00Z".into()),
                    ..Default::default()
                },
            )
            .await
            .is_err()
    );
    let mut ended = store.agent(&agent.id).await.unwrap().unwrap();
    ended.grant.expires_at = Some("2000-01-01T00:00:00Z".into());
    store.put_agent(&ended).await.unwrap();
    assert!(failed(
        &call(&engine, &actor, "list_projects", json!({})).await
    ));
    store.close().await;
}

#[tokio::test]
async fn coding_tools_connect_only_after_the_person_allows_them() {
    let (_temp, store, engine) = setup().await;
    let (request, channel) = engine
        .accounts()
        .request_enrollment("Claude Code", None)
        .await
        .unwrap();
    assert_eq!(request.code.len(), 7);
    // Waiting, and nothing to collect yet; a wrong channel learns nothing.
    let waiting = engine
        .accounts()
        .poll_enrollment(&request.id, channel.expose())
        .await
        .unwrap();
    assert_eq!(waiting.status, EnrollmentStatus::Pending);
    assert!(waiting.credential.is_none());
    assert!(
        engine
            .accounts()
            .poll_enrollment(&request.id, "wrong")
            .await
            .is_err()
    );
    assert_eq!(
        engine.accounts().enrollment_requests().await.unwrap().len(),
        1
    );
    // Agents can't answer requests.
    let other = Actor::Agent {
        id: "someone".into(),
    };
    assert!(
        engine
            .accounts()
            .decide_enrollment(
                &other,
                &request.id,
                true,
                vec!["apps:read".into()],
                None,
                false
            )
            .await
            .is_err()
    );
    engine
        .accounts()
        .decide_enrollment(
            &Actor::Person,
            &request.id,
            true,
            vec!["apps:read".into()],
            None,
            false,
        )
        .await
        .unwrap();
    // The credential is handed over once, to the channel holder.
    let allowed = engine
        .accounts()
        .poll_enrollment(&request.id, channel.expose())
        .await
        .unwrap();
    let credential = allowed.credential.unwrap();
    assert!(
        engine
            .accounts()
            .poll_enrollment(&request.id, channel.expose())
            .await
            .unwrap()
            .credential
            .is_none()
    );
    let actor = engine
        .agents()
        .authenticate_agent(&credential)
        .await
        .unwrap();
    let Actor::Agent { id } = &actor else {
        panic!("an agent")
    };
    let agent = store.agent(id).await.unwrap().unwrap();
    assert_eq!(agent.scopes, vec!["apps:read".to_owned()]);
    // A second request with the same name makes a new identity, never the old one.
    let (again, again_channel) = engine
        .accounts()
        .request_enrollment("Claude Code", None)
        .await
        .unwrap();
    engine
        .accounts()
        .decide_enrollment(&Actor::Person, &again.id, false, vec![], None, false)
        .await
        .unwrap();
    let denied = engine
        .accounts()
        .poll_enrollment(&again.id, again_channel.expose())
        .await
        .unwrap();
    assert_eq!(denied.status, EnrollmentStatus::Denied);
    assert!(denied.credential.is_none());
    assert!(
        engine
            .accounts()
            .decide_enrollment(
                &Actor::Person,
                &again.id,
                true,
                vec!["apps:read".into()],
                None,
                false
            )
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn the_consent_page_authorizes_only_from_its_own_page() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;
    let (_temp, store, engine) = setup().await;
    let engine = Arc::new(engine);
    let (request, channel) = engine
        .accounts()
        .request_enrollment("Claude Code", None)
        .await
        .unwrap();
    let pages = hoista_server::pages::router(engine.clone(), vec!["127.0.0.1:7444".into()]);
    async fn text(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }
    let get = |host: &str| {
        Request::get(format!("/connect/{}", request.id))
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap()
    };
    // Another hostname pointing here (DNS rebinding) gets nothing.
    let wrong = pages
        .clone()
        .oneshot(get("evil.example:7444"))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::MISDIRECTED_REQUEST);
    let page = pages.clone().oneshot(get("127.0.0.1:7444")).await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()[header::X_FRAME_OPTIONS], "DENY");
    let html = text(page).await;
    assert!(html.contains("Authorize Claude Code"));
    assert!(html.contains(&request.code));
    let token = html
        .split("name=\"token\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap()
        .to_owned();
    let post = |origin: Option<&str>, body: String| {
        let mut builder = Request::post(format!("/connect/{}", request.id))
            .header(header::HOST, "127.0.0.1:7444")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder.body(Body::from(body)).unwrap()
    };
    let allow =
        format!("token={token}&access=ask&env_ids=&scope=all&project_ids=&ends=7&decision=allow");
    // A form sent from another site, or with no origin, is refused.
    for origin in [None, Some("http://evil.example")] {
        let refused = pages
            .clone()
            .oneshot(post(origin, allow.clone()))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    }
    // A form without the page's token does nothing.
    let stale = pages
        .clone()
        .oneshot(post(
            Some("http://127.0.0.1:7444"),
            "token=0000&access=full&decision=allow".into(),
        ))
        .await
        .unwrap();
    assert!(text(stale).await.contains("out of date"));
    assert!(store.agents(None).await.unwrap().items.is_empty());
    // From its own page it authorizes, with the access picked there.
    let done = pages
        .clone()
        .oneshot(post(Some("http://127.0.0.1:7444"), allow))
        .await
        .unwrap();
    assert_eq!(done.status(), StatusCode::SEE_OTHER);
    let poll = engine
        .accounts()
        .poll_enrollment(&request.id, channel.expose())
        .await
        .unwrap();
    assert_eq!(poll.status, EnrollmentStatus::Allowed);
    let agents = store.agents(None).await.unwrap().items;
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].scopes,
        vec!["apps:read", "apps:write", "apps:create"]
    );
    // Ask for approval, every environment and project, ending in a week.
    assert!(!agents[0].grant.without_asking);
    assert_eq!(agents[0].grant.environment_ids, None);
    assert_eq!(agents[0].grant.projects, None);
    assert!(agents[0].grant.expires_at.is_some());
}

#[tokio::test]
async fn instructions_fit_what_clients_keep_and_deploys_can_be_waited_on() {
    let (_temp, _store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Reader".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("MCP fixture");
    let actor = Actor::Agent { id: agent.id };
    let hello = mcp::dispatch(
        &engine,
        &actor,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "t", "version": "1"},
            },
        }),
    )
    .await
    .expect("initialize answers");
    let words = hello["result"]["instructions"]
        .as_str()
        .expect("instructions");
    // Claude Code keeps the first 2048 characters and drops the rest.
    assert!(
        words.len() <= 2048,
        "instructions are {} characters",
        words.len()
    );
    assert!(words.contains("wait_for_deploy"));
    assert!(
        mcp::tools()
            .iter()
            .any(|tool| tool["name"] == "wait_for_deploy")
    );
    let missing = call(
        &engine,
        &actor,
        "wait_for_deploy",
        json!({"id":"no-such-deploy","timeout_seconds":1}),
    )
    .await;
    assert_eq!(missing["result"]["isError"], true);
}

#[tokio::test]
async fn an_agent_allowed_without_asking_needs_no_question_but_others_still_wait() {
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Background agent".into(),
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
            AgentTransport::Stdio,
        )
        .await
        .expect("MCP fixture");
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let first = engine
        .projects()
        .create_project(
            &Actor::Person,
            "first-project".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .unwrap();
    // Asking each time (the default): the call waits for the person.
    let waiting = call(&engine, &actor, "delete_project", json!({"id": first.id})).await;
    assert_eq!(
        waiting["result"]["structuredContent"]["status"], "pending",
        "{waiting}"
    );
    assert!(store.project(&first.id).await.unwrap().is_some());
    // The person lets it go ahead without asking.
    engine
        .agents()
        .set_agent_grant(
            &Actor::Person,
            &agent.id,
            AgentGrantInput {
                without_asking: true,
                ..AgentGrantInput::default()
            },
        )
        .await
        .unwrap();
    let second = engine
        .projects()
        .create_project(
            &Actor::Person,
            "second-project".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .unwrap();
    let done = call(&engine, &actor, "delete_project", json!({"id": second.id})).await;
    assert_eq!(done["result"]["isError"], false, "{done}");
    assert_ne!(done["result"]["structuredContent"]["status"], "pending");
    assert!(store.project(&second.id).await.unwrap().is_none());
    // An agent can't give itself this.
    assert!(
        engine
            .agents()
            .set_agent_grant(&actor, &agent.id, AgentGrantInput::default())
            .await
            .is_err()
    );
}

#[test]
fn only_deleting_or_removing_is_marked_destructive() {
    let tools = mcp::tools();
    let hint = |name: &str| {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} is a tool"));
        (
            tool["annotations"]["readOnlyHint"] == true,
            tool["annotations"]["destructiveHint"] == true,
        )
    };
    assert_eq!(hint("list_projects"), (true, false));
    assert_eq!(hint("deploy"), (false, false));
    assert_eq!(hint("redeploy_resource"), (false, false));
    assert_eq!(hint("delete_project"), (false, true));
    assert_eq!(hint("remove_service"), (false, true));
    assert_eq!(hint("remove_resource_domain"), (false, true));
}

#[tokio::test]
async fn each_hoista_environment_can_have_its_own_rule() {
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Full agent".into(),
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let production = engine
        .environments()
        .create_environment(&Actor::Person, "production")
        .await
        .unwrap();
    let live = engine
        .projects()
        .create_project_in(
            &Actor::Person,
            Some(&production.id),
            "live-site".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .unwrap();
    let trial = engine
        .projects()
        .create_project(
            &Actor::Person,
            "trial-site".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .unwrap();
    engine
        .agents()
        .set_agent_grant(
            &Actor::Person,
            &agent.id,
            AgentGrantInput {
                without_asking: true,
                environment_rules: [(production.id.clone(), ProductionRule::ViewOnly)]
                    .into_iter()
                    .collect(),
                ..AgentGrantInput::default()
            },
        )
        .await
        .unwrap();
    // The project in production can only be looked at.
    let refused = call(&engine, &actor, "delete_project", json!({"id": live.id})).await;
    assert!(
        refused.to_string().contains("can only look at"),
        "{refused}"
    );
    assert!(store.project(&live.id).await.unwrap().is_some());
    // The one in default follows the agent's own access.
    let done = call(&engine, &actor, "delete_project", json!({"id": trial.id})).await;
    assert!(!done.to_string().contains("can only look at"), "{done}");
}

#[tokio::test]
async fn exceptions_narrow_access_to_some_resources() {
    use hoista_server::domain::model::{AgentException, ResourceGroup, Restriction};
    let (_temp, store, engine) = setup().await;
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Full agent".into(),
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "shop".into(),
            Source::Unlinked,
            Mode::Watching,
        )
        .await
        .unwrap();
    let resource = |id: &str, kind| Resource {
        id: id.into(),
        project_id: Some(project.id.clone()),
        kind,
        provider: hoista_server::Provider::Railway,
        account_id: "account".into(),
        provider_ref: format!("service:{id}"),
        name: id.into(),
        region: None,
        status: ResourceStatus::Live,
        mode: Mode::Watching,
        url: None,
        group: None,
    };
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: hoista_server::Provider::Railway,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    for (id, kind) in [
        ("shop-web", ResourceKind::Web),
        ("shop-db", ResourceKind::Database),
        ("shop-cache", ResourceKind::Database),
    ] {
        store.put_resource(&resource(id, kind)).await.unwrap();
    }
    engine
        .agents()
        .set_agent_grant(
            &Actor::Person,
            &agent.id,
            AgentGrantInput {
                without_asking: true,
                exceptions: vec![
                    AgentException {
                        group: Some(ResourceGroup::Databases),
                        project_id: None,
                        resource_id: None,
                        access: Restriction::ViewOnly,
                    },
                    AgentException {
                        group: None,
                        project_id: None,
                        resource_id: Some("shop-cache".into()),
                        access: Restriction::NoAccess,
                    },
                ],
                ..AgentGrantInput::default()
            },
        )
        .await
        .unwrap();
    // A database can only be looked at.
    let refused = call(
        &engine,
        &actor,
        "restart_resource",
        json!({"resource_id": "shop-db"}),
    )
    .await;
    assert!(
        refused
            .to_string()
            .contains("may only look at that resource"),
        "{refused}"
    );
    // The web service follows the agent's own access.
    let web = call(
        &engine,
        &actor,
        "restart_resource",
        json!({"resource_id": "shop-web"}),
    )
    .await;
    assert!(!web.to_string().contains("may only look at"), "{web}");
    // The hidden one isn't there for it.
    let shown = call(&engine, &actor, "get_project", json!({"id": project.id})).await;
    assert!(!shown.to_string().contains("shop-cache"), "{shown}");
    assert!(shown.to_string().contains("shop-db"), "{shown}");
    let hidden = call(
        &engine,
        &actor,
        "get_resource",
        json!({"resource_id": "shop-cache"}),
    )
    .await;
    assert!(failed(&hidden), "{hidden}");
    // Changing the whole project needs all of it.
    let whole = call(&engine, &actor, "delete_project", json!({"id": project.id})).await;
    assert!(
        whole.to_string().contains("changes the whole project"),
        "{whole}"
    );
    assert!(store.project(&project.id).await.unwrap().is_some());
}

#[tokio::test]
async fn remote_agents_are_answered_on_the_consent_page_with_the_chosen_access() {
    use base64::Engine as _;
    use hoista_server::application::engine::{AuthorizationRequest, pairing_code};
    use sha2::{Digest, Sha256};
    let (_temp, store, engine) = setup().await;
    let engine = engine.with_public_url(SERVER.into());
    let client = engine
        .accounts()
        .oauth_register(
            "Remote fixture".into(),
            vec!["https://example.com/callback".into()],
        )
        .await
        .expect("client");
    let verifier = "w".repeat(43);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let resource = format!("{SERVER}/mcp");
    let (pending, code) = engine
        .accounts()
        .oauth_authorize(AuthorizationRequest {
            client_id: client.id.clone(),
            redirect_uri: client.redirect_uris[0].clone(),
            state: "s".into(),
            challenge,
            resource: resource.clone(),
            scopes: vec!["apps:read".into()],
        })
        .await
        .expect("authorization");
    let waiting = engine
        .accounts()
        .oauth_request(&pending.id)
        .await
        .expect("read")
        .expect("waiting");
    assert_eq!(waiting.name, "Remote fixture");
    assert_eq!(waiting.code, pairing_code(&pending.id));
    assert_eq!(waiting.returns_to, "example.com");
    let agent = Actor::Agent {
        id: pending.agent_id.clone(),
    };
    assert!(
        engine
            .accounts()
            .decide_oauth(
                &agent,
                &pending.id,
                true,
                vec![],
                AgentGrantInput::default()
            )
            .await
            .is_err(),
        "an agent can't answer its own request"
    );
    let full: Vec<String> = ["apps:read", "apps:write", "apps:create"]
        .map(String::from)
        .to_vec();
    engine
        .accounts()
        .decide_oauth(
            &Actor::Person,
            &pending.id,
            true,
            full.clone(),
            AgentGrantInput {
                without_asking: true,
                ..AgentGrantInput::default()
            },
        )
        .await
        .expect("allowed");
    assert!(
        engine
            .accounts()
            .oauth_request(&pending.id)
            .await
            .expect("read")
            .is_none(),
        "an answered request leaves the consent page"
    );
    let token = engine
        .accounts()
        .oauth_token(
            code.expose(),
            &client.id,
            &client.redirect_uris[0],
            &verifier,
            &resource,
        )
        .await
        .expect("grant");
    let signed_in = engine
        .agents()
        .authenticate_agent(token.expose())
        .await
        .expect("agent");
    let saved = store
        .agent(&pending.agent_id)
        .await
        .expect("read")
        .expect("agent");
    let mut sorted = full;
    sorted.sort();
    assert_eq!(saved.scopes, sorted);
    assert!(saved.grant.without_asking);
    let _ = signed_in;
    store.close().await;
}

#[tokio::test]
async fn nothing_an_agent_asks_for_is_allowed_without_the_owner_check() {
    let (temp, store, engine) = setup().await;
    // The same Hoista, but the owner prompt says no (or can't be shown).
    let refusing = Engine::new(
        Arc::new(store.clone()),
        engine.vault.clone(),
        Arc::new(LocalInspector),
        vec![],
        Arc::new(LocalBuilder::new(temp.path())),
    )
    .with_owner_check(Arc::new(
        hoista_server::application::ports::owner_check::FixedOwnerCheck(false),
    ));
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Writer".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("agent");
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "keep-me".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let pending = call(
        &refusing,
        &actor,
        "delete_project",
        json!({"id":project.id}),
    )
    .await;
    let approval = pending["result"]["structuredContent"]["approval_id"]
        .as_str()
        .expect("approval")
        .to_owned();
    // The agent passes on "allow" with the right name itself: refused.
    let answered = mcp::dispatch(
        &refusing,
        &actor,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "hoista/answer_approval",
            "params": {"approval_id": approval, "allow": true, "name": "keep-me"},
        }),
    )
    .await
    .expect("answer");
    assert!(answered["error"]["message"].is_string(), "{answered}");
    // So does anything reaching the owner connection.
    assert!(
        refusing
            .projects()
            .decide_approval(&approval, true, None)
            .await
            .is_err()
    );
    assert_eq!(
        store.approval(&approval).await.unwrap().unwrap().decision,
        ApprovalDecision::Pending
    );
    let deleted = call(
        &refusing,
        &actor,
        "delete_project",
        json!({"id":project.id,"approval_id":approval}),
    )
    .await;
    assert_eq!(deleted["result"]["isError"], true);
    assert!(store.project(&project.id).await.unwrap().is_some());
    // Saying no needs no check.
    assert!(
        refusing
            .projects()
            .decide_approval(&approval, false, None)
            .await
            .is_ok()
    );
    // An agent can't raise its own access through the owner connection.
    assert!(
        refusing
            .agents()
            .set_agent_scopes(
                &Actor::Person,
                &agent.id,
                vec![
                    "apps:create".into(),
                    "apps:read".into(),
                    "apps:write".into()
                ]
            )
            .await
            .is_err()
    );
    assert!(
        refusing
            .agents()
            .set_agent_grant(
                &Actor::Person,
                &agent.id,
                AgentGrantInput {
                    without_asking: true,
                    ..AgentGrantInput::default()
                }
            )
            .await
            .is_err()
    );
    // Narrowing needs no check.
    assert!(
        refusing
            .agents()
            .set_agent_scopes(&Actor::Person, &agent.id, vec!["apps:read".into()])
            .await
            .is_ok()
    );
    // Whoever holds the consent page can't authorize a new agent.
    let (request, channel) = refusing
        .accounts()
        .request_enrollment("Claude Code", None)
        .await
        .unwrap();
    assert!(
        refusing
            .accounts()
            .decide_enrollment(
                &Actor::Person,
                &request.id,
                true,
                vec!["apps:read".into()],
                None,
                false
            )
            .await
            .is_err()
    );
    // The request is saved, so it outlives a restart: a fresh engine on the
    // same store still has it, and the bridge can still collect it.
    assert_eq!(
        engine.accounts().enrollment_requests().await.unwrap().len(),
        1
    );
    engine
        .accounts()
        .decide_enrollment(
            &Actor::Person,
            &request.id,
            true,
            vec!["apps:read".into()],
            None,
            false,
        )
        .await
        .unwrap();
    let collected = refusing
        .accounts()
        .poll_enrollment(&request.id, channel.expose())
        .await
        .unwrap();
    assert!(collected.credential.is_some());
    assert!(
        refusing
            .agents()
            .authenticate_agent(&collected.credential.unwrap())
            .await
            .is_ok()
    );
    // Turning on the network page needs the check too.
    assert!(
        refusing
            .settings()
            .update_settings(
                &Actor::Person,
                SettingsPatch {
                    network_page: Some(true),
                    ..SettingsPatch::default()
                }
            )
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn environments_hold_projects_and_limit_agents() {
    let (_temp, store, engine) = setup().await;
    let (allowed, other) = two_projects(&engine).await;
    // Everything starts in default, which can't be removed.
    assert_eq!(allowed.environment_id, "env_default");
    let listed = engine.environments().list(&Actor::Person).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "default");
    assert!(
        engine
            .environments()
            .remove_environment(&Actor::Person, "env_default")
            .await
            .is_err()
    );

    let production = engine
        .environments()
        .create_environment(&Actor::Person, "production")
        .await
        .unwrap();
    assert!(
        engine
            .environments()
            .create_environment(&Actor::Person, "production")
            .await
            .is_err(),
        "names are unique"
    );
    assert!(
        engine
            .environments()
            .create_environment(&Actor::Person, "Bad Name")
            .await
            .is_err()
    );
    let moved = engine
        .projects()
        .move_project(&Actor::Person, &allowed.id, &production.id)
        .await
        .unwrap();
    assert_eq!(moved.environment_id, production.id);
    // An environment with projects in it stays until they leave.
    assert!(
        engine
            .environments()
            .remove_environment(&Actor::Person, &production.id)
            .await
            .is_err()
    );
    // The same name can live in two environments, but not twice in one.
    let copy = engine
        .projects()
        .create_project(
            &Actor::Person,
            "allowed".into(),
            Source::Folder {
                path: "/fixture/copy".into(),
            },
            Mode::Managed,
        )
        .await
        .unwrap();
    assert!(
        engine
            .projects()
            .move_project(&Actor::Person, &copy.id, &production.id)
            .await
            .is_err()
    );

    // An agent limited to production sees and touches only what's there.
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Prod only".into(),
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    engine
        .agents()
        .set_agent_grant(
            &Actor::Person,
            &agent.id,
            AgentGrantInput {
                environment_ids: Some(vec![production.id.clone()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!failed(
        &call(&engine, &actor, "get_project", json!({"id": allowed.id})).await
    ));
    let denied = call(&engine, &actor, "get_project", json!({"id": other.id})).await;
    assert!(failed(&denied));
    let listed = call(&engine, &actor, "list_projects", json!({})).await;
    let text = listed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&allowed.id) && !text.contains(&other.id));
    let environments = call(&engine, &actor, "list_environments", json!({})).await;
    let text = environments["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("production") && !text.contains("env_default"));
    // It can't pull a project out of an environment it can't see, or into default.
    assert!(failed(
        &call(
            &engine,
            &actor,
            "move_project",
            json!({"project_id": other.id, "environment_id": production.id})
        )
        .await
    ));
    assert!(failed(
        &call(
            &engine,
            &actor,
            "move_project",
            json!({"project_id": allowed.id, "environment_id": "env_default"})
        )
        .await
    ));
    // What it creates lands in production.
    let made = call(
        &engine,
        &actor,
        "create_project",
        json!({"name": "fresh", "source": {"kind": "folder", "path": "/fixture/fresh"}}),
    )
    .await;
    assert!(!failed(&made), "{made}");
    let text = made["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&production.id));
    // Unknown environments can't be granted.
    assert!(
        engine
            .agents()
            .set_agent_grant(
                &Actor::Person,
                &agent.id,
                AgentGrantInput {
                    environment_ids: Some(vec!["env_nope".into()]),
                    ..Default::default()
                },
            )
            .await
            .is_err()
    );
    store.close().await;
}

/// Allows a new connection from `name` and returns its agent id.
async fn connect_tool(engine: &Engine, name: &str, replace_earlier: bool) -> String {
    let (request, channel) = engine
        .accounts()
        .request_enrollment(name, None)
        .await
        .expect("request to connect");
    engine
        .accounts()
        .decide_enrollment(
            &Actor::Person,
            &request.id,
            true,
            vec!["apps:read".into()],
            None,
            replace_earlier,
        )
        .await
        .expect("allowed");
    let credential = engine
        .accounts()
        .poll_enrollment(&request.id, channel.expose())
        .await
        .expect("collected")
        .credential
        .expect("a credential");
    match engine
        .agents()
        .authenticate_agent(&credential)
        .await
        .expect("signed in")
    {
        Actor::Agent { id } => id,
        _ => panic!("an agent"),
    }
}

#[tokio::test]
async fn reconnecting_a_tool_replaces_its_earlier_connection() {
    let (_temp, store, engine) = setup().await;
    let first = connect_tool(&engine, "Claude Code", true).await;
    let other_tool = connect_tool(&engine, "Codex", true).await;
    // Kept when the person asks to keep both.
    let second = connect_tool(&engine, "Claude Code", false).await;
    assert!(!store.agent(&first).await.unwrap().unwrap().revoked);
    // Replaced by default: every earlier Claude Code goes, other tools stay.
    let third = connect_tool(&engine, "Claude Code", true).await;
    assert!(store.agent(&first).await.unwrap().unwrap().revoked);
    assert!(store.agent(&second).await.unwrap().unwrap().revoked);
    assert!(!store.agent(&third).await.unwrap().unwrap().revoked);
    assert!(!store.agent(&other_tool).await.unwrap().unwrap().revoked);
    assert_eq!(
        engine
            .accounts()
            .earlier_connections("Claude Code")
            .await
            .unwrap()
            .len(),
        1
    );
    store.close().await;
}
