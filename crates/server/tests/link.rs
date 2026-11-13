//! MCP over HTTP: a web agent signs in to the Hoista on a computer with
//! OAuth once the tunnel's address is known, the person allows it on the
//! computer, and forgetting the address shuts it out again.

use base64::Engine as _;
use hoista_server::{
    application::{
        engine::Engine,
        ports::{catalog::EngineError, vault::TokenVault},
    },
    domain::Secret,
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
    remote::{REVIEW_HEADER, RemoteDispatcher, RemoteRequest, RemoteResponse},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};
use tower::ServiceExt as _;

const ORIGIN: &str = "https://hoista-link.test";
const REDIRECT: &str = "https://agent.test/callback";
const PAGES: &str = "http://127.0.0.1:7444";

#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);
#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, key: &str) -> Result<Secret, EngineError> {
        self.0
            .lock()
            .expect("vault fixture")
            .get(key)
            .map(|value| Secret::new(value.clone()))
            .ok_or(EngineError::NotFound)
    }

    async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
        self.0
            .lock()
            .expect("vault fixture")
            .insert(key.into(), value.expose().into());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), EngineError> {
        self.0.lock().expect("vault fixture").remove(key);
        Ok(())
    }
}

async fn engine(temp: &tempfile::TempDir) -> Arc<Engine> {
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("state folder");
    let store = SqliteStore::open(&paths).await.expect("store");
    // The person confirms it's them (Touch ID on a Mac).
    Arc::new(
        Engine::new(
            Arc::new(store),
            Arc::new(Vault::default()),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(paths.root())),
        )
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
        )),
    )
}

async fn send(
    dispatcher: &RemoteDispatcher,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> RemoteResponse {
    dispatcher
        .dispatch(RemoteRequest {
            id: "test".into(),
            method: method.into(),
            path: path.into(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            body: body.into(),
        })
        .await
}

fn query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

#[tokio::test]
async fn a_web_agent_signs_in_to_a_computer_while_sign_in_is_open() {
    let temp = tempfile::tempdir().unwrap();
    let engine = engine(&temp).await;
    let dispatcher = RemoteDispatcher {
        engine: engine.clone(),
        local_pages: Some(PAGES.into()),
    };

    // Closed until the tunnel's address is known.
    let closed = send(&dispatcher, "POST", "/oauth/register", &[], "{}").await;
    assert_eq!(closed.status, 400);
    assert!(engine.public_url().is_none());
    assert!(engine.open_link("http://example.test").is_err());
    assert!(engine.open_link("https://example.test/path").is_err());
    engine.open_link(&format!("{ORIGIN}/")).unwrap();
    assert_eq!(engine.public_url().as_deref(), Some(ORIGIN));

    let client: Value = serde_json::from_str(
        &send(
            &dispatcher,
            "POST",
            "/oauth/register",
            &[],
            &json!({"client_name": "ChatGPT", "redirect_uris": [REDIRECT]}).to_string(),
        )
        .await
        .body,
    )
    .unwrap();
    let client_id = client["client_id"].as_str().unwrap();
    let verifier = "v".repeat(64);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let resource = format!("{ORIGIN}/mcp");
    let started = send(
        &dispatcher,
        "GET",
        &format!(
            "/oauth/authorize?{}",
            query(&[
                ("response_type", "code"),
                ("client_id", client_id),
                ("redirect_uri", REDIRECT),
                ("state", "s1"),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("resource", &resource),
                ("scope", "apps:read"),
            ])
        ),
        &[],
        "",
    )
    .await;
    assert_eq!(started.status, 303);
    let waiting_at = started.headers["location"]
        .strip_prefix(ORIGIN)
        .unwrap()
        .to_owned();
    let code = waiting_at.split("code=").nth(1).unwrap().to_owned();

    // The page anyone may have open only waits; the person answers on this
    // computer's own page, which only MCP over HTTP is told about.
    let waiting = send(&dispatcher, "GET", &waiting_at, &[], "").await;
    assert_eq!(waiting.status, 200);
    assert!(waiting.body.contains(r#"http-equiv="refresh""#));
    assert!(!waiting.body.contains("/connect/"));
    let pending = code.split('.').next().unwrap();
    assert_eq!(
        waiting.headers[REVIEW_HEADER],
        format!("{PAGES}/connect/oauth-{pending}")
    );

    // The person answers it in Hoista itself, through the owner API.
    let owner = hoista_server::catalog::router(engine.clone());
    let path = format!("/agents/oauth-requests/{pending}");
    let shown = owner
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(&path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let shown: Value =
        serde_json::from_slice(&axum::body::to_bytes(shown.into_body(), 4096).await.unwrap())
            .unwrap();
    assert_eq!(shown["name"], "ChatGPT");
    assert!(waiting.body.contains(shown["code"].as_str().unwrap()));
    let allowed = owner
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"allow": true, "scopes": ["apps:read"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), axum::http::StatusCode::OK);
    let answered = send(&dispatcher, "GET", &waiting_at, &[], "").await;
    assert_eq!(answered.status, 302);
    assert!(answered.headers["location"].starts_with(REDIRECT));
    assert!(!answered.headers.contains_key(REVIEW_HEADER));

    let token: Value = serde_json::from_str(
        &send(
            &dispatcher,
            "POST",
            "/oauth/token",
            &[],
            &query(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", client_id),
                ("redirect_uri", REDIRECT),
                ("code_verifier", &verifier),
                ("resource", &resource),
            ]),
        )
        .await
        .body,
    )
    .unwrap();
    let bearer = format!("Bearer {}", token["access_token"].as_str().unwrap());
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string();
    let listed = send(
        &dispatcher,
        "POST",
        "/mcp",
        &[("authorization", &bearer)],
        &call,
    )
    .await;
    assert_eq!(listed.status, 200);
    assert!(listed.body.contains("list_projects"));

    // Forgetting the address shuts the agent out.
    engine.close_link();
    let after = send(
        &dispatcher,
        "POST",
        "/mcp",
        &[("authorization", &bearer)],
        &call,
    )
    .await;
    assert_ne!(after.status, 200);
    // Back at the same address, it works again; at another, it signs in again.
    engine.open_link(ORIGIN).unwrap();
    let again = send(
        &dispatcher,
        "POST",
        "/mcp",
        &[("authorization", &bearer)],
        &call,
    )
    .await;
    assert_eq!(again.status, 200);
    engine.open_link("https://another-tunnel.test").unwrap();
    let elsewhere = send(
        &dispatcher,
        "POST",
        "/mcp",
        &[("authorization", &bearer)],
        &call,
    )
    .await;
    assert_eq!(elsewhere.status, 401);
}

#[tokio::test]
async fn a_server_keeps_its_own_address() {
    let temp = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(temp.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let engine = Engine::new(
        Arc::new(store),
        Arc::new(Vault::default()),
        Arc::new(LocalInspector),
        vec![],
        Arc::new(LocalBuilder::new(paths.root())),
    )
    .with_public_url("https://server.test".into());
    assert!(matches!(
        engine.open_link(ORIGIN),
        Err(EngineError::Conflict(_))
    ));
    assert_eq!(engine.public_url().as_deref(), Some("https://server.test"));
}
