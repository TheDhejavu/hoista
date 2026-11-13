//! Hoista on a server: one HTTPS address for remote agents (OAuth and MCP)
//! and for the owner, who signs in with a passkey (the first one is registered
//! with a one-time code from the server's log).
//!
//! Responses can't be cached or framed; other origins and host names are refused.

use crate::{
    agent_http,
    application::{
        agents::passkey::{self, Assertion, Expected, Passkey, Registration},
        engine::Engine,
        ports::owner_check::as_signed_in_owner,
    },
    remote::RemoteDispatcher,
    views,
};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_SESSIONS: usize = 64;
const MAX_CHALLENGES: usize = 256;
/// Addresses whose request rate is tracked at once.
const MAX_TRACKED_ADDRESSES: usize = 4096;
const REQUESTS_PER_MINUTE: u32 = 120;
/// The longest page to go on to after signing in.
const MAX_NEXT_BYTES: usize = 600;
const MAX_PASSKEYS: usize = 10;

const COOKIE: &str = "__Host-hoista";
const SESSION: Duration = Duration::from_secs(12 * 60 * 60);
/// How recent a passkey sign-in must be to answer an agent.
const FRESH: Duration = Duration::from_secs(10 * 60);
const CHALLENGE: Duration = Duration::from_secs(5 * 60);
const CLAIM_TRIES: u32 = 10;

mod devices;

/// The owner's passkeys, kept beside the state (public keys only).
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    passkeys: Vec<Passkey>,
    /// Computers signed in to manage this Hoista with `hoist`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    devices: Vec<devices::Device>,
}

struct Session {
    expires: Instant,
    fresh_until: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// First passkey, after the claim code.
    Claim,
    /// Another passkey, by a signed-in owner.
    Add,
    SignIn,
}

struct Claim {
    code: String,
    tries: u32,
}

/// Hoista on a server.
pub struct Cloud {
    engine: Arc<Engine>,
    dispatcher: RemoteDispatcher,
    /// `https://host`.
    origin: String,
    host: String,
    owner_file: PathBuf,
    owner: Mutex<Owner>,
    sessions: Mutex<HashMap<[u8; 32], Session>>,
    challenges: Mutex<HashMap<String, (Purpose, Instant)>>,
    claim: Mutex<Option<Claim>>,
    rates: Mutex<HashMap<String, (u64, u32)>>,
    /// Computer sign-ins waiting for the person, by the hash of their poll secret.
    waiting: Mutex<HashMap<String, devices::Waiting>>,
    /// Allows from the browser, waiting for the app, by the hash of their code.
    connects: Mutex<HashMap<String, devices::Connect>>,
    /// The image the Hoista that made this server named in `HOISTA_IMAGE`.
    /// What the provider says it started; check it against the release.
    image: Option<String>,
}

impl Cloud {
    /// Sets up the server at `public_url` (`https://host`), keeping the
    /// owner's passkeys in `state/owner.json`. While nobody owns it, the claim
    /// code is `preset` (set by the Hoista that created the server, which
    /// opens the claim page with it) or a new one; it's returned so the
    /// caller can print it to the server's own log.
    ///
    /// # Errors
    /// Rejects an address that isn't `https://host` (or `http://localhost:PORT`)
    /// and an unreadable owner file.
    pub fn new(
        engine: Arc<Engine>,
        state: &std::path::Path,
        public_url: &str,
        preset: Option<&str>,
    ) -> Result<(Arc<Self>, Option<String>), String> {
        let url = url::Url::parse(public_url)
            .map_err(|_| "HOISTA_PUBLIC_URL isn't an address.".to_owned())?;
        // Plain HTTP only on this computer (Hoista in Docker here): browsers
        // treat localhost as secure, so passkeys and secure cookies work.
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        let host = match (url.scheme(), url.host_str(), url.path()) {
            ("https", Some(host), "/" | "") | ("http", Some(host), "/" | "")
                if url.scheme() == "https" || loopback =>
            {
                match url.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_owned(),
                }
            }
            _ => {
                return Err("HOISTA_PUBLIC_URL must be https://your-host (or http://localhost:PORT on this computer), with no path.".into());
            }
        };
        let origin = format!("{}://{host}", url.scheme());
        let owner_file = state.join("owner.json");
        let owner: Owner = match std::fs::read(&owner_file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| "The owner file in the state folder isn't readable.".to_owned())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Owner::default(),
            Err(error) => return Err(format!("Couldn't read the owner file: {error}")),
        };
        let claim = owner.passkeys.is_empty().then(|| Claim {
            code: preset
                .filter(|code| passkey::is_claim_code(code))
                .map_or_else(passkey::claim_code, str::to_owned),
            tries: 0,
        });
        let code = claim.as_ref().map(|claim| claim.code.clone());
        Ok((
            Arc::new(Self {
                dispatcher: RemoteDispatcher {
                    engine: engine.clone(),
                    local_pages: None,
                },
                engine,
                origin,
                host,
                owner_file,
                owner: Mutex::new(owner),
                sessions: Mutex::default(),
                challenges: Mutex::default(),
                claim: Mutex::new(claim),
                rates: Mutex::default(),
                waiting: Mutex::default(),
                connects: Mutex::default(),
                image: std::env::var("HOISTA_IMAGE")
                    .ok()
                    .filter(|image| crate::is_pinned_image(image)),
            }),
            code,
        ))
    }

    fn rp_id(&self) -> &str {
        self.host.split(':').next().unwrap_or(&self.host)
    }

    fn save_owner(&self, owner: &Owner) -> std::io::Result<()> {
        use std::io::Write as _;
        let next = self.owner_file.with_extension("next");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&next)?;
        file.write_all(&serde_json::to_vec_pretty(owner).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        std::fs::rename(next, &self.owner_file)?;
        Ok(())
    }

    fn session(&self, headers: &HeaderMap) -> Option<(bool, [u8; 32])> {
        let token = cookie(headers)?;
        let key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut sessions = self.sessions.lock().ok()?;
        let now = Instant::now();
        sessions.retain(|_, session| session.expires > now);
        let session = sessions.get(&key)?;
        Some((session.fresh_until > now, key))
    }

    /// A new signed-in session; returns the cookie to set.
    fn sign_in(&self) -> Option<String> {
        let token = passkey::challenge();
        let key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let now = Instant::now();
        let mut sessions = self.sessions.lock().ok()?;
        if sessions.len() > MAX_SESSIONS {
            sessions.clear();
        }
        sessions.insert(
            key,
            Session {
                expires: now + SESSION,
                fresh_until: now + FRESH,
            },
        );
        Some(format!(
            "{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
            SESSION.as_secs()
        ))
    }

    fn issue(&self, purpose: Purpose) -> Option<String> {
        let challenge = passkey::challenge();
        let mut challenges = self.challenges.lock().ok()?;
        let now = Instant::now();
        challenges.retain(|_, (_, issued)| now.duration_since(*issued) < CHALLENGE);
        if challenges.len() > MAX_CHALLENGES {
            return None;
        }
        challenges.insert(challenge.clone(), (purpose, now));
        Some(challenge)
    }

    /// Uses up a challenge Hoista issued for `purpose`. One issued for
    /// something else is left as it is, so asking about each purpose in
    /// turn doesn't spend it.
    fn take(&self, challenge: &str, purpose: Purpose) -> bool {
        let Ok(mut challenges) = self.challenges.lock() else {
            return false;
        };
        let fits = matches!(
            challenges.get(challenge),
            Some((issued_for, at)) if *issued_for == purpose && at.elapsed() < CHALLENGE
        );
        if fits {
            challenges.remove(challenge);
        }
        fits
    }

    /// Up to 120 agent requests a minute from one address.
    fn allowed(&self, headers: &HeaderMap) -> bool {
        let address = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .unwrap_or("unknown")
            .trim()
            .to_owned();
        let minute = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |now| now.as_secs() / 60);
        let Ok(mut rates) = self.rates.lock() else {
            return false;
        };
        rates.retain(|_, (at, _)| *at == minute);
        if rates.len() >= MAX_TRACKED_ADDRESSES && !rates.contains_key(&address) {
            return false;
        }
        let rate = rates.entry(address).or_insert((minute, 0));
        rate.1 += 1;
        rate.1 <= REQUESTS_PER_MINUTE
    }
}

fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|resource| {
            resource
                .trim()
                .strip_prefix(&format!("{COOKIE}="))
                .map(str::to_owned)
        })
}

/// The server's routes, with no owner API.
pub fn router(cloud: Arc<Cloud>) -> Router {
    router_with_api(cloud, Router::new())
}

/// The server's routes, with `api` (the same owner API `hoist` uses on a
/// computer) under `/api` for signed-in computers.
pub fn router_with_api(cloud: Arc<Cloud>, api: Router) -> Router {
    let pages = super::pages::cloud_router(cloud.engine.clone(), &cloud.origin)
        .layer(middleware::from_fn_with_state(cloud.clone(), owner_only));
    let agents = Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(server_metadata),
        )
        .route(
            "/oauth/register",
            post(forward).options(agent_http::preflight),
        )
        .route("/oauth/authorize", get(forward))
        .route("/oauth/token", post(forward).options(agent_http::preflight))
        .route("/status", get(status))
        .route(
            "/mcp",
            post(forward)
                .get(forward)
                .delete(forward)
                .options(agent_http::preflight),
        );
    let owner = Router::new()
        .route("/", get(home))
        .route("/owner/claim", post(claim))
        .route("/owner/add", post(add_passkey))
        .route("/owner/register", post(register))
        .route("/owner/sign-in/options", post(sign_in_options))
        .route("/owner/sign-in", post(sign_in))
        .route("/owner/sign-out", post(sign_out))
        .route("/owner/passkeys/remove", post(remove_passkey));
    // Where agents connect: on a server, its own public address.
    let public = cloud.origin.clone();
    let api = api
        .route(
            "/agents/address",
            get(move || {
                let public = public.clone();
                async move {
                    Json(serde_json::json!({ "local": null, "network": null, "public": public }))
                }
            }),
        )
        .layer(middleware::from_fn_with_state(
            cloud.clone(),
            devices::device_only,
        ));
    Router::new()
        .merge(owner)
        .merge(agents)
        .merge(devices::routes())
        .with_state(cloud.clone())
        .merge(pages)
        .nest("/api", api)
        .layer(middleware::from_fn_with_state(cloud, guard))
        .route("/healthz", get(|| async { "ok" }))
}

/// Answers only for this server's own name, and forms only from it.
/// What anyone may know about this Hoista, for `hoist remote check`: that it
/// is one, the address it answers at, and whether it has an owner yet. The
/// front page already shows the same.
async fn status(State(cloud): State<Arc<Cloud>>) -> Json<serde_json::Value> {
    let claimed = cloud
        .owner
        .lock()
        .is_ok_and(|owner| !owner.passkeys.is_empty());
    Json(serde_json::json!({
        "hoista": true,
        "address": cloud.origin,
        "claimed": claimed,
    }))
}

async fn guard(State(cloud): State<Arc<Cloud>>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if host != cloud.host {
        // Says where it does answer, so `hoist remote check` can tell a tunnel
        // that changes the host name from a Hoista started with another address.
        let mut refused = (
            StatusCode::MISDIRECTED_REQUEST,
            format!("Use this Hoista's own address: {}", cloud.origin),
        )
            .into_response();
        if let Ok(address) = header::HeaderValue::from_str(&cloud.origin) {
            refused.headers_mut().insert("x-hoista-address", address);
        }
        return refused;
    }
    let path = request.uri().path().to_owned();
    // Agents and signed-in computers send their own keys, never this page's cookie.
    let agent_route = path.starts_with("/oauth/")
        || path == "/mcp"
        || path.starts_with("/.well-known/")
        || path.starts_with("/api/")
        || path == "/owner/devices/start"
        || path == "/owner/devices/poll"
        || path == "/owner/devices/token";
    if request.method() == Method::POST && !agent_route {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok());
        if origin != Some(cloud.origin.as_str()) {
            return (
                StatusCode::FORBIDDEN,
                "This only works from Hoista's own page.",
            )
                .into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "same-origin"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=31536000; includeSubDomains",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    if !headers.contains_key(header::CONTENT_SECURITY_POLICY) {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; img-src data:; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"),
        );
    }
    response
}

/// Consent pages: only for the owner who signed in with a passkey in the
/// last ten minutes, and run as that owner so Hoista's owner check passes.
async fn owner_only(State(cloud): State<Arc<Cloud>>, request: Request, next: Next) -> Response {
    match cloud.session(request.headers()) {
        Some((true, _)) => as_signed_in_owner(next.run(request)).await,
        _ if request.method() == Method::GET => {
            let next_path = request
                .uri()
                .path_and_query()
                .map_or_else(|| "/".to_owned(), |path| path.as_str().to_owned());
            Redirect::to(&format!(
                "/?next={}",
                url::form_urlencoded::byte_serialize(next_path.as_bytes()).collect::<String>()
            ))
            .into_response()
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            views::html(&views::MessagePage {
                title: "Sign in again",
                heading: "Sign in again",
                text: &[views::Inline::Text(
                    "It's been more than ten minutes since you signed in. Go back, reload the page and sign in with your passkey. Nothing was changed.",
                )],
                button: None,
            }),
        )
            .into_response(),
    }
}

async fn resource_metadata(State(cloud): State<Arc<Cloud>>) -> Response {
    agent_http::resource_metadata(&cloud.origin)
}

async fn server_metadata(State(cloud): State<Arc<Cloud>>) -> Response {
    agent_http::server_metadata(&cloud.origin)
}

/// OAuth and MCP for remote agents, handled by the [`RemoteDispatcher`].
async fn forward(State(cloud): State<Arc<Cloud>>, request: Request) -> Response {
    if !cloud.allowed(request.headers()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            "Too many requests",
        )
            .into_response();
    }
    match agent_http::read(request).await {
        Ok(request) => agent_http::respond(cloud.dispatcher.dispatch(request).await),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
struct NextQuery {
    next: Option<String>,
}

fn safe_next(next: Option<&str>) -> &str {
    next.filter(|next| {
        (next.starts_with("/connect/")
            || *next == "/owner/devices"
            || next.starts_with("/owner/devices/connect?"))
            && !next.contains("//")
            && next.len() < MAX_NEXT_BYTES
    })
    .unwrap_or("/")
}

async fn home(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<NextQuery>,
) -> Html<String> {
    let next = safe_next(query.next.as_deref());
    let unclaimed = cloud.claim.lock().is_ok_and(|claim| claim.is_some());
    // Coming from the app: say so, so the steps make sense.
    let for_app = next.starts_with("/owner/devices/connect?");
    if unclaimed {
        return views::html(&views::ClaimPage {
            title: "Set up this Hoista",
            next,
            for_app,
        });
    }
    if cloud.session(&headers).is_none() || next != "/" {
        return views::html(&views::SignInPage {
            title: "Sign in to Hoista",
            next,
            for_app,
        });
    }
    let (devices, passkeys) = cloud
        .owner
        .lock()
        .map(|owner| (owner.devices.clone(), owner.passkeys.clone()))
        .unwrap_or_default();
    views::html(&views::OwnerHomePage {
        title: "Your Hoista",
        next,
        host: &cloud.host,
        origin: &cloud.origin,
        devices: devices
            .iter()
            .map(|device| views::DeviceRow {
                id: &device.id,
                name: &device.name,
                added: views::day(&device.added),
            })
            .collect(),
        passkeys: passkeys
            .iter()
            .map(|key| views::PasskeyRow {
                id: &key.id,
                name: if key.name.trim().is_empty() {
                    "Passkey"
                } else {
                    &key.name
                },
            })
            .collect(),
        version: crate::VERSION,
        image: cloud.image.as_deref(),
    })
}

#[derive(Deserialize)]
struct PasskeyForm {
    id: String,
}

/// Removes a passkey, but never the last one: that would lock the owner out.
async fn remove_passkey(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    axum::Form(form): axum::Form<PasskeyForm>,
) -> Response {
    if !matches!(cloud.session(&headers), Some((true, _))) {
        return Redirect::to("/").into_response();
    }
    if let Ok(mut owner) = cloud.owner.lock()
        && owner.passkeys.len() > 1
    {
        let before = owner.passkeys.clone();
        owner.passkeys.retain(|key| key.id != form.id);
        if owner.passkeys.is_empty() || cloud.save_owner(&owner).is_err() {
            owner.passkeys = before;
        }
    }
    Redirect::to("/").into_response()
}

fn refuse(text: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "message": text })),
    )
        .into_response()
}

fn options(cloud: &Cloud, challenge: &str) -> Response {
    let exclude: Vec<String> = cloud
        .owner
        .lock()
        .map(|owner| owner.passkeys.iter().map(|key| key.id.clone()).collect())
        .unwrap_or_default();
    Json(serde_json::json!({
        "challenge": challenge,
        "rp": {"id": cloud.rp_id(), "name": "Hoista"},
        "user": {"id": "aG9pc3RhLW93bmVy", "name": "owner", "displayName": "Hoista owner"},
        "exclude": exclude,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ClaimInput {
    code: String,
}

async fn claim(State(cloud): State<Arc<Cloud>>, Json(input): Json<ClaimInput>) -> Response {
    let Ok(mut claim) = cloud.claim.lock() else {
        return refuse("Try again.");
    };
    let Some(current) = claim.as_mut() else {
        return refuse("This Hoista already has an owner. Sign in instead.");
    };
    if current.tries >= CLAIM_TRIES {
        return refuse("Too many wrong codes. Restart the server for a new one.");
    }
    current.tries += 1;
    let typed = input.code.trim().to_ascii_uppercase();
    if !bool::from(subtle::ConstantTimeEq::ct_eq(
        typed.as_bytes(),
        current.code.as_bytes(),
    )) {
        return refuse("That isn't the claim code in this server's log.");
    }
    drop(claim);
    match cloud.issue(Purpose::Claim) {
        Some(challenge) => options(&cloud, &challenge),
        None => refuse("Try again in a minute."),
    }
}

async fn add_passkey(State(cloud): State<Arc<Cloud>>, headers: HeaderMap) -> Response {
    if !matches!(cloud.session(&headers), Some((true, _))) {
        return refuse("Sign in again first.");
    }
    match cloud.issue(Purpose::Add) {
        Some(challenge) => options(&cloud, &challenge),
        None => refuse("Try again in a minute."),
    }
}

#[derive(Deserialize)]
struct RegisterInput {
    challenge: String,
    registration: Registration,
}

async fn register(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    Json(input): Json<RegisterInput>,
) -> Response {
    let purpose = if cloud.take(&input.challenge, Purpose::Claim) {
        Purpose::Claim
    } else if cloud.take(&input.challenge, Purpose::Add)
        && matches!(cloud.session(&headers), Some((true, _)))
    {
        Purpose::Add
    } else {
        return refuse("That passkey request is out of date. Reload the page and try again.");
    };
    let expected = Expected {
        challenge: &input.challenge,
        origin: &cloud.origin,
        rp_id: cloud.rp_id(),
    };
    let key = match passkey::register(&input.registration, &expected) {
        Ok(key) => key,
        Err(text) => return refuse(text),
    };
    {
        let Ok(mut owner) = cloud.owner.lock() else {
            return refuse("Try again.");
        };
        if purpose == Purpose::Claim && !owner.passkeys.is_empty() {
            return refuse("This Hoista already has an owner. Sign in instead.");
        }
        if owner.passkeys.len() >= MAX_PASSKEYS
            || owner.passkeys.iter().any(|known| known.id == key.id)
        {
            return refuse("That passkey is already added, or there are ten already.");
        }
        owner.passkeys.push(key);
        if cloud.save_owner(&owner).is_err() {
            owner.passkeys.pop();
            return refuse("Couldn't save the passkey on the server.");
        }
    }
    if purpose == Purpose::Claim
        && let Ok(mut claim) = cloud.claim.lock()
    {
        *claim = None;
    }
    signed_in(&cloud)
}

fn signed_in(cloud: &Cloud) -> Response {
    let Some(cookie) = cloud.sign_in() else {
        return refuse("Try again.");
    };
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

async fn sign_in_options(State(cloud): State<Arc<Cloud>>) -> Response {
    let allow: Vec<String> = cloud
        .owner
        .lock()
        .map(|owner| owner.passkeys.iter().map(|key| key.id.clone()).collect())
        .unwrap_or_default();
    if allow.is_empty() {
        return refuse("This Hoista has no owner yet.");
    }
    match cloud.issue(Purpose::SignIn) {
        Some(challenge) => Json(serde_json::json!({
            "challenge": challenge,
            "rp_id": cloud.rp_id(),
            "allow": allow,
        }))
        .into_response(),
        None => refuse("Try again in a minute."),
    }
}

#[derive(Deserialize)]
struct SignInInput {
    challenge: String,
    assertion: Assertion,
}

async fn sign_in(State(cloud): State<Arc<Cloud>>, Json(input): Json<SignInInput>) -> Response {
    if !cloud.take(&input.challenge, Purpose::SignIn) {
        return refuse("That sign-in is out of date. Reload the page and try again.");
    }
    let expected = Expected {
        challenge: &input.challenge,
        origin: &cloud.origin,
        rp_id: cloud.rp_id(),
    };
    {
        let Ok(mut owner) = cloud.owner.lock() else {
            return refuse("Try again.");
        };
        let Some(index) = owner
            .passkeys
            .iter()
            .position(|key| key.id == input.assertion.id.trim_end_matches('='))
        else {
            return refuse("That isn't one of this Hoista's passkeys.");
        };
        let mut key = owner.passkeys[index].clone();
        if let Err(text) = passkey::verify(&input.assertion, &mut key, &expected) {
            return refuse(text);
        }
        owner.passkeys[index] = key;
        let _ = cloud.save_owner(&owner);
    }
    signed_in(&cloud)
}

async fn sign_out(State(cloud): State<Arc<Cloud>>, headers: HeaderMap) -> Response {
    if let Some((_, key)) = cloud.session(&headers)
        && let Ok(mut sessions) = cloud.sessions.lock()
    {
        sessions.remove(&key);
    }
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "__Host-hoista=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
        ),
    );
    response
}
