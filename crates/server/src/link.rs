//! MCP over HTTP: web agents such as ChatGPT reach the Hoista on this
//! computer through a tunnel or proxy the person runs to `127.0.0.1:8788`,
//! and sign in with OAuth as they would on a server.
//!
//! It listens on loopback only, so nothing outside this computer reaches it
//! without a tunnel or proxy. It serves OAuth and MCP and nothing else: no
//! owner API, no approval pages. The person allows each agent on this
//! computer, with Touch ID or the password, on Hoista's own approval page;
//! nothing reached from outside can allow anything.
//!
//! The public address is learned from requests that come through (an `https`
//! name, as tunnels and proxies forward it), and follows the tunnel when it
//! gets a new one, as a quick tunnel does each time it restarts. An address the
//! person set stays: requests naming any other are refused until they forget it.

use super::{
    agent_http,
    catalog::ApiFault,
    remote::{REVIEW_HEADER, RemoteDispatcher},
};
use crate::application::engine::Engine;
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hoista_system::paths::StatePaths;
use serde::Deserialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// How often the tunnel's address is checked for a change.
const ADDRESS_POLL: std::time::Duration = std::time::Duration::from_secs(2);
const MAX_SHOWN_BYTES: usize = 256;
/// Agent calls one link takes a minute, and review pages it opens a minute.
const CALLS_PER_MINUTE: u32 = 600;
const PAGES_OPENED_PER_MINUTE: u32 = 6;

/// The port MCP over HTTP listens on, on loopback.
pub const PORT: u16 = 8788;

/// Where an address the person set is kept, so it outlasts a restart. A
/// learned address isn't kept: the next request through the tunnel teaches it
/// again.
fn address_file(paths: &StatePaths) -> std::path::PathBuf {
    paths.root().join("mcp-address")
}

// The owner's routes: what address is set, and changing it.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Address {
    url: String,
}

/// Owner IPC: where MCP over HTTP is, the address agents use, and setting
/// or forgetting that address.
pub fn router(engine: Arc<Engine>, paths: StatePaths) -> Router {
    Router::new()
        .route("/agents/link", get(status).put(set).delete(forget))
        .with_state(Owner { engine, paths })
}

#[derive(Clone)]
struct Owner {
    engine: Arc<Engine>,
    paths: StatePaths,
}

fn describe(engine: &Engine) -> serde_json::Value {
    let url = if engine.is_server() {
        None
    } else {
        engine.public_url()
    };
    serde_json::json!({
        "listening": engine.mcp_http_open(),
        "local": (!engine.is_server()).then(|| format!("http://127.0.0.1:{PORT}")),
        "url": url,
        "mcp": url.as_ref().map(|url| format!("{url}/mcp")),
        "seen_at": url.as_ref().and_then(|_| engine.link_seen()),
    })
}

async fn status(State(owner): State<Owner>) -> Json<serde_json::Value> {
    Json(describe(&owner.engine))
}

async fn set(
    State(owner): State<Owner>,
    Json(input): Json<Address>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    owner.engine.open_link(&input.url)?;
    if let Some(url) = owner.engine.public_url()
        && let Err(error) = std::fs::write(address_file(&owner.paths), url)
    {
        tracing::warn!(code = "mcp_http.address_unsaved", %error);
    }
    Ok(Json(describe(&owner.engine)))
}

async fn forget(State(owner): State<Owner>) -> Json<serde_json::Value> {
    if !owner.engine.is_server() {
        owner.engine.close_link();
        let _ = std::fs::remove_file(address_file(&owner.paths));
    }
    Json(describe(&owner.engine))
}

// The listener agents reach.

#[derive(Clone)]
struct Port {
    dispatcher: Arc<RemoteDispatcher>,
    /// The latest requests sent to the person, so each opens its page once.
    shown: Arc<Mutex<VecDeque<String>>>,
    /// Requests this minute, so a flood from outside is cut off.
    rate: Arc<PerMinute>,
    /// Pages opened this minute: a flood of sign-ins never floods the screen.
    opened: Arc<PerMinute>,
    /// Where an address the person set is kept: while it's there, the address stays.
    set_address: std::path::PathBuf,
}

/// What to do with a request from outside, by the address it names.
#[derive(Debug, PartialEq, Eq)]
enum Arrival {
    /// It names the address in use.
    Accept,
    /// It came through the tunnel at an address Hoista should use from now on.
    Learn(String),
    /// It names another address.
    Refuse,
}

/// `current` is the address in use, `arrived` the `https` address the request
/// came through (see [`forwarded_origin`]), `host` its Host header, and `set`
/// whether the person set the address. Only a tunnel or proxy the person runs
/// to this port forwards an `https` name without a port, so when the address
/// was only learned, a new one is the tunnel's new address and is followed.
fn arrival(current: Option<&str>, arrived: Option<&str>, host: &str, set: bool) -> Arrival {
    match (current, arrived) {
        (Some(url), arrived)
            if url
                .split_once("://")
                .is_some_and(|(_, name)| name.eq_ignore_ascii_case(host))
                || arrived == Some(url) =>
        {
            Arrival::Accept
        }
        (Some(_), Some(arrived)) if !set => Arrival::Learn(arrived.to_owned()),
        (None, Some(arrived)) => Arrival::Learn(arrived.to_owned()),
        _ => Arrival::Refuse,
    }
}

/// A count that starts again every minute.
#[derive(Default)]
struct PerMinute(Mutex<(u64, u32)>);

impl PerMinute {
    /// Counts one more, and says whether it is still within `limit` this minute.
    fn allow(&self, limit: u32) -> bool {
        let minute = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |now| now.as_secs() / 60);
        let Ok(mut count) = self.0.lock() else {
            return false;
        };
        if count.0 != minute {
            *count = (minute, 0);
        }
        count.1 = count.1.saturating_add(1);
        count.1 <= limit
    }
}

/// Serves MCP over HTTP while the setting is on, following it while
/// Hoista runs, until `shutdown`.
pub async fn serve(
    engine: Arc<Engine>,
    paths: StatePaths,
    shutdown: tokio_util::sync::CancellationToken,
) {
    // An address the person set before a restart.
    if let Ok(url) = std::fs::read_to_string(address_file(&paths))
        && engine.open_link(url.trim()).is_err()
    {
        let _ = std::fs::remove_file(address_file(&paths));
    }
    let mut open: Option<(
        tokio_util::sync::CancellationToken,
        tokio::task::JoinHandle<()>,
    )> = None;
    loop {
        // A listener that stopped by itself is opened again on the next turn.
        if open.as_ref().is_some_and(|(_, task)| task.is_finished()) {
            open = None;
            engine.set_mcp_http_open(false);
            tracing::warn!(code = "mcp_http.stopped");
        }
        let wanted = engine
            .repository
            .settings()
            .await
            .is_ok_and(|settings| settings.mcp_http);
        match (wanted, open.is_some()) {
            (true, false) => match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
                Ok(listener) => {
                    let stop = shutdown.child_token();
                    let until = stop.clone();
                    let port = Port {
                        dispatcher: Arc::new(RemoteDispatcher {
                            engine: engine.clone(),
                            local_pages: super::pages::addresses(&paths)
                                .map(|addresses| addresses.local),
                        }),
                        shown: Arc::default(),
                        rate: Arc::default(),
                        opened: Arc::default(),
                        set_address: address_file(&paths),
                    };
                    let routes = routes(port);
                    let task = tokio::spawn(async move {
                        let _ = axum::serve(listener, routes)
                            .with_graceful_shutdown(until.cancelled_owned())
                            .await;
                    });
                    engine.set_mcp_http_open(true);
                    open = Some((stop, task));
                    tracing::info!(code = "mcp_http.ready", port = PORT);
                }
                Err(error) => {
                    tracing::warn!(code = "mcp_http.unavailable", port = PORT, %error);
                }
            },
            (false, true) => {
                if let Some((stop, task)) = open.take() {
                    stop.cancel();
                    // Requests under way finish before the port is free again.
                    let _ = task.await;
                }
                engine.set_mcp_http_open(false);
                tracing::info!(code = "mcp_http.closed");
            }
            _ => {}
        }
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(ADDRESS_POLL) => {}
        }
    }
    // Hoista is stopping: let requests under way finish.
    if let Some((stop, task)) = open {
        stop.cancel();
        let _ = task.await;
    }
    engine.set_mcp_http_open(false);
}

fn routes(port: Port) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(server_metadata),
        )
        .route("/oauth/register", post(pass).options(agent_http::preflight))
        .route("/oauth/authorize", get(pass))
        .route("/oauth/token", post(pass).options(agent_http::preflight))
        .route(
            "/mcp",
            post(pass)
                .get(pass)
                .delete(pass)
                .options(agent_http::preflight),
        )
        .layer(middleware::from_fn_with_state(port.clone(), guard))
        .with_state(port)
}

/// The public address a request came in on through a tunnel or proxy: an
/// `https` name with no port, as tunnels (Cloudflare Tunnel, ngrok, Tailscale
/// Funnel) and proxies forward it. A page in a browser on this computer that points a name at
/// the port (DNS rebinding) arrives over plain HTTP with the port in its
/// name, so it never counts.
#[must_use]
fn forwarded_origin(headers: &HeaderMap) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let https = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|proto| proto.eq_ignore_ascii_case("https"));
    let name_only = host.len() <= crate::domain::domains::MAX_HOSTNAME_CHARS
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && host.parse::<std::net::IpAddr>().is_err();
    (https && name_only).then(|| format!("https://{}", host.to_ascii_lowercase()))
}

fn local_host(host: &str) -> bool {
    host == format!("127.0.0.1:{PORT}") || host == format!("localhost:{PORT}")
}

/// Answers only under its public address (set, or learned from the first
/// request from outside) and this computer's own, and never lets a page frame or cache
/// it.
async fn guard(State(port): State<Port>, request: Request, next: Next) -> Response {
    let engine = &port.dispatcher.engine;
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if local_host(&host) {
        // A page in a browser on this computer can send requests here; an
        // agent or tool never sends an Origin.
        if request.headers().contains_key(header::ORIGIN) {
            return (
                StatusCode::FORBIDDEN,
                "Web pages can't use MCP over HTTP on this computer.",
            )
                .into_response();
        }
    } else {
        let current = engine.public_url();
        let arrived = forwarded_origin(request.headers());
        let set = port.set_address.exists();
        let accepted = match arrival(current.as_deref(), arrived.as_deref(), &host, set) {
            Arrival::Accept => true,
            Arrival::Learn(url) => {
                let learned = engine.open_link(&url).is_ok();
                if learned {
                    tracing::info!(code = "mcp_http.address", url = %url, replaced = current.is_some());
                }
                learned
            }
            Arrival::Refuse => false,
        };
        if !accepted {
            let place = engine
                .public_url()
                .unwrap_or_else(|| "your tunnel's or proxy's https:// address".to_owned());
            return (
                StatusCode::MISDIRECTED_REQUEST,
                format!("Use this Hoista's own address: {place}"),
            )
                .into_response();
        }
        engine.note_link_seen();
    }
    if !port.rate.allow(CALLS_PER_MINUTE) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            "Too many requests",
        )
            .into_response();
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

/// Until the public address is known there's nowhere to sign in.
fn no_address() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Hoista doesn't know its public address yet. Open it at your tunnel's or proxy's https:// address, or set it with hoist mcp http --url.",
    )
        .into_response()
}

async fn resource_metadata(State(port): State<Port>) -> Response {
    port.dispatcher
        .engine
        .public_url()
        .map_or_else(no_address, |url| agent_http::resource_metadata(&url))
}

async fn server_metadata(State(port): State<Port>) -> Response {
    port.dispatcher
        .engine
        .public_url()
        .map_or_else(no_address, |url| agent_http::server_metadata(&url))
}

/// Handles an OAuth or MCP request with the same code as on a server.
async fn pass(State(port): State<Port>, request: Request) -> Response {
    if port.dispatcher.engine.public_url().is_none() {
        return no_address();
    }
    let request = match agent_http::read(request).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    let reply = port.dispatcher.dispatch(request).await;
    // An agent waiting for the person: open its page on this computer, once.
    if let Some(review) = reply.headers.get(REVIEW_HEADER).cloned() {
        let first = port.shown.lock().is_ok_and(|mut shown| {
            if shown.contains(&review) {
                return false;
            }
            if shown.len() >= MAX_SHOWN_BYTES {
                shown.pop_front();
            }
            shown.push_back(review.clone());
            true
        });
        if first && port.opened.allow(PAGES_OPENED_PER_MINUTE) {
            super::stdio::open_in_browser(&review);
        }
    }
    agent_http::respond(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str, proto: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().expect("host"));
        if let Some(proto) = proto {
            headers.insert("x-forwarded-proto", proto.parse().expect("proto"));
        }
        headers
    }

    #[test]
    fn only_a_forwarded_https_name_counts_as_the_address() {
        assert_eq!(
            forwarded_origin(&headers("Calm-Words.trycloudflare.com", Some("https"))).as_deref(),
            Some("https://calm-words.trycloudflare.com")
        );
        // Plain HTTP, a port in the name (a page rebinding a name to the
        // port), an address or a bare name are never a public address.
        for (host, proto) in [
            ("hoista.example.com", Some("http")),
            ("hoista.example.com", None),
            ("evil.example:8788", Some("https")),
            ("127.0.0.1", Some("https")),
            ("localhost", Some("https")),
            ("bad_name.example.com", Some("https")),
        ] {
            assert_eq!(forwarded_origin(&headers(host, proto)), None, "{host}");
        }
    }

    #[test]
    fn a_learned_address_follows_the_tunnel_and_a_set_one_stays() {
        let old = "https://old-words.trycloudflare.com";
        let new = "https://new-words.trycloudflare.com";
        let host = "new-words.trycloudflare.com";
        // The address in use is accepted, by its name or as it arrived.
        assert_eq!(
            arrival(Some(old), None, "old-words.trycloudflare.com", false),
            Arrival::Accept
        );
        assert_eq!(arrival(Some(old), Some(old), "x", true), Arrival::Accept);
        // The tunnel came back at a new address: followed while it was learned.
        assert_eq!(
            arrival(Some(old), Some(new), host, false),
            Arrival::Learn(new.into())
        );
        assert_eq!(
            arrival(None, Some(new), host, false),
            Arrival::Learn(new.into())
        );
        // An address the person set stays.
        assert_eq!(arrival(Some(old), Some(new), host, true), Arrival::Refuse);
        // Nothing forwarded as https (a page rebinding a name, plain HTTP) never moves it.
        assert_eq!(
            arrival(Some(old), None, "evil.example:8788", false),
            Arrival::Refuse
        );
        assert_eq!(
            arrival(None, None, "evil.example:8788", false),
            Arrival::Refuse
        );
    }
}
