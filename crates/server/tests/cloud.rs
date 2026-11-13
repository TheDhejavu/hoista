//! Hoista on a server: claimed with a passkey, then a remote agent connects
//! with OAuth and calls MCP. The browser's WebAuthn is played by the test.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use base64::Engine as _;
use hoista_server::{
    application::{
        engine::Engine,
        ports::{catalog::EngineError, vault::TokenVault},
    },
    cloud::{Cloud, router},
    domain::Secret,
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

const HOST: &str = "hoista.test";
const ORIGIN: &str = "https://hoista.test";

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

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    text: String,
}

async fn send(
    project: &axum::Router,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: Option<String>,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, HOST);
    for (name, value) in extra {
        request = request.header(*name, *value);
    }
    let request = request
        .body(body.map_or_else(Body::empty, Body::from))
        .expect("cloud fixture");
    let response = project
        .clone()
        .oneshot(request)
        .await
        .expect("cloud fixture");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("cloud fixture");
    Reply {
        status,
        headers,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn find<'a>(text: &'a str, before: &str, until: char) -> &'a str {
    let start = text
        .find(before)
        .unwrap_or_else(|| panic!("{before} not in {text}"))
        + before.len();
    let rest = &text[start..];
    &rest[..rest.find(until).expect("cloud fixture")]
}

#[tokio::test]
async fn a_server_is_claimed_with_a_passkey_and_agents_connect_with_oauth() {
    let temp = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(temp.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault::default()),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(paths.root())),
        )
        .with_public_url(ORIGIN.into())
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::PasskeyCheck,
        )),
    );
    let (cloud, code) = Cloud::new(engine.clone(), paths.root(), ORIGIN, None).unwrap();
    let code = code.expect("a new server has a claim code");
    let project = router(cloud);
    let json_post = [("origin", ORIGIN), ("content-type", "application/json")];

    // Another host name gets nothing; a form from another site is refused.
    let mut wrong = Request::get("/")
        .header(header::HOST, "evil.test")
        .body(Body::empty())
        .unwrap();
    wrong
        .headers_mut()
        .insert(header::HOST, "evil.test".parse().unwrap());
    let refused = project.clone().oneshot(wrong).await.unwrap();
    assert_eq!(refused.status(), StatusCode::MISDIRECTED_REQUEST);
    // It says where it does answer, for hoist remote check.
    assert_eq!(refused.headers()["x-hoista-address"], ORIGIN);
    // Anyone may see that it's a Hoista, where, and whether it has an owner.
    let status = send(&project, "GET", "/status", &[], None).await;
    assert_eq!(status.status, StatusCode::OK);
    let status: serde_json::Value = serde_json::from_str(&status.text).unwrap();
    assert_eq!(
        status,
        json!({"hoista": true, "address": ORIGIN, "claimed": false})
    );
    let other = send(
        &project,
        "POST",
        "/owner/claim",
        &[
            ("origin", "https://evil.test"),
            ("content-type", "application/json"),
        ],
        Some(json!({"code": code}).to_string()),
    )
    .await;
    assert_eq!(other.status, StatusCode::FORBIDDEN);
    assert!(
        send(&project, "GET", "/", &[], None)
            .await
            .text
            .contains("Set up this Hoista")
    );
    let wrong_code = send(
        &project,
        "POST",
        "/owner/claim",
        &json_post,
        Some(json!({"code": "NOPE"}).to_string()),
    )
    .await;
    assert_eq!(wrong_code.status, StatusCode::BAD_REQUEST);

    // Claim: the code, then a passkey.
    let options: Value = serde_json::from_str(
        &send(
            &project,
            "POST",
            "/owner/claim",
            &json_post,
            Some(json!({"code": code}).to_string()),
        )
        .await
        .text,
    )
    .unwrap();
    let challenge = options["challenge"].as_str().unwrap();
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let mut spki = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    spki.extend_from_slice(pair.public_key().as_ref());
    let id = b"test-credential";
    let rp = Sha256::digest(HOST.as_bytes()).to_vec();
    let client = |kind: &str, challenge: &str| {
        json!({"type": kind, "challenge": challenge, "origin": ORIGIN, "crossOrigin": false})
            .to_string()
            .into_bytes()
    };
    let mut auth = rp.clone();
    auth.push(0x45);
    auth.extend_from_slice(&0u32.to_be_bytes());
    auth.extend_from_slice(&[0u8; 16]);
    auth.extend_from_slice(&u16::try_from(id.len()).unwrap().to_be_bytes());
    auth.extend_from_slice(id);
    auth.push(0xa5);
    let registered = send(
        &project,
        "POST",
        "/owner/register",
        &json_post,
        Some(
            json!({
                "challenge": challenge,
                "registration": {
                    "id": b64(id),
                    "client_data": b64(&client("webauthn.create", challenge)),
                    "authenticator_data": b64(&auth),
                    "public_key": b64(&spki),
                    "alg": -7,
                    "name": "Laptop",
                },
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(registered.status, StatusCode::OK, "{}", registered.text);
    // The claim code is used up.
    let again = send(
        &project,
        "POST",
        "/owner/claim",
        &json_post,
        Some(json!({"code": code}).to_string()),
    )
    .await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    let status = send(&project, "GET", "/status", &[], None).await;
    assert!(status.text.contains(r#""claimed":true"#), "{}", status.text);

    // A remote agent registers and starts OAuth with PKCE.
    let registration: Value = serde_json::from_str(&send(&project, "POST", "/oauth/register", &[("content-type", "application/json")], Some(json!({"client_name": "Remote test", "redirect_uris": ["https://agent.example/cb"], "token_endpoint_auth_method": "none"}).to_string())).await.text).unwrap();
    let client_id = registration["client_id"].as_str().unwrap();
    let verifier = "v".repeat(50);
    let pkce = b64(&Sha256::digest(verifier.as_bytes()));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", "https://agent.example/cb")
        .append_pair("state", "st")
        .append_pair("code_challenge", &pkce)
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", &format!("{ORIGIN}/mcp"))
        .append_pair("scope", "apps:read apps:write apps:create")
        .finish();
    let started = send(
        &project,
        "GET",
        &format!("/oauth/authorize?{query}"),
        &[],
        None,
    )
    .await;
    assert_eq!(started.status, StatusCode::SEE_OTHER);
    let waiting = started.headers[header::LOCATION]
        .to_str()
        .unwrap()
        .trim_start_matches(ORIGIN)
        .to_owned();
    let page = send(&project, "GET", &waiting, &[], None).await.text;
    let review = find(&page, "href=\"https://hoista.test", '"').replace("&amp;", "&");

    // Not signed in: sent to sign in, and a form post is refused.
    let blocked = send(&project, "GET", &review, &[], None).await;
    assert_eq!(blocked.status, StatusCode::SEE_OTHER);
    assert!(
        blocked.headers[header::LOCATION]
            .to_str()
            .unwrap()
            .starts_with("/?next=")
    );

    // Sign in with the passkey.
    let options: Value = serde_json::from_str(
        &send(
            &project,
            "POST",
            "/owner/sign-in/options",
            &json_post,
            Some("{}".into()),
        )
        .await
        .text,
    )
    .unwrap();
    let challenge = options["challenge"].as_str().unwrap();
    let mut data = rp.clone();
    data.push(0x05);
    data.extend_from_slice(&1u32.to_be_bytes());
    let cdj = client("webauthn.get", challenge);
    let mut signed = data.clone();
    signed.extend_from_slice(&Sha256::digest(&cdj));
    let signature = pair.sign(&rng, &signed).unwrap();
    let signed_in = send(
        &project,
        "POST",
        "/owner/sign-in",
        &json_post,
        Some(
            json!({
                "challenge": challenge,
                "assertion": {
                    "id": b64(id),
                    "client_data": b64(&cdj),
                    "authenticator_data": b64(&data),
                    "signature": b64(signature.as_ref()),
                },
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.text);
    let cookie = signed_in.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    // Review and allow; the browser goes back to the agent with a code.
    let consent = send(&project, "GET", &review, &[("cookie", &cookie)], None)
        .await
        .text;
    assert!(consent.contains("Remote test") && consent.contains("your Hoista at hoista.test"));
    let token = find(&consent, "name=\"token\" value=\"", '"').to_owned();
    let back = find(&consent, "name=\"back\" value=\"", '"').replace("&amp;", "&");
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", &token)
        .append_pair("back", &back)
        .append_pair("decision", "allow")
        .append_pair("access", "view")
        .append_pair("production", "same")
        .append_pair("project", "all")
        .append_pair("environments", "all")
        .append_pair("ends", "0")
        .finish();
    let path = review.split('?').next().unwrap();
    let answered = send(
        &project,
        "POST",
        path,
        &[
            ("origin", ORIGIN),
            ("content-type", "application/x-www-form-urlencoded"),
            ("cookie", &cookie),
        ],
        Some(form),
    )
    .await;
    assert_eq!(answered.status, StatusCode::SEE_OTHER, "{}", answered.text);
    let sent_back = send(&project, "GET", &back, &[], None).await;
    assert_eq!(sent_back.status, StatusCode::FOUND);
    let location = sent_back.headers[header::LOCATION].to_str().unwrap();
    assert!(location.starts_with("https://agent.example/cb?code="));
    let code = url::Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .into_owned();

    // The agent trades the code for a token and calls MCP.
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", &code)
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", "https://agent.example/cb")
        .append_pair("code_verifier", &verifier)
        .append_pair("resource", &format!("{ORIGIN}/mcp"))
        .finish();
    let issued: Value = serde_json::from_str(
        &send(
            &project,
            "POST",
            "/oauth/token",
            &[("content-type", "application/x-www-form-urlencoded")],
            Some(form),
        )
        .await
        .text,
    )
    .unwrap();
    let access = issued["access_token"].as_str().expect("a token");
    let bearer = format!("Bearer {access}");
    let called = send(
        &project,
        "POST",
        "/mcp",
        &[
            ("authorization", &bearer),
            ("content-type", "application/json"),
        ],
        Some(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": "get_my_access", "arguments": {}},
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(called.status, StatusCode::OK);
    assert!(called.text.contains("every project"), "{}", called.text);
    // An agent's token is not an owner session.
    let fake = format!("__Host-hoista={access}");
    assert!(
        send(&project, "GET", "/", &[("cookie", &fake)], None)
            .await
            .text
            .contains("Sign in to Hoista")
    );
    store.close().await;
}

#[tokio::test]
async fn a_server_made_by_hoista_uses_the_code_it_was_given() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault::default()),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(paths.root())),
        )
        .with_public_url(ORIGIN.into()),
    );
    let given = hoista_server::application::agents::passkey::claim_code();
    let (_, code) = Cloud::new(engine.clone(), paths.root(), ORIGIN, Some(&given)).expect("cloud");
    assert_eq!(code.as_deref(), Some(given.as_str()));
    // Something that isn't a claim code is ignored for a fresh one.
    let (_, code) =
        Cloud::new(engine.clone(), paths.root(), ORIGIN, Some("password")).expect("cloud");
    assert_ne!(code.as_deref(), Some("password"));
    // Plain HTTP only for a Hoista on this computer (in Docker here).
    assert!(Cloud::new(engine.clone(), paths.root(), "http://localhost:8787", None).is_ok());
    assert!(Cloud::new(engine, paths.root(), "http://hoista.example.com", None).is_err());
    store.close().await;
}

/// Claims a fresh server at `base` (whose public address is `origin`) with
/// a new passkey; returns the signed-in session cookie.
async fn claim(http: &reqwest::Client, base: &str, host: &str, origin: &str, code: &str) -> String {
    let post = |path: &str, body: Value| {
        http.post(format!("{base}{path}"))
            .header("origin", origin)
            .json(&body)
    };
    let options: Value = post("/owner/claim", json!({"code": code}))
        .send()
        .await
        .expect("claim")
        .json()
        .await
        .expect("claim options");
    let registered = register(
        http,
        base,
        host,
        origin,
        options["challenge"].as_str().expect("challenge"),
        None,
        b"computer-test",
    )
    .await;
    assert!(registered.status().is_success());
    registered.headers()["set-cookie"]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie")
        .to_owned()
}

/// Makes a new passkey for `challenge` (a test authenticator's) and sends it,
/// signed in with `cookie` when adding one to an owned server.
async fn register(
    http: &reqwest::Client,
    base: &str,
    host: &str,
    origin: &str,
    challenge: &str,
    cookie: Option<&str>,
    id: &[u8],
) -> reqwest::Response {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).expect("key");
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
        .expect("key");
    let mut spki = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    spki.extend_from_slice(pair.public_key().as_ref());
    let rp_id = host.split(':').next().expect("host");
    let mut auth = Sha256::digest(rp_id.as_bytes()).to_vec();
    auth.push(0x45);
    auth.extend_from_slice(&0u32.to_be_bytes());
    auth.extend_from_slice(&[0u8; 16]);
    auth.extend_from_slice(&u16::try_from(id.len()).expect("id").to_be_bytes());
    auth.extend_from_slice(id);
    auth.push(0xa5);
    let client_data = json!({"type": "webauthn.create", "challenge": challenge, "origin": origin, "crossOrigin": false}).to_string();
    let mut request = http
        .post(format!("{base}/owner/register"))
        .header("origin", origin)
        .json(&json!({
            "challenge": challenge,
            "registration": {
                "id": b64(id),
                "client_data": b64(client_data.as_bytes()),
                "authenticator_data": b64(&auth),
                "public_key": b64(&spki),
                "alg": -7,
                "name": "Laptop",
            },
        }));
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    request.send().await.expect("register")
}

#[tokio::test]
async fn the_owner_adds_a_second_passkey() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let host = listener.local_addr().expect("address").to_string();
    let origin = format!("https://{host}");
    let base = format!("http://{host}");
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault::default()),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(paths.root())),
        )
        .with_public_url(origin.clone()),
    );
    let (cloud, code) = Cloud::new(engine, paths.root(), &origin, None).expect("cloud");
    let project = hoista_server::cloud::router(cloud);
    tokio::spawn(async move {
        let _ = axum::serve(listener, project).await;
    });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    let cookie = claim(&http, &base, &host, &origin, &code.expect("claim code")).await;
    // Signed in just now: another device's passkey can be added.
    let options: Value = http
        .post(format!("{base}/owner/add"))
        .header("origin", &origin)
        .header("cookie", &cookie)
        .json(&json!({}))
        .send()
        .await
        .expect("add")
        .json()
        .await
        .expect("add options");
    let added = register(
        &http,
        &base,
        &host,
        &origin,
        options["challenge"].as_str().expect("challenge"),
        Some(&cookie),
        b"phone-test",
    )
    .await;
    let status = added.status();
    assert!(
        status.is_success(),
        "{status}: {}",
        added.text().await.unwrap_or_default()
    );
    store.close().await;
}

#[tokio::test]
async fn a_computer_signed_in_on_the_servers_page_manages_it_with_hoist() {
    use hoista_server::client::{ClientError, LocalClient};
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let host = listener.local_addr().expect("address").to_string();
    let origin = format!("https://{host}");
    let base = format!("http://{host}");
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault::default()),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(paths.root())),
        )
        .with_public_url(origin.clone())
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::PasskeyCheck,
        ))
        .with_transfer_seal(Arc::new(
            hoista_server::infrastructure::transfer_seal::LocalTransferSeal::default(),
        )),
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let api = hoista_server::http::router_with_lifecycle(
        store.clone(),
        stop.clone(),
        Some(engine.clone()),
        None,
    )
    .merge(hoista_server::catalog::router(engine.clone()));
    let (cloud, code) = Cloud::new(engine, paths.root(), &origin, None).expect("cloud");
    let project = hoista_server::cloud::router_with_api(cloud, api);
    tokio::spawn(async move {
        let _ = axum::serve(listener, project).await;
    });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    let start = || {
        http.post(format!("{base}/owner/devices/start"))
            .json(&json!({"name": "Work laptop"}))
            .send()
    };

    // Before anyone owns it, no computer can start signing in.
    assert_eq!(
        start().await.expect("start").status(),
        reqwest::StatusCode::CONFLICT
    );
    let cookie = claim(&http, &base, &host, &origin, &code.expect("claim code")).await;

    let started: Value = start().await.expect("start").json().await.expect("start");
    let poll = started["poll"].as_str().expect("poll").to_owned();
    let device_code = started["code"].as_str().expect("code").to_owned();
    assert!(
        started["link"]
            .as_str()
            .expect("link")
            .ends_with(&format!("/owner/devices#code={device_code}"))
    );
    let polled = || {
        http.post(format!("{base}/owner/devices/poll"))
            .json(&json!({"poll": poll}))
            .send()
    };
    assert_eq!(
        polled().await.expect("poll").status(),
        reqwest::StatusCode::ACCEPTED
    );

    // The page needs the owner; a wrong code or another site allows nothing.
    let page = || {
        http.get(format!("{base}/owner/devices"))
            .header("cookie", &cookie)
            .send()
    };
    let text = page().await.expect("page").text().await.expect("page");
    assert!(text.contains("Allow a computer"));
    let home = || {
        http.get(format!("{base}/"))
            .header("cookie", &cookie)
            .send()
    };
    let listed = home().await.expect("home").text().await.expect("home");
    assert!(listed.contains("Signed-in computers") && listed.contains("None yet"));
    let signed_out = http
        .get(format!("{base}/owner/devices"))
        .send()
        .await
        .expect("page");
    assert_eq!(signed_out.headers()["location"], "/?next=/owner/devices");
    let allow = |code: &str, from: &str| {
        http.post(format!("{base}/owner/devices/allow"))
            .header("cookie", &cookie)
            .header("origin", from)
            .form(&[("code", code)])
            .send()
    };
    let wrong = allow("AAAA-BBBB-CCCC", &origin).await.expect("allow");
    assert!(
        wrong
            .text()
            .await
            .expect("allow")
            .contains("No computer with that code")
    );
    let elsewhere = allow(&device_code, "https://evil.test")
        .await
        .expect("allow");
    assert_eq!(elsewhere.status(), reqwest::StatusCode::FORBIDDEN);
    let allowed = allow(&device_code.to_lowercase(), &origin)
        .await
        .expect("allow")
        .text()
        .await
        .expect("allow");
    assert!(
        allowed.contains("Work laptop can use your Hoista"),
        "{allowed}"
    );

    // The computer's next poll gets its key, once; the server keeps only a hash.
    let got: Value = polled().await.expect("poll").json().await.expect("poll");
    let key = got["key"].as_str().expect("key").to_owned();
    assert_eq!(
        polled().await.expect("poll").status(),
        reqwest::StatusCode::GONE
    );
    let owner_file = std::fs::read_to_string(paths.root().join("owner.json")).expect("owner");
    assert!(owner_file.contains("Work laptop") && !owner_file.contains(&key));

    // hoist on that computer, pointed at the server.
    let client = LocalClient::on_server(&paths, &base, Secret::new(key.clone())).expect("client");
    assert!(client.is_on_server());
    assert!(client.health().await.is_ok());
    let projects = client.get_json("/projects").await.expect("projects");
    assert!(projects["items"].is_array(), "{projects}");
    // A computer's key isn't a passkey sign-in: guarded changes still need the page.
    let guarded = client
        .patch_json("/settings", &json!({"network_page": true}))
        .await;
    assert!(
        matches!(&guarded, Err(ClientError::Rejected(text)) if text.contains("passkey")),
        "{guarded:?}"
    );
    // Any other key gets nothing.
    let stranger =
        LocalClient::on_server(&paths, &base, Secret::new("not-a-key".into())).expect("client");
    assert!(matches!(
        stranger.get_json("/projects").await,
        Err(ClientError::Rejected(text)) if text.contains("hoist remote login")
    ));
    // Agents' credentials don't go through hoist to a server.
    assert!(
        client
            .authenticated::<Value>(axum::http::Method::POST, "/mcp", None, Some("agent"))
            .await
            .is_err()
    );
    // Only the server's own kind of address is accepted.
    assert!(
        LocalClient::on_server(&paths, "http://example.com", Secret::new(key.clone())).is_err()
    );

    // Removed on the page: the key stops working.
    let text = home().await.expect("home").text().await.expect("home");
    let id = text
        .split(r#"name="id" value=""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("device id")
        .to_owned();
    let removed = http
        .post(format!("{base}/owner/devices/remove"))
        .header("cookie", &cookie)
        .header("origin", &origin)
        .form(&[("id", id.as_str())])
        .send()
        .await
        .expect("remove");
    assert!(removed.status().is_redirection());
    assert!(client.get_json("/projects").await.is_err());

    // The desktop app signs in through the browser instead: OAuth with PKCE, back
    // to its loopback address. No code to compare.
    use base64::Engine as _;
    use sha2::Digest as _;
    let verifier = "a-long-random-secret-only-the-app-keeps-0123456789";
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    let back = "http://127.0.0.1:54321/done";
    let connect_path = format!(
        "/owner/devices/connect?name=Sam%27s%20MacBook&redirect={}&challenge={challenge}&state=s1",
        url::form_urlencoded::byte_serialize(back.as_bytes()).collect::<String>()
    );
    // Signed out, it goes to sign in first, then back to the same page.
    let signed_out = http
        .get(format!("{base}{connect_path}"))
        .send()
        .await
        .expect("connect");
    let to = signed_out.headers()["location"].to_str().expect("location");
    assert!(
        to.starts_with("/?next=%2Fowner%2Fdevices%2Fconnect%3F"),
        "{to}"
    );
    let page = http
        .get(format!("{base}{connect_path}"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("connect")
        .text()
        .await
        .expect("connect");
    assert!(
        page.contains("Hoista on Sam&#39;s MacBook") || page.contains("Hoista on Sam's MacBook"),
        "{page}"
    );
    // Only back to this computer: any other address is refused.
    let elsewhere = http
        .get(format!(
            "{base}/owner/devices/connect?name=x&redirect=https%3A%2F%2Fevil.test%2F&challenge={challenge}&state=s1"
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("connect")
        .text()
        .await
        .expect("connect");
    assert!(elsewhere.contains("Start again in the app"));
    let allowed: Value = http
        .post(format!("{base}/owner/devices/connect"))
        .header("cookie", &cookie)
        .header("origin", &origin)
        .json(&json!({"name": "Sam's MacBook", "redirect": back, "challenge": challenge, "state": "s1"}))
        .send()
        .await
        .expect("allow")
        .json()
        .await
        .expect("allow");
    let to = url::Url::parse(allowed["to"].as_str().expect("to")).expect("to");
    assert_eq!(to.origin().ascii_serialization(), "http://127.0.0.1:54321");
    let pairs: std::collections::HashMap<_, _> = to.query_pairs().into_owned().collect();
    assert_eq!(pairs.get("state").map(String::as_str), Some("s1"));
    let one_time = pairs.get("code").expect("code").clone();
    let trade = |verifier: &str| {
        http.post(format!("{base}/owner/devices/token"))
            .json(&json!({"code": one_time, "verifier": verifier, "redirect": back}))
            .send()
    };
    // The wrong secret spends the code: it can't be tried again.
    assert_eq!(
        trade("someone-else-guessing-the-secret-0123456789abc")
            .await
            .expect("token")
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let allowed_again: Value = http
        .post(format!("{base}/owner/devices/connect"))
        .header("cookie", &cookie)
        .header("origin", &origin)
        .json(&json!({"name": "Sam's MacBook", "redirect": back, "challenge": challenge, "state": "s1"}))
        .send()
        .await
        .expect("allow")
        .json()
        .await
        .expect("allow");
    let to = url::Url::parse(allowed_again["to"].as_str().expect("to")).expect("to");
    let one_time = to
        .query_pairs()
        .find(|(name, _)| name == "code")
        .map(|(_, value)| value.into_owned())
        .expect("code");
    let trade = |verifier: &str| {
        http.post(format!("{base}/owner/devices/token"))
            .json(&json!({"code": one_time, "verifier": verifier, "redirect": back}))
            .send()
    };
    let got: Value = trade(verifier)
        .await
        .expect("token")
        .json()
        .await
        .expect("token");
    let key = got["key"].as_str().expect("key").to_owned();
    assert_eq!(got["name"], "Sam's MacBook");
    assert_eq!(
        trade(verifier).await.expect("token").status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let app = LocalClient::on_server(&paths, &base, Secret::new(key)).expect("client");
    assert!(app.get_json("/projects").await.is_ok());
    stop.cancel();
    store.close().await;
}

/// Signs a computer in on the server's page; returns its key.
async fn allow_computer(http: &reqwest::Client, base: &str, origin: &str, cookie: &str) -> String {
    let started: Value = http
        .post(format!("{base}/owner/devices/start"))
        .json(&json!({"name": "Laptop"}))
        .send()
        .await
        .expect("start")
        .json()
        .await
        .expect("start");
    let allowed = http
        .post(format!("{base}/owner/devices/allow"))
        .header("cookie", cookie)
        .header("origin", origin)
        .form(&[("code", started["code"].as_str().expect("code"))])
        .send()
        .await
        .expect("allow");
    assert!(allowed.status().is_success());
    let got: Value = http
        .post(format!("{base}/owner/devices/poll"))
        .json(&json!({"poll": started["poll"]}))
        .send()
        .await
        .expect("poll")
        .json()
        .await
        .expect("poll");
    got["key"].as_str().expect("key").to_owned()
}

#[tokio::test]
async fn a_computer_moves_its_hoista_to_an_empty_server() {
    use hoista_server::application::ports::catalog::CatalogRepository as _;
    use hoista_server::client::{ClientError, LocalClient};
    use hoista_server::domain::model::*;
    // This computer: one connected account with its sign-in, and a project.
    let here = tempfile::tempdir().expect("temp");
    let here_paths = StatePaths::new(Some(here.path().join("state"))).expect("paths");
    let here_store = SqliteStore::open(&here_paths).await.expect("store");
    let here_vault = Arc::new(Vault::default());
    let computer = Arc::new(
        Engine::new(
            Arc::new(here_store.clone()),
            here_vault.clone(),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(here_paths.root())),
        )
        .with_transfer_seal(Arc::new(
            hoista_server::infrastructure::transfer_seal::LocalTransferSeal::default(),
        )),
    );
    here_store
        .put_account(&Account {
            last_discovery: None,
            id: "acct-1".into(),
            provider: hoista_server::Provider::Railway,
            label: "Sam's workspace".into(),
            external_id: "ws".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    here_vault
        .write("account/acct-1", Secret::new("railway-token-123".into()))
        .await
        .expect("token");
    let source = tempfile::tempdir().expect("source");
    let project = computer
        .projects()
        .create_project(
            &hoista_server::application::engine::Actor::Person,
            "shop".into(),
            Source::Folder {
                path: source.path().to_str().expect("path").into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let (exports, _work) = hoista_server::transfer::router(computer.clone());
    let file = here.path().join("move.hoista");
    let passphrase = "one-time-passphrase-0123456789abcdef";
    let exported = exports
        .oneshot(
            Request::post("/transfer/export")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"path": file, "passphrase": passphrase}).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("export");
    assert_eq!(exported.status(), StatusCode::OK);
    let bytes = std::fs::read(&file).expect("file");
    assert!(
        !String::from_utf8_lossy(&bytes).contains("railway-token-123"),
        "the export is sealed"
    );

    // The server, empty, with this computer signed in.
    let there = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(there.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let host = listener.local_addr().expect("address").to_string();
    let origin = format!("https://{host}");
    let base = format!("http://{host}");
    let server_vault = Arc::new(Vault::default());
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            server_vault.clone(),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(paths.root())),
        )
        .with_public_url(origin.clone())
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::PasskeyCheck,
        ))
        .with_transfer_seal(Arc::new(
            hoista_server::infrastructure::transfer_seal::LocalTransferSeal::default(),
        )),
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let (moves, _work) = hoista_server::transfer::receive_router(engine.clone());
    let api = hoista_server::http::router_with_lifecycle(
        store.clone(),
        stop.clone(),
        Some(engine.clone()),
        None,
    )
    .merge(hoista_server::catalog::router(engine.clone()))
    .merge(moves);
    let (cloud, code) = Cloud::new(engine.clone(), paths.root(), &origin, None).expect("cloud");
    let served = hoista_server::cloud::router_with_api(cloud, api);
    tokio::spawn(async move {
        let _ = axum::serve(listener, served).await;
    });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    let cookie = claim(&http, &base, &host, &origin, &code.expect("code")).await;
    let key = allow_computer(&http, &base, &origin, &cookie).await;
    let client = LocalClient::on_server(&here_paths, &base, Secret::new(key)).expect("client");

    // A wrong passphrase restores nothing.
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    assert!(
        client
            .post_json_long_running(
                "/transfer/receive",
                &json!({"passphrase": "not-the-passphrase-at-all", "data": data}),
            )
            .await
            .is_err()
    );
    let restored = client
        .post_json_long_running(
            "/transfer/receive",
            &json!({"passphrase": passphrase, "data": data}),
        )
        .await
        .expect("restored");
    assert_eq!(restored["credentials_staged"], 1);

    // The server now has the project and the account, with its sign-in.
    let projects = client.get_json("/projects").await.expect("projects");
    assert_eq!(projects["items"][0]["id"], project.id.as_str());
    let accounts = client.get_json("/accounts").await.expect("accounts");
    let moved = accounts["items"][0]["id"]
        .as_str()
        .expect("account")
        .to_owned();
    assert_eq!(accounts["items"][0]["label"], "Sam's workspace");
    assert_eq!(
        server_vault
            .read(&format!("account/{moved}"))
            .await
            .expect("token")
            .expose(),
        "railway-token-123"
    );
    // A second move can't overwrite it.
    assert!(matches!(
        client
            .post_json_long_running(
                "/transfer/receive",
                &json!({"passphrase": passphrase, "data": data}),
            )
            .await,
        Err(ClientError::Rejected(_))
    ));
    stop.cancel();
    store.close().await;
    here_store.close().await;
}
