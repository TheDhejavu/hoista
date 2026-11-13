//! The browser pages where the person approves an agent's connection request
//! (a device-style flow: the agent shows a code, the person checks it here).
//!
//! These listeners serve pages only, no MCP or owner API. Each page needs its
//! one-time id, and each form a matching token and a same-origin request.

use crate::application::agents::tool::Tool;
use crate::views::{
    AccountUse, Choice, ConnectedPage, ConsentPage, EnvironmentChoice, LevelSaid, Permission,
    ProjectRow, ProviderMark, ResultPage, Tickbox, html,
};
use crate::{
    application::{
        engine::{Actor, Engine, EnrollmentRequest, ProjectListQuery},
        ports::catalog::EngineError,
    },
    domain::model::*,
};
use axum::{
    Form, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use std::{net::SocketAddr, sync::Arc};

/// Ports the consent page tries, in order.
const PAGE_PORTS: std::ops::Range<u16> = 7444..7464;
/// How often the network address is checked for a change.
const ADDRESS_POLL: std::time::Duration = std::time::Duration::from_secs(2);
const COOKIE_KEY_BYTES: usize = 32;
const SHORT_DIGEST_BYTES: usize = 16;
const MAX_CODE_BYTES: usize = 200;
const MAX_ID_BYTES: usize = 128;
const MAX_LISTED: usize = 500;
/// Names written out before the rest are counted.
const MAX_NAMES_LISTED: usize = 3;

/// The addresses the pages answer on, for Settings and for agents.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Addresses {
    /// `http://127.0.0.1:PORT`: this computer only.
    pub local: String,
    /// `http://192.168.x.y:PORT` on the current network, when there is one.
    pub network: Option<String>,
}

#[derive(Clone)]
struct Pages {
    engine: Arc<Engine>,
    /// `host:port` values a request may name, so a page can't be reached
    /// through another hostname (DNS rebinding).
    hosts: Arc<Vec<String>>,
    /// Signs each form to the page that rendered it.
    key: Arc<[u8; 32]>,
    /// Where a form may come from: `http://host` here, `https://host` on a server.
    origins: Arc<Vec<String>>,
    /// How the page names this Hoista ("Hoista on this computer").
    place: Arc<String>,
}

/// The first private (RFC 1918) IPv4 address of this computer, found by
/// asking the OS which address it would use; nothing is sent.
#[must_use]
pub fn private_ip() -> Option<std::net::Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if ip.is_private() => Some(ip),
        _ => None,
    }
}

/// Binds the pages on loopback from port 7444 up, and on the private network
/// address while Settings → Agents has that on (`network` is its value at
/// start), and keeps `address.json` saying where.
///
/// # Errors
/// Reports that no port was free.
pub async fn serve(
    engine: Arc<Engine>,
    paths: &hoista_system::paths::StatePaths,
    network: bool,
    shutdown: tokio_util::sync::CancellationToken,
) -> std::io::Result<Addresses> {
    let mut chosen = None;
    for port in PAGE_PORTS {
        if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            chosen = Some((port, listener));
            break;
        }
    }
    let Some((port, local)) = chosen else {
        return Err(std::io::Error::other(
            "No free port for Hoista's pages (7444 to 7463).",
        ));
    };
    let local_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let local_address = format!("http://127.0.0.1:{port}");
    let project = router(engine.clone(), local_hosts);
    {
        let stop = shutdown.clone();
        tokio::spawn(async move {
            let _ = axum::serve(
                local,
                project.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(stop.cancelled_owned())
            .await;
        });
    }
    // The network address follows the setting and the network this computer
    // is on, so turning it on or changing Wi-Fi needs no restart.
    let mut lan = NetworkPage::default();
    let network_now = lan
        .follow(&engine, port, network.then(private_ip).flatten(), &shutdown)
        .await;
    let addresses = Addresses {
        local: local_address.clone(),
        network: network_now,
    };
    write_addresses(paths, &addresses);
    {
        let engine = engine.clone();
        let paths = paths.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(ADDRESS_POLL) => {}
                }
                let wanted = engine
                    .repository
                    .settings()
                    .await
                    .is_ok_and(|settings| settings.network_page);
                let ip = if wanted { private_ip() } else { None };
                if ip == lan.ip {
                    continue;
                }
                let network = lan.follow(&engine, port, ip, &shutdown).await;
                write_addresses(
                    &paths,
                    &Addresses {
                        local: local_address.clone(),
                        network,
                    },
                );
                tracing::info!(code = "pages.network", on = lan.ip.is_some());
            }
        });
    }
    tracing::info!(code = "pages.ready", port);
    Ok(addresses)
}

/// The pages on this computer's private network address, while that's on.
#[derive(Default)]
struct NetworkPage {
    /// The address they answer on now.
    ip: Option<std::net::Ipv4Addr>,
    /// Stops the listener on that address.
    stop: Option<tokio_util::sync::CancellationToken>,
}

impl NetworkPage {
    /// Moves the listener to `ip`, or closes it for `None`. Returns the
    /// address it answers on.
    async fn follow(
        &mut self,
        engine: &Arc<Engine>,
        port: u16,
        ip: Option<std::net::Ipv4Addr>,
        shutdown: &tokio_util::sync::CancellationToken,
    ) -> Option<String> {
        if let Some(stop) = self.stop.take() {
            stop.cancel();
        }
        self.ip = None;
        let ip = ip?;
        let listener = tokio::net::TcpListener::bind((ip, port)).await.ok()?;
        let project = router(engine.clone(), vec![format!("{ip}:{port}")]);
        let stop = shutdown.child_token();
        let until = stop.clone();
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                project.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(until.cancelled_owned())
            .await;
        });
        self.ip = Some(ip);
        self.stop = Some(stop);
        Some(format!("http://{ip}:{port}"))
    }
}

fn write_addresses(paths: &hoista_system::paths::StatePaths, addresses: &Addresses) {
    let _ = std::fs::write(
        paths.root().join("address.json"),
        serde_json::to_vec(addresses).unwrap_or_default(),
    );
}

/// Where the pages are, as `hoistd` last recorded it.
#[must_use]
pub fn addresses(paths: &hoista_system::paths::StatePaths) -> Option<Addresses> {
    let bytes = std::fs::read(paths.root().join("address.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Owner IPC: where the pages are, for Settings → Agents.
pub fn address_router(paths: hoista_system::paths::StatePaths) -> Router {
    Router::new().route(
        "/agents/address",
        get(move || {
            let paths = paths.clone();
            async move { axum::Json(addresses(&paths)) }
        }),
    )
}

/// The pages, answering only under `hosts` (`host:port` values).
pub fn router(engine: Arc<Engine>, hosts: Vec<String>) -> Router {
    let origins = hosts.iter().map(|host| format!("http://{host}")).collect();
    build(
        engine,
        hosts,
        origins,
        "Hoista on this computer".into(),
        true,
    )
}

/// The same pages for Hoista on a server at `public_url` (`https://host`).
/// The caller makes sure only the signed-in owner reaches them.
pub fn cloud_router(engine: Arc<Engine>, public_url: &str) -> Router {
    let host = url::Url::parse(public_url)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            })
        })
        .unwrap_or_default();
    let place = format!("your Hoista at {host}");
    build(
        engine,
        vec![host],
        vec![public_url.to_owned()],
        place,
        false,
    )
}

fn build(
    engine: Arc<Engine>,
    hosts: Vec<String>,
    origins: Vec<String>,
    place: String,
    local: bool,
) -> Router {
    let mut key = [0u8; COOKIE_KEY_BYTES];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let state = Pages {
        engine,
        hosts: Arc::new(hosts),
        key: Arc::new(key),
        origins: Arc::new(origins),
        place: Arc::new(place),
    };
    // This computer's pages say Hoista is running at their root.
    let router = if local {
        Router::new().route("/", get(home))
    } else {
        Router::new()
    };
    router
        .route("/connect/{id}", get(consent).post(answer_consent))
        .route("/connect/{id}/done", get(connected))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Pages answer only under their own addresses, and forms only from them.
async fn guard(State(pages): State<Pages>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if !pages.hosts.contains(&host) {
        return (
            StatusCode::MISDIRECTED_REQUEST,
            "Open this page from Hoista's own address.",
        )
            .into_response();
    }
    if request.method() == axum::http::Method::POST {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok());
        if !origin.is_some_and(|origin| pages.origins.iter().any(|allowed| allowed == origin)) {
            return (
                StatusCode::FORBIDDEN,
                "This form only works from its own page.",
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
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; form-action 'self'; frame-ancestors 'none'",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn token(pages: &Pages, kind: &str, id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(pages.key.as_slice());
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(id.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(SHORT_DIGEST_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn same(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

async fn home() -> Html<String> {
    html(&ResultPage {
        title: "Hoista is running",
        text: "When an agent asks to connect, it opens a page here for you to authorize it.",
        left: None,
        done: false,
    })
}

/// Where a remote agent's requests start in a page address.
const REMOTE: &str = "oauth-";

/// Someone waiting on the consent page: a local tool (`hoist mcp`), or an
/// agent over HTTP (Hoista on a server, or MCP over HTTP on this computer).
struct Asking {
    /// The id in the page address.
    id: String,
    name: String,
    code: String,
    /// Whether its project can show Hoista's questions.
    can_ask: Option<bool>,
    /// A coding tool on this computer Hoista can set up.
    tool: Option<Tool>,
    /// For a remote agent, the site the browser returns to.
    returns_to: Option<String>,
}

async fn find_request(engine: &Engine, id: &str) -> Result<Option<Asking>, EngineError> {
    if let Some(remote) = id.strip_prefix(REMOTE) {
        return Ok(engine
            .accounts()
            .oauth_request(remote)
            .await?
            .map(|request| Asking {
                id: id.to_owned(),
                name: request.name,
                code: request.code,
                // Remote agents reach Hoista over HTTP, where Hoista can't ask.
                can_ask: Some(false),
                tool: None,
                returns_to: Some(request.returns_to),
            }));
    }
    Ok(engine
        .accounts()
        .enrollment_requests()
        .await?
        .into_iter()
        .find(|item| item.id == id)
        .map(|request: EnrollmentRequest| Asking {
            tool: Tool::from_name(&request.name),
            id: request.id,
            name: request.name,
            code: request.code,
            can_ask: request.can_ask,
            returns_to: None,
        }))
}

/// Access levels: the value sent and its name.
const LEVELS: [(&str, &str); 3] = [
    ("view", "View only"),
    ("ask", "Ask for approval"),
    ("full", "Full access"),
];

/// What an agent can do, with the scope it needs and the lowest level that has it.
const PERMISSIONS: [(&str, &str, &str, usize); 3] = [
    (
        "See your projects",
        "their status, logs, deploys, metrics and bills",
        "apps:read",
        0,
    ),
    (
        "Deploy and change your projects",
        "deploy, restart, roll back, and edit settings and variables",
        "apps:write",
        1,
    ),
    (
        "Create new things",
        "projects, databases and services",
        "apps:create",
        1,
    ),
];

/// What asks first at each level, under the permissions.
fn asks(level: usize, name: &str) -> String {
    match level {
        0 => "It can't change anything.".to_owned(),
        1 => format!(
            "Deleting or removing anything, or connecting an account, asks you in {name} first."
        ),
        _ => "It doesn't ask you first, even before deleting something.".to_owned(),
    }
}

fn level_index(level: &str) -> Option<usize> {
    LEVELS.iter().position(|(value, _)| *value == level)
}

fn scopes(level: &str) -> Option<Vec<String>> {
    let index = level_index(level)?;
    Some(
        PERMISSIONS
            .iter()
            .filter(|(_, _, _, from)| *from <= index)
            .map(|(_, _, scope, _)| (*scope).to_owned())
            .collect(),
    )
}

/// Days until access ends, and their names. Zero is no end date.
const ENDS: [(u32, &str); 5] = [
    (0, "No end date"),
    (1, "In 1 day"),
    (7, "In 7 days"),
    (30, "In 30 days"),
    (90, "In 90 days"),
];

fn choices<'a>(items: &[(&'a str, &'a str)], selected: &str) -> Vec<Choice<'a>> {
    items
        .iter()
        .map(|(value, label)| Choice {
            value,
            label,
            selected: *value == selected,
        })
        .collect()
}

/// Where the browser goes after a remote agent's answer: back to the OAuth
/// step that sends it on to the agent. Only that exact path is accepted.
#[derive(Deserialize, Default)]
struct Back {
    back: Option<String>,
}

fn safe_back(back: Option<&str>) -> Option<&str> {
    back.filter(|back| {
        back.strip_prefix("/oauth/authorize?code=")
            .is_some_and(|code| {
                !code.is_empty()
                    && code.len() <= MAX_CODE_BYTES
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
            })
    })
}

/// The box that replaces a tool's earlier connections, ticked; nothing when
/// there are none.
fn replace_choice(name: &str, earlier: &[Agent]) -> Option<Tickbox> {
    match earlier {
        [] => None,
        [one] => Some(Tickbox {
            title: format!("Replace the earlier {name} connection"),
            hint: format!(
                "Connected before on this computer, last used {}. It stops working once you allow this one.",
                last_used(&one.last_seen)
            ),
        }),
        more => Some(Tickbox {
            title: format!("Replace the {} earlier {name} connections", more.len()),
            hint: "Connected before on this computer. They stop working once you allow this one."
                .to_owned(),
        }),
    }
}

/// `2026-11-08T14:03:00Z` as `8 Nov`; the text as given if it isn't a time.
fn last_used(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|time| time.format("%-d %b").to_string())
        .unwrap_or_else(|_| at.to_owned())
}

async fn consent(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    Query(back): Query<Back>,
) -> Response {
    let engine = &pages.engine;
    let Ok(Some(request)) = find_request(engine, &id).await else {
        return gone(
            "This request has ended",
            "It was answered, here or in the Hoista app, or ten minutes passed. If the agent still can't connect, ask it to connect again.",
        );
    };
    let accounts = engine
        .repository
        .accounts(None)
        .await
        .map(|page| page.items)
        .unwrap_or_default();
    let projects = engine
        .accounts()
        .project_summaries(ProjectListQuery::default())
        .await
        .map(|page| page.items)
        .unwrap_or_default();

    // The tool as people know it ("Claude Code"), not its id ("claude-code").
    let name = shown(&request.name);
    // A coding tool on this computer connecting again replaces its earlier
    // connection, unless the person keeps both.
    let earlier = if request.returns_to.is_none() {
        engine
            .accounts()
            .earlier_connections(&request.name)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // Each connected account, with the projects that use it, so the page can
    // say which accounts the agent will use for the projects picked.
    let uses: Vec<AccountUse> = accounts
        .iter()
        .filter(|account| account.status == AccountStatus::Connected)
        .map(|account| AccountUse {
            project_ids: projects
                .iter()
                .filter(|project| {
                    project
                        .resources
                        .iter()
                        .any(|resource| resource.account_id == account.id)
                })
                .map(|project| project.project.id.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            logo: crate::wiring::provider_logo(account.provider),
            provider: account.provider.display_name(),
            label: &account.label,
        })
        .collect();
    // Your Hoista environments, and each project with the environment it's in,
    // so picking environments narrows the projects offered.
    let environment_list = engine
        .environments()
        .list(&Actor::Person)
        .await
        .unwrap_or_default();
    let environment_name = |id: &str| {
        environment_list
            .iter()
            .find(|environment| environment.id == id)
            .map_or_else(|| id.to_owned(), |environment| environment.name.clone())
    };
    // One row per project: its name, its environment where there are several,
    // and the providers it runs on.
    let project_rows: Vec<ProjectRow> = projects
        .iter()
        .map(|project| {
            let mut providers: Vec<crate::Provider> = Vec::new();
            for resource in &project.resources {
                if !providers.contains(&resource.provider) {
                    providers.push(resource.provider);
                }
            }
            ProjectRow {
                id: &project.project.id,
                name: &project.project.name,
                find: project.project.name.to_lowercase(),
                environment_id: &project.project.environment_id,
                environment: (environment_list.len() > 1)
                    .then(|| environment_name(&project.project.environment_id)),
                providers: providers
                    .into_iter()
                    .map(|provider| ProviderMark {
                        name: provider.display_name(),
                        logo: crate::wiring::provider_logo(provider),
                    })
                    .collect(),
            }
        })
        .collect();
    // A project that can't show Hoista's question can't "always ask".
    let can_ask = request.can_ask != Some(false);
    let levels: Vec<(&str, &str)> = LEVELS
        .iter()
        .copied()
        .filter(|(value, _)| can_ask || *value != "ask")
        .collect();
    // Starts on the person's choice in Settings → Agents; a project that can't
    // show Hoista's question can't start on asking.
    let chosen = engine
        .repository
        .settings()
        .await
        .map(|settings| settings.default_access)
        .unwrap_or_else(|_| "ask".into());
    let first_level = match chosen.as_str() {
        "ask" if !can_ask => "view",
        "view" | "ask" | "full" => chosen.as_str(),
        _ if can_ask => "ask",
        _ => "view",
    };
    let first = level_index(first_level).unwrap_or(0);
    let ends: Vec<(String, &str)> = ENDS
        .iter()
        .map(|(days, label)| (days.to_string(), *label))
        .collect();
    let ends: Vec<(&str, &str)> = ends
        .iter()
        .map(|(days, label)| (days.as_str(), *label))
        .collect();
    html(&ConsentPage {
        title: format!("Authorize {}", request.name),
        place: &pages.place,
        returns_to: request.returns_to.as_deref(),
        token: token(&pages, "connect", &request.id),
        back: safe_back(back.back.as_deref()),
        name: &name,
        code: &request.code,
        levels_said: (0..LEVELS.len())
            .map(|index| LevelSaid {
                summary: crate::views::summary(index, &name),
                asks: asks(index, &name),
            })
            .collect(),
        summary_first: crate::views::summary(first, &name),
        asks_first: asks(first, &name),
        permissions: PERMISSIONS
            .iter()
            .map(|(title, detail, scope, from)| Permission {
                title,
                detail,
                scope,
                from: *from,
                hidden: *from > first,
            })
            .collect(),
        can_ask,
        chosen: format!(
            "{} · Every environment · Every project · No end date",
            LEVELS[first].1
        ),
        environments: environment_list
            .iter()
            .map(|environment| EnvironmentChoice {
                id: &environment.id,
                name: &environment.name,
            })
            .collect(),
        projects: project_rows,
        uses,
        levels: choices(&levels, first_level),
        ends: choices(&ends, "0"),
        replace: replace_choice(&name, &earlier),
        setup: request.tool.map(|tool| match tool {
            Tool::ClaudeCode => Tickbox {
                title: "Set up Claude Code for Hoista".into(),
                hint: "Adds Hoista's skills, and lets Hoista's own questions be the only ones Claude Code shows about it.".into(),
            },
            Tool::Codex => Tickbox {
                title: "Set up Codex for Hoista".into(),
                hint: "Adds Hoista's skills, so Codex knows how to deploy, fix and manage your projects.".into(),
            },
        }),
        follow: request.returns_to.is_none(),
    })
    .into_response()
}

#[derive(Deserialize)]
struct ConsentForm {
    token: String,
    back: Option<String>,
    decision: String,
    access: Option<String>,
    /// Hoista environment ids, comma-separated; empty for every one.
    env_ids: Option<String>,
    /// `all`, or `some` with `project_ids`.
    scope: Option<String>,
    /// Project ids, comma-separated.
    project_ids: Option<String>,
    ends: Option<u32>,
    setup: Option<String>,
    /// `on` to replace the earlier connections from the same tool.
    replace: Option<String>,
}

/// Comma-separated ids from a form field, without blanks or repeats.
fn ids(field: Option<&str>) -> Vec<String> {
    let mut list: Vec<String> = field
        .unwrap_or_default()
        .split(',')
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty() && id.len() <= MAX_ID_BYTES)
        .collect();
    list.dedup();
    list.truncate(MAX_LISTED);
    list
}

async fn answer_consent(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    Form(form): Form<ConsentForm>,
) -> Response {
    if !same(&form.token, &token(&pages, "connect", &id)) {
        return gone(
            "This page is out of date",
            "Ask the agent to connect again.",
        );
    }
    let allow = form.decision == "allow";
    let level = form.access.as_deref().unwrap_or("view");
    let Some(scopes) = scopes(level) else {
        return gone(
            "That access level isn't one Hoista offers",
            "Go back and pick View only, Ask for approval or Full access.",
        );
    };
    let environment_list = pages
        .engine
        .environments()
        .list(&Actor::Person)
        .await
        .unwrap_or_default();
    let picked_environments = ids(form.env_ids.as_deref());
    // None picked means every one, including environments added later.
    let environment_ids = (!picked_environments.is_empty()
        && picked_environments.len() < environment_list.len())
    .then_some(picked_environments);
    let picked_projects = ids(form.project_ids.as_deref());
    let projects = match form.scope.as_deref() {
        Some("some") => {
            if picked_projects.is_empty() {
                return gone(
                    "Pick at least one project",
                    "Go back and pick the projects it may touch, or choose every project.",
                );
            }
            Some(picked_projects)
        }
        _ => None,
    };
    let days = form
        .ends
        .filter(|days| ENDS.iter().any(|(value, _)| value == days))
        .unwrap_or(0);
    let expires_at = (days > 0).then(|| {
        (chrono::Utc::now() + chrono::Duration::days(i64::from(days)))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    });
    let summaries = pages
        .engine
        .accounts()
        .project_summaries(ProjectListQuery::default())
        .await
        .map(|page| page.items)
        .unwrap_or_default();
    let environment_names = environment_ids.as_ref().map_or_else(
        || "Every environment".to_owned(),
        |chosen| {
            environment_list
                .iter()
                .filter(|environment| chosen.contains(&environment.id))
                .map(|environment| environment.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        },
    );
    let project_names = projects.as_ref().map_or_else(
        || "Every project".to_owned(),
        |chosen| {
            let names: Vec<String> = summaries
                .iter()
                .filter(|item| chosen.contains(&item.project.id))
                .map(|item| item.project.name.clone())
                .collect();
            match names.len() {
                0..=MAX_NAMES_LISTED => names.join(", "),
                more => format!(
                    "{}, and {} more",
                    names[..MAX_NAMES_LISTED].join(", "),
                    more - MAX_NAMES_LISTED
                ),
            }
        },
    );
    let grant = AgentGrantInput {
        projects,
        environment_ids,
        expires_at,
        without_asking: level == "full",
        ..AgentGrantInput::default()
    };
    let answered = match id.strip_prefix(REMOTE) {
        Some(remote) => pages
            .engine
            .accounts()
            .decide_oauth(&Actor::Person, remote, allow, scopes, grant)
            .await
            .map(|request| (request.name, None)),
        None => pages
            .engine
            .accounts()
            .decide_enrollment(
                &Actor::Person,
                &id,
                allow,
                scopes,
                Some(grant),
                form.replace.as_deref() == Some("on"),
            )
            .await
            .map(|request| {
                let tool = Tool::from_name(&request.name);
                (request.name, tool)
            }),
    };
    match answered {
        Ok((name, tool)) if allow => {
            // Set the tool up when the person left the box ticked.
            let tool = tool.filter(|_| form.setup.as_deref() == Some("on"));
            let set = match tool {
                Some(tool) => {
                    match tokio::task::spawn_blocking(move || crate::wiring::set_up_tool(tool))
                        .await
                    {
                        Ok(Some(Ok(_))) => "done",
                        Ok(None) => "no",
                        _ => "failed",
                    }
                }
                None => "no",
            };
            Redirect::to(
                &(format!(
                    "/connect/{}/done?access={level}&name={}&projects={}&envs={}&ends={days}&set={set}",
                    url_resource(&id),
                    url_resource(&shown(&name)),
                    url_resource(&project_names),
                    url_resource(&environment_names),
                ) + &safe_back(form.back.as_deref())
                    .map_or_else(String::new, |back| format!("&back={}", url_resource(back)))),
            )
            .into_response()
        }
        Ok((name, _)) => gone(
            &format!("{} wasn't connected", shown(&name)),
            "Nothing was shared. You can close this tab.",
        ),
        Err(error) => {
            let keychain = matches!(
                &error,
                crate::application::ports::catalog::EngineError::Provider { code, .. }
                    if code == "keychain.unavailable"
            );
            if keychain {
                gone(
                    "Couldn't save this connection",
                    "Hoista keeps the agent's key in your keychain, and the keychain didn't open. Unlock it, then ask the agent to connect again.",
                )
            } else {
                gone("Couldn't connect the agent", &error.to_string())
            }
        }
    }
}

#[derive(Deserialize)]
struct Done {
    back: Option<String>,
    access: Option<String>,
    name: Option<String>,
    projects: Option<String>,
    envs: Option<String>,
    ends: Option<u32>,
    set: Option<String>,
}

async fn connected(Query(done): Query<Done>) -> Html<String> {
    let level = LEVELS
        .iter()
        .find(|(value, _)| Some(*value) == done.access.as_deref())
        .map_or("View only", |(_, label)| label);
    let ends = ENDS
        .iter()
        .find(|(days, _)| Some(*days) == done.ends)
        .map_or("No end date", |(_, label)| label);
    html(&ConnectedPage {
        title: "Connected",
        name: done.name.as_deref().unwrap_or("The agent"),
        environments: done.envs.as_deref().unwrap_or("Every environment"),
        projects: done.projects.as_deref().unwrap_or("Every project"),
        level,
        ends,
        set_up: match done.set.as_deref() {
            Some("done") => Some(true),
            Some("failed") => Some(false),
            _ => None,
        },
        back: safe_back(done.back.as_deref()),
    })
}

fn gone(title: &str, text: &str) -> Response {
    (
        StatusCode::OK,
        html(&ResultPage {
            title,
            text,
            left: None,
            done: false,
        }),
    )
        .into_response()
}

/// The tool as people know it ("Claude Code"), not its id ("claude-code").
fn shown(name: &str) -> String {
    Tool::from_name(name).map_or_else(|| name.to_owned(), |tool| tool.label().to_owned())
}

fn url_resource(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_map_to_scopes() {
        assert_eq!(scopes("view"), Some(vec!["apps:read".to_owned()]));
        assert_eq!(scopes("full").map(|list| list.len()), Some(3));
        assert_eq!(scopes("root"), None);
        assert_eq!(url_resource("Claude Code"), "Claude%20Code");
    }
}
