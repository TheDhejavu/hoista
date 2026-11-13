//! The conformance test every provider adapter must pass, run against the fake
//! provider declared in its `ENTRY` entry (`ProviderEntry::fake`).
//!
//! Unknown changes fail with an unclear outcome, so the test can check that no
//! change is sent twice; it also checks tokens reach only the provider's API.

use super::{ENTRIES, ProviderEntry, http::ProviderHttp};
use crate::{
    application::ports::{
        catalog::EngineError,
        providers::{Artifact, ProviderAction, ProviderAdapter, RemoteResource, ServerSource},
    },
    domain::{Secret, logs::LogHistoryQuery, model::*},
};
use axum::{
    Router,
    body::Bytes,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt as _;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// A fake provider: enough answers to connect and find one resource.
pub(crate) struct Fake {
    /// A token in the shape the provider's sign-in expects.
    pub token: &'static str,
    /// The account the token reaches (`AccountChoice::external_id`).
    pub account: &'static str,
    /// A resource it finds, as its reference; the probes act on it.
    pub resource: &'static str,
    /// What kind of resource it is.
    pub kind: ResourceKind,
    /// Answers, first match wins.
    pub routes: &'static [Route],
    /// The answer to a read no route matches: the provider's own shape for
    /// "nothing here".
    pub empty: &'static str,
}

/// One answer. A request matches when its path and query contain `path`
/// and its body contains `body`.
pub(crate) struct Route {
    pub path: &'static str,
    pub body: &'static str,
    pub answer: &'static str,
}

/// The token for providers that take any string as a token.
pub(crate) const TOKEN: &str = "conformance-token-5f2b";

/// How long one probe may take, and how much of a request body a failure prints.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_LOGGED_BODY_CHARS: usize = 160;

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    target: String,
    body: String,
    headers: Vec<(String, String)>,
}

/// A read: `GET`/`HEAD`, or a GraphQL query (not a mutation).
fn is_read(seen: &Seen) -> bool {
    matches!(seen.method.as_str(), "GET" | "HEAD")
        || serde_json::from_str::<serde_json::Value>(&seen.body)
            .ok()
            .and_then(|body| {
                body["query"]
                    .as_str()
                    .map(str::trim_start)
                    .map(str::to_owned)
            })
            .is_some_and(|query| query.starts_with("query") || query.starts_with('{'))
}

struct Server {
    http: ProviderHttp,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: tokio_util::sync::CancellationToken,
}

async fn serve(fake: &'static Fake) -> Server {
    let seen = Arc::new(Mutex::new(Vec::<Seen>::new()));
    let log = seen.clone();
    let router = Router::new().fallback(move |request: Request| {
        let log = log.clone();
        async move {
            let method = request.method().to_string();
            let target = request
                .uri()
                .path_and_query()
                .map(ToString::to_string)
                .unwrap_or_default();
            let headers = request
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
                })
                .collect();
            let bytes: Bytes = request
                .into_body()
                .collect()
                .await
                .map(http_body_util::Collected::to_bytes)
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let request = Seen {
                method,
                target,
                body,
                headers,
            };
            log.lock().unwrap().push(request.clone());
            answer(fake, &request)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let cancelled = stop.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(cancelled.cancelled_owned())
            .await;
    });
    Server {
        http: ProviderHttp::local_fixture(base),
        seen,
        stop,
    }
}

fn answer(fake: &Fake, request: &Seen) -> Response {
    let json = |status: StatusCode, body: &str| {
        (
            status,
            [("content-type", "application/json")],
            body.to_owned(),
        )
            .into_response()
    };
    if let Some(route) = fake
        .routes
        .iter()
        .find(|route| request.target.contains(route.path) && request.body.contains(route.body))
    {
        return json(StatusCode::OK, route.answer);
    }
    if is_read(request) {
        json(StatusCode::OK, fake.empty)
    } else {
        // A change the fake doesn't know: the provider fails after receiving it,
        // so whether it happened is unclear.
        json(StatusCode::BAD_GATEWAY, "{}")
    }
}

/// What one probe found.
struct Outcome {
    refused: bool,
    error: Option<String>,
    seen: Vec<Seen>,
    destinations: Vec<String>,
}

fn not_supported(error: &EngineError) -> bool {
    matches!(error, EngineError::Provider { code, .. } if code == "provider.not_supported")
}

async fn probe<T, F>(server: &Server, call: F) -> Outcome
where
    F: std::future::Future<Output = Result<T, EngineError>>,
{
    server.seen.lock().unwrap().clear();
    let _ = server.http.take_destinations();
    let result = tokio::time::timeout(PROBE_TIMEOUT, call).await;
    let (refused, error) = match result {
        Ok(Ok(_)) => (false, None),
        Ok(Err(error)) => (not_supported(&error), Some(error.to_string())),
        // Still talking to the provider after the time allowed: not refused.
        Err(_) => (false, None),
    };
    Outcome {
        refused,
        error,
        seen: server.seen.lock().unwrap().clone(),
        destinations: server.http.take_destinations(),
    }
}

/// The checks every call must pass, whatever it does.
fn always(entry: &ProviderEntry, what: &str, outcome: &Outcome, problems: &mut Vec<String>) {
    let name = entry.provider.slug();
    for destination in &outcome.destinations {
        if !entry.api.hosts.contains(&destination.as_str()) {
            problems.push(format!(
                "{name} {what}: sent a request to {destination}, not one of its API hosts"
            ));
        }
    }
    // Every request is signed in exactly the way the provider's definition says.
    let signed = entry
        .api
        .sign_in
        .headers(&Secret::new(entry.fake.token.into()))
        .unwrap_or_default();
    for seen in &outcome.seen {
        let carried = |header: &str| -> Vec<&str> {
            seen.headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case(header))
                .map(|(_, value)| value.as_str())
                .collect()
        };
        let exact = signed
            .iter()
            .all(|(header, value)| carried(header) == [value.as_str()]);
        let unsigned = signed.iter().all(|(header, _)| carried(header).is_empty());
        // Token endpoints (OAuth) sign in with a form, not the token.
        if !(exact || (unsigned && seen.target.contains("oauth"))) {
            problems.push(format!(
                "{name} {what}: {} {} wasn't signed in exactly as {name} takes its token",
                seen.method, seen.target
            ));
        }
    }
    let writes: Vec<&Seen> = outcome.seen.iter().filter(|seen| !is_read(seen)).collect();
    for (index, write) in writes.iter().enumerate() {
        if writes[..index]
            .iter()
            .any(|earlier| earlier.target == write.target && earlier.body == write.body)
        {
            problems.push(format!(
                "{name} {what}: repeated a change after an unclear answer: {} {}",
                write.method, write.target
            ));
        }
    }
    if outcome
        .error
        .as_deref()
        .is_some_and(|error| error.contains(entry.fake.token))
    {
        problems.push(format!("{name} {what}: an error repeated the token"));
    }
}

/// A claimed capability must be implemented; an unclaimed one must be refused
/// without asking the provider.
fn matches_claim(
    entry: &ProviderEntry,
    what: &str,
    claimed: bool,
    outcome: &Outcome,
    problems: &mut Vec<String>,
) {
    let name = entry.provider.slug();
    if claimed && outcome.refused {
        problems.push(format!(
            "{name} claims {what} but refuses it as not supported"
        ));
    }
    if !claimed && !outcome.refused {
        problems.push(format!(
            "{name} doesn't claim {what} but doesn't refuse it ({})",
            outcome.error.as_deref().unwrap_or("it answered")
        ));
    }
    if !claimed && !outcome.seen.is_empty() {
        problems.push(format!(
            "{name} doesn't claim {what} but asked the provider ({} requests)",
            outcome.seen.len()
        ));
    }
}

async fn check(entry: &'static ProviderEntry, problems: &mut Vec<String>) {
    let server = serve(&entry.fake).await;
    let adapter = (entry.adapter)(server.http.clone());
    check_adapter(entry, &server, adapter.as_ref(), problems).await;
    server.stop.cancel();
}

async fn check_adapter(
    entry: &'static ProviderEntry,
    server: &Server,
    adapter: &dyn ProviderAdapter,
    problems: &mut Vec<String>,
) {
    let fake = &entry.fake;
    let can = adapter.capabilities();
    let token = Secret::new(fake.token.into());
    let account = fake.account;
    let reference = fake.resource;
    let name = entry.provider.slug();

    // 1. Connect and find resources.
    let choices = probe(server, adapter.account_choices(&token)).await;
    always(entry, "listing accounts", &choices, problems);
    match adapter.account_choices(&token).await {
        Ok(found) if found.iter().any(|choice| choice.external_id == account) => {}
        other => problems.push(format!("{name}: couldn't list its account: {other:?}")),
    }
    let verified = probe(server, adapter.verify(&token, account)).await;
    always(entry, "verifying", &verified, problems);
    if let Some(error) = verified.error {
        problems.push(format!("{name}: couldn't verify its account: {error}"));
    }
    let discovered = probe(server, adapter.discover(&token, account)).await;
    always(entry, "finding apps", &discovered, problems);
    if std::env::var("CONFORMANCE_TRACE").is_ok() {
        for seen in choices
            .seen
            .iter()
            .chain(&verified.seen)
            .chain(&discovered.seen)
        {
            eprintln!(
                "{name}: {} {} {}",
                seen.method,
                seen.target,
                seen.body
                    .chars()
                    .take(MAX_LOGGED_BODY_CHARS)
                    .collect::<String>()
            );
        }
    }
    match adapter.discover(&token, account).await {
        Ok(found) if found.iter().any(|resource| resource.reference == reference) => {}
        // A provider that runs nothing (GitHub) finds nothing.
        Ok(found) if !can.discover && found.is_empty() => {}
        other => problems.push(format!(
            "{name}: didn't find {reference}: {:?}",
            other.map(|found| found.into_iter().map(|r| r.reference).collect::<Vec<_>>())
        )),
    }

    // 2–5. Each capability, claimed or not.
    let resource = Resource {
        id: "resource".into(),
        project_id: Some("project".into()),
        kind: fake.kind,
        provider: entry.provider,
        account_id: "account".into(),
        provider_ref: reference.into(),
        name: "web".into(),
        region: None,
        status: ResourceStatus::Live,
        mode: Mode::Watching,
        url: None,
        group: None,
    };
    let plan = ResourcePlan {
        schedules: None,
        provider_ref: Some(reference.into()),
        name: "web".into(),
        kind: fake.kind,
        provider: entry.provider,
        account_id: "account".into(),
        region: None,
        depends_on: vec![],
    };
    let artifact = Artifact {
        provenance: None,
        assets: vec![],
        worker: Some(b"export default {}".to_vec()),
        worker_name: "index.js".into(),
        worker_modules: vec![],
        image: Some("registry.example/app@sha256:00".into()),
        secrets: vec![],
    };
    let release = Release {
        schedules: None,
        reference: reference.into(),
        version: "v1".into(),
        url: None,
    };
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("index.html"), "hi").unwrap();
    let value = Secret::new("conformance-value".into());
    let history = LogHistoryQuery {
        from: "2026-11-01T00:00:00Z".into(),
        to: "2026-11-01T01:00:00Z".into(),
        query: String::new(),
        cursor: None,
    };
    let now = chrono::Utc::now();
    let variables = std::collections::BTreeMap::new();
    let source = ServerSource::Image("registry.example/hoista@sha256:00".into());

    macro_rules! claim {
        ($what:literal, $claimed:expr, $call:expr) => {{
            let outcome = probe(server, $call).await;
            always(entry, $what, &outcome, problems);
            matches_claim(entry, $what, $claimed, &outcome, problems);
        }};
    }
    claim!(
        "deploy",
        can.deploy,
        adapter.deploy(&token, account, &plan, &artifact)
    );
    claim!(
        "rollback",
        can.rollback,
        adapter.rollback(&token, account, &release)
    );
    claim!(
        "pause",
        can.pause,
        adapter.pause(&token, account, reference)
    );
    claim!(
        "domains",
        can.domains,
        adapter.configure_domain(
            &token,
            account,
            reference,
            "app.example.com",
            "example.com",
            false
        )
    );
    claim!(
        "secrets",
        can.secrets,
        adapter.set_secret(&token, account, reference, "NAME", &value)
    );
    claim!(
        "logs",
        can.logs,
        adapter.log_history(&token, account, reference, &history)
    );
    claim!(
        "usage",
        can.usage,
        adapter.usage(&token, account, &resource, "2026-11-01")
    );
    // Metrics come as request counts (provider_traffic) or as charts
    // (provider_metrics): a provider claiming metrics answers at least one.
    {
        let traffic = probe(
            server,
            adapter.provider_traffic(
                &token,
                account,
                reference,
                now - chrono::Duration::hours(1),
                now,
            ),
        )
        .await;
        always(entry, "metrics", &traffic, problems);
        let charts = probe(
            server,
            adapter.provider_metrics(
                &token,
                account,
                reference,
                now - chrono::Duration::hours(1),
                now,
                60,
            ),
        )
        .await;
        always(entry, "metrics", &charts, problems);
        let outcome = Outcome {
            refused: traffic.refused && charts.refused,
            error: charts.error.or(traffic.error),
            seen: [traffic.seen, charts.seen].concat(),
            destinations: [traffic.destinations, charts.destinations].concat(),
        };
        matches_claim(entry, "metrics", can.metrics, &outcome, problems);
    }
    claim!(
        "provider_details",
        can.provider_details,
        adapter.provider_details(&token, account, reference)
    );
    claim!(
        "provider_deploys",
        can.provider_deploys,
        adapter.provider_action(&token, account, reference, &ProviderAction::Redeploy)
    );
    claim!(
        "provider_variables",
        can.provider_variables,
        adapter.set_provider_variable(&token, account, reference, "NAME", &value)
    );
    claim!(
        "upload",
        can.upload,
        adapter.provider_upload(&token, account, reference, folder.path(), None)
    );
    claim!(
        "new_project",
        can.new_project,
        adapter.provider_new_project(&token, account, "project", "owner/repo", None)
    );
    claim!(
        "new_from_folder",
        can.new_from_folder,
        adapter.provider_new_from_folder(&token, account, "project", folder.path())
    );
    {
        use crate::application::ports::providers::ProviderNew;
        claim!(
            "new_database",
            can.new_database,
            adapter.provider_add(
                &token,
                account,
                "project",
                None,
                &ProviderNew::Database {
                    name: "db",
                    engine: "postgres",
                    region: None,
                }
            )
        );
        claim!(
            "new_bucket",
            can.new_bucket,
            adapter.provider_add(
                &token,
                account,
                "project",
                None,
                &ProviderNew::Bucket { name: "files" }
            )
        );
        claim!(
            "new_email_domain",
            can.new_email_domain,
            adapter.provider_add(
                &token,
                account,
                "project",
                None,
                &ProviderNew::EmailDomain {
                    domain: "mail.example.com"
                }
            )
        );
        claim!(
            "new_from_image",
            can.new_from_image,
            adapter.provider_add(
                &token,
                account,
                "project",
                None,
                &ProviderNew::Image {
                    name: "web",
                    image: "nginx:latest",
                    gpu: None,
                    cpu: None,
                    port: Some(80),
                    args: None,
                }
            )
        );
    }
    claim!("bills", can.bills, adapter.provider_bill(&token, account));
    claim!(
        "delete",
        can.delete,
        adapter.delete(&token, account, reference)
    );
    claim!(
        "server",
        can.server,
        adapter.provider_new_hoista_server(&token, account, &source, &variables)
    );
    if can.provider_operations == adapter.provider_operations().is_empty() {
        problems.push(format!(
            "{name}: provider_operations is {} but its catalog has {} operations",
            can.provider_operations,
            adapter.provider_operations().len()
        ));
    }
    if can.browser_sign_in != adapter.oauth().is_some() {
        problems.push(format!(
            "{name}: browser_sign_in is {} but it {} a sign-in app",
            can.browser_sign_in,
            if adapter.oauth().is_some() {
                "has"
            } else {
                "has no"
            }
        ));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_provider_adapter_passes() {
    let mut problems = Vec::new();
    for entry in ENTRIES {
        check(entry, &mut problems).await;
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

/// An adapter that breaks each rule once, to show the suite catches it.
struct Liar {
    http: ProviderHttp,
}

#[async_trait::async_trait]
impl ProviderAdapter for Liar {
    fn provider(&self) -> crate::Provider {
        crate::Provider::Vercel
    }

    fn capabilities(&self) -> Capabilities {
        // Claims bills, which it doesn't implement.
        Capabilities {
            discover: true,
            bills: true,
            ..Capabilities::default()
        }
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        super::accounts::list(&self.http, crate::Provider::Vercel, token).await
    }

    async fn verify(
        &self,
        _token: &Secret,
        _external_id: &str,
    ) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(
        &self,
        token: &Secret,
        _external_id: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        // Addresses another provider's API with this provider's token.
        let _ = self
            .http
            .json(
                crate::Provider::Supabase,
                token,
                reqwest::Method::GET,
                &["projects"],
                &[],
                None,
            )
            .await;
        Ok(vec![RemoteResource {
            reference: "res-1".into(),
            name: "web".into(),
            kind: ResourceKind::Web,
            region: None,
            status: ResourceStatus::Live,
            url: None,
            group: None,
        }])
    }

    async fn pause(
        &self,
        token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<(), EngineError> {
        // Doesn't claim pause, asks the provider anyway, and sends the change twice.
        for _ in 0..2 {
            let _ = self
                .http
                .json(
                    crate::Provider::Vercel,
                    token,
                    reqwest::Method::POST,
                    &["pause"],
                    &[],
                    None,
                )
                .await;
        }
        // And repeats the token in its error.
        Err(EngineError::Provider {
            evidence: None,
            code: "provider.rejected".into(),
            message: format!("refused {}", token.expose()),
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_suite_catches_an_adapter_that_breaks_the_rules() {
    let entry = super::entry(crate::Provider::Vercel);
    let server = serve(&entry.fake).await;
    let liar = Liar {
        http: server.http.clone(),
    };
    let mut problems = Vec::new();
    check_adapter(entry, &server, &liar, &mut problems).await;
    server.stop.cancel();
    let found = problems.join("\n");
    for rule in [
        "claims bills but refuses it",
        "sent a request to api.supabase.com",
        "doesn't claim pause but doesn't refuse it",
        "doesn't claim pause but asked the provider",
        "repeated a change after an unclear answer",
        "an error repeated the token",
    ] {
        assert!(found.contains(rule), "missed: {rule}\n{found}");
    }
}
