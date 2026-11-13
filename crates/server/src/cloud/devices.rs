//! Signing a computer in to a remote, by browser (OAuth code with PKCE on a
//! loopback address) or by typed code. The computer gets its own key; the
//! server keeps only its hash.
//!
//! The key is not a passkey sign-in: owner checks still need the server's page.

use super::{Cloud, refuse};
use crate::views::{self, Inline, MessagePage};
use crate::{application::agents::passkey, contracts::ErrorDto};
use axum::{
    Form, Json, Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_REDIRECT_BYTES: usize = 200;
const MAX_STATE_BYTES: usize = 128;
const MAX_DEVICE_NAME_CHARS: usize = 60;
/// A SHA-256 in unpadded base64url.
const PKCE_CHALLENGE_CHARS: usize = 43;
/// The lengths PKCE allows a verifier.
const PKCE_VERIFIER_CHARS: std::ops::RangeInclusive<usize> = 43..=128;

/// How long a started sign-in waits for the person.
const WAIT: Duration = Duration::from_secs(10 * 60);
/// How long the app has to trade an Allow's one-time code for its key.
const CODE_LIFE: Duration = Duration::from_secs(2 * 60);
const MOST_WAITING: usize = 16;
const MOST_DEVICES: usize = 20;

/// A computer allowed to use the server's API. Only the key's hash is kept.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Device {
    pub(super) id: String,
    pub(super) name: String,
    /// Hex SHA-256 of the key.
    pub(super) key: String,
    pub(super) added: String,
}

/// A sign-in waiting for the person.
pub(super) struct Waiting {
    code: String,
    name: String,
    started: Instant,
    /// Set once the person allows it; handed out on the next poll.
    key: Option<String>,
}

/// An Allow in the browser, waiting for the app to trade its one-time code.
pub(super) struct Connect {
    name: String,
    redirect: String,
    challenge: String,
    issued: Instant,
}

fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn same(left: &str, right: &str) -> bool {
    bool::from(subtle::ConstantTimeEq::ct_eq(
        left.as_bytes(),
        right.as_bytes(),
    ))
}

fn answer(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(ErrorDto::new(code, message))).into_response()
}

pub(super) fn routes() -> Router<Arc<Cloud>> {
    Router::new()
        .route("/owner/devices", get(page))
        .route("/owner/devices/start", post(start))
        .route("/owner/devices/poll", post(poll))
        .route("/owner/devices/allow", post(allow))
        .route("/owner/devices/remove", post(remove))
        .route("/owner/devices/connect", get(connect_page).post(connect))
        .route("/owner/devices/token", post(token))
}

/// Keeps a new computer, by the hash of a new key it gets once.
fn add_device(cloud: &Cloud, name: &str) -> Result<String, &'static str> {
    let key = passkey::challenge();
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_owned(),
        key: hash(&key),
        added: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    let Ok(mut owner) = cloud.owner.lock() else {
        return Err("Try again.");
    };
    if owner.devices.len() >= MOST_DEVICES {
        return Err(
            "There are 20 computers signed in already. Remove one you no longer use, then try again.",
        );
    }
    owner.devices.push(device);
    if cloud.save_owner(&owner).is_err() {
        owner.devices.pop();
        return Err("Couldn't save it on the server. Nothing changed.");
    }
    Ok(key)
}

fn good_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_DEVICE_NAME_CHARS
        && !name.chars().any(char::is_control)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartInput {
    name: String,
}

/// From the computer: starts a sign-in and returns its code.
async fn start(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    Json(input): Json<StartInput>,
) -> Response {
    if !cloud.allowed(&headers) {
        return answer(
            StatusCode::TOO_MANY_REQUESTS,
            "request.rate",
            "Too many requests. Wait a minute.",
        );
    }
    if cloud
        .owner
        .lock()
        .map_or(true, |owner| owner.passkeys.is_empty())
    {
        return answer(
            StatusCode::CONFLICT,
            "cloud.unclaimed",
            "Claim this Hoista with a passkey first, then sign in this computer.",
        );
    }
    let name = input.name.trim();
    if !good_name(name) {
        return answer(
            StatusCode::BAD_REQUEST,
            "device.name",
            "Give this computer a name of up to 60 characters.",
        );
    }
    let poll = passkey::challenge();
    let code = passkey::claim_code();
    {
        let Ok(mut waiting) = cloud.waiting.lock() else {
            return answer(
                StatusCode::SERVICE_UNAVAILABLE,
                "request.retry",
                "Try again.",
            );
        };
        waiting.retain(|_, item| item.started.elapsed() < WAIT);
        if waiting.len() >= MOST_WAITING {
            return answer(
                StatusCode::TOO_MANY_REQUESTS,
                "request.rate",
                "Too many sign-ins are waiting. Try again in ten minutes.",
            );
        }
        waiting.insert(
            hash(&poll),
            Waiting {
                code: code.clone(),
                name: name.to_owned(),
                started: Instant::now(),
                key: None,
            },
        );
    }
    Json(serde_json::json!({
        "poll": poll,
        "code": code,
        // The code rides after `#`, so it never reaches a server log.
        "link": format!("{}/owner/devices#code={code}", cloud.origin),
        "expires_in": WAIT.as_secs(),
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PollInput {
    poll: String,
}

/// From the computer: waiting, its key (once), or gone.
async fn poll(State(cloud): State<Arc<Cloud>>, Json(input): Json<PollInput>) -> Response {
    let Ok(mut waiting) = cloud.waiting.lock() else {
        return answer(
            StatusCode::SERVICE_UNAVAILABLE,
            "request.retry",
            "Try again.",
        );
    };
    waiting.retain(|_, item| item.started.elapsed() < WAIT);
    let id = hash(&input.poll);
    match waiting.get(&id).map(|item| item.key.is_some()) {
        Some(true) => {
            let Some(item) = waiting.remove(&id) else {
                return answer(StatusCode::GONE, "device.expired", "Start again.");
            };
            Json(serde_json::json!({ "key": item.key, "name": item.name })).into_response()
        }
        Some(false) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "waiting": true })),
        )
            .into_response(),
        None => answer(
            StatusCode::GONE,
            "device.expired",
            "That sign-in ran out after ten minutes. Run hoist remote login again.",
        ),
    }
}

fn card(title: &str, heading: &str, text: &[Inline], button: Option<(&str, &str)>) -> Html<String> {
    views::html(&MessagePage {
        title,
        heading,
        text,
        button,
    })
}

fn sign_in_again() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        card(
            "Sign in again",
            "Sign in again",
            &[
                Inline::Text("It's been more than ten minutes since you signed in. "),
                Inline::Link("Sign in with your passkey", "/"),
                Inline::Text(", then try again. Nothing was changed."),
            ],
            None,
        ),
    )
        .into_response()
}

/// For `hoist remote login` without a browser: allow a computer by the
/// code on its screen. Its link carries the code; the list of computers is
/// on the home page.
async fn page(State(cloud): State<Arc<Cloud>>, headers: HeaderMap) -> Response {
    if !matches!(cloud.session(&headers), Some((true, _))) {
        return Redirect::to("/?next=/owner/devices").into_response();
    }
    views::html(&views::AllowComputerPage {
        title: "Allow a computer",
    })
    .into_response()
}

#[derive(Deserialize)]
struct CodeForm {
    code: String,
}

async fn allow(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response {
    if !matches!(cloud.session(&headers), Some((true, _))) {
        return sign_in_again();
    }
    let typed = form.code.trim().to_ascii_uppercase();
    let name = {
        let Ok(mut waiting) = cloud.waiting.lock() else {
            return refuse("Try again.");
        };
        waiting.retain(|_, item| item.started.elapsed() < WAIT);
        let Some(item) = waiting
            .values_mut()
            .find(|item| item.key.is_none() && same(&item.code, &typed))
        else {
            return card(
                "No computer with that code",
                "No computer with that code",
                &[
                    Inline::Text("Check the code on that computer's screen. Codes run out after ten minutes; then run "),
                    Inline::Code("hoist remote login"),
                    Inline::Text(" again."),
                ],
                Some(("Back", "/owner/devices")),
            )
            .into_response();
        };
        match add_device(&cloud, &item.name) {
            Ok(key) => item.key = Some(key),
            Err(text) => {
                return card(
                    "Not allowed",
                    "Couldn't allow it",
                    &[Inline::Text(text)],
                    Some(("Back", "/")),
                )
                .into_response();
            }
        }
        item.name.clone()
    };
    card(
        "Computer allowed",
        &format!("{name} can use your Hoista"),
        &[Inline::Text(
            "Go back to that computer; it finishes by itself. Remove it from Signed-in computers any time.",
        )],
        Some(("Done", "/")),
    )
    .into_response()
}

#[derive(Deserialize)]
struct IdForm {
    id: String,
}

async fn remove(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    Form(form): Form<IdForm>,
) -> Response {
    if !matches!(cloud.session(&headers), Some((true, _))) {
        return sign_in_again();
    }
    {
        let Ok(mut owner) = cloud.owner.lock() else {
            return refuse("Try again.");
        };
        let before = owner.devices.clone();
        owner.devices.retain(|device| device.id != form.id);
        if cloud.save_owner(&owner).is_err() {
            owner.devices = before;
            return refuse("Couldn't save it on the server. Nothing changed.");
        }
    }
    Redirect::to("/#computers").into_response()
}

#[derive(Deserialize)]
struct ConnectQuery {
    name: String,
    redirect: String,
    challenge: String,
    state: String,
}

/// The app's address on its own computer (RFC 8252): plain HTTP to a
/// loopback address with a port, nothing else.
fn loopback(redirect: &str) -> bool {
    url::Url::parse(redirect).is_ok_and(|url| {
        url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            && url.port().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}

/// Checks what the app sent; the reason it can't go on, if any.
fn check_connect(query: &ConnectQuery) -> Option<&'static str> {
    if !good_name(query.name.trim()) {
        return Some("The app didn't say which computer this is.");
    }
    if !loopback(&query.redirect) || query.redirect.len() > MAX_REDIRECT_BYTES {
        return Some("This sign-in has to go back to the Hoista app on the same computer.");
    }
    // A SHA-256, base64url without padding, as PKCE's S256 sends it.
    let challenge_ok = query.challenge.len() == PKCE_CHALLENGE_CHARS
        && query
            .challenge
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !challenge_ok || query.state.is_empty() || query.state.len() > MAX_STATE_BYTES {
        return Some("The app sent a sign-in this page can't read.");
    }
    None
}

/// The address to send the browser back to, with `params` added.
fn back_to(redirect: &str, params: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(redirect).unwrap_or_else(|_| {
        url::Url::parse("http://127.0.0.1:1/").unwrap_or_else(|_| unreachable!())
    });
    url.query_pairs_mut().extend_pairs(params);
    url.to_string()
}

/// From the app, in the browser: set up or sign in first, then Allow.
async fn connect_page(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let Ok(axum::extract::Query(query)) =
        axum::extract::Query::<ConnectQuery>::try_from_uri(request.uri())
    else {
        return card(
            "Start again in the app",
            "Start again in the app",
            &[Inline::Text(
                "This link is missing something. In the Hoista app, add this Hoista again.",
            )],
            None,
        )
        .into_response();
    };
    if let Some(problem) = check_connect(&query) {
        return card(
            "Start again in the app",
            "Start again in the app",
            &[
                Inline::Text(problem),
                Inline::Text(" In the Hoista app, add this Hoista again."),
            ],
            None,
        )
        .into_response();
    }
    if !matches!(cloud.session(&headers), Some((true, _))) {
        // Set up or sign in on the home page, which comes back here.
        let here = request
            .uri()
            .path_and_query()
            .map_or("/owner/devices/connect", |path| path.as_str());
        return Redirect::to(&format!(
            "/?next={}",
            url::form_urlencoded::byte_serialize(here.as_bytes()).collect::<String>()
        ))
        .into_response();
    }
    let deny = back_to(
        &query.redirect,
        &[("error", "access_denied"), ("state", &query.state)],
    );
    let fields = serde_json::json!({
        "name": query.name.trim(),
        "redirect": query.redirect,
        "challenge": query.challenge,
        "state": query.state,
    });
    views::html(&views::ConnectAppPage {
        title: "Allow the Hoista app",
        name: query.name.trim(),
        host: &cloud.host,
        deny: &deny,
        fields: fields.to_string(),
    })
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectInput {
    name: String,
    redirect: String,
    challenge: String,
    state: String,
}

/// From the Allow button: a one-time code for the app, and where to take it.
async fn connect(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    Json(input): Json<ConnectInput>,
) -> Response {
    if !matches!(cloud.session(&headers), Some((true, _))) {
        return refuse(
            "It's been more than ten minutes since you signed in. Reload the page and sign in again.",
        );
    }
    let query = ConnectQuery {
        name: input.name,
        redirect: input.redirect,
        challenge: input.challenge,
        state: input.state,
    };
    if let Some(problem) = check_connect(&query) {
        return refuse(problem);
    }
    let code = passkey::challenge();
    {
        let Ok(mut connects) = cloud.connects.lock() else {
            return refuse("Try again.");
        };
        connects.retain(|_, item| item.issued.elapsed() < CODE_LIFE);
        if connects.len() >= MOST_WAITING {
            return refuse("Too many sign-ins at once. Wait two minutes and try again.");
        }
        connects.insert(
            hash(&code),
            Connect {
                name: query.name.trim().to_owned(),
                redirect: query.redirect.clone(),
                challenge: query.challenge,
                issued: Instant::now(),
            },
        );
    }
    Json(serde_json::json!({
        "to": back_to(&query.redirect, &[("code", &code), ("state", &query.state)]),
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenInput {
    code: String,
    verifier: String,
    redirect: String,
}

/// From the app: its one-time code and PKCE secret, for its own key, once.
async fn token(
    State(cloud): State<Arc<Cloud>>,
    headers: HeaderMap,
    Json(input): Json<TokenInput>,
) -> Response {
    if !cloud.allowed(&headers) {
        return answer(
            StatusCode::TOO_MANY_REQUESTS,
            "request.rate",
            "Too many requests. Wait a minute.",
        );
    }
    let item = {
        let Ok(mut connects) = cloud.connects.lock() else {
            return answer(
                StatusCode::SERVICE_UNAVAILABLE,
                "request.retry",
                "Try again.",
            );
        };
        connects.retain(|_, item| item.issued.elapsed() < CODE_LIFE);
        // Used once, right or wrong.
        connects.remove(&hash(&input.code))
    };
    let expired = || {
        answer(
            StatusCode::BAD_REQUEST,
            "device.expired",
            "That sign-in ran out or was already used. Add this Hoista in the app again.",
        )
    };
    let Some(item) = item else {
        return expired();
    };
    use base64::Engine as _;
    let made = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(input.verifier.as_bytes()));
    if !PKCE_VERIFIER_CHARS.contains(&input.verifier.len())
        || !same(&made, &item.challenge)
        || !same(&input.redirect, &item.redirect)
    {
        return expired();
    }
    match add_device(&cloud, &item.name) {
        Ok(key) => Json(serde_json::json!({ "key": key, "name": item.name })).into_response(),
        Err(text) => answer(StatusCode::CONFLICT, "device.full", text),
    }
}

/// `/api`: only for a signed-in computer's key.
pub(super) async fn device_only(
    State(cloud): State<Arc<Cloud>>,
    request: Request,
    next: Next,
) -> Response {
    let known = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|key| {
            let key = hash(key);
            cloud.owner.lock().is_ok_and(|owner| {
                owner
                    .devices
                    .iter()
                    .fold(false, |found, device| found | same(&device.key, &key))
            })
        });
    if known {
        // A signed-in computer's screens make many requests; only guesses are limited.
        return next.run(request).await;
    }
    if !cloud.allowed(request.headers()) {
        return answer(
            StatusCode::TOO_MANY_REQUESTS,
            "request.rate",
            "Too many requests. Wait a minute.",
        );
    }
    answer(
        StatusCode::UNAUTHORIZED,
        "device.signed_out",
        "This computer isn't signed in to your Hoista. In the Hoista app, sign in to it again from Settings → Instances, or run hoist remote login.",
    )
}
