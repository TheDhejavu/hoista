//! Runpod pods, Serverless endpoints and network volumes, through the REST
//! API v2. The key's account comes from the GraphQL `myself` query.
//!
//! References: `pod:<id>`, `endpoint:<id>`, `volume:<id>`.

use super::http::ProviderHttp;
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{Observed, ProviderAdapter, ProviderBill, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::{Value, json};

/// Set when Runpod refuses every bill request, so refreshes stop asking.
static BILL_REFUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runpod's API adapter.
pub struct Runpod {
    http: ProviderHttp,
}

const MAX_TEXT_CHARS: usize = 256;
const MAX_NAME_CHARS: usize = 128;
const MAX_CURSOR_BYTES: usize = 2048;
/// A log read: the most lines, and how long the stream is listened to.
const MAX_LOG_LINES: usize = 500;
const LOG_WINDOW: std::time::Duration = std::time::Duration::from_secs(4);
/// Releases and audit entries shown as deployments.
const MAX_DEPLOYMENTS: usize = 20;
/// An audit entry names at most this many changed settings.
const MAX_FIELDS_NAMED: usize = 6;
const MAX_FIELD_CHARS: usize = 40;
/// Serverless workers whose logs are read.
const MAX_WORKERS_READ: usize = 3;

const GRAPHQL: &str = "https://api.runpod.io/graphql";

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M21.6 19.4C22.8 18.8 23.6 17.6 23.6 16.2L23.6 7.8C23.6 6.4 22.8 5.2 21.6 4.6L14.4 0.8C13.2 0.2 11.8 0.2 10.6 0.8L3.4 4.6C2.2 5.2 1.4 6.4 1.4 7.8L1.4 16.2C1.4 17.6 2.2 18.8 3.4 19.4L10.6 23.2C11.8 23.8 13.2 23.8 14.4 23.2L21.6 19.4ZM21.6 16.2C21.6 16.8 21.2 17.4 20.6 17.7L17.2 19.2C16.8 19.4 16.6 19.5 16.4 19.5C16.2 19.5 16.1 19.4 16 19.2C15.9 19 15.9 18.6 15.9 17.8L15.9 15.2C15.9 14.6 16.3 14 16.9 13.7L19.3 12.2C19.7 12 19.9 11.9 20 11.8C20.1 11.7 20.2 11.6 20.2 11.4C20.3 11.2 20.3 11 20.3 10.6L20.3 10.4C20.3 9.8 20.3 9.5 20.1 9.3C20 9.1 19.8 9 19.6 9C19.4 9 19.2 9.2 18.8 9.6L15.9 11.2C14.7 11.8 13.3 12.2 13.3 15.2V21.2C13.3 21.6 13.1 21.9 12.8 22C12.2 22.3 11.8 22.3 11.2 22L4 19.4C3.4 19.1 3 18.5 3 17.9L3 11.9C3 11.3 3 10.9 3.1 10.7C3.2 10.5 3.4 10.4 3.6 10.4C3.8 10.4 4 10.5 4.2 10.7L7.4 12.2C8.2 12.5 8.6 13.1 8.6 13.7L8.6 15.7C8.6 16.1 8.6 16.3 8.7 16.5C8.8 16.7 8.9 16.8 9 16.9C9.1 17 9.2 17.1 9.4 17.2L9.5 17.2C9.8 17.4 9.9 17.5 10 17.5C10.1 17.5 10.2 17.4 10.3 17.2C10.4 17 10.4 16.6 10.4 15.8L10.4 13.7C10.4 12.3 9.6 11.1 8.4 10.5L2.8 7.8C2.4 7.6 2 7.2 2 6.8C2 6.2 2.4 5.6 3 5.3L11.2 1.5C12 1.2 12.8 1.2 13.6 1.5L17.4 3.2C17.8 3.4 18 3.5 18.1 3.7C18.2 3.8 18.2 4 18.1 4.2C18 4.4 17.8 4.5 17.4 4.7L13.6 6.4C12.8 6.7 12 6.7 11.2 6.4L8.8 5.2C8.4 5 8.2 4.9 8 4.8C7.8 4.8 7.6 4.8 7.4 4.9C7.2 5 7 5.1 6.8 5.3L6.4 5.5C6 5.7 5.8 5.8 5.7 6C5.6 6.1 5.6 6.3 5.7 6.5C5.8 6.7 6 6.8 6.4 7L9.6 8.5C10.8 9.1 12.2 9.1 13.4 8.5L20.2 5.3C20.6 5.1 21 5.1 21.4 5.3C22 5.6 22.4 6.2 22.4 6.8V16.2Z" fill="#673ab7" fill-rule="evenodd"/></svg>"##;

/// Runpod's entry in the provider list. A key reaches one account, so the
/// adapter answers account choices itself from `myself`.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Runpod,
    logo: LOGO,
    capabilities: Capabilities {
        discover: true,
        provider_operations: true,
        bills: true,
        delete: true,
        logs: true,
        metrics: true,
        provider_details: true,
        upload: true,
        new_from_folder: true,
        new_from_image: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://api.runpod.io/",
        hosts: &["api.runpod.io"],
        explains_refusals: true,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Graphql {
            url: Some(GRAPHQL),
            query: "query { myself { id } }",
            cursor: None,
        },
        rows: &["data"],
        item: &["myself"],
        id: "id",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(Runpod::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of the provider for the conformance suite: enough to connect and find
/// one pod.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "user-1",
    resource: "pod:pod-1",
    kind: crate::domain::model::ResourceKind::Service,
    routes: &[
        super::conformance::Route {
            path: "/graphql",
            body: "myself",
            answer: r#"{"data":{"myself":{"id":"user-1"}}}"#,
        },
        super::conformance::Route {
            path: "/v2/pods",
            body: "",
            answer: r#"{"pods":[{"id":"pod-1","name":"trainer","status":"RUNNING","ports":["8888/http"],"dataCenterId":"EU-RO-1"}],"pagination":{"nextCursor":null,"hasNextPage":false}}"#,
        },
    ],
    empty: r#"{"pods":[],"endpoints":[],"networkVolumes":[],"pagination":{"nextCursor":null,"hasNextPage":false}}"#,
};

fn invalid() -> EngineError {
    super::answers::incomplete("Runpod")
}

/// Runpod's IDs: letters, digits, dashes and underscores.
fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// What a reference names: a pod, a Serverless endpoint or a network volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pod,
    Endpoint,
    Volume,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Pod => "pod",
            Kind::Endpoint => "endpoint",
            Kind::Volume => "volume",
        }
    }

    /// The REST collection it lives in.
    fn path(self) -> &'static str {
        match self {
            Kind::Pod => "pods",
            Kind::Endpoint => "serverless",
            Kind::Volume => "network-volumes",
        }
    }
}

fn split(reference: &str) -> Result<(Kind, &str), EngineError> {
    let (kind, id) = reference.split_once(':').ok_or_else(invalid)?;
    let kind = match kind {
        "pod" => Kind::Pod,
        "endpoint" => Kind::Endpoint,
        "volume" => Kind::Volume,
        _ => return Err(invalid()),
    };
    if !plain_id(id) {
        return Err(invalid());
    }
    Ok((kind, id))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= MAX_TEXT_CHARS)
        .map(str::to_owned)
}

/// A pod's status as a resource status. A terminated pod is gone.
fn pod_status(status: &str) -> ResourceStatus {
    match status {
        "RUNNING" => ResourceStatus::Live,
        "PROVISIONING" | "STARTING" => ResourceStatus::Deploying,
        "EXITED" => ResourceStatus::Stopped,
        "ERROR" => ResourceStatus::Failing,
        _ => ResourceStatus::Unknown,
    }
}

/// The address of a pod's first HTTP port, through Runpod's proxy.
fn pod_url(pod: &Value) -> Option<String> {
    let id = text(pod, "id")?;
    let port = pod["ports"].as_array()?.iter().find_map(|port| {
        let (number, protocol) = port.as_str()?.split_once('/')?;
        (protocol == "http" && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| number.to_owned())
    })?;
    Some(format!("https://{id}-{port}.proxy.runpod.net"))
}

/// Where a Serverless endpoint takes requests: its load-balancing address,
/// or its queue address without the operation.
fn endpoint_url(endpoint: &Value) -> Option<String> {
    let urls = &endpoint["requestUrls"];
    if let Some(base) = urls["base"].as_str() {
        return Some(base.to_owned());
    }
    urls["run"]
        .as_str()
        .map(|run| run.trim_end_matches("/run").to_owned())
}

/// One line of a pod's or worker's log stream (`{source, line, ts}`).
/// Runpod's own field names first, then the usual alternatives; a line
/// that isn't JSON is the log text itself.
fn log_line(data: &str) -> Option<(Option<String>, String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        let line = data.trim();
        return (!line.is_empty()).then(|| (None, "container".to_owned(), line.to_owned()));
    };
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| value.get(*key).and_then(Value::as_str))
            .map(str::to_owned)
    };
    let line = pick(&["line", "message", "msg", "log", "text"])?;
    let source = pick(&["source", "stream"]).unwrap_or_else(|| "container".to_owned());
    let at = pick(&["ts", "timestamp", "time"]);
    Some((at, source, line))
}

impl Runpod {
    /// The newest lines of a pod's or worker's log stream since a time:
    /// Runpod streams logs, so Hoista reads what it backfills and a few
    /// seconds more, then stops.
    async fn stream_logs(
        &self,
        token: &Secret,
        path: &[&str],
        since: Option<&str>,
    ) -> Result<Vec<(Option<String>, String, String)>, EngineError> {
        let mut query = vec![("tail", "500")];
        if let Some(since) = since {
            query.push(("since", since));
        }
        Ok(self
            .http
            .sse_window(
                Provider::Runpod,
                token,
                path,
                &query,
                MAX_LOG_LINES,
                LOG_WINDOW,
            )
            .await?
            .iter()
            .filter_map(|data| log_line(data))
            .collect())
    }

    /// Shares the allow-listed HTTP boundary.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    /// The account the key belongs to.
    async fn myself(&self, token: &Secret) -> Result<String, EngineError> {
        let answer = self
            .http
            .post_at(
                Provider::Runpod,
                token,
                GRAPHQL,
                &json!({"query": "query { myself { id } }"}),
                true,
            )
            .await?;
        text(&answer["data"]["myself"], "id")
            .filter(|id| plain_id(id))
            .ok_or_else(invalid)
    }

    /// Every row of one of Runpod's lists, which page by a cursor.
    async fn every(
        &self,
        token: &Secret,
        path: &str,
        rows: &str,
    ) -> Result<Vec<Value>, EngineError> {
        let mut all: Vec<Value> = Vec::new();
        let mut cursor = String::new();
        for _ in 0..100 {
            let mut query = vec![("limit", "100")];
            if !cursor.is_empty() {
                query.push(("cursor", cursor.as_str()));
            }
            let value = self
                .http
                .json(
                    Provider::Runpod,
                    token,
                    Method::GET,
                    &["v2", path],
                    &query,
                    None,
                )
                .await?;
            all.extend(value[rows].as_array().ok_or_else(invalid)?.iter().cloned());
            let pagination = &value["pagination"];
            if pagination["hasNextPage"].as_bool() != Some(true) {
                return Ok(all);
            }
            let next = pagination["nextCursor"]
                .as_str()
                .filter(|next| !next.is_empty() && next.len() <= MAX_CURSOR_BYTES)
                .ok_or_else(invalid)?;
            if next == cursor {
                return Err(invalid());
            }
            next.clone_into(&mut cursor);
        }
        Err(invalid())
    }

    /// One GraphQL query, its `data`.
    async fn graphql(
        &self,
        token: &Secret,
        query: &str,
        variables: Value,
    ) -> Result<Value, EngineError> {
        let answer = self
            .http
            .post_at(
                Provider::Runpod,
                token,
                GRAPHQL,
                &json!({"query": query, "variables": variables}),
                true,
            )
            .await?;
        if answer["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
        {
            return Err(invalid());
        }
        Ok(answer["data"].clone())
    }

    /// Every Flash app's environments and the endpoints each runs, as
    /// Runpod's own CLI reads them.
    async fn flash_environments(
        &self,
        token: &Secret,
    ) -> Result<Vec<FlashEnvironment>, EngineError> {
        let apps = self
            .graphql(
                token,
                "query { myself { flashApps { name } } }",
                Value::Null,
            )
            .await?;
        let names: Vec<String> = apps["myself"]["flashApps"]
            .as_array()
            .map(|apps| apps.iter().filter_map(|app| text(app, "name")).collect())
            .unwrap_or_default();
        let mut found = Vec::new();
        for name in names.into_iter().filter(|name| flash_name(name)).take(200) {
            let app = self
                .graphql(
                    token,
                    "query($flashAppName: String!) { flashAppByName(flashAppName: $flashAppName) { name flashEnvironments { name activeBuildId endpoints { id } } } }",
                    json!({"flashAppName": name}),
                )
                .await?;
            found.extend(flash_environments(&app["flashAppByName"]));
        }
        Ok(found)
    }

    /// The Flash app and environment a Serverless endpoint belongs to.
    async fn flash_home(
        &self,
        token: &Secret,
        endpoint: &str,
    ) -> Result<FlashEnvironment, EngineError> {
        self.flash_environments(token)
            .await?
            .into_iter()
            .find(|environment| environment.endpoints.iter().any(|id| id == endpoint))
            .ok_or_else(|| EngineError::Provider {
                evidence: None,
                code: "provider.not_flash".into(),
                message: "This endpoint isn't part of a Runpod Flash app, so it doesn't deploy from a folder. Change it with Runpod's endpoint operations instead.".into(),
            })
    }

    /// Runs one of Runpod's own commands (Flash), aimed at an app's
    /// environment.
    async fn command(
        &self,
        token: &Secret,
        app: Option<&str>,
        environment: &str,
        folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        let cli = manifest().cli.as_ref().ok_or(EngineError::Invalid(
            "Runpod's command line isn't available in Hoista.",
        ))?;
        let mut context = vec![("environment", environment)];
        context.extend(app.map(|app| ("app", app)));
        super::cli::run(
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
        .await
    }

    /// Deploys every Flash app in `folder` to `environment`, then finds the
    /// endpoints of each app whose build there changed (or that's new).
    async fn deploy_flash(
        &self,
        token: &Secret,
        environment: &str,
        folder: &std::path::Path,
    ) -> Result<Vec<String>, EngineError> {
        let before = self.flash_environments(token).await?;
        self.command(
            token,
            None,
            environment,
            Some(folder),
            "rp flash deploy",
            json!({}),
        )
        .await?;
        let after = self.flash_environments(token).await?;
        let deployed = deployed_endpoints(&before, &after, environment);
        if deployed.is_empty() {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.not_found".into(),
                message: format!(
                    "Runpod deployed the folder, but Hoista can't see a Flash app's endpoints in its {environment} environment yet. Refresh in a moment."
                ),
            });
        }
        Ok(deployed)
    }
}

/// The environment a first Flash deploy goes to, as Runpod's own CLI picks.
const FLASH_ENVIRONMENT: &str = "production";

/// One Flash app environment and the Serverless endpoints it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FlashEnvironment {
    app: String,
    name: String,
    build: Option<String>,
    endpoints: Vec<String>,
}

/// A Flash app or environment name Hoista can pass to Runpod's CLI.
fn flash_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// A Flash app's environments, from `flashAppByName`.
fn flash_environments(app: &Value) -> Vec<FlashEnvironment> {
    let Some(name) = text(app, "name").filter(|name| flash_name(name)) else {
        return Vec::new();
    };
    app["flashEnvironments"]
        .as_array()
        .map(|environments| {
            environments
                .iter()
                .filter_map(|environment| {
                    Some(FlashEnvironment {
                        app: name.clone(),
                        name: text(environment, "name").filter(|env| flash_name(env))?,
                        build: text(environment, "activeBuildId"),
                        endpoints: environment["endpoints"]
                            .as_array()
                            .map(|endpoints| {
                                endpoints
                                    .iter()
                                    .filter_map(|endpoint| text(endpoint, "id"))
                                    .filter(|id| plain_id(id))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The endpoints a deploy to `environment` made or moved to a new build:
/// those of each app whose environment is new or runs another build now.
fn deployed_endpoints(
    before: &[FlashEnvironment],
    after: &[FlashEnvironment],
    environment: &str,
) -> Vec<String> {
    let mut endpoints: Vec<String> = after
        .iter()
        .filter(|now| now.name == environment)
        .filter(|now| {
            !before
                .iter()
                .any(|then| then.app == now.app && then.name == now.name && then.build == now.build)
        })
        .flat_map(|now| now.endpoints.iter().map(|id| format!("endpoint:{id}")))
        .collect();
    endpoints.sort();
    endpoints.dedup();
    endpoints
}

fn manifest() -> &'static super::manifest::Manifest {
    super::manifest::builtin("runpod")
}

/// One found thing as a resource.
fn resource(kind: Kind, item: &Value) -> Result<RemoteResource, EngineError> {
    let id = text(item, "id")
        .filter(|id| plain_id(id))
        .ok_or_else(invalid)?;
    let name = text(item, "name").unwrap_or_else(|| id.clone());
    Ok(match kind {
        Kind::Pod => RemoteResource {
            reference: format!("pod:{id}"),
            name,
            kind: ResourceKind::Service,
            region: text(item, "dataCenterId"),
            status: pod_status(item["status"].as_str().unwrap_or_default()),
            url: pod_url(item),
            group: None,
        },
        Kind::Endpoint => RemoteResource {
            reference: format!("endpoint:{id}"),
            name,
            kind: ResourceKind::Service,
            region: None,
            status: ResourceStatus::Live,
            url: endpoint_url(item),
            group: None,
        },
        Kind::Volume => RemoteResource {
            reference: format!("volume:{id}"),
            name,
            kind: ResourceKind::Bucket,
            region: text(item, "dataCenter"),
            status: ResourceStatus::Live,
            url: None,
            group: None,
        },
    })
}

/// When Runpod says it was made, as RFC 3339. Runpod writes either RFC 3339
/// or Go's default form, `2024-07-12 19:14:40.144 +0000 UTC`.
fn when(value: &Value) -> Option<String> {
    let said = text(value, "createdAt")?;
    let at = chrono::DateTime::parse_from_rfc3339(&said)
        .or_else(|_| {
            chrono::DateTime::parse_from_str(
                said.trim_end_matches(" UTC"),
                "%Y-%m-%d %H:%M:%S%.f %z",
            )
        })
        .ok()?;
    Some(
        at.with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
}

/// An endpoint's releases, newest first: the newest is what runs.
fn releases(history: &Value) -> Vec<crate::application::ports::providers::ProviderDeployment> {
    use crate::application::ports::providers::{ProviderDeployState, ProviderDeployment};
    let rows = history["releases"].as_array().cloned().unwrap_or_default();
    rows.iter()
        .take(MAX_DEPLOYMENTS)
        .enumerate()
        .filter_map(|(index, row)| {
            let at = when(row)?;
            let live = index == 0;
            let built = row["source"].as_str() == Some("GIT_BUILD");
            Some(ProviderDeployment {
                id: text(row, "id")?,
                status: if live {
                    "LIVE".into()
                } else {
                    "REPLACED".into()
                },
                // Each went live; the older ones were later replaced.
                state: ProviderDeployState::Succeeded,
                created_at: at.clone(),
                url: None,
                commit: None,
                message: Some(match (built, row["version"].as_u64()) {
                    (true, Some(version)) => format!("Version {version}, built from GitHub"),
                    (true, None) => "Built from GitHub".into(),
                    (false, Some(version)) => format!("Version {version}"),
                    (false, None) => "Configuration change".into(),
                }),
                branch: None,
                repo: None,
                finished_at: Some(at),
                can_roll_back: None,
                author: None,
                provider_page: None,
            })
        })
        .collect()
}

/// A pod's changes from Runpod's audit log, newest first: its creation,
/// each change to its settings and each reset. Starting and stopping a pod
/// isn't a deploy, so those are left out. A change's details can hold
/// variable values, so only the names of the settings it changed are kept.
fn pod_changes(
    log: &Value,
    pod: &str,
) -> Vec<crate::application::ports::providers::ProviderDeployment> {
    use crate::application::ports::providers::{ProviderDeployState, ProviderDeployment};
    let mut rows: Vec<&Value> = log["auditLogs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            row["resourceId"]
                .as_str()
                .is_some_and(|ids| ids.split(',').any(|id| id.trim() == pod))
                && matches!(row["action"].as_str(), Some("create" | "update" | "reset"))
        })
        .collect();
    rows.sort_by(|a, b| b["timestamp"].as_str().cmp(&a["timestamp"].as_str()));
    rows.iter()
        .take(MAX_DEPLOYMENTS)
        .enumerate()
        .filter_map(|(index, row)| {
            let at = chrono::DateTime::parse_from_rfc3339(row["timestamp"].as_str()?)
                .ok()?
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let action = row["action"].as_str()?;
            let changed: Vec<String> = row["value"]
                .as_str()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| {
                    value
                        .as_object()
                        .map(|fields| fields.keys().cloned().collect::<Vec<_>>())
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|key| key != "auditChunk" && key.len() <= MAX_FIELD_CHARS)
                .take(MAX_FIELDS_NAMED)
                .collect();
            let message = match action {
                "create" => "Pod created".to_owned(),
                "reset" => "Reset: a fresh container from its image".to_owned(),
                _ if changed.is_empty() => "Settings changed".to_owned(),
                _ => format!("Changed {}", changed.join(", ")),
            };
            Some(ProviderDeployment {
                id: format!("audit-{action}-{}", at.replace([':', '.'], "")),
                status: if index == 0 {
                    "LIVE".into()
                } else {
                    "REPLACED".into()
                },
                state: ProviderDeployState::Succeeded,
                created_at: at.clone(),
                url: None,
                commit: None,
                message: Some(message),
                branch: None,
                repo: None,
                finished_at: Some(at),
                can_roll_back: None,
                author: row["email"]
                    .as_str()
                    .filter(|email| !email.is_empty())
                    .map(str::to_owned),
                provider_page: None,
            })
        })
        .collect()
}

/// A resource's creation as its one deploy, when it says when.
fn created(
    found: &Value,
    note: &str,
) -> Option<crate::application::ports::providers::ProviderDeployment> {
    use crate::application::ports::providers::{ProviderDeployState, ProviderDeployment};
    let at = when(found)?;
    Some(ProviderDeployment {
        id: format!("created-{}", text(found, "id")?),
        status: "CREATED".into(),
        state: ProviderDeployState::Succeeded,
        created_at: at.clone(),
        url: None,
        commit: None,
        message: Some(note.to_owned()),
        branch: None,
        repo: None,
        finished_at: Some(at),
        can_roll_back: None,
        author: None,
        provider_page: None,
    })
}

/// The start of the month a time is in, as Runpod bills by the calendar month.
pub(super) fn month_start(now: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    use chrono::{Datelike, TimeZone};
    chrono::Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now)
}

#[async_trait::async_trait]
impl ProviderAdapter for Runpod {
    fn provider(&self) -> Provider {
        Provider::Runpod
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        Ok(vec![AccountChoice {
            external_id: self.myself(token).await?,
            name: "Runpod account".into(),
        }])
    }

    async fn verify(&self, token: &Secret, account: &str) -> Result<Vec<String>, EngineError> {
        if !plain_id(account) {
            return Err(EngineError::Invalid("A Runpod account is required."));
        }
        if self.myself(token).await? != account {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.token_scope".into(),
                message: "This key is for a different Runpod account than the one connected. \
                          Connect it as a new account."
                    .into(),
            });
        }
        self.http
            .json(
                Provider::Runpod,
                token,
                Method::GET,
                &["v2", "pods"],
                &[("limit", "1")],
                None,
            )
            .await?;
        Ok(vec!["Reads pods (verified by request)".into()])
    }

    /// A Runpod Flash app from a folder, or a pod running an image, on one
    /// GPU of the type given (a CPU pod without one), with its port public
    /// through Runpod's proxy.
    async fn provider_add(
        &self,
        token: &Secret,
        account: &str,
        _project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        use crate::application::ports::providers::ProviderNew;
        match *what {
            ProviderNew::Folder { name, folder } => {
                self.provider_new_from_folder(token, account, name, folder)
                    .await
            }
            ProviderNew::Image {
                name,
                image,
                gpu,
                cpu,
                port,
                args,
            } => {
                let body = pod_body(name, image, gpu, cpu, port, args)?;
                let value = self
                    .http
                    .json(
                        Provider::Runpod,
                        token,
                        Method::POST,
                        &["v2", "pods"],
                        &[],
                        Some(&body),
                    )
                    .await?;
                let found = resource(Kind::Pod, &value)?;
                Ok((
                    vec![found.reference],
                    match (gpu, cpu) {
                        (Some(gpu), _) => {
                            format!("Started the pod {name} running {image} on a {gpu}")
                        }
                        (None, Some(cpu)) => format!(
                            "Started the pod {name} running {image} on {} {} vCPUs",
                            cpu.vcpus, cpu.flavor
                        ),
                        (None, None) => format!("Started the pod {name} running {image}"),
                    },
                ))
            }
            _ => Err(crate::application::ports::providers::unsupported()),
        }
    }

    async fn discover(
        &self,
        token: &Secret,
        _account: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        for pod in self.every(token, "pods", "pods").await? {
            if pod["status"].as_str() == Some("TERMINATED") {
                continue;
            }
            resources.push(resource(Kind::Pod, &pod)?);
        }
        for endpoint in self.every(token, "serverless", "endpoints").await? {
            resources.push(resource(Kind::Endpoint, &endpoint)?);
        }
        for volume in self
            .every(token, "network-volumes", "networkVolumes")
            .await?
        {
            resources.push(resource(Kind::Volume, &volume)?);
        }
        if resources.len() > MAX_DISCOVERED_RESOURCES {
            return Err(invalid());
        }
        Ok(resources)
    }

    /// A pod is as its status says; a deleted one is gone.
    async fn observe_resource(
        &self,
        token: &Secret,
        _account: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let (kind, id) = split(&resource.provider_ref)?;
        let answer = self
            .http
            .json(
                Provider::Runpod,
                token,
                Method::GET,
                &["v2", kind.path(), id],
                &[],
                None,
            )
            .await;
        let found = match answer {
            Ok(found) => found,
            Err(EngineError::Provider { code, .. }) if code == "provider.not_found" => {
                return Ok(Observed {
                    status: ResourceStatus::Stopped,
                    url: None,
                    kind: Some(resource.kind),
                });
            }
            Err(error) => return Err(error),
        };
        let seen = self::resource(kind, &found)?;
        Ok(Observed {
            status: if kind == Kind::Pod && found["status"].as_str() == Some("TERMINATED") {
                ResourceStatus::Stopped
            } else {
                seen.status
            },
            url: seen.url,
            kind: Some(seen.kind),
        })
    }

    /// What Runpod records of a resource's deploys: an endpoint's releases
    /// (each configuration or build it went live with), or a pod's start from
    /// its image, and the address it serves on.
    async fn provider_details(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        use crate::application::ports::providers::{
            ProviderDetails, ProviderDomain, ProviderDomainState,
        };
        let (kind, id) = split(reference)?;
        let found = self
            .http
            .json(
                Provider::Runpod,
                token,
                Method::GET,
                &["v2", kind.path(), id],
                &[],
                None,
            )
            .await?;
        let (deployments, url) = match kind {
            Kind::Endpoint => {
                let history = self
                    .http
                    .json(
                        Provider::Runpod,
                        token,
                        Method::GET,
                        &["v2", "serverless", id, "releases"],
                        &[("limit", "20")],
                        None,
                    )
                    .await?;
                let mut deployments = releases(&history);
                if deployments.is_empty() {
                    deployments.extend(created(&found, "Endpoint created"));
                }
                (deployments, endpoint_url(&found))
            }
            Kind::Pod => {
                let note = text(&found, "image").map_or_else(
                    || "Pod created".to_owned(),
                    |image| format!("Pod created from {image}"),
                );
                // Its changes come from the audit log; without that (an older
                // key, or nothing kept that far back), only its creation.
                let log = self
                    .http
                    .json(
                        Provider::Runpod,
                        token,
                        Method::GET,
                        &["v2", "audit-logs"],
                        &[("search", id), ("limit", "200")],
                        None,
                    )
                    .await
                    .unwrap_or(Value::Null);
                let mut deployments = pod_changes(&log, id);
                // The creation says which image; the audit row may not.
                match deployments
                    .iter_mut()
                    .find(|deploy| deploy.id.starts_with("audit-create"))
                {
                    Some(first) => first.message = Some(note),
                    None => deployments.extend(created(&found, &note)),
                }
                if let Some(newest) = deployments.first_mut() {
                    newest.status = "LIVE".into();
                }
                (deployments, pod_url(&found))
            }
            Kind::Volume => (Vec::new(), None),
        };
        let domains = url
            .map(|url| ProviderDomain {
                name: url
                    .trim_start_matches("https://")
                    .trim_end_matches('/')
                    .to_owned(),
                custom: false,
                state: Some(ProviderDomainState::Working),
                records: Vec::new(),
                port: None,
                certificate: None,
                dns: Vec::new(),
            })
            .into_iter()
            .collect();
        Ok(ProviderDetails {
            deployments,
            variable_names: found["env"]
                .as_object()
                .map(|env| env.keys().cloned().collect()),
            shared_variable_names: None,
            domains,
            references: Vec::new(),
            sleeps: (kind == Kind::Endpoint).then_some(true),
            config: None,
            volumes: Vec::new(),
            tcp_proxies: Vec::new(),
            private_domain: None,
            addresses: Vec::new(),
        })
    }

    /// Terminates a pod, or deletes a Serverless endpoint or network volume.
    async fn delete(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let (kind, id) = split(reference)?;
        self.http
            .json(
                Provider::Runpod,
                token,
                Method::DELETE,
                &["v2", kind.path(), id],
                &[],
                None,
            )
            .await?;
        Ok(())
    }

    /// What Runpod has billed this calendar month, as its billing records say.
    async fn provider_bill(
        &self,
        token: &Secret,
        _account: &str,
    ) -> Result<Option<ProviderBill>, EngineError> {
        // Once Runpod has refused both asks, stop asking until the daemon restarts.
        if BILL_REFUSED.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
        }
        let now = chrono::Utc::now();
        let start = month_start(now).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let end = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        // Runpod refuses an open-ended range, so the range is closed at now;
        // a month bucket is the fallback for accounts that refuse that too.
        // A bill Runpod won't give is no bill, not an error on every refresh.
        let asks: [&[(&str, &str)]; 2] = [
            &[
                ("startTime", start.as_str()),
                ("endTime", end.as_str()),
                ("bucketSize", "day"),
            ],
            &[("bucketSize", "month"), ("lastN", "1")],
        ];
        let mut answer = None;
        for query in asks {
            match self
                .http
                .json(
                    Provider::Runpod,
                    token,
                    Method::GET,
                    &["v2", "billing"],
                    query,
                    None,
                )
                .await
            {
                Ok(value) => {
                    answer = Some(value);
                    break;
                }
                Err(EngineError::Provider { code, .. }) if code == "provider.rejected" => {}
                Err(error) => return Err(error),
            }
        }
        let Some(answer) = answer else {
            BILL_REFUSED.store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(None);
        };
        let total = answer["metadata"]["totals"]["totalAmount"]
            .as_f64()
            .or_else(|| {
                // The month bucket lists its one period's amount.
                answer
                    .as_array()
                    .or_else(|| answer["data"].as_array())
                    .and_then(|rows| rows.first())
                    .and_then(|row| row["amount"].as_f64())
            });
        let Some(total) = total else {
            return Ok(None);
        };
        if !total.is_finite() || total < 0.0 {
            return Err(invalid());
        }
        let cents = crate::units::cents(total);
        Ok(Some(ProviderBill {
            billed: Money {
                cents,
                currency: "USD".into(),
            },
            period_start: Some(start),
            period_end: None,
            soft_limit: None,
            hard_limit: None,
            over_limit: false,
            customer_id: None,
        }))
    }

    /// A pod's container and system logs, or an endpoint's workers' (the
    /// first three), within the range.
    async fn log_history(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        use crate::domain::logs::{LogEntry, LogHistory, LogLevel};
        let (from, to) = query.milliseconds().map_err(EngineError::Invalid)?;
        let (kind, id) = split(reference)?;
        let mut lines = Vec::new();
        match kind {
            Kind::Pod => {
                lines = self
                    .stream_logs(token, &["v2", "pods", id, "logs"], Some(&query.from))
                    .await?;
            }
            Kind::Endpoint => {
                let workers = self
                    .http
                    .json(
                        Provider::Runpod,
                        token,
                        Method::GET,
                        &["v2", "serverless", id, "workers"],
                        &[],
                        None,
                    )
                    .await?;
                for worker in workers["workers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(MAX_WORKERS_READ)
                {
                    let Some(worker) = worker["id"].as_str().filter(|id| plain_id(id)) else {
                        continue;
                    };
                    lines.extend(
                        self.stream_logs(
                            token,
                            &["v2", "serverless", id, "workers", worker, "logs"],
                            Some(&query.from),
                        )
                        .await?,
                    );
                }
            }
            Kind::Volume => return Err(crate::application::ports::providers::unsupported()),
        }
        let scanner = crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
        let observed = chrono::Utc::now().to_rfc3339();
        let needle = query.query.to_lowercase();
        let mut items: Vec<LogEntry> = lines
            .into_iter()
            .filter(|(at, _, line)| {
                let when = at
                    .as_deref()
                    .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| at.timestamp_millis());
                when.is_none_or(|when| when >= from && when <= to)
                    && !line.trim().is_empty()
                    && (needle.is_empty() || line.to_lowercase().contains(&needle))
            })
            .map(|(at, source, line)| {
                let message = crate::domain::logs::clipped(
                    scanner.redact(line.trim_end(), std::slice::from_ref(token)),
                );
                let lower = message.to_lowercase();
                let level = if lower.contains("error") || lower.contains("traceback") {
                    LogLevel::Error
                } else if lower.contains("warn") {
                    LogLevel::Warn
                } else {
                    LogLevel::Info
                };
                LogEntry {
                    timestamp: at,
                    observed_at: observed.clone(),
                    level,
                    source,
                    message,
                }
            })
            .collect();
        items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        // The newest hundred.
        let skip = items.len().saturating_sub(100);
        items.drain(..skip);
        Ok(LogHistory {
            items,
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: false,
        })
    }

    /// What Runpod reports right now: a pod's GPU, CPU and memory use, or
    /// an endpoint's workers by state. Runpod keeps no history of these,
    /// so each chart is one point, now.
    async fn provider_metrics(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        _step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::{chart, line, stamp};
        use crate::application::ports::providers::{
            MetricSummary as S, MetricUnit as U, ProviderPoint,
        };
        let (kind, id) = split(reference)?;
        let now = stamp(chrono::Utc::now().min(to));
        let point = |value: Option<f64>| {
            value
                .filter(|value| value.is_finite())
                .map(|value| {
                    vec![ProviderPoint {
                        at: now.clone(),
                        value,
                    }]
                })
                .unwrap_or_default()
        };
        let charts = match kind {
            Kind::Pod => {
                let pod = self
                    .http
                    .json(
                        Provider::Runpod,
                        token,
                        Method::GET,
                        &["v2", "pods", id],
                        &[],
                        None,
                    )
                    .await?;
                let runtime = &pod["runtime"];
                let gpus: Vec<&Value> = runtime["gpus"].as_array().into_iter().flatten().collect();
                vec![
                    chart(
                        "gpu",
                        "GPU use now",
                        U::Percent,
                        S::Latest,
                        gpus.iter()
                            .enumerate()
                            .map(|(index, gpu)| {
                                line(&format!("GPU {index}"), point(gpu["util"].as_f64()))
                            })
                            .collect(),
                    ),
                    chart(
                        "gpu_memory",
                        "GPU memory now",
                        U::Percent,
                        S::Latest,
                        gpus.iter()
                            .enumerate()
                            .map(|(index, gpu)| {
                                line(&format!("GPU {index}"), point(gpu["memoryUtil"].as_f64()))
                            })
                            .collect(),
                    ),
                    chart(
                        "cpu",
                        "CPU use now",
                        U::Percent,
                        S::Latest,
                        vec![line("CPU", point(runtime["cpu"]["util"].as_f64()))],
                    ),
                    chart(
                        "memory",
                        "Memory use now",
                        U::Percent,
                        S::Latest,
                        vec![line("Memory", point(runtime["memory"]["util"].as_f64()))],
                    ),
                ]
            }
            Kind::Endpoint => {
                let workers = self
                    .http
                    .json(
                        Provider::Runpod,
                        token,
                        Method::GET,
                        &["v2", "serverless", id, "workers"],
                        &[],
                        None,
                    )
                    .await?;
                let summary = &workers["summary"];
                vec![chart(
                    "workers",
                    "Workers now",
                    U::Count,
                    S::Latest,
                    ["running", "idle", "initializing", "throttled", "unhealthy"]
                        .iter()
                        .map(|state| {
                            let label = format!("{}{}", state[..1].to_uppercase(), &state[1..]);
                            line(&label, point(summary[*state].as_f64()))
                        })
                        .collect(),
                )]
            }
            Kind::Volume => return Err(crate::application::ports::providers::unsupported()),
        };
        Ok(super::metrics::metrics(from, to, charts, &[], "Runpod"))
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        manifest().operations()
    }

    fn prepare_tools(&self) {
        if let Some(cli) = manifest().cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    async fn provider_new_from_folder(
        &self,
        token: &Secret,
        _account: &str,
        name: &str,
        folder: &std::path::Path,
    ) -> Result<(Vec<String>, String), EngineError> {
        let endpoints = self.deploy_flash(token, FLASH_ENVIRONMENT, folder).await?;
        Ok((
            endpoints,
            format!(
                "Deployed {name} to Runpod Flash's {FLASH_ENVIRONMENT} environment from its folder"
            ),
        ))
    }

    async fn provider_upload(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
        folder: &std::path::Path,
        _message: Option<&str>,
    ) -> Result<String, EngineError> {
        let (kind, id) = split(reference)?;
        if kind != Kind::Endpoint {
            return Err(EngineError::Invalid(
                "Only a Runpod Flash app's endpoint deploys from a folder.",
            ));
        }
        let home = self.flash_home(token, id).await?;
        self.deploy_flash(token, &home.name, folder).await?;
        Ok(format!(
            "Deployed {} to Runpod Flash's {} environment from its folder",
            home.app, home.name
        ))
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        super::manifest::builtin("runpod").actions.clone()
    }

    /// Runpod's API, aimed at the pod, endpoint or volume.
    async fn provider_operation(
        &self,
        token: &Secret,
        _account: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        // Runpod's own CLI, for Flash: aimed at the app and environment the
        // endpoint belongs to.
        if manifest()
            .cli
            .as_ref()
            .is_some_and(|cli| super::cli::has(cli, name))
        {
            let Some(reference) = reference else {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "provider.needs_folder".into(),
                    message: format!(
                        "{name} works on a Flash app Hoista watches: give the resource_id of one of its endpoints. For a first deploy, call create_project_at_provider with folder."
                    ),
                });
            };
            let (kind, id) = split(reference)?;
            if kind != Kind::Endpoint {
                return Err(EngineError::Invalid(
                    "Runpod Flash commands work on a Flash app's endpoint.",
                ));
            }
            let home = self.flash_home(token, id).await?;
            return self
                .command(token, Some(&home.app), &home.name, folder, name, args)
                .await;
        }
        let mut context = Vec::new();
        if let Some(reference) = reference {
            let (kind, id) = split(reference)?;
            // Logs stream; read a window of them instead of one JSON answer.
            if kind == Kind::Pod && name == "getPodLogs" {
                let lines = self
                    .stream_logs(token, &["v2", "pods", id, "logs"], None)
                    .await?;
                let mut answer =
                    json!(lines
                    .into_iter()
                    .map(|(at, source, line)| json!({"ts": at, "source": source, "line": line}))
                    .collect::<Vec<_>>());
                super::api_catalog::clean(&mut answer, token)?;
                return Ok(answer);
            }
            if kind == Kind::Endpoint && name == "getWorkerLogs" {
                let worker = args["workerId"]
                    .as_str()
                    .filter(|worker| plain_id(worker))
                    .ok_or(EngineError::Invalid("Choose a worker, from Workers."))?;
                let lines = self
                    .stream_logs(
                        token,
                        &["v2", "serverless", id, "workers", worker, "logs"],
                        None,
                    )
                    .await?;
                let mut answer =
                    json!(lines
                    .into_iter()
                    .map(|(at, source, line)| json!({"ts": at, "source": source, "line": line}))
                    .collect::<Vec<_>>());
                super::api_catalog::clean(&mut answer, token)?;
                return Ok(answer);
            }
            context.push((kind.prefix(), id));
        }
        super::api_catalog::run_manifest(
            &self.http,
            Provider::Runpod,
            super::manifest::builtin("runpod"),
            token,
            &context,
            name,
            args,
        )
        .await
    }
}

/// What Runpod's create-pod call takes for an image on a GPU or on CPUs.
fn pod_body(
    name: &str,
    image: &str,
    gpu: Option<&str>,
    cpu: Option<crate::application::ports::providers::CpuMachine<'_>>,
    port: Option<u16>,
    args: Option<&str>,
) -> Result<Value, EngineError> {
    if gpu.is_some() && cpu.is_some() {
        return Err(EngineError::Invalid(
            "A pod runs on a GPU or on CPUs: give gpu or cpu, not both.",
        ));
    }
    let mut body = json!({
        "name": name,
        "image": image,
        "disk": 50,
        "cloud": "SECURE",
    });
    if let Some(gpu) = gpu {
        body["gpu"] = json!({"id": gpu, "count": 1});
    }
    if let Some(cpu) = cpu {
        if !cpu.vcpus.is_power_of_two() {
            return Err(EngineError::Invalid(
                "Give vcpus as a power of two, such as 2, 4 or 8.",
            ));
        }
        body["cpu"] = json!({"id": cpu.flavor, "vcpuCount": cpu.vcpus});
    }
    if let Some(port) = port {
        body["ports"] = json!([format!("{port}/http")]);
    }
    if let Some(args) = args {
        body["args"] = args.into();
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pod_runs_on_a_gpu_or_on_cpus() {
        use crate::application::ports::providers::CpuMachine;
        let cpu = CpuMachine {
            flavor: "cpu3c",
            vcpus: 4,
        };
        let body = pod_body("api", "nginx:latest", None, Some(cpu), Some(80), None).unwrap();
        assert_eq!(body["cpu"], json!({"id": "cpu3c", "vcpuCount": 4}));
        assert!(body.get("gpu").is_none());
        assert_eq!(body["ports"], json!(["80/http"]));
        let body = pod_body("llm", "vllm", Some("NVIDIA A40"), None, None, None).unwrap();
        assert_eq!(body["gpu"], json!({"id": "NVIDIA A40", "count": 1}));
        assert!(pod_body("x", "i", Some("A40"), Some(cpu), None, None).is_err());
        let odd = CpuMachine {
            flavor: "cpu3c",
            vcpus: 3,
        };
        assert!(pod_body("x", "i", None, Some(odd), None, None).is_err());
    }

    #[test]
    fn flash_apps_read_as_environments_and_their_endpoints() {
        let app = json!({
            "name": "whisper",
            "flashEnvironments": [
                {
                    "name": "production",
                    "activeBuildId": "b2",
                    "endpoints": [{"id": "ep1"}, {"id": "../x"}],
                },
                {"name": "-bad", "activeBuildId": "b1", "endpoints": []},
            ],
        });
        assert_eq!(
            flash_environments(&app),
            vec![FlashEnvironment {
                app: "whisper".into(),
                name: "production".into(),
                build: Some("b2".into()),
                endpoints: vec!["ep1".into()],
            }]
        );
        assert!(flash_environments(&json!({"name": "--app"})).is_empty());
    }

    #[test]
    fn a_deploy_finds_the_endpoints_whose_build_changed_or_are_new() {
        let env = |app: &str, name: &str, build: &str, endpoints: &[&str]| FlashEnvironment {
            app: app.into(),
            name: name.into(),
            build: Some(build.into()),
            endpoints: endpoints.iter().map(|id| (*id).to_owned()).collect(),
        };
        let before = vec![
            env("whisper", "production", "b1", &["ep1"]),
            env("resizer", "production", "r1", &["ep9"]),
        ];
        let after = vec![
            env("whisper", "production", "b2", &["ep1", "ep2"]),
            env("resizer", "production", "r1", &["ep9"]),
            env("chat", "production", "c1", &["ep5"]),
            env("chat", "staging", "c0", &["ep6"]),
        ];
        assert_eq!(
            deployed_endpoints(&before, &after, "production"),
            vec!["endpoint:ep1", "endpoint:ep2", "endpoint:ep5"]
        );
        assert!(deployed_endpoints(&after, &after, "production").is_empty());
    }

    #[test]
    fn releases_and_creation_read_as_deploys() {
        let history = serde_json::json!({
            "releases": [
                {
                    "id": "r2",
                    "source": "GIT_BUILD",
                    "version": 2,
                    "createdAt": "2026-11-06T10:00:00Z",
                    "workerCount": 1,
                    "diff": [],
                },
                {
                    "id": "r1",
                    "source": "MANUAL",
                    "version": 1,
                    "createdAt": "2026-11-05 09:30:00.5 +0000 UTC",
                    "workerCount": 0,
                    "diff": [],
                },
            ],
        });
        let deploys = releases(&history);
        assert_eq!(deploys.len(), 2);
        assert_eq!(deploys[0].created_at, "2026-11-06T10:00:00.000Z");
        assert_eq!(
            deploys[0].message.as_deref(),
            Some("Version 2, built from GitHub")
        );
        assert_eq!(deploys[1].created_at, "2026-11-05T09:30:00.500Z");
        assert_eq!(deploys[1].status, "REPLACED");
        let pod = created(
            &serde_json::json!({"id": "abc", "createdAt": "2026-11-04T08:00:00Z"}),
            "Pod created",
        );
        assert_eq!(
            pod.map(|deploy| deploy.created_at),
            Some("2026-11-04T08:00:00.000Z".into())
        );
    }

    #[test]
    fn log_lines_read_in_each_shape() {
        assert_eq!(
            log_line(r#"{"source":"system","line":"pulling image","ts":"2026-11-06T10:00:00Z"}"#),
            Some((
                Some("2026-11-06T10:00:00Z".into()),
                "system".into(),
                "pulling image".into()
            ))
        );
        assert_eq!(
            log_line(r#"{"message":"ready","timestamp":"2026-11-06T10:00:01Z"}"#),
            Some((
                Some("2026-11-06T10:00:01Z".into()),
                "container".into(),
                "ready".into()
            ))
        );
        assert_eq!(
            log_line("plain text"),
            Some((None, "container".into(), "plain text".into()))
        );
        assert_eq!(log_line("   "), None);
    }

    #[test]
    fn a_pods_changes_come_from_the_audit_log_without_values() {
        let log = serde_json::json!({
            "auditLogs": [
                {
                    "resourceType": "pod",
                    "resourceId": "pod1",
                    "action": "create",
                    "timestamp": "2026-11-06T17:26:00Z",
                    "email": "sam@example.com",
                    "value": null,
                    "actorId": null,
                },
                {
                    "resourceType": "pod",
                    "resourceId": "pod1",
                    "action": "start",
                    "timestamp": "2026-11-06T17:33:00Z",
                    "email": null,
                    "value": null,
                    "actorId": null,
                },
                {
                    "resourceType": "pod",
                    "resourceId": "pod1",
                    "action": "update",
                    "timestamp": "2026-11-06T17:58:00Z",
                    "email": null,
                    "value": "{\"dockerStartCmd\":\"python app.py\",\"env\":{\"KEY\":\"secret-value\"}}",
                    "actorId": null,
                },
                {
                    "resourceType": "pod",
                    "resourceId": "other",
                    "action": "update",
                    "timestamp": "2026-11-06T18:00:00Z",
                    "email": null,
                    "value": null,
                    "actorId": null,
                },
            ],
        });
        let deploys = pod_changes(&log, "pod1");
        assert_eq!(deploys.len(), 2);
        assert_eq!(
            deploys[0].message.as_deref(),
            Some("Changed dockerStartCmd, env")
        );
        assert!(
            !serde_json::to_string(&deploys)
                .unwrap()
                .contains("secret-value")
        );
        assert_eq!(deploys[1].author.as_deref(), Some("sam@example.com"));
    }

    #[test]
    fn references_name_a_kind_and_a_plain_id() {
        assert_eq!(split("pod:abc123").unwrap(), (Kind::Pod, "abc123"));
        assert_eq!(split("endpoint:e-1").unwrap(), (Kind::Endpoint, "e-1"));
        assert_eq!(split("volume:v_1").unwrap(), (Kind::Volume, "v_1"));
        assert!(split("pod:a/../b").is_err());
        assert!(split("machine:x").is_err());
        assert!(split("plain").is_err());
    }

    #[test]
    fn pods_read_as_services_with_their_proxy_address() {
        let pod = json!({
            "id": "abc",
            "name": "trainer",
            "status": "RUNNING",
            "ports": ["22/tcp", "8888/http"],
            "dataCenterId": "EU-RO-1",
        });
        let found = resource(Kind::Pod, &pod).unwrap();
        assert_eq!(found.reference, "pod:abc");
        assert_eq!(found.kind, ResourceKind::Service);
        assert_eq!(found.status, ResourceStatus::Live);
        assert_eq!(
            found.url.as_deref(),
            Some("https://abc-8888.proxy.runpod.net")
        );
        assert_eq!(pod_status("EXITED"), ResourceStatus::Stopped);
        assert_eq!(pod_status("STARTING"), ResourceStatus::Deploying);
        let endpoint = json!({"id":"e1","name":"sdxl","requestUrls":{"run":"https://api.runpod.ai/v2/e1/run"}});
        assert_eq!(
            resource(Kind::Endpoint, &endpoint).unwrap().url.as_deref(),
            Some("https://api.runpod.ai/v2/e1")
        );
        let volume = json!({"id":"v1","name":"models","dataCenter":"US-KS-2","size":100});
        assert_eq!(
            resource(Kind::Volume, &volume).unwrap().kind,
            ResourceKind::Bucket
        );
    }

    #[test]
    fn the_api_is_aimed_at_the_resource_and_keeps_credentials_out() {
        let manifest = super::super::manifest::builtin("runpod");
        assert!(manifest.api_operation("getPod").unwrap().aimed_at("pod"));
        assert!(
            manifest
                .api_operation("getEndpoint")
                .unwrap()
                .aimed_at("endpoint")
        );
        assert!(
            manifest
                .api_operation("getNetworkVolume")
                .unwrap()
                .aimed_at("volume")
        );
        for name in [
            "listSecrets",
            "createSecret",
            "updateSshKeys",
            "listRegistries",
            "deletePod",
            "deleteEndpoint",
            "listOrganizationUsers",
        ] {
            assert!(manifest.api_operation(name).is_none(), "{name}");
        }
        // Making pods, endpoints and templates is how something gets
        // deployed on Runpod, with the container's own settings.
        for name in ["createPod", "createEndpoint", "createTemplate"] {
            let op = manifest.api_operation(name).unwrap();
            for field in ["image", "env", "ports"] {
                assert!(op.takes(field), "{name} takes {field}");
            }
        }
        // A pod action can terminate it for good.
        let action = manifest
            .operations()
            .into_iter()
            .find(|op| op.name == "podAction")
            .unwrap();
        assert_eq!(
            action.risk,
            crate::application::ports::providers::ProviderRisk::Destroy
        );
    }
}
