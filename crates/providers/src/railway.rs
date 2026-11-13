//! Railway, through its public GraphQL API. References are `project/service`.
//!
//! Logs belong to a single deployment on Railway, so history and live tails
//! read only the service's latest deployment.

use super::{
    answers::{field, invalid},
    http::ProviderHttp,
};
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{
            LogSession, OAuthGrant, OAuthTokens, Observed, ProviderAction, ProviderAdapter,
            ProviderBill, ProviderConfig, ProviderCreate, ProviderDeployState, ProviderDeployment,
            ProviderDetails, ProviderDnsCheck, ProviderDnsRecord, ProviderDomain,
            ProviderDomainState, ProviderErrorCount, ProviderOAuth, ProviderReference,
            ProviderRequest, ProviderTcpProxy, ProviderTraffic, ProviderVolume, RemoteResource,
            RestartPolicy, ServiceConfig, VariableAddress,
        },
    },
    domain::{
        Secret,
        domains::{MAX_HOSTNAME_CHARS, MAX_LABEL_CHARS, valid_hostname},
        leaks::LeakScanner,
        logs::{LogEntry, LogHistory, LogHistoryQuery, LogLevel},
        model::{
            AccountChoice, Capabilities, MAX_DISCOVERED_RESOURCES, Money, Resource, ResourceKind,
            ResourceStatus, Usage,
        },
    },
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

/// Log lines a tail remembers to skip repeats at its time boundary.
const MAX_SEEN_LINES: usize = 10_000;

/// Railway's API host: GraphQL, uploads and sign-in.
const BACKBOARD: &str = "https://backboard.railway.com/";
/// Railway's GraphQL API.
const GRAPHQL: &str = "https://backboard.railway.com/graphql/v2/";
/// Where Railway's sign-in page sends people.
const OAUTH_AUTHORIZE_URL: &str = "https://backboard.railway.com/oauth/auth";
/// Where Railway's sign-in codes and refresh tokens are exchanged.
const OAUTH_TOKEN_URL: &str = "https://backboard.railway.com/oauth/token";

/// A Railway project's page on its dashboard.
fn project_page(project: &str) -> String {
    format!("https://railway.com/project/{project}")
}

/// Hoista's Railway OAuth app ("Login with Railway"): a native app, so a
/// public client with PKCE and no secret. `workspace:admin` asks for the
/// person's own role in the workspaces they choose on Railway's consent page
/// (Railway caps it at that role): a Member can build and change things but
/// only an Admin can delete services, volumes and projects, and Hoista still
/// holds every delete for the person's approval. `offline_access` (with
/// `prompt=consent`) returns a refresh token, since access tokens last an hour.
/// Registered with `http://127.0.0.1:{47129,47130}/callback`. Without
/// `HOISTA_RAILWAY_CLIENT_ID` at build time, Railway is connected with a token.
const CLIENT_ID: Option<&str> = option_env!("HOISTA_RAILWAY_CLIENT_ID");

fn oauth_app() -> Option<ProviderOAuth> {
    CLIENT_ID.map(|client_id| ProviderOAuth {
        authorize_url: OAUTH_AUTHORIZE_URL,
        client_id,
        // `email` and `profile` are required for `me { workspaces }` to list the
        // workspaces the person shared on the consent page.
        scopes: "openid email profile offline_access workspace:admin",
        ports: &[47129, 47130],
        extra: &[("prompt", "consent")],
        redirect_host: "127.0.0.1",
        logo: LOGO,
    })
}

/// Railway's mark, for Hoista's sign-in pages.
const LOGO: &str = r#"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M.113 10.27A13.026 13.026 0 000 11.48h18.23c-.064-.125-.15-.237-.235-.347-3.117-4.027-4.793-3.677-7.19-3.78-.8-.034-1.34-.048-4.524-.048-1.704 0-3.555.005-5.358.01-.234.63-.459 1.24-.567 1.737h9.342v1.216H.113v.002zm18.26 2.426H.009c.02.326.05.645.094.961h16.955c.754 0 1.179-.429 1.315-.96zm-17.318 4.28s2.81 6.902 10.93 7.024c4.855 0 9.027-2.883 10.92-7.024H1.056zM11.988 0C7.5 0 3.593 2.466 1.531 6.108l4.75-.005v-.002c3.71 0 3.849.016 4.573.047l.448.016c1.563.052 3.485.22 4.996 1.364.82.621 2.007 1.99 2.712 2.965.654.902.842 1.94.396 2.934-.408.914-1.289 1.458-2.353 1.458H.391s.099.42.249.886h22.748A12.026 12.026 0 0024 12.005C24 5.377 18.621 0 11.988 0z" fill="currentColor"/></svg>"#;

/// Exchanges a code or refresh token at Railway's token endpoint, as a public
/// client (the client id in the form, no secret).
///
/// # Errors
/// Reports refused grants, provider failures and malformed responses.
pub(super) async fn oauth_token(
    http: &ProviderHttp,
    grant: OAuthGrant<'_>,
) -> Result<OAuthTokens, EngineError> {
    let client_id = CLIENT_ID.ok_or_else(super::oauth::not_set_up)?;
    super::oauth::public_token(http, OAUTH_TOKEN_URL, client_id, grant).await
}

/// Largest message kept from one log line.
const MAX_MESSAGE: usize = 8 * 1024;
/// Metric samples are read at this interval; usage sums value × interval.
const SAMPLE_SECONDS: i64 = 300;
/// Lines per history request, and how many requests one history read may make.
const HISTORY_PAGE: usize = 1000;
const HISTORY_PAGES: usize = 5;
/// How often a live tail asks Railway for new lines; slow enough for Railway's hourly API limit.
const POLL: Duration = Duration::from_secs(10);

/// A service's newest deployment.
pub(super) struct Latest {
    pub(super) id: String,
    pub(super) status: ResourceStatus,
    pub(super) url: Option<String>,
    pub(super) environment_id: Option<String>,
}

/// Railway's database templates keep their engine's name ("Postgres",
/// "MySQL", "Redis", "MongoDB"); every other service runs code.
pub(super) fn kind_of(name: &str) -> ResourceKind {
    let name = name.to_ascii_lowercase();
    if ["postgres", "mysql", "redis", "mongo", "mariadb"]
        .iter()
        .any(|engine| name.contains(engine))
    {
        ResourceKind::Database
    } else {
        ResourceKind::Service
    }
}

pub(super) fn split(reference: &str) -> Result<(&str, &str), EngineError> {
    reference
        .split_once('/')
        .filter(|(project, service)| !project.is_empty() && !service.is_empty())
        .ok_or(EngineError::Invalid(
            "This is not a Railway service reference.",
        ))
}

/// Railway deployment states mapped to what Hoista shows, in Railway's own
/// sense: building, deploying and waiting are all "deploying".
fn status_of(value: &str) -> ResourceStatus {
    match value {
        "SUCCESS" => ResourceStatus::Live,
        "SLEEPING" => ResourceStatus::Sleeping,
        "FAILED" | "CRASHED" => ResourceStatus::Failing,
        "BUILDING" | "DEPLOYING" | "INITIALIZING" | "QUEUED" | "WAITING" | "NEEDS_APPROVAL" => {
            ResourceStatus::Deploying
        }
        "REMOVED" | "REMOVING" | "SKIPPED" => ResourceStatus::Stopped,
        _ => ResourceStatus::Unknown,
    }
}

/// How long an identical Railway read is answered from memory. Railway's
/// public API allows only a few hundred to a few thousand requests an hour
/// (by plan), and several screens and the monitor ask the same questions.
const READ_REUSE: Duration = Duration::from_secs(30);
/// Reads remembered at once; the oldest is dropped past this.
const READ_REUSE_ENTRIES: usize = 256;
/// After Railway says "too many requests", Hoista sends nothing more with that
/// token for this long and answers reads from what it last saw. Retrying at
/// once only spends the next hour's allowance.
const RATE_LIMIT_PAUSE: Duration = Duration::from_secs(120);
/// How old a remembered read may be to stand in while Railway is paused.
const STALE_LIMIT: Duration = Duration::from_secs(1800);

// The most of each thing read from one answer.
const MAX_VARIABLES: usize = 500;
const MAX_TCP_PROXIES: usize = 20;
const MAX_VOLUMES: usize = 100;
const MAX_DOMAINS: usize = 50;
const MAX_DNS_RECORDS: usize = 10;
/// The most of an error message from Railway that's shown.
const MAX_ERROR_CHARS: usize = 200;
const MAX_CERTIFICATE_ERROR_CHARS: usize = 300;
const MAX_REPO_CHARS: usize = 200;
const MAX_PATH_CHARS: usize = 200;
const MAX_SETTING_BYTES: usize = 1024;
/// A commit id, short or full.
const COMMIT_ID_CHARS: std::ops::RangeInclusive<usize> = 7..=64;
/// How many copies of a service Railway runs.
const REPLICAS: std::ops::RangeInclusive<u8> = 1..=20;
const MAX_ID_CHARS: usize = 128;
const MAX_VARIABLE_CHARS: usize = 256;
/// Traffic: the longest range, how much of a path is kept, and how many of
/// the most frequent errors are listed.
const MAX_TRAFFIC_DAYS: i64 = 7;
const MAX_REQUEST_PATH_CHARS: usize = 300;
const MAX_TOP_ERRORS: usize = 5;
/// Services whose latest deployment discovery reads for status and address.
const MAX_STATE_LOOKUPS: usize = 50;
/// How often a template being added is checked for what it made.
#[cfg(not(test))]
const TEMPLATE_POLL: Duration = Duration::from_secs(3);

fn paused() -> &'static std::sync::Mutex<std::collections::HashMap<[u8; 32], std::time::Instant>> {
    static PAUSED: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<[u8; 32], std::time::Instant>>,
    > = std::sync::OnceLock::new();
    PAUSED.get_or_init(Default::default)
}

fn token_key(token: &Secret) -> [u8; 32] {
    use sha2::Digest;
    <[u8; 32]>::from(sha2::Sha256::digest(token.expose().as_bytes()))
}

fn rate_limited() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.rate_limited".into(),
        message: "Railway asked Hoista to slow down. Hoista shows what it last saw and tries again in a couple of minutes.".into(),
    }
}

type ReadCache = std::sync::Mutex<std::collections::HashMap<[u8; 32], (std::time::Instant, Value)>>;

fn read_cache() -> &'static ReadCache {
    static CACHE: std::sync::OnceLock<ReadCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Forgets every remembered read, after a change made outside [`graphql`].
pub(super) fn forget_reads() {
    if let Ok(mut cache) = read_cache().lock() {
        cache.clear();
    }
    super::railway_catalog::forget_answers();
}

/// A GraphQL document's operation name (`HoistaMetrics` in
/// `query HoistaMetrics(…)`), or empty.
fn operation_name(query: &str) -> &str {
    let query = query.trim_start();
    ["query ", "mutation ", "subscription "]
        .iter()
        .find_map(|kind| query.strip_prefix(kind))
        .and_then(|rest| rest.split(['(', '{', ' ']).next())
        .filter(|name| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or_default()
}

pub(super) async fn graphql(
    http: &ProviderHttp,
    token: &Secret,
    query: &str,
    variables: Value,
) -> Result<Value, EngineError> {
    let body = json!({ "query": query, "variables": variables });
    // Reads only, keyed by the token too, so one account never sees another's
    // answer. Tests talk to scripted fixtures and always ask.
    let key = (query.starts_with("query") && !cfg!(test)).then(|| {
        use sha2::Digest;
        let mut digest = sha2::Sha256::new();
        digest.update(token.expose().as_bytes());
        digest.update([0]);
        digest.update(body.to_string().as_bytes());
        <[u8; 32]>::from(digest.finalize())
    });
    if let Some(key) = &key
        && let Ok(cache) = read_cache().lock()
        && let Some((at, value)) = cache.get(key)
        && at.elapsed() < READ_REUSE
    {
        return Ok(value.clone());
    }
    let stale = || {
        key.as_ref().and_then(|key| {
            read_cache().lock().ok().and_then(|cache| {
                cache
                    .get(key)
                    .filter(|(at, _)| at.elapsed() < STALE_LIMIT)
                    .map(|(_, value)| value.clone())
            })
        })
    };
    let holder = token_key(token);
    let waiting = paused()
        .lock()
        .ok()
        .and_then(|paused| paused.get(&holder).copied())
        .is_some_and(|until| std::time::Instant::now() < until);
    if waiting && !cfg!(test) {
        return stale().ok_or_else(rate_limited);
    }
    let value = match http
        .json(
            Provider::Railway,
            token,
            Method::POST,
            &[],
            &[],
            Some(&body),
        )
        .await
    {
        Ok(value) => value,
        Err(EngineError::Provider { code, .. }) if code == "provider.rate_limited" => {
            if let Ok(mut paused) = paused().lock() {
                paused.insert(holder, std::time::Instant::now() + RATE_LIMIT_PAUSE);
            }
            return stale().ok_or_else(rate_limited);
        }
        Err(error) => {
            // Which of Hoista's queries Railway refused, by its name, for
            // the log: the query's variables stay out of it.
            tracing::warn!(
                code = error.code(),
                operation = operation_name(query),
                "Railway refused a query"
            );
            return Err(error);
        }
    };
    // A change makes every remembered read possibly out of date.
    if key.is_none()
        && let Ok(mut cache) = read_cache().lock()
    {
        cache.clear();
        super::railway_catalog::forget_answers();
    }
    if let Some(key) = key
        && let Ok(mut cache) = read_cache().lock()
    {
        cache.retain(|_, (at, _)| at.elapsed() < STALE_LIMIT);
        if cache.len() >= READ_REUSE_ENTRIES
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| *key)
        {
            cache.remove(&oldest);
        }
        cache.insert(key, (std::time::Instant::now(), value.clone()));
    }
    Ok(value)
}

/// The service's newest deployment, or None when it has never deployed.
///
/// # Errors
/// Reports invalid references, provider failures and malformed responses.
pub(super) async fn latest(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
) -> Result<Option<Latest>, EngineError> {
    let (project, service) = split(reference)?;
    let value = graphql(
        http,
        token,
        "query HoistaLatest($input:DeploymentListInput!){deployments(input:$input,first:1){edges{node{id status url staticUrl environmentId}}}}",
        json!({ "input": { "projectId": project, "serviceId": service } }),
    )
    .await?;
    let edges = value["data"]["deployments"]["edges"]
        .as_array()
        .ok_or_else(invalid)?;
    let Some(node) = edges.first().map(|edge| &edge["node"]) else {
        return Ok(None);
    };
    let host = node["staticUrl"]
        .as_str()
        .or_else(|| node["url"].as_str())
        .filter(|host| {
            !host.is_empty()
                && host.len() <= MAX_HOSTNAME_CHARS
                && !host.contains(char::is_whitespace)
        });
    Ok(Some(Latest {
        id: node["id"].as_str().ok_or_else(invalid)?.into(),
        status: status_of(node["status"].as_str().ok_or_else(invalid)?),
        url: host.map(|host| {
            if host.starts_with("https://") {
                host.to_owned()
            } else {
                format!("https://{host}")
            }
        }),
        environment_id: node["environmentId"].as_str().map(str::to_owned),
    }))
}

/// Railway's deployment history (newest 20), variable names and domains for
/// a service. Variable values arrive in Railway's response and are dropped
/// on the spot; only names are returned. Variables and domains are
/// best effort: a token that can't read them leaves them null or empty
/// rather than hiding the deployment history.
///
/// # Errors
/// Reports invalid references, provider failures and malformed history.
pub(super) async fn details(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
) -> Result<ProviderDetails, EngineError> {
    let (project, service) = split(reference)?;
    let value = graphql(
        http,
        token,
        "query HoistaHistory($input:DeploymentListInput!){deployments(input:$input,first:20){edges{node{id status createdAt statusUpdatedAt canRollback url staticUrl environmentId meta}}}}",
        json!({ "input": { "projectId": project, "serviceId": service } }),
    )
    .await?;
    let edges = value["data"]["deployments"]["edges"]
        .as_array()
        .ok_or_else(invalid)?;
    let text = |value: &Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(|text| text.chars().take(MAX_ERROR_CHARS).collect::<String>())
    };
    let mut environment = None;
    let mut deployments = Vec::with_capacity(edges.len());
    for edge in edges {
        let node = &edge["node"];
        if environment.is_none() {
            environment = node["environmentId"].as_str().map(str::to_owned);
        }
        let status = node["status"].as_str().ok_or_else(invalid)?.to_owned();
        let status_word = status.clone();
        deployments.push(ProviderDeployment {
            id: node["id"].as_str().ok_or_else(invalid)?.into(),
            state: match status.as_str() {
                "SUCCESS" | "SLEEPING" => ProviderDeployState::Succeeded,
                "FAILED" | "CRASHED" => ProviderDeployState::Failed,
                "REMOVED" | "REMOVING" | "SKIPPED" => ProviderDeployState::Stopped,
                _ => ProviderDeployState::InProgress,
            },
            status,
            created_at: node["createdAt"].as_str().ok_or_else(invalid)?.into(),
            url: text(&node["staticUrl"])
                .or_else(|| text(&node["url"]))
                .map(|host| {
                    if host.starts_with("https://") {
                        host
                    } else {
                        format!("https://{host}")
                    }
                }),
            commit: text(&node["meta"]["commitHash"]),
            message: text(&node["meta"]["commitMessage"]),
            branch: text(&node["meta"]["branch"]),
            repo: text(&node["meta"]["repo"]),
            // Railway moves statusUpdatedAt with every later status (sleeping,
            // crashed, replaced), so it's the build's end only for these.
            finished_at: if matches!(status_word.as_str(), "SUCCESS" | "FAILED") {
                text(&node["statusUpdatedAt"])
            } else {
                None
            },
            can_roll_back: node["canRollback"].as_bool(),
            author: text(&node["meta"]["commitAuthor"]),
            provider_page: node["id"].as_str().map(|id| {
                let environment = node["environmentId"]
                    .as_str()
                    .map(|env| format!("environmentId={env}&"))
                    .unwrap_or_default();
                format!(
                    "{}/service/{service}?{environment}id={id}",
                    project_page(project)
                )
            }),
        });
    }
    let mut details = ProviderDetails {
        deployments,
        variable_names: None,
        shared_variable_names: None,
        domains: vec![],
        references: vec![],
        sleeps: None,
        config: None,
        volumes: vec![],
        tcp_proxies: vec![],
        private_domain: None,
        addresses: vec![],
    };
    let Some(environment) = environment else {
        return Ok(details);
    };
    // Unrendered, a reference stays `${{Postgres.DATABASE_URL}}` instead of
    // the connection string it stands for. Older API versions may not accept
    // the flag; then only names are read.
    let variables = match graphql(
        http,
        token,
        "query HoistaVariables($project:String!,$env:String!,$service:String){variables(projectId:$project,environmentId:$env,serviceId:$service,unrendered:true)}",
        json!({ "project": project, "env": environment, "service": service }),
    )
    .await
    {
        Ok(value) if value["data"]["variables"].is_object() => Some((value, true)),
        _ => graphql(
            http,
            token,
            "query HoistaVariableNames($project:String!,$env:String!,$service:String){variables(projectId:$project,environmentId:$env,serviceId:$service)}",
            json!({ "project": project, "env": environment, "service": service }),
        )
        .await
        .ok()
        .map(|value| (value, false)),
    };
    // Service variables that stand for a shared one: (its name, the shared name).
    let mut shared_uses: Vec<(String, String)> = Vec::new();
    if let Some((value, unrendered)) = variables
        && let Some(map) = value["data"]["variables"].as_object()
    {
        let mut names: Vec<String> = map.keys().take(MAX_VARIABLES).cloned().collect();
        names.sort();
        details.variable_names = Some(names);
        if unrendered {
            for (name, value) in map.iter().take(MAX_VARIABLES) {
                if let Some(shared) = value.as_str().and_then(shared_name) {
                    shared_uses.push((name.clone(), shared));
                }
                if let Some(address) = value
                    .as_str()
                    .and_then(|value| VariableAddress::from_value(name, value))
                {
                    details.addresses.push(address);
                }
                for service in value.as_str().map(references).unwrap_or_default() {
                    details.references.push(ProviderReference {
                        variable: name.clone(),
                        service,
                    });
                }
            }
            details
                .references
                .sort_by(|a, b| a.variable.cmp(&b.variable));
        }
    }
    // The project's shared variables: names only.
    if let Ok(value) = graphql(
        http,
        token,
        "query HoistaSharedNames($project:String!,$env:String!){variables(projectId:$project,environmentId:$env)}",
        json!({ "project": project, "env": environment }),
    )
    .await
        && let Some(map) = value["data"]["variables"].as_object()
    {
        let mut names: Vec<String> = map.keys().take(MAX_VARIABLES).cloned().collect();
        names.sort();
        // A shared address this service uses through `${{shared.NAME}}`,
        // under the service's own variable name.
        for (variable, shared) in &shared_uses {
            if let Some(address) = map
                .get(shared)
                .and_then(Value::as_str)
                .and_then(|value| VariableAddress::from_value(variable, value))
            {
                details.addresses.push(address);
            }
        }
        details.shared_variable_names = Some(names);
    }
    // Settings, sleep, TCP proxies and disks in one request (Railway counts requests).
    if let Ok(value) = graphql(
        http,
        token,
        "query HoistaService($service:String!,$env:String!,$project:String!){serviceInstance(serviceId:$service,environmentId:$env){startCommand buildCommand rootDirectory cronSchedule healthcheckPath numReplicas region restartPolicyType sleepApplication source{repo image}} tcpProxies(environmentId:$env,serviceId:$service){id domain proxyPort applicationPort} project(id:$project){volumes{edges{node{id name volumeInstances{edges{node{id mountPath serviceId environmentId currentSizeMB sizeMB region}}}}}}}}",
        json!({ "service": service, "env": environment, "project": project }),
    )
    .await
    {
        let instance = &value["data"]["serviceInstance"];
        details.sleeps = instance["sleepApplication"].as_bool();
        if instance.is_object() {
            details.config = Some(ProviderConfig {
                repo: text(&instance["source"]["repo"]),
                image: text(&instance["source"]["image"]),
                start_command: text(&instance["startCommand"]),
                build_command: text(&instance["buildCommand"]),
                root_directory: text(&instance["rootDirectory"]),
                cron_schedule: text(&instance["cronSchedule"]),
                healthcheck_path: text(&instance["healthcheckPath"]),
                replicas: instance["numReplicas"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok()),
                region: text(&instance["region"]),
                restart_policy: text(&instance["restartPolicyType"]),
            });
        }
        details.tcp_proxies = value["data"]["tcpProxies"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_TCP_PROXIES)
            .filter_map(|proxy| {
                Some(ProviderTcpProxy {
                    id: proxy["id"].as_str()?.to_owned(),
                    domain: proxy["domain"].as_str()?.to_owned(),
                    proxy_port: u32::try_from(proxy["proxyPort"].as_u64()?).ok()?,
                    application_port: u32::try_from(proxy["applicationPort"].as_u64()?).ok()?,
                })
            })
            .collect();
        for edge in value["data"]["project"]["volumes"]["edges"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_VOLUMES)
        {
            let node = &edge["node"];
            let mounted = node["volumeInstances"]["edges"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|edge| &edge["node"])
                .find(|instance| {
                    instance["serviceId"].as_str() == Some(service)
                        && instance["environmentId"].as_str() == Some(environment.as_str())
                });
            if let (Some(instance), Some(id), Some(name)) =
                (mounted, node["id"].as_str(), node["name"].as_str())
            {
                details.volumes.push(ProviderVolume {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    mount_path: text(&instance["mountPath"]),
                    instance_id: text(&instance["id"]),
                    used_mb: mb(&instance["currentSizeMB"]),
                    size_mb: mb(&instance["sizeMB"]),
                    region: text(&instance["region"]),
                });
            }
        }
    }
    // With readiness first; an API without those fields still gives the names.
    let domains = match graphql(
        http,
        token,
        "query HoistaDomainStatus($project:String!,$env:String!,$service:String!){domains(projectId:$project,environmentId:$env,serviceId:$service){serviceDomains{domain targetPort} customDomains{domain targetPort status{certificateStatus certificateErrorMessage dnsRecords{fqdn recordType requiredValue currentValue status}}}}}",
        json!({ "project": project, "env": environment, "service": service }),
    )
    .await
    {
        Ok(value) if value["data"]["domains"].is_object() => Some(value),
        _ => graphql(
            http,
            token,
            "query HoistaDomains($project:String!,$env:String!,$service:String!){domains(projectId:$project,environmentId:$env,serviceId:$service){serviceDomains{domain} customDomains{domain}}}",
            json!({ "project": project, "env": environment, "service": service }),
        )
        .await
        .ok(),
    };
    if let Some(value) = domains {
        for (field, custom) in [("customDomains", true), ("serviceDomains", false)] {
            for item in value["data"]["domains"][field]
                .as_array()
                .into_iter()
                .flatten()
                .take(MAX_DOMAINS)
            {
                let Some(name) = item["domain"]
                    .as_str()
                    .filter(|name| name.len() <= MAX_HOSTNAME_CHARS)
                else {
                    continue;
                };
                let status = &item["status"];
                let records: Vec<ProviderDnsRecord> = status["dnsRecords"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|record| {
                        record["status"].as_str() != Some("DNS_RECORD_STATUS_PROPAGATED")
                    })
                    .filter_map(|record| {
                        Some(ProviderDnsRecord {
                            kind: record["recordType"]
                                .as_str()?
                                .trim_start_matches("DNS_RECORD_TYPE_")
                                .to_owned(),
                            name: record["fqdn"].as_str()?.to_owned(),
                            value: record["requiredValue"].as_str()?.to_owned(),
                        })
                    })
                    .take(MAX_DNS_RECORDS)
                    .collect();
                let certificate = status["certificateStatus"].as_str();
                let dns: Vec<ProviderDnsCheck> = status["dnsRecords"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|record| {
                        Some(ProviderDnsCheck {
                            kind: record["recordType"]
                                .as_str()?
                                .trim_start_matches("DNS_RECORD_TYPE_")
                                .to_owned(),
                            name: record["fqdn"].as_str()?.to_owned(),
                            value: record["requiredValue"].as_str()?.to_owned(),
                            current: record["currentValue"]
                                .as_str()
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned),
                            ok: record["status"].as_str() == Some("DNS_RECORD_STATUS_PROPAGATED"),
                        })
                    })
                    .take(MAX_DNS_RECORDS)
                    .collect();
                let certificate_words = certificate.map(|value| {
                    if value.contains("FAIL") {
                        let why = status["certificateErrorMessage"].as_str().filter(|text| {
                            !text.is_empty() && text.len() <= MAX_CERTIFICATE_ERROR_CHARS
                        });
                        why.map_or_else(|| "Failed".to_owned(), |why| format!("Failed: {why}"))
                    } else if value.contains("VALID") && !value.contains("VALIDATING") {
                        "Valid".to_owned()
                    } else {
                        "Issuing".to_owned()
                    }
                });
                details.domains.push(ProviderDomain {
                    port: item["targetPort"]
                        .as_u64()
                        .and_then(|port| u16::try_from(port).ok()),
                    certificate: certificate_words,
                    dns,
                    name: name.to_owned(),
                    custom,
                    state: if !custom {
                        Some(ProviderDomainState::Working)
                    } else if certificate.is_none() && status.is_null() {
                        None
                    } else if certificate.is_some_and(|value| value.contains("FAIL")) {
                        Some(ProviderDomainState::Failed)
                    } else if records.is_empty()
                        && certificate.is_some_and(|value| {
                            value.contains("VALID") && !value.contains("VALIDATING")
                        })
                    {
                        Some(ProviderDomainState::Working)
                    } else {
                        Some(ProviderDomainState::Pending)
                    },
                    records,
                });
            }
        }
    }
    // Private networking: the name other services here reach it by. Best
    // effort; a token or environment without it leaves it null.
    if let Ok(networks) = graphql(
        http,
        token,
        "query HoistaNetworks($env:String!){privateNetworks(environmentId:$env){publicId}}",
        json!({ "env": environment }),
    )
    .await
        && let Some(network) = networks["data"]["privateNetworks"][0]["publicId"].as_str()
        && let Ok(endpoint) = graphql(
            http,
            token,
            "query HoistaPrivateName($network:String!,$env:String!,$service:String!){privateNetworkEndpoint(privateNetworkId:$network,environmentId:$env,serviceId:$service){dnsName}}",
            json!({ "network": network, "env": environment, "service": service }),
        )
        .await
    {
        details.private_domain = text(&endpoint["data"]["privateNetworkEndpoint"]["dnsName"])
            .filter(|name| {
                name.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            })
            .map(|name| {
                if name.contains('.') {
                    name
                } else {
                    format!("{name}.railway.internal")
                }
            });
    }
    Ok(details)
}

/// The service's environment, from its newest deployment.
async fn environment(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
) -> Result<String, EngineError> {
    latest(http, token, reference)
        .await?
        .and_then(|latest| latest.environment_id)
        .ok_or(EngineError::Conflict(
            "This Railway service has no environment yet; deploy it once on Railway first.",
        ))
}

/// Railway's bare "Not Authorized", said so the person knows what to do:
/// the sign-in's role can't do this (deletes need a workspace Admin).
pub(super) fn explain_refusal(error: EngineError) -> EngineError {
    match error {
        EngineError::Provider { message, .. } if message.contains("Not Authorized") => {
            EngineError::Provider {
                evidence: None,
                code: "provider.token_scope".into(),
                message: "Railway says this sign-in isn't allowed to do that. Deleting services, volumes and projects needs Admin access to the workspace. Sign in to Railway again from Accounts and allow admin access (Railway gives only the role you have); Hoista still asks you before anything is deleted.".into(),
            }
        }
        other => other,
    }
}

/// Acts on a service the way its Railway page does: redeploy, restart, stop
/// or roll back a deployment, deploy a commit, change how it builds and runs,
/// connect a repository, sleep when idle, add or remove domains, volumes and
/// TCP proxies, or delete the service. Returns a sentence for the activity log.
///
/// # Errors
/// Reports invalid references, a service that has never deployed, invalid
/// settings, provider failures and refusals.
pub(super) async fn act(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    action: &ProviderAction<'_>,
) -> Result<String, EngineError> {
    act_inner(http, token, reference, action)
        .await
        .map_err(explain_refusal)
}

async fn act_inner(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    action: &ProviderAction<'_>,
) -> Result<String, EngineError> {
    let (project, service) = split(reference)?;
    let current = latest(http, token, reference)
        .await?
        .ok_or(EngineError::Conflict(
            "This Railway service has never deployed; deploy it once on Railway first.",
        ))?;
    let environment = current.environment_id.clone().ok_or(EngineError::Conflict(
        "This Railway service has no environment yet; deploy it once on Railway first.",
    ))?;
    let instance = |input: Value| {
        (
            "mutation HoistaServiceInstanceUpdate($serviceId:String!,$environmentId:String,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}",
            json!({ "serviceId": service, "environmentId": environment, "input": input }),
            "serviceInstanceUpdate",
        )
    };
    let (query, variables, field, done): (&str, Value, &str, String) = match action {
        ProviderAction::Redeploy => (
            "mutation HoistaRedeploy($id:String!){deploymentRedeploy(id:$id){id}}",
            json!({ "id": current.id }),
            "deploymentRedeploy",
            "Redeployed the current version".into(),
        ),
        ProviderAction::Restart => (
            "mutation HoistaRestart($id:String!){deploymentRestart(id:$id)}",
            json!({ "id": current.id }),
            "deploymentRestart",
            "Restarted".into(),
        ),
        ProviderAction::Stop => (
            "mutation HoistaStop($id:String!){deploymentStop(id:$id)}",
            json!({ "id": current.id }),
            "deploymentStop",
            "Stopped the running deployment".into(),
        ),
        ProviderAction::Rollback { deployment_id } => (
            "mutation HoistaRollback($id:String!){deploymentRollback(id:$id)}",
            json!({ "id": deployment_id }),
            "deploymentRollback",
            "Rolled back to an earlier deployment".into(),
        ),
        ProviderAction::DeployLatest => (
            "mutation HoistaDeployLatest($serviceId:String!,$environmentId:String!){serviceInstanceDeploy(serviceId:$serviceId,environmentId:$environmentId,latestCommit:true)}",
            json!({ "serviceId": service, "environmentId": environment }),
            "serviceInstanceDeploy",
            "Deployed the newest commit".into(),
        ),
        ProviderAction::DeployCommit { sha } => {
            if !COMMIT_ID_CHARS.contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(EngineError::Invalid("Give the commit as its hash."));
            }
            (
                "mutation HoistaDeployCommit($serviceId:String!,$environmentId:String!,$sha:String!){serviceInstanceDeploy(serviceId:$serviceId,environmentId:$environmentId,commitSha:$sha)}",
                json!({ "serviceId": service, "environmentId": environment, "sha": sha }),
                "serviceInstanceDeploy",
                format!("Deployed commit {}", &sha[..7]),
            )
        }
        ProviderAction::Sleep { on } => {
            let (query, variables, field) = instance(json!({ "sleepApplication": on }));
            (
                query,
                variables,
                field,
                if *on {
                    "Turned on sleep when idle".into()
                } else {
                    "Turned off sleep when idle".into()
                },
            )
        }
        ProviderAction::Configure(config) => {
            let (input, changed) = config_input(config)?;
            let (query, variables, field) = instance(input);
            (
                query,
                variables,
                field,
                format!("Changed {changed}; it applies on the next deploy"),
            )
        }
        ProviderAction::ConnectRepo { repo, branch } => {
            let valid = |text: &str| {
                !text.is_empty()
                    && text.len() <= MAX_REPO_CHARS
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
            };
            if repo.split('/').count() != 2 || !valid(repo) || !branch.is_none_or(valid) {
                return Err(EngineError::Invalid(
                    "Name the repository as owner/name, with an optional branch.",
                ));
            }
            (
                "mutation HoistaConnect($id:String!,$input:ServiceConnectInput!){serviceConnect(id:$id,input:$input){id}}",
                json!({ "id": service, "input": { "repo": repo, "branch": branch } }),
                "serviceConnect",
                format!("Connected {repo} as its source"),
            )
        }
        ProviderAction::AddDomain { domain, port } => match domain {
            Some(domain) => {
                if !valid_hostname(domain) {
                    return Err(EngineError::Invalid(
                        "Give the domain as a host name, such as api.example.com.",
                    ));
                }
                (
                    "mutation HoistaCustomDomain($input:CustomDomainCreateInput!){customDomainCreate(input:$input){id}}",
                    json!({
                        "input": {
                            "domain": domain,
                            "projectId": project,
                            "environmentId": environment,
                            "serviceId": service,
                            "targetPort": port,
                        },
                    }),
                    "customDomainCreate",
                    format!("Added {domain}; point its DNS at the record in Domains"),
                )
            }
            None => (
                "mutation HoistaServiceDomain($input:ServiceDomainCreateInput!){serviceDomainCreate(input:$input){id domain}}",
                json!({
                    "input": {
                        "environmentId": environment,
                        "serviceId": service,
                        "targetPort": port,
                    },
                }),
                "serviceDomainCreate",
                "Added a railway.app address".into(),
            ),
        },
        ProviderAction::UpdateDomain { domain, port } => {
            let (kind, id) = domain_id(http, token, project, service, &environment, domain).await?;
            if kind == "customDomains" {
                (
                    "mutation HoistaUpdateCustomDomain($environmentId:String!,$id:String!,$targetPort:Int){customDomainUpdate(environmentId:$environmentId,id:$id,targetPort:$targetPort)}",
                    json!({ "environmentId": environment, "id": id, "targetPort": port }),
                    "customDomainUpdate",
                    format!("{domain} now sends traffic to port {port}"),
                )
            } else {
                (
                    "mutation HoistaUpdateServiceDomain($input:ServiceDomainUpdateInput!){serviceDomainUpdate(input:$input)}",
                    json!({
                        "input": {
                            "serviceDomainId": id,
                            "environmentId": environment,
                            "serviceId": service,
                            "domain": domain,
                            "targetPort": port,
                        },
                    }),
                    "serviceDomainUpdate",
                    format!("{domain} now sends traffic to port {port}"),
                )
            }
        }
        ProviderAction::RenameDomain { domain, to } => {
            let (kind, id) = domain_id(http, token, project, service, &environment, domain).await?;
            let suffix = domain
                .split_once('.')
                .map(|(_, rest)| rest)
                .unwrap_or_default();
            let label = to.strip_suffix(&format!(".{suffix}")).unwrap_or(to);
            if kind != "serviceDomains"
                || suffix.is_empty()
                || label.is_empty()
                || label.len() > MAX_LABEL_CHARS
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || label.starts_with('-')
                || label.ends_with('-')
            {
                return Err(EngineError::Invalid(
                    "Only Railway's own address can be renamed: use lowercase letters, digits and dashes.",
                ));
            }
            let renamed = format!("{label}.{suffix}");
            (
                "mutation HoistaRenameServiceDomain($input:ServiceDomainUpdateInput!){serviceDomainUpdate(input:$input)}",
                json!({
                    "input": {
                        "serviceDomainId": id,
                        "environmentId": environment,
                        "serviceId": service,
                        "domain": renamed,
                    },
                }),
                "serviceDomainUpdate",
                format!("{domain} is now {renamed}"),
            )
        }
        ProviderAction::RetryCertificate { domain } => {
            let (kind, id) = domain_id(http, token, project, service, &environment, domain).await?;
            if kind != "customDomains" {
                return Err(EngineError::Invalid(
                    "Only your own domains have certificates to retry.",
                ));
            }
            (
                "mutation HoistaRetryCertificate($id:String!){customDomainIssueCertificate(id:$id)}",
                json!({ "id": id }),
                "customDomainIssueCertificate",
                format!("Asked Railway to issue {domain}'s certificate again"),
            )
        }
        ProviderAction::DisconnectSource => (
            "mutation HoistaDisconnect($id:String!){serviceDisconnect(id:$id){id}}",
            json!({ "id": service }),
            "serviceDisconnect",
            "Disconnected its repository or image".into(),
        ),
        ProviderAction::AddVolume { mount_path } => {
            if !mount_path.starts_with('/')
                || mount_path.len() > MAX_PATH_CHARS
                || mount_path.chars().any(char::is_control)
            {
                return Err(EngineError::Invalid(
                    "Give the mount path as an absolute path, such as /data.",
                ));
            }
            (
                "mutation HoistaVolume($input:VolumeCreateInput!){volumeCreate(input:$input){id}}",
                json!({
                    "input": {
                        "projectId": project,
                        "environmentId": environment,
                        "serviceId": service,
                        "mountPath": mount_path,
                    },
                }),
                "volumeCreate",
                format!("Attached a volume at {mount_path}"),
            )
        }
        ProviderAction::RemoveVolume { volume_id } => (
            "mutation HoistaRemoveVolume($id:String!){volumeDelete(volumeId:$id)}",
            json!({ "id": volume_id }),
            "volumeDelete",
            "Deleted the volume and its data".into(),
        ),
        ProviderAction::AddTcpProxy { port } => (
            "mutation HoistaTcpProxy($input:TCPProxyCreateInput!){tcpProxyCreate(input:$input){id}}",
            json!({
                "input": {
                    "applicationPort": port,
                    "environmentId": environment,
                    "serviceId": service,
                },
            }),
            "tcpProxyCreate",
            format!("Opened a public TCP endpoint to port {port}"),
        ),
        ProviderAction::RemoveTcpProxy { proxy_id } => (
            "mutation HoistaRemoveTcpProxy($id:String!){tcpProxyDelete(id:$id)}",
            json!({ "id": proxy_id }),
            "tcpProxyDelete",
            "Closed the public TCP endpoint".into(),
        ),
        ProviderAction::RenamePrivateHost { name } => {
            if name.is_empty()
                || name.len() > MAX_LABEL_CHARS
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(EngineError::Invalid(
                    "Use lowercase letters, digits and dashes for the private name.",
                ));
            }
            let networks = graphql(
                http,
                token,
                "query HoistaNetworks($env:String!){privateNetworks(environmentId:$env){publicId}}",
                json!({ "env": environment }),
            )
            .await?;
            let network = networks["data"]["privateNetworks"][0]["publicId"]
                .as_str()
                .ok_or(EngineError::Conflict(
                    "This environment has no private network yet.",
                ))?
                .to_owned();
            let endpoint = graphql(
                http,
                token,
                "query HoistaEndpoint($network:String!,$env:String!,$service:String!){privateNetworkEndpoint(privateNetworkId:$network,environmentId:$env,serviceId:$service){publicId}}",
                json!({ "network": network, "env": environment, "service": service }),
            )
            .await?;
            let id = endpoint["data"]["privateNetworkEndpoint"]["publicId"]
                .as_str()
                .ok_or(EngineError::Conflict(
                    "This service has no private address yet.",
                ))?
                .to_owned();
            (
                "mutation HoistaRename($id:String!,$dnsName:String!,$privateNetworkId:String!){privateNetworkEndpointRename(id:$id,dnsName:$dnsName,privateNetworkId:$privateNetworkId)}",
                json!({ "id": id, "dnsName": name, "privateNetworkId": network }),
                "privateNetworkEndpointRename",
                format!("Renamed its private address to {name}.railway.internal"),
            )
        }
        ProviderAction::RemoveService => (
            "mutation HoistaRemoveService($environmentId:String!,$id:String!){serviceDelete(environmentId:$environmentId,id:$id)}",
            json!({ "environmentId": environment, "id": service }),
            "serviceDelete",
            "Deleted the service at Railway".into(),
        ),
        ProviderAction::RemoveDomain { domain } => {
            let listed = graphql(
                http,
                token,
                "query HoistaDomainIds($project:String!,$env:String!,$service:String!){domains(projectId:$project,environmentId:$env,serviceId:$service){serviceDomains{id domain} customDomains{id domain}}}",
                json!({ "project": project, "env": environment, "service": service }),
            )
            .await?;
            let find = |kind: &str| {
                listed["data"]["domains"][kind]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|item| {
                        item["domain"]
                            .as_str()
                            .is_some_and(|name| name.eq_ignore_ascii_case(domain))
                    })
                    .and_then(|item| item["id"].as_str().map(str::to_owned))
            };
            if let Some(id) = find("customDomains") {
                (
                    "mutation HoistaRemoveCustomDomain($id:String!){customDomainDelete(id:$id)}",
                    json!({ "id": id }),
                    "customDomainDelete",
                    format!("Removed {domain}"),
                )
            } else if let Some(id) = find("serviceDomains") {
                (
                    "mutation HoistaRemoveServiceDomain($id:String!){serviceDomainDelete(id:$id)}",
                    json!({ "id": id }),
                    "serviceDomainDelete",
                    format!("Removed {domain}"),
                )
            } else {
                return Err(EngineError::NotFound);
            }
        }
    };
    let response = graphql(http, token, query, variables).await?;
    let answer = &response["data"][field];
    if answer == &Value::Bool(true) || answer["id"].is_string() {
        Ok(done)
    } else {
        Err(invalid())
    }
}

/// Which list a domain is in (`customDomains` or `serviceDomains`) and its id.
async fn domain_id(
    http: &ProviderHttp,
    token: &Secret,
    project: &str,
    service: &str,
    environment: &str,
    domain: &str,
) -> Result<(&'static str, String), EngineError> {
    let listed = graphql(
        http,
        token,
        "query HoistaDomainIds($project:String!,$env:String!,$service:String!){domains(projectId:$project,environmentId:$env,serviceId:$service){serviceDomains{id domain} customDomains{id domain}}}",
        json!({ "project": project, "env": environment, "service": service }),
    )
    .await?;
    for kind in ["customDomains", "serviceDomains"] {
        if let Some(id) = listed["data"]["domains"][kind]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| {
                item["domain"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(domain))
            })
            .and_then(|item| item["id"].as_str())
        {
            return Ok((kind, id.to_owned()));
        }
    }
    Err(EngineError::NotFound)
}

/// The services in a project, by id.
async fn project_services(
    http: &ProviderHttp,
    token: &Secret,
    project: &str,
) -> Result<Vec<String>, EngineError> {
    let value = graphql(
        http,
        token,
        "query HoistaProjectServices($id:String!){project(id:$id){services{edges{node{id}}}}}",
        json!({ "id": project }),
    )
    .await?;
    Ok(value["data"]["project"]["services"]["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|edge| edge["node"]["id"].as_str().map(str::to_owned))
        .collect())
}

/// Creates a service, a template (such as a database) or a bucket in the
/// project of an existing service. Returns the new services' references.
///
/// # Errors
/// Reports invalid input, templates needing values that weren't given, and
/// provider failures.
pub(super) async fn create(
    http: &ProviderHttp,
    token: &Secret,
    beside: &str,
    what: &ProviderCreate<'_>,
) -> Result<(Vec<String>, String), EngineError> {
    let (project, _) = split(beside)?;
    let environment = environment(http, token, beside).await?;
    create_in(http, token, project, &environment, what).await
}

/// Creates a service, template or bucket in a Railway project's environment.
async fn create_in(
    http: &ProviderHttp,
    token: &Secret,
    project: &str,
    environment: &str,
    what: &ProviderCreate<'_>,
) -> Result<(Vec<String>, String), EngineError> {
    let name_ok = |name: &str| {
        !name.is_empty()
            && name.len() <= MAX_LABEL_CHARS
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    match what {
        ProviderCreate::Service {
            name,
            repo,
            branch,
            image,
        } => {
            if !name_ok(name) || (repo.is_some() && image.is_some()) {
                return Err(EngineError::Invalid(
                    "Name the service with letters, digits and dashes, and give a repository or an image, not both.",
                ));
            }
            let source = match (repo, image) {
                (Some(repo), None) => json!({ "repo": repo }),
                (None, Some(image)) => json!({ "image": image }),
                _ => Value::Null,
            };
            let value = graphql(
                http,
                token,
                "mutation HoistaCreateService($input:ServiceCreateInput!){serviceCreate(input:$input){id name}}",
                json!({
                    "input": {
                        "name": name,
                        "projectId": project,
                        "environmentId": environment,
                        "source": source,
                        "branch": branch,
                    },
                }),
            )
            .await?;
            let id = value["data"]["serviceCreate"]["id"]
                .as_str()
                .ok_or_else(invalid)?;
            Ok((
                vec![format!("{project}/{id}")],
                format!("Created the service {name} at Railway"),
            ))
        }
        ProviderCreate::Template { code, variables } => {
            if !name_ok(code) {
                return Err(EngineError::Invalid(
                    "Give the template's code, such as postgres.",
                ));
            }
            let detail = graphql(
                http,
                token,
                "query HoistaTemplate($code:String!){template(code:$code){id name serializedConfig}}",
                json!({ "code": code }),
            )
            .await?;
            let template = &detail["data"]["template"];
            let id = template["id"]
                .as_str()
                .ok_or(EngineError::NotFound)?
                .to_owned();
            let label = template["name"].as_str().unwrap_or(code).to_owned();
            let mut config = template["serializedConfig"].clone();
            let mut missing = Vec::new();
            if let Some(services) = config["services"].as_object_mut() {
                for service in services.values_mut() {
                    let Some(vars) = service["variables"].as_object_mut() else {
                        continue;
                    };
                    for (key, variable) in vars.iter_mut() {
                        let Some(variable) = variable.as_object_mut() else {
                            continue;
                        };
                        let default = variable
                            .get("defaultValue")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                            .map(str::to_owned);
                        let optional = variable
                            .get("isOptional")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        match variables.get(key).cloned().or(default) {
                            Some(value) => {
                                variable.insert("value".into(), Value::String(value));
                            }
                            None if !optional => missing.push(key.clone()),
                            None => {}
                        }
                    }
                }
            }
            if !missing.is_empty() {
                missing.sort();
                missing.dedup();
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "template.variables_required".into(),
                    message: format!(
                        "The {label} template needs values for: {}. Pass them as variables.",
                        missing.join(", ")
                    ),
                });
            }
            let before = project_services(http, token, project).await?;
            graphql(
                http,
                token,
                "mutation HoistaTemplateDeploy($input:TemplateDeployV2Input!){templateDeployV2(input:$input){projectId workflowId}}",
                json!({
                    "input": {
                        "projectId": project,
                        "environmentId": environment,
                        "templateId": id,
                        "serializedConfig": config,
                    },
                }),
            )
            .await?;
            // Railway creates the services in the background; wait briefly so
            // their references can be returned.
            let mut created = Vec::new();
            for _ in 0..10 {
                let now = project_services(http, token, project).await?;
                created = now
                    .into_iter()
                    .filter(|id| !before.contains(id))
                    .map(|id| format!("{project}/{id}"))
                    .collect();
                if !created.is_empty() {
                    break;
                }

                #[cfg(not(test))]
                tokio::time::sleep(TEMPLATE_POLL).await;
            }
            Ok((created, format!("Added {label} from Railway's templates")))
        }
        ProviderCreate::Bucket { name } => {
            if !name_ok(name) {
                return Err(EngineError::Invalid(
                    "Name the bucket with letters, digits and dashes.",
                ));
            }
            graphql(
                http,
                token,
                "mutation HoistaBucket($input:BucketCreateInput!){bucketCreate(input:$input){id name}}",
                json!({ "input": { "projectId": project, "environmentId": environment, "name": name } }),
            )
            .await?;
            Ok((vec![], format!("Created the bucket {name} at Railway")))
        }
    }
}

/// Railway's input for the settings given, and a few words naming them.
fn config_input(config: &ServiceConfig) -> Result<(Value, String), EngineError> {
    let mut input = serde_json::Map::new();
    let mut names = Vec::new();
    let mut text = |key: &str, label: &str, value: &Option<String>| -> Result<(), EngineError> {
        if let Some(value) = value {
            if value.len() > MAX_SETTING_BYTES || value.chars().any(|c| c.is_control()) {
                return Err(EngineError::Invalid(
                    "Settings are single lines of up to 1,024 characters.",
                ));
            }
            input.insert(key.into(), Value::String(value.clone()));
            names.push(label.to_owned());
        }
        Ok(())
    };
    text("startCommand", "the start command", &config.start_command)?;
    text("buildCommand", "the build command", &config.build_command)?;
    text("rootDirectory", "the root folder", &config.root_directory)?;
    text("cronSchedule", "the schedule", &config.cron_schedule)?;
    text(
        "healthcheckPath",
        "the health check",
        &config.healthcheck_path,
    )?;
    text("region", "the region", &config.region)?;
    if let Some(replicas) = config.replicas {
        if !REPLICAS.contains(&replicas) {
            return Err(EngineError::Invalid("Run 1 to 20 copies."));
        }
        input.insert("numReplicas".into(), json!(replicas));
        names.push("the number of copies".into());
    }
    if let Some(policy) = config.restart_policy {
        input.insert(
            "restartPolicyType".into(),
            json!(match policy {
                RestartPolicy::OnFailure => "ON_FAILURE",
                RestartPolicy::Always => "ALWAYS",
                RestartPolicy::Never => "NEVER",
            }),
        );
        names.push("the restart policy".into());
    }
    if names.is_empty() {
        return Err(EngineError::Invalid("Say which setting to change."));
    }
    let changed = match names.as_slice() {
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => String::new(),
    };
    Ok((Value::Object(input), changed))
}

/// Largest compressed folder Hoista sends to Railway in one deploy.
const UPLOAD_LIMIT: usize = 200 * 1024 * 1024;
/// Large folders take longer than ordinary API calls.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Creates a new Railway project the way its "Deploy from GitHub repo" does:
/// a project called `name` in the workspace, one service deploying the
/// repository (Railway starts the first build itself), and a railway.app
/// address. Returns the service's reference and a sentence for the activity log.
///
/// # Errors
/// Reports an invalid name or repository, a missing workspace and provider failures.
pub(super) async fn new_project(
    http: &ProviderHttp,
    token: &Secret,
    workspace: &str,
    name: &str,
    repo: &str,
    branch: Option<&str>,
) -> Result<(Vec<String>, String), EngineError> {
    let valid_repo = repo.split_once('/').is_some_and(|(owner, name)| {
        !owner.is_empty()
            && !name.is_empty()
            && !name.contains('/')
            && repo
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
    });
    if !valid_repo {
        return Err(EngineError::Invalid(
            "Give the GitHub repository as owner/name.",
        ));
    }
    if workspace.is_empty() {
        return Err(EngineError::Conflict(
            "This Railway account has no workspace Hoista can create projects in. Connect Railway again from Accounts.",
        ));
    }
    let project = graphql(
        http,
        token,
        "mutation HoistaNewProject($input:ProjectCreateInput!){projectCreate(input:$input){id baseEnvironmentId}}",
        json!({ "input": { "name": name, "workspaceId": workspace } }),
    )
    .await
    .map_err(explain_refusal)?;
    let project_id = project["data"]["projectCreate"]["id"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    let environment = project["data"]["projectCreate"]["baseEnvironmentId"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    let service_name: String = repo
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .trim_end_matches(".git")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .take(MAX_LABEL_CHARS)
        .collect();
    let service = graphql(
        http,
        token,
        "mutation HoistaNewService($input:ServiceCreateInput!){serviceCreate(input:$input){id name}}",
        json!({
            "input": {
                "name": service_name,
                "projectId": project_id,
                "environmentId": environment,
                "source": {"repo": repo},
                "branch": branch,
            },
        }),
    )
    .await?;
    let service_id = service["data"]["serviceCreate"]["id"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    let address = graphql(
        http,
        token,
        "mutation HoistaNewDomain($input:ServiceDomainCreateInput!){serviceDomainCreate(input:$input){id domain}}",
        json!({ "input": { "environmentId": environment, "serviceId": service_id } }),
    )
    .await
    .ok()
    .and_then(|value| {
        value["data"]["serviceDomainCreate"]["domain"]
            .as_str()
            .map(str::to_owned)
    });
    let done = match address {
        Some(domain) => format!(
            "Created the Railway project {name} with {service_name} from {repo}, at {domain}"
        ),
        None => format!("Created the Railway project {name} with {service_name} from {repo}"),
    };
    Ok((vec![format!("{project_id}/{service_id}")], done))
}

/// Makes something for a Hoista project at Railway: next to `beside` (a
/// service already in the project, so they share a Railway project), or in
/// a new Railway project named after the Hoista project.
pub(super) async fn add(
    http: &ProviderHttp,
    token: &Secret,
    workspace: &str,
    project_name: &str,
    beside: Option<&str>,
    what: &crate::application::ports::providers::ProviderNew<'_>,
) -> Result<(Vec<String>, String), EngineError> {
    use crate::application::ports::providers::ProviderNew;
    let empty = std::collections::BTreeMap::new();
    let create = match *what {
        ProviderNew::Repo {
            name, repo, branch, ..
        } => ProviderCreate::Service {
            name,
            repo: Some(repo),
            branch,
            image: None,
        },
        ProviderNew::Image {
            name,
            image,
            gpu: None,
            cpu: None,
            args: None,
            ..
        } => ProviderCreate::Service {
            name,
            repo: None,
            branch: None,
            image: Some(image),
        },
        ProviderNew::Database {
            engine,
            region: None,
            ..
        } => {
            if !matches!(engine, "postgres" | "mysql" | "redis" | "mongo") {
                return Err(EngineError::Invalid(
                    "Railway makes postgres, mysql, redis or mongo databases.",
                ));
            }
            ProviderCreate::Template {
                code: engine,
                variables: &empty,
            }
        }
        ProviderNew::Database { .. } => {
            return Err(EngineError::Invalid(
                "Railway puts a database in the project's own region: leave out region.",
            ));
        }
        ProviderNew::Bucket { name } => ProviderCreate::Bucket { name },
        ProviderNew::Image { .. } => {
            return Err(EngineError::Invalid(
                "Railway runs an image as it is: leave out gpu and args.",
            ));
        }
        _ => return Err(crate::application::ports::providers::unsupported()),
    };
    if let ProviderNew::Repo {
        runs_as: Some(kind),
        ..
    } = *what
        && !matches!(kind, "web" | "worker" | "cron")
    {
        return Err(EngineError::Invalid(
            "On Railway a repository runs as web, worker or cron.",
        ));
    }
    if let ProviderNew::Repo {
        runs_as: Some("cron"),
        schedule: None,
        ..
    } = *what
    {
        return Err(EngineError::Invalid(
            "Give schedule, a five-field UTC cron expression, for a cron job.",
        ));
    }
    let (project, environment, made_project) = match beside {
        Some(beside) => {
            let (project, _) = split(beside)?;
            (
                project.to_owned(),
                environment(http, token, beside).await?,
                false,
            )
        }
        None => {
            if workspace.is_empty() {
                return Err(EngineError::Conflict(
                    "This Railway account has no workspace Hoista can create projects in. Connect Railway again from Accounts.",
                ));
            }
            let made = graphql(
                http,
                token,
                "mutation HoistaNewProject($input:ProjectCreateInput!){projectCreate(input:$input){id baseEnvironmentId}}",
                json!({ "input": { "name": project_name, "workspaceId": workspace } }),
            )
            .await
            .map_err(explain_refusal)?;
            let created = &made["data"]["projectCreate"];
            (
                created["id"].as_str().ok_or_else(invalid)?.to_owned(),
                created["baseEnvironmentId"]
                    .as_str()
                    .ok_or_else(invalid)?
                    .to_owned(),
                true,
            )
        }
    };
    let (references, mut done) = create_in(http, token, &project, &environment, &create).await?;
    // Code from a repository: a public address for web, a schedule for cron.
    if let (
        ProviderNew::Repo {
            runs_as, schedule, ..
        },
        Some(reference),
    ) = (*what, references.first())
    {
        let (_, service) = split(reference)?;
        match runs_as.unwrap_or("web") {
            "cron" => {
                graphql(
                    http,
                    token,
                    "mutation HoistaCron($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}",
                    json!({
                        "serviceId": service,
                        "environmentId": environment,
                        "input": {"cronSchedule": schedule},
                    }),
                )
                .await?;
                done = format!(
                    "{done}, running on the schedule {}",
                    schedule.unwrap_or_default()
                );
            }
            "web" => {
                if let Some(domain) = graphql(
                    http,
                    token,
                    "mutation HoistaNewDomain($input:ServiceDomainCreateInput!){serviceDomainCreate(input:$input){id domain}}",
                    json!({ "input": { "environmentId": environment, "serviceId": service } }),
                )
                .await
                .ok()
                .and_then(|value| {
                    value["data"]["serviceDomainCreate"]["domain"]
                        .as_str()
                        .map(str::to_owned)
                }) {
                    done = format!("{done}, at {domain}");
                }
            }
            _ => {}
        }
    }
    if made_project {
        done = format!("{done}, in the new Railway project {project_name}");
    }
    Ok((references, done))
}

/// Hoista itself on Railway: a project of its own with one service built
/// from `repo` (its Dockerfile), the given variables set before the first
/// build, a volume at `/data` and a public address. Returns the reference
/// and the address.
///
/// # Errors
/// Reports a bad repository, a missing workspace and provider failures.
pub(super) async fn new_hoista_server(
    http: &ProviderHttp,
    token: &Secret,
    workspace: &str,
    source: &crate::application::ports::providers::ServerSource,
    variables: &std::collections::BTreeMap<String, Secret>,
) -> Result<(String, String), EngineError> {
    use crate::application::ports::providers::ServerSource;
    if workspace.is_empty() {
        return Err(EngineError::Conflict(
            "This Railway account has no workspace Hoista can create projects in. Connect Railway again from Accounts.",
        ));
    }
    let (source, branch) = match source {
        // Pinned by digest, so a redeploy runs exactly this image.
        ServerSource::Image(image) if crate::is_pinned_image(image) => {
            (json!({ "image": image }), None)
        }
        ServerSource::Image(_) => {
            return Err(EngineError::Invalid(
                "The server image must be pinned by its sha256 digest.",
            ));
        }
        ServerSource::Repo { repo, branch } => {
            if !repo.split_once('/').is_some_and(|(owner, name)| {
                !owner.is_empty()
                    && !name.is_empty()
                    && !name.contains('/')
                    && repo
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_./".contains(&b))
            }) {
                return Err(EngineError::Invalid(
                    "Give the GitHub repository as owner/name.",
                ));
            }
            (json!({ "repo": repo }), branch.clone())
        }
    };
    let from_repo = branch.is_some() || source.get("repo").is_some();
    let project = graphql(
        http,
        token,
        "mutation HoistaServerProject($input:ProjectCreateInput!){projectCreate(input:$input){id baseEnvironmentId}}",
        json!({ "input": { "name": "hoista", "workspaceId": workspace } }),
    )
    .await
    .map_err(explain_refusal)?;
    let project_id = project["data"]["projectCreate"]["id"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    let environment = project["data"]["projectCreate"]["baseEnvironmentId"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    let mut values = serde_json::Map::new();
    for (name, value) in variables {
        values.insert(name.clone(), json!(value.expose()));
    }
    // Railway fills in the address it generates below.
    values.insert(
        "HOISTA_PUBLIC_URL".into(),
        json!("https://${{RAILWAY_PUBLIC_DOMAIN}}"),
    );
    if from_repo {
        values.insert(
            "RAILWAY_DOCKERFILE_PATH".into(),
            json!("deploy/server/Dockerfile"),
        );
    }
    let service = graphql(
        http,
        token,
        "mutation HoistaServerService($input:ServiceCreateInput!){serviceCreate(input:$input){id}}",
        json!({
            "input": {
                "name": "hoista",
                "projectId": project_id,
                "environmentId": environment,
                "source": source,
                "branch": branch,
                "variables": values,
            },
        }),
    )
    .await?;
    drop(values);
    let service_id = service["data"]["serviceCreate"]["id"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    graphql(
        http,
        token,
        "mutation HoistaServerVolume($input:VolumeCreateInput!){volumeCreate(input:$input){id}}",
        json!({
            "input": {
                "projectId": project_id,
                "environmentId": environment,
                "serviceId": service_id,
                "mountPath": "/data",
            },
        }),
    )
    .await?;
    let address = graphql(
        http,
        token,
        "mutation HoistaServerDomain($input:ServiceDomainCreateInput!){serviceDomainCreate(input:$input){domain}}",
        json!({ "input": { "environmentId": environment, "serviceId": service_id } }),
    )
    .await?;
    let domain = address["data"]["serviceDomainCreate"]["domain"]
        .as_str()
        .ok_or_else(invalid)?
        .to_owned();
    Ok((format!("{project_id}/{service_id}"), domain))
}

/// Seals `HOISTA_SEALED_KEY` on a Hoista server's service, so Railway never
/// shows it again in its dashboard, API or CLI (it still gives it to the
/// service). Railway seals through a staged environment change, then
/// commits it. Returns whether Railway now reports the key as sealed.
///
/// # Errors
/// Reports provider failures.
pub(super) async fn seal_server_key(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
) -> Result<bool, EngineError> {
    let (project, service) = split(reference)?;
    let environment = environment(http, token, reference).await?;
    graphql(
        http,
        token,
        "mutation HoistaSealKey($environmentId:String!,$input:EnvironmentConfig!){environmentStageChanges(environmentId:$environmentId,input:$input,merge:true){id}}",
        json!({
            "environmentId": environment,
            "input": {
                         "services": {
                                         service: {
                                                      "variables": {
                                                                       "HOISTA_SEALED_KEY": {
                                                                                                "isSealed": true,
                                                                                            },
                                                                   },
                                                  },
                                     },
                     },
        }),
    )
    .await?;
    graphql(
        http,
        token,
        "mutation HoistaSealKeyCommit($environmentId:String!){environmentPatchCommitStaged(environmentId:$environmentId,message:\"Seal Hoista's key\")}",
        json!({ "environmentId": environment }),
    )
    .await?;
    sealed(http, token, project, &environment, service).await
}

/// Whether Railway reports `HOISTA_SEALED_KEY` as sealed: it lists a sealed
/// variable's name with no value.
async fn sealed(
    http: &ProviderHttp,
    token: &Secret,
    project: &str,
    environment: &str,
    service: &str,
) -> Result<bool, EngineError> {
    let answer = graphql(
        http,
        token,
        "query HoistaKeySealed($projectId:String!,$environmentId:String!,$serviceId:String){variables(projectId:$projectId,environmentId:$environmentId,serviceId:$serviceId,unrendered:true)}",
        json!({ "projectId": project, "environmentId": environment, "serviceId": service }),
    )
    .await?;
    let variables = &answer["data"]["variables"];
    Ok(variables.as_object().is_some_and(|names| {
        names
            .get("HOISTA_SEALED_KEY")
            .is_some_and(serde_json::Value::is_null)
    }))
}

/// Packs a folder the way `railway up` does (respecting `.gitignore` and
/// `.railwayignore`, leaving out `.git` and `node_modules`), uploads it to the
/// service and starts a deployment. Returns a sentence for the activity log.
///
/// # Errors
/// Reports unreadable or oversized folders, invalid references and provider failures.
pub(super) async fn upload(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    folder: &std::path::Path,
    message: Option<&str>,
) -> Result<String, EngineError> {
    let (project, service) = split(reference)?;
    let environment = environment(http, token, reference).await?;
    let root = folder.to_path_buf();
    let tarball = tokio::task::spawn_blocking(move || pack(&root))
        .await
        .map_err(|_| EngineError::Store)??;
    // As `railway up` does: `POST /project/{p}/environment/{e}/up`. Never
    // retried, since a repeat would start a second deployment.
    let mut query = vec![("serviceId", service)];
    if let Some(message) = message {
        query.push(("message", message));
    }
    let url = http.at(
        Provider::Railway,
        BACKBOARD,
        &["project", project, "environment", &environment, "up"],
        &query,
    )?;
    let request = http
        .signed(Method::POST, Provider::Railway, url, token)?
        .header("Content-Type", "application/gzip")
        .timeout(UPLOAD_TIMEOUT)
        .body(tarball);
    let answer = http.send_request(request, false).await?;
    let deployment = answer["deploymentId"].as_str().ok_or_else(invalid)?;
    Ok(format!(
        "Uploaded the folder and started deployment {}",
        deployment.chars().take(8).collect::<String>()
    ))
}

/// The sorted file names [`pack`] puts in the tarball, for tests.
#[cfg(test)]
pub(super) fn pack_for_test(root: &std::path::Path) -> Result<Vec<String>, EngineError> {
    use std::io::Read;
    let bytes = pack(root)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes.as_slice()));
    let mut names = Vec::new();
    for entry in archive.entries().map_err(|_| EngineError::Store)? {
        let mut entry = entry.map_err(|_| EngineError::Store)?;
        let mut sink = Vec::new();
        entry
            .read_to_end(&mut sink)
            .map_err(|_| EngineError::Store)?;
        names.push(
            entry
                .path()
                .map_err(|_| EngineError::Store)?
                .to_string_lossy()
                .into_owned(),
        );
    }
    names.sort();
    Ok(names)
}

fn pack(root: &std::path::Path) -> Result<Vec<u8>, EngineError> {
    let unreadable = || EngineError::Invalid("Hoista couldn't read that folder.");
    if !root.is_dir() {
        return Err(EngineError::Invalid(
            "That folder doesn't exist on this computer.",
        ));
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut archive = tar::Builder::new(&mut encoder);
        archive.follow_symlinks(false);
        let mut walk = ignore::WalkBuilder::new(root);
        walk.add_custom_ignore_filename(".railwayignore")
            .hidden(false)
            .follow_links(false)
            .require_git(false);
        for entry in walk.build() {
            let entry = entry.map_err(|_| unreadable())?;
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(|_| unreadable())?;
            if relative.as_os_str().is_empty()
                || relative.components().any(|resource| {
                    matches!(resource.as_os_str().to_str(), Some(".git" | "node_modules"))
                })
            {
                continue;
            }
            if entry.file_type().is_some_and(|kind| kind.is_file()) {
                archive
                    .append_path_with_name(path, std::path::Path::new(".").join(relative))
                    .map_err(|_| unreadable())?;
                if archive.get_ref().get_ref().len() > UPLOAD_LIMIT {
                    return Err(EngineError::Invalid(
                        "That folder is over 200 MB compressed. Add a .railwayignore for build output and large files.",
                    ));
                }
            }
        }
        archive.finish().map_err(|_| unreadable())?;
    }
    encoder.finish().map_err(|_| unreadable())
}

/// Sets a service variable. Railway redeploys the service to apply it.
///
/// # Errors
/// Reports invalid references, provider failures and refusals.
pub(super) async fn set_variable(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    name: &str,
    value: &Secret,
) -> Result<(), EngineError> {
    let (project, service) = split(reference)?;
    let environment = environment(http, token, reference).await?;
    let response = graphql(
        http,
        token,
        "mutation HoistaSetVariable($input:VariableUpsertInput!){variableUpsert(input:$input)}",
        json!({
            "input": {
                "projectId": project,
                "environmentId": environment,
                "serviceId": service,
                "name": name,
                "value": value.expose(),
            },
        }),
    )
    .await?;
    if response["data"]["variableUpsert"] == true {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Sets many variables in one change, on the service or (`shared`) on the
/// project's shared variables. `redeploy` false asks Railway to hold off.
///
/// # Errors
/// Reports invalid references, provider failures and refusals.
pub(super) async fn set_variables(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    values: &[(String, Secret)],
    shared: bool,
    redeploy: bool,
) -> Result<(), EngineError> {
    let (project, service) = split(reference)?;
    let environment = environment(http, token, reference).await?;
    let variables: serde_json::Map<String, Value> = values
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.expose().to_owned())))
        .collect();
    let mut input = json!({
        "projectId": project,
        "environmentId": environment,
        "variables": variables,
        "replace": false,
        "skipDeploys": !redeploy,
    });
    if !shared {
        input["serviceId"] = Value::String(service.to_owned());
    }
    let response = graphql(
        http,
        token,
        "mutation HoistaSetVariables($input:VariableCollectionUpsertInput!){variableCollectionUpsert(input:$input)}",
        json!({ "input": input }),
    )
    .await?;
    if response["data"]["variableCollectionUpsert"] == true {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Deletes one of the project's shared variables.
///
/// # Errors
/// Reports invalid references, provider failures and refusals.
pub(super) async fn delete_shared_variable(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    name: &str,
) -> Result<(), EngineError> {
    let (project, _) = split(reference)?;
    let environment = environment(http, token, reference).await?;
    let response = graphql(
        http,
        token,
        "mutation HoistaDeleteShared($input:VariableDeleteInput!){variableDelete(input:$input)}",
        json!({ "input": { "projectId": project, "environmentId": environment, "name": name } }),
    )
    .await?;
    if response["data"]["variableDelete"] == true {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Deletes a service variable. Railway redeploys the service to apply it.
///
/// # Errors
/// Reports invalid references, provider failures and refusals.
pub(super) async fn delete_variable(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    name: &str,
) -> Result<(), EngineError> {
    let (project, service) = split(reference)?;
    let environment = environment(http, token, reference).await?;
    let response = graphql(
        http,
        token,
        "mutation HoistaDeleteVariable($input:VariableDeleteInput!){variableDelete(input:$input)}",
        json!({
            "input": {
                "projectId": project,
                "environmentId": environment,
                "serviceId": service,
                "name": name,
            },
        }),
    )
    .await?;
    if response["data"]["variableDelete"] == true {
        Ok(())
    } else {
        Err(invalid())
    }
}

async fn deployment_logs(
    http: &ProviderHttp,
    token: &Secret,
    deployment: &str,
    start: Option<&str>,
    end: Option<&str>,
    filter: &str,
    limit: usize,
) -> Result<Vec<Value>, EngineError> {
    let value = graphql(
        http,
        token,
        "query HoistaLogs($id:String!,$limit:Int,$start:DateTime,$end:DateTime,$filter:String){deploymentLogs(deploymentId:$id,limit:$limit,startDate:$start,endDate:$end,filter:$filter){timestamp message severity}}",
        json!({
            "id": deployment,
            "limit": limit,
            "start": start,
            "end": end,
            "filter": if filter.is_empty() { None } else { Some(filter) },
        }),
    )
    .await?;
    value["data"]["deploymentLogs"]
        .as_array()
        .cloned()
        .ok_or_else(invalid)
}

fn entry(row: &Value, scanner: &LeakScanner, token: &Secret) -> Result<LogEntry, EngineError> {
    let text = row["message"].as_str().ok_or_else(invalid)?;
    let level = match row["severity"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("debug") => LogLevel::Debug,
        Some("warn" | "warning") => LogLevel::Warn,
        // Railway marks everything written to stderr as an error, and many
        // programs (Postgres among them) log routine lines there. Trust the
        // level the line states about itself when it states one.
        Some("error" | "err" | "fatal") => stated_level(text).unwrap_or(LogLevel::Error),
        _ => LogLevel::Info,
    };
    let mut message = scanner.redact(text, std::slice::from_ref(token));
    if message.len() > MAX_MESSAGE {
        let mut cut = MAX_MESSAGE;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        message.truncate(cut);
    }
    Ok(LogEntry {
        timestamp: row["timestamp"].as_str().map(str::to_owned),
        observed_at: chrono::Utc::now().to_rfc3339(),
        level,
        source: "deployment".into(),
        message,
    })
}

/// One deployment's build log then its runtime log, oldest first, each at
/// most 200 lines, redacted. The build stream is labelled `build`.
///
/// # Errors
/// Reports provider failures and malformed responses.
pub(super) async fn deployment_lines(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    deployment: &str,
) -> Result<Vec<LogEntry>, EngineError> {
    split(reference)?;
    if deployment.is_empty() || deployment.len() > MAX_ID_CHARS {
        return Err(EngineError::Invalid("That is not a Railway deployment id."));
    }
    let scanner = LeakScanner::new().map_err(|_| invalid())?;
    let build = graphql(
        http,
        token,
        "query HoistaBuildLogs($id:String!,$limit:Int){buildLogs(deploymentId:$id,limit:$limit){timestamp message severity}}",
        json!({ "id": deployment, "limit": 200 }),
    )
    .await?;
    let mut lines = Vec::new();
    for row in build["data"]["buildLogs"].as_array().ok_or_else(invalid)? {
        let mut line = entry(row, &scanner, token)?;
        line.source = "build".into();
        lines.push(line);
    }
    for row in deployment_logs(http, token, deployment, None, None, "", 200).await? {
        lines.push(entry(&row, &scanner, token)?);
    }
    Ok(lines)
}

/// `NAME` when the whole value is `${{shared.NAME}}`.
fn shared_name(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix("${{")?.strip_suffix("}}")?.trim();
    let name = inner.strip_prefix("shared.")?.trim();
    (!name.is_empty() && name.len() <= MAX_VARIABLE_CHARS).then(|| name.to_owned())
}

/// Service names a Railway variable refers to: `${{Postgres.DATABASE_URL}}`
/// refers to `Postgres`. Shared (`shared.`) and environment references are
/// not services and are skipped.
fn references(value: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find("${{") {
        rest = &rest[start + 3..];
        let Some(end) = rest.find("}}") else { break };
        let inner = rest[..end].trim();
        rest = &rest[end + 2..];
        if let Some((service, _)) = inner.split_once('.') {
            let service = service.trim();
            if !service.is_empty()
                && !matches!(service, "shared" | "RAILWAY")
                && service.len() <= MAX_ID_CHARS
                && !found.iter().any(|known| known == service)
            {
                found.push(service.to_owned());
            }
        }
    }
    found
}

/// The level a log line names for itself, such as Postgres's `LOG:` or a
/// logger's `level=info` / `[WARN]`, when it names one.
fn stated_level(message: &str) -> Option<LogLevel> {
    let upper = message.to_ascii_uppercase();
    let named = |words: &[&str]| {
        words.iter().any(|word| {
            upper.contains(&format!(" {word}:"))
                || upper.starts_with(&format!("{word}:"))
                || upper.contains(&format!("[{word}]"))
                || upper.contains(&format!("LEVEL={word}"))
                || upper.contains(&format!("\"LEVEL\":\"{word}\""))
        })
    };
    if named(&["ERROR", "FATAL", "PANIC", "ERR"]) {
        Some(LogLevel::Error)
    } else if named(&["WARNING", "WARN"]) {
        Some(LogLevel::Warn)
    } else if named(&["DEBUG", "TRACE"]) {
        Some(LogLevel::Debug)
    } else if named(&["LOG", "INFO", "NOTICE", "STATEMENT", "DETAIL", "HINT"]) {
        Some(LogLevel::Info)
    } else {
        None
    }
}

/// Up to 100 runtime log lines of the latest deployment in the query's range.
///
/// # Errors
/// Reports invalid queries, provider failures and malformed responses.
pub(super) async fn log_history(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    query: &LogHistoryQuery,
) -> Result<LogHistory, EngineError> {
    query.milliseconds().map_err(EngineError::Invalid)?;
    let empty = LogHistory {
        items: vec![],
        next_cursor: None,
        query_sampling_level: None,
        coverage_complete: false,
    };
    let Some(deployment) = latest(http, token, reference).await? else {
        return Ok(empty);
    };
    let scanner = LeakScanner::new().map_err(|_| invalid())?;
    // Railway returns lines forward from the start of the range, so a busy
    // range would show its oldest lines. Page forward, up to a limit, and keep the
    // newest 100, which is what a log viewer opens on.
    let mut rows: Vec<Value> = Vec::new();
    let mut from = query.from.clone();
    for _ in 0..HISTORY_PAGES {
        let page = deployment_logs(
            http,
            token,
            &deployment.id,
            Some(&from),
            Some(&query.to),
            &query.query,
            HISTORY_PAGE,
        )
        .await?;
        let full = page.len() >= HISTORY_PAGE;
        let last = page
            .iter()
            .filter_map(|row| row["timestamp"].as_str())
            .max()
            .map(str::to_owned);
        rows.extend(page);
        if rows.len() > 100 {
            rows.drain(..rows.len() - 100);
        }
        match last {
            Some(last) if full && last > from => from = last,
            _ => break,
        }
    }
    let mut seen = HashSet::new();
    rows.retain(|row| {
        seen.insert((
            row["timestamp"].as_str().unwrap_or_default().to_owned(),
            row["message"].as_str().unwrap_or_default().to_owned(),
        ))
    });
    Ok(LogHistory {
        items: rows
            .iter()
            .map(|row| entry(row, &scanner, token))
            .collect::<Result<_, _>>()?,
        ..empty
    })
}

/// A live tail of the latest deployment's logs: Railway's subscription while
/// it holds, then polling for lines newer than the last one seen.
pub(super) struct Tail {
    /// Railway's live stream of new lines, while it holds; reading every few
    /// seconds takes over if it drops.
    live: Option<super::railway_live::Subscription>,
    http: ProviderHttp,
    token: Secret,
    deployment: String,
    filter: String,
    since: String,
    seen: HashSet<(String, String)>,
    pending: VecDeque<LogEntry>,
    scanner: LeakScanner,
    first: bool,
}

/// Opens a live tail starting now.
///
/// # Errors
/// Reports a service that has never deployed, provider failures and malformed responses.
pub(super) async fn open_tail(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    filter: &str,
) -> Result<Tail, EngineError> {
    let deployment = latest(http, token, reference)
        .await?
        .ok_or(EngineError::Conflict(
            "This Railway service has no deployment to read logs from yet.",
        ))?;
    // Live first: lines arrive as they are written and cost no requests.
    let live = if cfg!(test) {
        None
    } else {
        super::railway_live::Subscription::start(
            token,
            "subscription HoistaTail($id:String!,$filter:String,$limit:Int){deploymentLogs(deploymentId:$id,filter:$filter,limit:$limit){timestamp message severity}}",
            json!({
                "id": deployment.id,
                "filter": if filter.is_empty() { None } else { Some(filter) },
                "limit": 1,
            }),
        )
        .await
        .ok()
    };
    Ok(Tail {
        live,
        http: http.clone(),
        token: Secret::new(token.expose().to_owned()),
        deployment: deployment.id,
        filter: filter.into(),
        since: chrono::Utc::now().to_rfc3339(),
        seen: HashSet::new(),
        pending: VecDeque::new(),
        scanner: LeakScanner::new().map_err(|_| invalid())?,
        first: true,
    })
}

#[async_trait::async_trait]
impl LogSession for Tail {
    async fn next(&mut self) -> Result<Option<LogEntry>, EngineError> {
        loop {
            if let Some(entry) = self.pending.pop_front() {
                return Ok(Some(entry));
            }
            let rows = if let Some(live) = self.live.as_mut() {
                match live.next().await {
                    Ok(Some(data)) => data["deploymentLogs"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                    // Ended or dropped: read every few seconds from here on.
                    Ok(None) | Err(_) => {
                        if let Some(mut live) = self.live.take() {
                            live.close().await;
                        }
                        continue;
                    }
                }
            } else {
                if !self.first {
                    tokio::time::sleep(POLL).await;
                }
                self.first = false;
                deployment_logs(
                    &self.http,
                    &self.token,
                    &self.deployment,
                    Some(&self.since),
                    None,
                    &self.filter,
                    100,
                )
                .await?
            };
            for row in rows {
                let at = row["timestamp"].as_str().unwrap_or_default().to_owned();
                let key = (
                    at.clone(),
                    row["message"].as_str().unwrap_or_default().to_owned(),
                );
                // The start bound is inclusive, so lines at the boundary repeat; skip them.
                if !self.seen.insert(key) {
                    continue;
                }
                if at > self.since {
                    self.since.clone_from(&at);
                }
                self.pending
                    .push_back(entry(&row, &self.scanner, &self.token)?);
            }
            if self.seen.len() > MAX_SEEN_LINES {
                self.seen.clear();
            }
        }
    }

    async fn close(&mut self) -> Result<(), EngineError> {
        self.pending.clear();
        if let Some(mut live) = self.live.take() {
            live.close().await;
        }
        Ok(())
    }
}

/// A size in MB from Railway, which may send a fraction.
fn mb(value: &serde_json::Value) -> Option<u64> {
    value
        .as_f64()
        .filter(|mb| mb.is_finite() && *mb >= 0.0)
        .map(|mb| mb.round() as u64)
}

/// One day of measured CPU, memory and disk for a service. Railway bills the
/// workspace, not a service, so there is no per-service money figure.
/// Unmeasured values stay null.
///
/// # Errors
/// Reports provider failures and malformed responses.
pub(super) async fn usage(
    http: &ProviderHttp,
    token: &Secret,
    resource: &Resource,
    day: &str,
) -> Result<Usage, EngineError> {
    let mut usage = Usage {
        resource_id: resource.id.clone(),
        day: day.into(),
        requests: None,
        errors: None,
        compute_seconds: None,
        storage_bytes: None,
        egress_bytes: None,
    };
    let (_, service) = split(&resource.provider_ref)?;
    let Some(environment) = latest(http, token, &resource.provider_ref)
        .await?
        .and_then(|latest| latest.environment_id)
    else {
        return Ok(usage);
    };
    let start = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| EngineError::Invalid("Usage days are YYYY-MM-DD."))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(invalid)?
        .and_utc();
    let end = (start + chrono::Duration::days(1)).min(chrono::Utc::now());
    let value = graphql(
        http,
        token,
        "query HoistaMetrics($env:String!,$service:String,$start:DateTime!,$end:DateTime,$rate:Int){metrics(environmentId:$env,serviceId:$service,startDate:$start,endDate:$end,sampleRateSeconds:$rate,measurements:[CPU_USAGE,MEMORY_USAGE_GB,DISK_USAGE_GB]){measurement values{ts value}}}",
        json!({
            "env": environment,
            "service": service,
            "start": start.to_rfc3339(),
            "end": end.to_rfc3339(),
            "rate": SAMPLE_SECONDS,
        }),
    )
    .await?;
    let series = value["data"]["metrics"].as_array().ok_or_else(invalid)?;
    let samples = |name: &str| -> Result<Vec<(i64, f64)>, EngineError> {
        let Some(found) = series.iter().find(|item| item["measurement"] == name) else {
            return Ok(vec![]);
        };
        let mut samples = found["values"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|sample| {
                let at = sample["ts"]
                    .as_i64()
                    .or_else(|| sample["ts"].as_f64().map(|ts| ts as i64))
                    .ok_or_else(invalid)?;
                let value = sample["value"]
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .ok_or_else(invalid)?;
                Ok((at, value))
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        samples.sort_by_key(|(at, _)| *at);
        Ok(samples)
    };
    let cpu = samples("CPU_USAGE")?;
    let memory = samples("MEMORY_USAGE_GB")?;
    let disk = samples("DISK_USAGE_GB")?;
    if cpu.is_empty() && memory.is_empty() {
        return Ok(usage);
    }
    let cpu_minutes = integrate(&cpu) / 60.0;
    usage.compute_seconds = Some((cpu_minutes * 60.0).round() as i64);
    usage.storage_bytes = disk.last().map(|(_, gb)| (gb * 1e9).round() as i64);
    // Requests and server errors come from Railway's edge request logs. A
    // database serves no HTTP. Too many to count, or unreadable: unknown.
    if resource.kind != ResourceKind::Database
        && let Ok(Some((requests, errors))) =
            http_traffic(http, token, &resource.provider_ref, start, end).await
    {
        usage.requests = Some(requests);
        usage.errors = Some(errors);
    }
    Ok(usage)
}

/// What Railway has billed the workspace so far this billing period, and its
/// usage limits, as Railway's own `railway usage` reads it.
///
/// # Errors
/// Reports provider failures and malformed responses.
pub(super) async fn bill(
    http: &ProviderHttp,
    token: &Secret,
    workspace: &str,
) -> Result<Option<ProviderBill>, EngineError> {
    if workspace.is_empty() {
        return Ok(None);
    }
    let value = graphql(
        http,
        token,
        "query HoistaBill($workspace:String!){workspace(workspaceId:$workspace){customer{id currentUsage billingPeriod{start end} usageLimit{softLimit hardLimit isOverLimit}}}}",
        json!({ "workspace": workspace }),
    )
    .await?;
    let customer = &value["data"]["workspace"]["customer"];
    let Some(dollars) = customer["currentUsage"].as_f64() else {
        return Ok(None);
    };
    let money = |dollars: f64| Money {
        cents: crate::units::cents(dollars),
        currency: "USD".into(),
    };
    let limit = &customer["usageLimit"];
    Ok(Some(ProviderBill {
        billed: money(dollars),
        period_start: customer["billingPeriod"]["start"]
            .as_str()
            .map(str::to_owned),
        period_end: customer["billingPeriod"]["end"].as_str().map(str::to_owned),
        soft_limit: limit["softLimit"].as_f64().map(money),
        hard_limit: limit["hardLimit"].as_f64().map(money),
        over_limit: limit["isOverLimit"].as_bool().unwrap_or(false),
        customer_id: customer["id"].as_str().map(str::to_owned),
    }))
}

/// One row of Railway's edge request log.
struct EdgeRequest {
    at: String,
    request_id: String,
    method: String,
    path: String,
    status: u16,
    duration_ms: Option<u32>,
}

/// Every request Railway's edge served for a service in a time range, across
/// the deployments that were serving then, oldest first. None when there
/// were more than Hoista reads in one pass, so a partial set is never taken
/// for the whole.
async fn edge_requests(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<Option<Vec<EdgeRequest>>, EngineError> {
    let (project, service) = split(reference)?;
    let value = graphql(
        http,
        token,
        "query HoistaServing($input:DeploymentListInput!){deployments(input:$input,first:20){edges{node{id createdAt}}}}",
        json!({ "input": { "projectId": project, "serviceId": service } }),
    )
    .await?;
    let mut serving = Vec::new();
    // Newest first: each deployment served from its creation until the next one.
    let mut until = end;
    for edge in value["data"]["deployments"]["edges"]
        .as_array()
        .ok_or_else(invalid)?
    {
        let node = &edge["node"];
        let created =
            chrono::DateTime::parse_from_rfc3339(node["createdAt"].as_str().ok_or_else(invalid)?)
                .map_err(|_| invalid())?
                .with_timezone(&chrono::Utc);
        let from = created.max(start);
        if from < until {
            serving.push((
                node["id"].as_str().ok_or_else(invalid)?.to_owned(),
                from,
                until,
            ));
        }
        if created <= start {
            break;
        }
        until = until.min(created);
    }
    let mut rows = Vec::new();
    for (deployment, from, to) in serving {
        let mut cursor = from.to_rfc3339();
        let mut seen = HashSet::new();
        let mut pages = 0;
        loop {
            let value = graphql(
                http,
                token,
                "query HoistaRequests($id:String!,$start:DateTime,$end:DateTime,$limit:Int){httpLogs(deploymentId:$id,startDate:$start,endDate:$end,limit:$limit){timestamp requestId httpStatus method path totalDuration}}",
                json!({ "id": deployment, "start": cursor, "end": to.to_rfc3339(), "limit": HISTORY_PAGE }),
            )
            .await?;
            let page = value["data"]["httpLogs"].as_array().ok_or_else(invalid)?;
            let mut last = None;
            for row in page {
                let key = (
                    row["timestamp"].as_str().unwrap_or_default().to_owned(),
                    row["requestId"].as_str().unwrap_or_default().to_owned(),
                );
                if !seen.insert(key.clone()) {
                    continue;
                }
                if last.as_ref().is_none_or(|at: &String| key.0 > *at) {
                    last = Some(key.0.clone());
                }
                // The query string may carry tokens; only the path is kept.
                let path = row["path"]
                    .as_str()
                    .unwrap_or("/")
                    .split(['?', '#'])
                    .next()
                    .unwrap_or("/")
                    .chars()
                    .take(MAX_REQUEST_PATH_CHARS)
                    .collect();
                rows.push(EdgeRequest {
                    at: key.0,
                    request_id: key.1,
                    method: row["method"]
                        .as_str()
                        .unwrap_or("GET")
                        .chars()
                        .take(MAX_DNS_RECORDS)
                        .collect(),
                    path,
                    status: row["httpStatus"]
                        .as_u64()
                        .and_then(|status| u16::try_from(status).ok())
                        .unwrap_or(0),
                    duration_ms: row["totalDuration"]
                        .as_u64()
                        .and_then(|ms| u32::try_from(ms).ok()),
                });
            }
            pages += 1;
            if page.len() < HISTORY_PAGE {
                break;
            }
            match last {
                Some(at) if pages < HISTORY_PAGES && at > cursor => cursor = at,
                _ => return Ok(None),
            }
        }
    }
    rows.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(Some(rows))
}

/// Requests and 5xx responses in a time range, or None when too many to count.
async fn http_traffic(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<Option<(i64, i64)>, EngineError> {
    Ok(edge_requests(http, token, reference, start, end)
        .await?
        .map(|rows| {
            let errors = rows.iter().filter(|row| row.status >= 500).count();
            (
                i64::try_from(rows.len()).unwrap_or(i64::MAX),
                i64::try_from(errors).unwrap_or(i64::MAX),
            )
        }))
}

/// What a service's address served in a time range: counts, response-time
/// percentiles, the most frequent failures and the newest requests.
///
/// # Errors
/// Reports ranges over a week, invalid references and provider failures.
pub(super) async fn traffic(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<ProviderTraffic, EngineError> {
    if to <= from || to - from > chrono::Duration::days(MAX_TRAFFIC_DAYS) {
        return Err(EngineError::Invalid(
            "Read traffic for up to 7 days at a time.",
        ));
    }
    let mut traffic = ProviderTraffic {
        from: from.to_rfc3339(),
        to: to.to_rfc3339(),
        ..ProviderTraffic::default()
    };
    let Some(rows) = edge_requests(http, token, reference, from, to).await? else {
        return Ok(traffic);
    };
    let count = |test: &dyn Fn(u16) -> bool| {
        u32::try_from(rows.iter().filter(|row| test(row.status)).count()).ok()
    };
    traffic.requests = u32::try_from(rows.len()).ok();
    traffic.client_errors = count(&|status| (400..500).contains(&status));
    traffic.server_errors = count(&|status| status >= 500);
    let mut durations: Vec<u32> = rows.iter().filter_map(|row| row.duration_ms).collect();
    durations.sort_unstable();
    let percentile = |p: usize| {
        (!durations.is_empty())
            .then(|| durations[(durations.len() * p / 100).min(durations.len() - 1)])
    };
    traffic.p50_ms = percentile(50);
    traffic.p95_ms = percentile(95);
    traffic.p99_ms = percentile(99);
    let mut failing: HashMap<(u16, String, String), u32> = HashMap::new();
    for row in rows.iter().filter(|row| row.status >= 400) {
        *failing
            .entry((row.status, row.method.clone(), row.path.clone()))
            .or_default() += 1;
    }
    let mut top: Vec<ProviderErrorCount> = failing
        .into_iter()
        .map(|((status, method, path), count)| ProviderErrorCount {
            status,
            method,
            path,
            count,
        })
        .collect();
    // Server errors before client errors at the same count: they need fixing.
    top.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then((b.status >= 500).cmp(&(a.status >= 500)))
            .then(a.path.cmp(&b.path))
    });
    top.truncate(MAX_TOP_ERRORS);
    traffic.top_errors = top;
    traffic.recent = rows
        .iter()
        .rev()
        .take(200)
        .map(|row| ProviderRequest {
            at: row.at.clone(),
            method: row.method.clone(),
            path: row.path.clone(),
            status: row.status,
            duration_ms: row.duration_ms,
            request_id: row.request_id.clone(),
        })
        .collect();
    Ok(traffic)
}

/// Value × seconds over a series. Railway may return samples further apart
/// than requested, so each sample counts until the next one; a gap longer
/// than twice the usual spacing (the service was down or unmeasured) and the
/// last sample count for one usual spacing.
fn integrate(samples: &[(i64, f64)]) -> f64 {
    let mut gaps: Vec<i64> = samples
        .windows(2)
        .map(|pair| pair[1].0 - pair[0].0)
        .filter(|gap| *gap > 0)
        .collect();
    gaps.sort_unstable();
    let usual = gaps
        .get(gaps.len().saturating_sub(1) / 2)
        .copied()
        .unwrap_or(SAMPLE_SECONDS);
    samples
        .iter()
        .enumerate()
        .map(|(index, (at, value))| {
            let span = samples.get(index + 1).map_or(usual, |(next, _)| {
                let gap = (next - at).max(0);
                if gap > usual * 2 { usual } else { gap }
            });
            value * span as f64
        })
        .sum()
}

/// Railway's entry in the provider list.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Railway,
    logo: LOGO,
    capabilities: Capabilities {
        logs: true,
        metrics: true,
        usage: true,
        discover: true,
        server: true,
        provider_details: true,
        provider_deploys: true,
        provider_variables: true,
        provider_operations: true,
        upload: true,
        new_project: true,
        new_database: true,
        new_bucket: true,
        new_from_image: true,
        bills: true,
        browser_sign_in: CLIENT_ID.is_some(),
        delete: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: GRAPHQL,
        hosts: &["backboard.railway.com"],
        explains_refusals: true,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Graphql {
            url: None,
            query: "query HoistaAccountChoices { me { workspaces { id name } } }",
            cursor: None,
        },
        rows: &["data", "me", "workspaces"],
        item: &[],
        id: "id",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(Railway::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of the provider for the conformance suite: enough to connect and
/// find one resource.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "acct-1",
    resource: "p1/s1",
    kind: crate::domain::model::ResourceKind::Web,
    routes: &[
        super::conformance::Route {
            path: "/",
            body: "HoistaAccountChoices",
            answer: r#"{"data":{"me":{"workspaces":[{"id":"acct-1","name":"Sam"}]}}}"#,
        },
        super::conformance::Route {
            path: "/",
            body: "HoistaProjects",
            answer: r#"{"data":{"projects":{"edges":[{"node":{"id":"p1","name":"P"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#,
        },
        super::conformance::Route {
            path: "/",
            body: "HoistaServices",
            answer: r#"{"data":{"project":{"services":{"edges":[{"node":{"id":"s1","name":"api"}}],"pageInfo":{"hasNextPage":false}}}}}"#,
        },
    ],
    empty: r#"{"data":{}}"#,
};

/// Railway adapter: finds services in a workspace's projects, watches them,
/// and changes them through Railway's own API.
pub struct Railway {
    provider: Provider,
    http: ProviderHttp,
}

impl Railway {
    /// An adapter on the shared provider HTTP client.
    #[must_use]
    pub fn new(http: ProviderHttp) -> Self {
        Self {
            provider: Provider::Railway,
            http,
        }
    }

    async fn services(
        &self,
        token: &Secret,
        workspace: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut result = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let query = "query HoistaProjects($workspace:String,$after:String){projects(workspaceId:$workspace,first:100,after:$after){edges{node{id name}}pageInfo{hasNextPage endCursor}}}";
            let value = self
                .http
                .json(
                    self.provider,
                    token,
                    Method::POST,
                    &[],
                    &[],
                    Some(&json!({
                        "query": query,
                        "variables": {
                            "workspace": if workspace.is_empty(){None}else{Some(workspace)},
                            "after": cursor,
                        },
                    })),
                )
                .await?;
            let page = &value["data"]["projects"];
            for edge in page["edges"].as_array().ok_or_else(invalid)? {
                let project = &edge["node"];
                let id = field(project, "id")?;
                let mut service_cursor: Option<String> = None;
                let mut service_seen = std::collections::HashSet::new();
                loop {
                    let query = "query HoistaServices($id:String!,$after:String){project(id:$id){services(first:100,after:$after){edges{node{id name}}pageInfo{hasNextPage endCursor}}}}";
                    let value=self.http.json(self.provider,token,Method::POST,&[],&[],Some(&json!({"query":query,"variables":{"id":id,"after":service_cursor}}))).await?;
                    let services = &value["data"]["project"]["services"];
                    for edge in services["edges"].as_array().ok_or_else(invalid)? {
                        let p = &edge["node"];
                        result.push(RemoteResource {
                            reference: format!("{}/{}", id, field(p, "id")?),
                            kind: kind_of(&field(p, "name")?),
                            name: field(p, "name")?,
                            region: None,
                            status: ResourceStatus::Unknown,
                            url: None,
                            group: Some(field(project, "name")?),
                        });
                    }
                    if result.len() > MAX_DISCOVERED_RESOURCES {
                        return Err(EngineError::Conflict(
                            "Railway discovery found more services than Hoista keeps.",
                        ));
                    }
                    if services["pageInfo"]["hasNextPage"] == false {
                        break;
                    }
                    let next = field(&services["pageInfo"], "endCursor")?;
                    if !service_seen.insert(next.clone()) || service_seen.len() > 100 {
                        return Err(invalid());
                    }
                    service_cursor = Some(next);
                }
            }
            if page["pageInfo"]["hasNextPage"] == false {
                self.states(token, &mut result).await;
                return Ok(result);
            }
            let next = field(&page["pageInfo"], "endCursor")?;
            if !seen.insert(next.clone()) {
                return Err(invalid());
            }
            cursor = Some(next);
        }
        Err(EngineError::Conflict(
            "Railway discovery exceeded its project limit.",
        ))
    }

    /// Fills in status and URL from each service's latest deployment. Bounded,
    /// and a failed read leaves that service unknown rather than failing discovery.
    async fn states(&self, token: &Secret, resources: &mut [RemoteResource]) {
        for resource in resources.iter_mut().take(MAX_STATE_LOOKUPS) {
            if let Ok(Some(latest)) = latest(&self.http, token, &resource.reference).await {
                resource.status = latest.status;
                resource.url = latest.url;
            }
        }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Railway {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// A Railway service's reference starts with its project's id.
    fn provider_project(&self, reference: &str) -> Option<String> {
        reference
            .split_once('/')
            .map(|(project, _)| project.to_owned())
    }

    /// Railway removes a service per environment, through its own action.
    fn removes_services_by_action(&self) -> bool {
        true
    }

    /// Railway names a database after its template.
    fn names_added_databases(&self) -> bool {
        false
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        super::accounts::list(&self.http, self.provider, token).await
    }

    async fn verify(&self, token: &Secret, external_id: &str) -> Result<Vec<String>, EngineError> {
        self.discover(token, external_id).await?;
        Ok(vec![
            "projects, services and deployments: read (verified by request)".into(),
            "deployment logs and metrics: read with the same token".into(),
        ])
    }

    async fn discover(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        self.services(token, external_id).await
    }

    fn server_key_steps(&self, reference: &str) -> Option<(String, Option<String>)> {
        Some((
            "Seal HOISTA_SEALED_KEY so nobody can read it at Railway, even with your account: in the hoista service open Variables, then ⋯ next to HOISTA_SEALED_KEY, then Seal.".to_owned(),
            reference
                .split_once('/')
                .map(|(project, _)| project_page(project)),
        ))
    }

    async fn observe_resource(
        &self,
        token: &Secret,
        _external_id: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let latest = latest(&self.http, token, &resource.provider_ref).await?;
        Ok(Observed {
            status: latest
                .as_ref()
                // A service with no deployment at all hasn't been deployed yet.
                .map_or(ResourceStatus::NotDeployed, |latest| latest.status),
            url: latest.and_then(|latest| latest.url),
            kind: Some(kind_of(&resource.name)),
        })
    }

    fn oauth(&self) -> Option<ProviderOAuth> {
        oauth_app()
    }

    async fn oauth_token(&self, grant: OAuthGrant<'_>) -> Result<OAuthTokens, EngineError> {
        oauth_token(&self.http, grant).await
    }

    async fn set_provider_variable(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        set_variable(&self.http, token, reference, name, value).await
    }

    async fn delete_provider_variable(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        delete_variable(&self.http, token, reference, name).await
    }

    async fn set_provider_variables(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        values: &[(String, Secret)],
        shared: bool,
        redeploy: bool,
    ) -> Result<(), EngineError> {
        set_variables(&self.http, token, reference, values, shared, redeploy).await
    }

    async fn delete_shared_variable(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        delete_shared_variable(&self.http, token, reference, name).await
    }

    /// Deletes the service (or database) from its Railway project.
    async fn delete(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let (_, service) = split(reference)?;
        let value = graphql(
            &self.http,
            token,
            "mutation HoistaServiceDelete($id:String!){serviceDelete(id:$id)}",
            json!({ "id": service }),
        )
        .await?;
        if value["data"]["serviceDelete"].as_bool() != Some(true) {
            return Err(invalid());
        }
        Ok(())
    }

    async fn provider_action(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        action: &crate::application::ports::providers::ProviderAction<'_>,
    ) -> Result<String, EngineError> {
        act(&self.http, token, reference, action).await
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        let mut operations = super::railway_catalog::operations();
        operations.extend(manifest().operations());
        operations
    }

    fn prepare_tools(&self) {
        if let Some(cli) = super::manifest::builtin("railway").cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        manifest().actions.clone()
    }

    async fn provider_operation(
        &self,
        token: &Secret,
        external_id: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        if let Some(cli) = manifest().cli.as_ref()
            && super::cli::has(cli, name)
        {
            let reference = reference.ok_or(EngineError::Invalid(
                "Railway commands run on one service: give the resource_id of one Hoista watches.",
            ))?;
            let (project, service) = split(reference)?;
            let environment =
                super::railway_catalog::environment_of(&self.http, token, reference).await?;
            let mut context = vec![("project", project), ("service", service)];
            context.extend(environment.as_deref().map(|id| ("environment", id)));
            return super::cli::run(
                cli,
                &self.http,
                token,
                super::cli::Aim {
                    context: &context,
                    folder,
                },
                name,
                args,
            )
            .await;
        }
        super::railway_catalog::run(&self.http, token, external_id, reference, name, args).await
    }

    async fn provider_empty_projects(
        &self,
        token: &Secret,
        workspace: &str,
    ) -> Result<Vec<crate::application::ports::providers::ProviderEmptyProject>, EngineError> {
        let mut empty = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let query = "query HoistaEmptyProjects($workspace:String,$after:String){projects(workspaceId:$workspace,first:100,after:$after){edges{node{id name services(first:1){edges{node{id}}}}}pageInfo{hasNextPage endCursor}}}";
            let value = self
                .http
                .json(
                    Provider::Railway,
                    token,
                    Method::POST,
                    &[],
                    &[],
                    Some(&json!({
                        "query": query,
                        "variables": {
                            "workspace": if workspace.is_empty() { None } else { Some(workspace) },
                            "after": cursor,
                        },
                    })),
                )
                .await?;
            let page = &value["data"]["projects"];
            for edge in page["edges"].as_array().ok_or_else(invalid)? {
                let project = &edge["node"];
                let holds = project["services"]["edges"]
                    .as_array()
                    .is_some_and(|services| !services.is_empty());
                if !holds {
                    let id = field(project, "id")?;
                    empty.push(crate::application::ports::providers::ProviderEmptyProject {
                        url: Some(project_page(&id)),
                        name: field(project, "name")?,
                        id,
                    });
                }
            }
            if page["pageInfo"]["hasNextPage"] != true {
                break;
            }
            let next = field(&page["pageInfo"], "endCursor")?;
            if !seen.insert(next.clone()) {
                return Err(invalid());
            }
            cursor = Some(next);
        }
        Ok(empty)
    }

    async fn provider_bill(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Option<crate::application::ports::providers::ProviderBill>, EngineError> {
        bill(&self.http, token, external_id).await
    }

    async fn provider_traffic(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<crate::application::ports::providers::ProviderTraffic, EngineError> {
        traffic(&self.http, token, reference, from, to).await
    }

    async fn provider_create(
        &self,
        token: &Secret,
        _external_id: &str,
        beside: &str,
        what: &crate::application::ports::providers::ProviderCreate<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        create(&self.http, token, beside, what).await
    }

    async fn provider_add(
        &self,
        token: &Secret,
        external_id: &str,
        project: &str,
        beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        add(&self.http, token, external_id, project, beside, what).await
    }

    async fn provider_new_project(
        &self,
        token: &Secret,
        external_id: &str,
        name: &str,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<(Vec<String>, String), EngineError> {
        new_project(&self.http, token, external_id, name, repo, branch).await
    }

    async fn provider_new_hoista_server(
        &self,
        token: &Secret,
        external_id: &str,
        source: &crate::application::ports::providers::ServerSource,
        variables: &std::collections::BTreeMap<String, Secret>,
    ) -> Result<(String, String), EngineError> {
        new_hoista_server(&self.http, token, external_id, source, variables).await
    }

    async fn provider_hide_server_key(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
    ) -> Result<bool, EngineError> {
        seal_server_key(&self.http, token, reference).await
    }

    async fn provider_upload(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        folder: &std::path::Path,
        message: Option<&str>,
    ) -> Result<String, EngineError> {
        upload(&self.http, token, reference, folder, message).await
    }

    async fn provider_deployment_logs(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        deployment: &str,
    ) -> Result<Vec<crate::domain::logs::LogEntry>, EngineError> {
        deployment_lines(&self.http, token, reference, deployment).await
    }

    async fn provider_details(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
    ) -> Result<ProviderDetails, EngineError> {
        details(&self.http, token, reference).await
    }

    async fn log_history(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        query: &LogHistoryQuery,
    ) -> Result<LogHistory, EngineError> {
        log_history(&self.http, token, reference, query).await
    }

    async fn open_logs(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        query: &str,
    ) -> Result<Box<dyn LogSession>, EngineError> {
        Ok(Box::new(
            open_tail(&self.http, token, reference, query).await?,
        ))
    }

    async fn usage(
        &self,
        token: &Secret,
        _external_id: &str,
        resource: &Resource,
        day: &str,
    ) -> Result<Usage, EngineError> {
        usage(&self.http, token, resource, day).await
    }
}

/// Railway's manifest: its pinned CLI and the buttons Hoista shows for it.
fn manifest() -> &'static super::manifest::Manifest {
    super::manifest::builtin("railway")
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_query_is_named_by_its_operation() {
        assert_eq!(
            super::operation_name("query HoistaMetrics($env:String!){x}"),
            "HoistaMetrics"
        );
        assert_eq!(super::operation_name("mutation Go { y }"), "Go");
        assert_eq!(super::operation_name("{ me { id } }"), "");
    }

    use super::*;

    #[test]
    fn deployment_states_never_guess_health() {
        assert_eq!(status_of("SUCCESS"), ResourceStatus::Live);
        assert_eq!(status_of("CRASHED"), ResourceStatus::Failing);
        assert_eq!(status_of("SLEEPING"), ResourceStatus::Sleeping);
        assert_eq!(status_of("BUILDING"), ResourceStatus::Deploying);
        assert_eq!(status_of("REMOVED"), ResourceStatus::Stopped);
        assert_eq!(status_of("SOMETHING_NEW"), ResourceStatus::Unknown);
        assert!(split("project-only").is_err());
        assert_eq!(split("p/s").unwrap(), ("p", "s"));
        assert_eq!(kind_of("Postgres"), ResourceKind::Database);
        assert_eq!(kind_of("api"), ResourceKind::Service);
        assert_eq!(
            stated_level("2026-10-31 23:10:50 UTC [110] LOG:  checkpoint complete"),
            Some(LogLevel::Info)
        );
        assert_eq!(
            stated_level("[110] ERROR:  relation \"users\" does not exist"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            stated_level(r#"{"level":"warn","msg":"slow"}"#),
            Some(LogLevel::Warn)
        );
        assert_eq!(
            stated_level("Error: connect ECONNREFUSED"),
            Some(LogLevel::Error)
        );
        assert_eq!(stated_level("listening on 8080"), None);
        assert_eq!(references("${{Postgres.DATABASE_URL}}"), vec!["Postgres"]);
        assert_eq!(
            references("redis://${{ Redis.REDISHOST }}:${{Redis.REDISPORT}}/${{shared.X}}"),
            vec!["Redis"]
        );
        assert!(references("plain-secret-value").is_empty());
        // Hourly samples are counted as hours, not as five-minute samples.
        let hourly = [(0, 1.0), (3600, 1.0), (7200, 1.0)];
        assert!((integrate(&hourly) - 3.0 * 3600.0).abs() < f64::EPSILON);
        // A six-hour hole is not counted as running.
        let gap = [(0, 1.0), (300, 1.0), (21_900, 1.0)];
        assert!((integrate(&gap) - 900.0).abs() < f64::EPSILON);
    }

    #[test]
    fn addresses_show_where_a_variable_points_never_its_secret() {
        let url = VariableAddress::from_value(
            "DATABASE_URL",
            "postgresql://postgres.abcdefghijklmnopqrst:hunter2@aws-0-eu-west-3.pooler.supabase.com:6543/postgres",
        )
        .unwrap();
        assert_eq!(url.host, "aws-0-eu-west-3.pooler.supabase.com");
        assert_eq!(url.user.as_deref(), Some("postgres.abcdefghijklmnopqrst"));
        assert!(!format!("{url:?}").contains("hunter2"));
        let api =
            VariableAddress::from_value("SUPABASE_URL", "https://abcdefghijklmnopqrst.supabase.co")
                .unwrap();
        assert_eq!(api.host, "abcdefghijklmnopqrst.supabase.co");
        assert_eq!(api.user, None);
        assert_eq!(
            VariableAddress::from_value("KEY", "eyJhbGciOi.eyJzdWIi.c2ln"),
            None
        );
        assert_eq!(
            VariableAddress::from_value("URL", "${{Postgres.DATABASE_URL}}"),
            None
        );
        assert_eq!(
            shared_name("${{ shared.SUPABASE_URL }}").as_deref(),
            Some("SUPABASE_URL")
        );
    }
}
