//! Render services, cron jobs, Postgres and Key Value in a workspace, through
//! Render's API and, where the API falls short, its CLI. The API catalog
//! (`render/api_ops.json`) is generated from Render's OpenAPI spec.

use super::http::ProviderHttp;
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{Observed, ProviderAdapter, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::Value;

/// Render's API adapter, scoped to one workspace (Render calls it an owner).
pub struct Render {
    http: ProviderHttp,
}

/// Render's dashboard.
const MAX_TEXT_CHARS: usize = 256;
const MB_PER_GB: u64 = 1024;
/// Render's finest chart resolution.
const MIN_STEP_SECS: u32 = 30;
const DASHBOARD: &str = "https://dashboard.render.com";

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M18.263.007c-3.121-.147-5.744 2.109-6.192 5.082-.018.138-.045.272-.067.405-.696 3.703-3.936 6.507-7.827 6.507-1.388 0-2.691-.356-3.825-.979a.2024.2024 0 0 0-.302.178V24H12v-8.999c0-1.656 1.338-3 2.987-3h2.988c3.382 0 6.103-2.817 5.97-6.244-.12-3.084-2.61-5.603-5.682-5.75" fill="currentColor"/></svg>"##;

/// Render's entry in the provider list.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Render,
    logo: LOGO,
    capabilities: Capabilities {
        provider_variables: true,
        discover: true,
        provider_operations: true,
        provider_details: true,
        metrics: true,
        delete: true,
        new_project: true,
        new_database: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://api.render.com/v1/",
        hosts: &["api.render.com"],
        explains_refusals: true,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["owners"],
            query: &[("limit", "100")],
            page: None,
            cursor: None,
        },
        rows: &[],
        item: &["owner"],
        id: "id",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(Render::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of the provider for the conformance suite: enough to connect and
/// find one resource.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "tea-1",
    resource: "srv-1",
    kind: crate::domain::model::ResourceKind::Web,
    routes: &[
        super::conformance::Route {
            path: "/owners",
            body: "",
            answer: r#"[{"cursor":"c1","owner":{"id":"tea-1","name":"Sam","type":"team"}}]"#,
        },
        super::conformance::Route {
            path: "/services?",
            body: "",
            answer: r#"[{"cursor":"c1","service":{"id":"srv-1","name":"site","type":"web_service","suspended":"not_suspended","serviceDetails":{"url":"https://site.onrender.com","region":"oregon"}}}]"#,
        },
    ],
    empty: r#"[]"#,
};

fn invalid() -> EngineError {
    super::answers::incomplete("Render")
}

fn text(value: &Value, key: &str) -> Result<String, EngineError> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= MAX_TEXT_CHARS)
        .map(str::to_owned)
        .ok_or_else(invalid)
}

/// Render's IDs: letters, digits and dashes (`srv-…`, `dpg-…`, `red-…`).
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A service's id: Postgres and key-value stores have no variables.
fn service_id(id: &str) -> Result<&str, EngineError> {
    if plain_id(id) && !id.starts_with("dpg-") && !id.starts_with("red-") {
        Ok(id)
    } else {
        Err(EngineError::Invalid("Expected a Render service."))
    }
}

/// What each kind of Render service is in Hoista.
fn kind_of(service_type: &str) -> ResourceKind {
    match service_type {
        "web_service" | "static_site" => ResourceKind::Web,
        "cron_job" => ResourceKind::Job,
        _ => ResourceKind::Service,
    }
}

/// A Render deploy's status as a resource status.
fn deploy_status(status: &str) -> ResourceStatus {
    match status {
        "live" => ResourceStatus::Live,
        "created"
        | "queued"
        | "build_in_progress"
        | "update_in_progress"
        | "pre_deploy_in_progress" => ResourceStatus::Deploying,
        "build_failed" | "update_failed" | "pre_deploy_failed" => ResourceStatus::Failing,
        "deactivated" | "canceled" => ResourceStatus::Stopped,
        _ => ResourceStatus::Unknown,
    }
}

/// One Render deploy as a provider deployment.
fn render_deploy(
    service: &str,
    row: &Value,
) -> Option<crate::application::ports::providers::ProviderDeployment> {
    use crate::application::ports::providers::{ProviderDeployState, ProviderDeployment};
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let status = row["status"].as_str().unwrap_or("created");
    let state = match deploy_status(status) {
        ResourceStatus::Failing => ProviderDeployState::Failed,
        ResourceStatus::Deploying => ProviderDeployState::InProgress,
        ResourceStatus::Stopped if status == "canceled" => ProviderDeployState::Stopped,
        // A deactivated deploy went live and was later replaced.
        ResourceStatus::Live | ResourceStatus::Stopped => ProviderDeployState::Succeeded,
        _ => ProviderDeployState::InProgress,
    };
    let id = text(&row["id"])?;
    Some(ProviderDeployment {
        provider_page: Some(format!("{DASHBOARD}/web/{service}/deploys/{id}")),
        status: status.to_owned(),
        state,
        created_at: text(&row["createdAt"])?,
        url: None,
        commit: text(&row["commit"]["id"]),
        message: text(&row["commit"]["message"])
            .map(|said| said.lines().next().unwrap_or("").to_owned())
            .or_else(|| text(&row["trigger"])),
        branch: None,
        repo: None,
        finished_at: text(&row["finishedAt"]),
        can_roll_back: None,
        author: None,
        id,
    })
}

/// Render keeps Postgres and Key Value apart, while Hoista calls both a
/// database: the database details action reads whichever one the resource is.
fn for_resource<'a>(name: &'a str, reference: Option<&str>) -> &'a str {
    match (name, reference) {
        ("retrievePostgres", Some(id)) if id.starts_with("red-") => "retrieveKeyValue",
        _ => name,
    }
}

impl Render {
    /// Shares the allow-listed HTTP boundary.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    async fn get(
        &self,
        token: &Secret,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<Value, EngineError> {
        self.http
            .json(Provider::Render, token, Method::GET, path, query, None)
            .await
    }

    /// Every row of one of Render's cursor-paged lists, each row's `key`
    /// object. Render pages by the last row's `cursor`.
    async fn every(
        &self,
        token: &Secret,
        path: &[&str],
        owner: &str,
        key: &str,
    ) -> Result<Vec<Value>, EngineError> {
        let mut all = Vec::new();
        let mut cursor = String::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let mut query = vec![("ownerId", owner), ("limit", "100")];
            if !cursor.is_empty() {
                query.push(("cursor", cursor.as_str()));
            }
            let value = self.get(token, path, &query).await?;
            let rows = value.as_array().ok_or_else(invalid)?;
            for row in rows {
                all.push(row.get(key).cloned().ok_or_else(invalid)?);
            }
            if rows.len() < 100 {
                return Ok(all);
            }
            let next = rows
                .last()
                .and_then(|row| row["cursor"].as_str())
                .ok_or_else(invalid)?
                .to_owned();
            if !seen.insert(next.clone()) {
                return Err(invalid());
            }
            cursor = next;
        }
        Err(invalid())
    }
}

/// One point of one of Render's series: its labels, time and value.
type RenderRow = (Vec<(String, String)>, String, f64);

/// Render's time series as `(labels, time, value)` rows: each series has
/// labels such as `statusCode` and `quantile`, and its values over time.
fn render_rows(value: &Value) -> Vec<RenderRow> {
    let mut rows = Vec::new();
    for series in value.as_array().into_iter().flatten() {
        let labels: Vec<(String, String)> = series["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|label| {
                Some((
                    label["field"].as_str()?.to_owned(),
                    label["value"].as_str()?.to_owned(),
                ))
            })
            .collect();
        let scale = bytes_in(series["unit"].as_str().unwrap_or_default());
        for point in series["values"].as_array().into_iter().flatten() {
            if let (Some(at), Some(n)) = (
                super::metrics::time_of(&point["timestamp"]),
                super::metrics::number(&point["value"]),
            ) {
                rows.push((labels.clone(), at, n * scale));
            }
        }
    }
    rows
}

/// How many bytes one of Render's size units is; 1 for anything else.
fn bytes_in(unit: &str) -> f64 {
    match unit.to_ascii_uppercase().as_str() {
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "KIB" => 1024.0,
        "MIB" => 1024.0 * 1024.0,
        "GIB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    }
}

fn label<'a>(labels: &'a [(String, String)], field: &str) -> Option<&'a str> {
    labels
        .iter()
        .find(|(name, _)| name == field)
        .map(|(_, value)| value.as_str())
}

impl Render {
    /// One of Render's metrics for the resource, or None when Render
    /// doesn't answer it for this kind of resource.
    async fn series(
        &self,
        token: &Secret,
        name: &str,
        query: &[(&str, &str)],
    ) -> Option<Vec<RenderRow>> {
        self.get(token, &["metrics", name], query)
            .await
            .ok()
            .map(|value| render_rows(&value))
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Render {
    fn provider(&self) -> Provider {
        Provider::Render
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// A web service from the repository's Dockerfile, as a new project.
    async fn provider_new_project(
        &self,
        token: &Secret,
        owner: &str,
        name: &str,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<(Vec<String>, String), EngineError> {
        self.provider_add(
            token,
            owner,
            name,
            None,
            &crate::application::ports::providers::ProviderNew::Repo {
                name,
                repo,
                branch,
                runs_as: None,
                schedule: None,
                build: crate::application::ports::providers::RepoBuild::default(),
            },
        )
        .await
    }

    /// A Render service from a GitHub repository (a web service, static
    /// site, background worker or cron job, deploying on every push), or a
    /// Postgres or Key Value (Redis-compatible) database. Web services,
    /// static sites and databases start on Render's free plan; workers and
    /// cron jobs need its Starter plan, which Render bills.
    async fn provider_add(
        &self,
        token: &Secret,
        owner: &str,
        _project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        use crate::application::ports::providers::ProviderNew;
        match *what {
            ProviderNew::Repo {
                name,
                repo,
                branch,
                runs_as,
                schedule,
                build,
            } => {
                let valid_repo = repo.split_once('/').is_some_and(|(org, rest)| {
                    !org.is_empty()
                        && !rest.is_empty()
                        && !rest.contains('/')
                        && repo
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b) || b == b'/')
                });
                if !valid_repo {
                    return Err(EngineError::Invalid(
                        "Give the GitHub repository as owner/name.",
                    ));
                }
                let runtime = build.runtime.unwrap_or("docker");
                if !matches!(
                    runtime,
                    "node" | "python" | "go" | "ruby" | "rust" | "elixir" | "docker"
                ) {
                    return Err(EngineError::Invalid(
                        "Render's runtimes: node, python, go, ruby, rust, elixir or docker.",
                    ));
                }
                if runtime != "docker" && build.start.is_none() && runs_as != Some("static") {
                    return Err(EngineError::Invalid(
                        "Render needs start_command (and usually build_command) for a runtime other than docker.",
                    ));
                }
                let env = if runtime == "docker" {
                    serde_json::json!({})
                } else {
                    serde_json::json!({
                        "buildCommand": build.build.unwrap_or(""),
                        "startCommand": build.start.unwrap_or(""),
                    })
                };
                let (kind, details) = match runs_as.unwrap_or("web") {
                    "web" => (
                        "web_service",
                        serde_json::json!({"runtime": runtime, "plan": "free", "envSpecificDetails": env}),
                    ),
                    "static" => (
                        "static_site",
                        serde_json::json!({
                            "buildCommand": build.build.unwrap_or(""),
                            "publishPath": build.publish.ok_or(EngineError::Invalid(
                                "Give publish_dir, the folder the build writes the site to.",
                            ))?,
                        }),
                    ),
                    "worker" => (
                        "background_worker",
                        serde_json::json!({"runtime": runtime, "plan": "starter", "envSpecificDetails": env}),
                    ),
                    "cron" => (
                        "cron_job",
                        serde_json::json!({
                            "runtime": runtime,
                            "plan": "starter",
                            "schedule": schedule.ok_or(EngineError::Invalid(
                                "Give schedule, a five-field UTC cron expression, for a cron job.",
                            ))?,
                            "envSpecificDetails": env,
                        }),
                    ),
                    _ => {
                        return Err(EngineError::Invalid(
                            "On Render a repository runs as web, static, worker or cron.",
                        ));
                    }
                };
                let mut body = serde_json::json!({
                    "type": kind,
                    "name": name,
                    "ownerId": owner,
                    "repo": format!("https://github.com/{repo}"),
                    "autoDeploy": "yes",
                    "serviceDetails": details,
                });
                if let Some(branch) = branch {
                    body["branch"] = branch.into();
                }
                let value = self
                    .http
                    .json(
                        Provider::Render,
                        token,
                        Method::POST,
                        &["services"],
                        &[],
                        Some(&body),
                    )
                    .await?;
                let service = if value["service"].is_object() {
                    &value["service"]
                } else {
                    &value
                };
                let id = text(service, "id")?;
                let label = match kind {
                    "web_service" => "web service",
                    "static_site" => "static site",
                    "background_worker" => "background worker",
                    _ => "cron job",
                };
                Ok((
                    vec![id],
                    format!("Created the Render {label} {name} from {repo}"),
                ))
            }
            ProviderNew::Database {
                name,
                engine,
                region,
            } => {
                let (path, label, mut body) = match engine {
                    "postgres" => (
                        "postgres",
                        "Postgres database",
                        serde_json::json!({"name": name, "ownerId": owner, "plan": "free", "version": "16"}),
                    ),
                    "redis" => (
                        "key-value",
                        "Key Value (Redis-compatible) store",
                        serde_json::json!({"name": name, "ownerId": owner, "plan": "free"}),
                    ),
                    _ => {
                        return Err(EngineError::Invalid(
                            "Render makes postgres or redis (Key Value) databases.",
                        ));
                    }
                };
                if let Some(region) = region {
                    body["region"] = region.into();
                }
                let value = self
                    .http
                    .json(
                        Provider::Render,
                        token,
                        Method::POST,
                        &[path],
                        &[],
                        Some(&body),
                    )
                    .await?;
                let id = text(&value, "id")?;
                Ok((vec![id], format!("Created the Render {label} {name}")))
            }
            _ => Err(crate::application::ports::providers::unsupported()),
        }
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        super::accounts::list(&self.http, Provider::Render, token).await
    }

    async fn verify(&self, token: &Secret, owner: &str) -> Result<Vec<String>, EngineError> {
        if !plain_id(owner) {
            return Err(EngineError::Invalid("A Render workspace ID is required."));
        }
        // Reading the workspace's services proves the key reaches it.
        let value = self
            .get(token, &["services"], &[("ownerId", owner), ("limit", "1")])
            .await?;
        if !value.is_array() {
            return Err(invalid());
        }
        Ok(vec!["Workspace services read (verified by request)".into()])
    }

    async fn provider_empty_projects(
        &self,
        token: &Secret,
        owner: &str,
    ) -> Result<Vec<crate::application::ports::providers::ProviderEmptyProject>, EngineError> {
        // A project's environments hold its services and stores; one whose
        // environments hold none of them is empty.
        let mut used = std::collections::HashSet::new();
        for (path, key) in [
            ("services", "service"),
            ("postgres", "postgres"),
            ("key-value", "keyValue"),
        ] {
            for item in self.every(token, &[path], owner, key).await? {
                if let Some(environment) = item["environmentId"].as_str() {
                    used.insert(environment.to_owned());
                }
            }
        }
        let mut empty = Vec::new();
        for project in self.every(token, &["projects"], owner, "project").await? {
            let id = text(&project, "id")?;
            if !plain_id(&id) {
                return Err(invalid());
            }
            let holds = project["environmentIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|environment| used.contains(environment));
            if !holds {
                empty.push(crate::application::ports::providers::ProviderEmptyProject {
                    url: Some(format!("{DASHBOARD}/project/{id}")),
                    name: text(&project, "name")?,
                    id,
                });
            }
        }
        Ok(empty)
    }

    async fn discover(
        &self,
        token: &Secret,
        owner: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        for service in self.every(token, &["services"], owner, "service").await? {
            let id = text(&service, "id")?;
            if !plain_id(&id) {
                return Err(invalid());
            }
            let details = &service["serviceDetails"];
            let suspended = service["suspended"].as_str() == Some("suspended");
            resources.push(RemoteResource {
                reference: id,
                name: text(&service, "name")?,
                kind: kind_of(service["type"].as_str().unwrap_or_default()),
                region: details["region"].as_str().map(str::to_owned),
                status: if suspended {
                    ResourceStatus::Paused
                } else {
                    ResourceStatus::Unknown
                },
                url: details["url"].as_str().map(str::to_owned),
                group: None,
            });
        }
        for (path, key) in [("postgres", "postgres"), ("key-value", "keyValue")] {
            for store in self.every(token, &[path], owner, key).await? {
                let id = text(&store, "id")?;
                if !plain_id(&id) {
                    return Err(invalid());
                }
                resources.push(RemoteResource {
                    reference: id,
                    name: text(&store, "name")?,
                    kind: ResourceKind::Database,
                    region: store["region"].as_str().map(str::to_owned),
                    status: ResourceStatus::Unknown,
                    url: None,
                    group: None,
                });
            }
        }
        if resources.len() > MAX_DISCOVERED_RESOURCES {
            return Err(invalid());
        }
        Ok(resources)
    }

    /// A service is as its latest deploy says, unless it's suspended; a
    /// Postgres or Key Value instance is as Render reports it.
    async fn observe_resource(
        &self,
        token: &Secret,
        _owner: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let id = resource.provider_ref.as_str();
        if !plain_id(id) {
            return Err(invalid());
        }
        let store = if id.starts_with("dpg-") {
            Some("postgres")
        } else if id.starts_with("red-") {
            Some("key-value")
        } else {
            None
        };
        if let Some(path) = store {
            let value = self.get(token, &[path, id], &[]).await?;
            let status = match value["status"].as_str() {
                Some("available") => ResourceStatus::Live,
                Some("creating" | "updating_instance" | "recovering") => ResourceStatus::Deploying,
                Some("suspended") => ResourceStatus::Paused,
                Some("unavailable" | "unknown") => ResourceStatus::Failing,
                _ => ResourceStatus::Unknown,
            };
            return Ok(Observed {
                status,
                url: None,
                kind: Some(ResourceKind::Database),
            });
        }
        let service = self.get(token, &["services", id], &[]).await?;
        let url = service["serviceDetails"]["url"].as_str().map(str::to_owned);
        if service["suspended"].as_str() == Some("suspended") {
            return Ok(Observed {
                status: ResourceStatus::Paused,
                url,
                kind: None,
            });
        }
        let deploys = self
            .get(token, &["services", id, "deploys"], &[("limit", "1")])
            .await?;
        let status = deploys
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|row| row["deploy"]["status"].as_str())
            .map_or(ResourceStatus::NotDeployed, deploy_status);
        Ok(Observed {
            status,
            url,
            kind: None,
        })
    }

    /// A service as Render keeps it: deploys, variable names, its
    /// onrender.com address and custom domains with their verification,
    /// build and start commands, region, instances and disk. A Postgres or
    /// Key Value instance has none of these.
    async fn provider_details(
        &self,
        token: &Secret,
        _owner: &str,
        id: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        use crate::application::ports::providers::{
            ProviderConfig, ProviderDomain, ProviderDomainState, ProviderVolume,
        };
        if !plain_id(id) {
            return Err(invalid());
        }
        let empty = crate::application::ports::providers::ProviderDetails {
            deployments: vec![],
            variable_names: None,
            shared_variable_names: None,
            domains: vec![],
            references: vec![],
            sleeps: None,
            config: None,
            volumes: vec![],
            tcp_proxies: vec![],
            addresses: vec![],
            private_domain: None,
        };
        if !(id.starts_with("srv-") || id.starts_with("crn-")) {
            return Ok(empty);
        }
        let rows = self
            .get(token, &["services", id, "deploys"], &[("limit", "25")])
            .await?;
        let deployments = rows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| render_deploy(id, &row["deploy"]))
            .collect();
        let service = self
            .get(token, &["services", id], &[])
            .await
            .unwrap_or_default();
        let details = &service["serviceDetails"];
        let text = |value: &Value| {
            value
                .as_str()
                .filter(|said| !said.is_empty())
                .map(str::to_owned)
        };
        let mut names: Vec<String> = self
            .get(token, &["services", id, "env-vars"], &[("limit", "100")])
            .await
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|row| row["envVar"]["key"].as_str().map(str::to_owned))
            .collect();
        names.sort();
        names.dedup();
        let mut domains: Vec<ProviderDomain> = text(&details["url"])
            .map(|url| ProviderDomain {
                name: url
                    .trim_start_matches("https://")
                    .trim_end_matches('/')
                    .to_owned(),
                custom: false,
                state: Some(ProviderDomainState::Working),
                records: vec![],
                port: None,
                certificate: None,
                dns: vec![],
            })
            .into_iter()
            .collect();
        domains.extend(
            self.get(
                token,
                &["services", id, "custom-domains"],
                &[("limit", "100")],
            )
            .await
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|row| {
                let domain = &row["customDomain"];
                let verified = domain["verificationStatus"].as_str() == Some("verified");
                Some(ProviderDomain {
                    name: domain["name"].as_str()?.to_owned(),
                    custom: true,
                    state: Some(if verified {
                        ProviderDomainState::Working
                    } else {
                        ProviderDomainState::Pending
                    }),
                    records: vec![],
                    port: None,
                    certificate: domain["verificationStatus"].as_str().map(str::to_owned),
                    dns: vec![],
                })
            }),
        );
        let commands = &details["envSpecificDetails"];
        let disk = &details["disk"];
        let volumes = text(&disk["id"])
            .map(|disk_id| ProviderVolume {
                name: text(&disk["name"]).unwrap_or_else(|| disk_id.clone()),
                mount_path: text(&disk["mountPath"]),
                instance_id: None,
                used_mb: None,
                size_mb: disk["sizeGB"].as_u64().map(|gb| gb * MB_PER_GB),
                region: None,
                id: disk_id,
            })
            .into_iter()
            .collect();
        let config = (!service.is_null()).then(|| ProviderConfig {
            repo: text(&service["repo"]).map(|repo| {
                repo.trim_start_matches("https://github.com/")
                    .trim_end_matches(".git")
                    .to_owned()
            }),
            image: text(&service["imagePath"]).or_else(|| text(&details["image"]["imagePath"])),
            start_command: text(&commands["startCommand"])
                .or_else(|| text(&commands["dockerCommand"])),
            build_command: text(&commands["buildCommand"]),
            root_directory: text(&service["rootDir"]),
            cron_schedule: text(&details["schedule"]),
            healthcheck_path: text(&details["healthCheckPath"]),
            replicas: details["numInstances"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok()),
            region: text(&details["region"]),
            restart_policy: None,
        });
        Ok(crate::application::ports::providers::ProviderDetails {
            deployments,
            variable_names: Some(names),
            domains,
            config,
            volumes,
            // A free instance spins down when idle.
            sleeps: text(&details["plan"]).map(|plan| plan == "free"),
            ..empty
        })
    }

    /// Render's own metrics: requests by status and latency for services
    /// that serve HTTP, and CPU, memory and more for everything.
    async fn provider_metrics(
        &self,
        token: &Secret,
        _owner: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::{chart, line, points};
        use crate::application::ports::providers::{MetricSummary as S, MetricUnit as U};
        if !plain_id(reference) {
            return Err(invalid());
        }
        let (start, end) = (super::metrics::stamp(from), super::metrics::stamp(to));
        let step = step_seconds.max(MIN_STEP_SECS).to_string();
        let base = [
            ("resource", reference),
            ("startTime", start.as_str()),
            ("endTime", end.as_str()),
            ("resolutionSeconds", step.as_str()),
        ];
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut query = base.to_vec();
            query.extend_from_slice(extra);
            query
        };
        let store = reference.starts_with("dpg-") || reference.starts_with("red-");
        let (requests, latency, cpu, memory, instances, connections, disk, bandwidth) = tokio::join!(
            async {
                if store {
                    None
                } else {
                    self.series(
                        token,
                        "http-requests",
                        &with(&[("aggregateBy", "statusCode")]),
                    )
                    .await
                }
            },
            async {
                if store {
                    None
                } else {
                    self.series(
                        token,
                        "http-latency",
                        &with(&[
                            ("quantile", "0.5"),
                            ("quantile", "0.95"),
                            ("quantile", "0.99"),
                        ]),
                    )
                    .await
                }
            },
            self.series(token, "cpu", &base),
            self.series(token, "memory", &base),
            async {
                if store {
                    None
                } else {
                    self.series(token, "instance-count", &base).await
                }
            },
            async {
                if store {
                    self.series(token, "active-connections", &base).await
                } else {
                    None
                }
            },
            async {
                if store {
                    self.series(token, "disk-usage", &base).await
                } else {
                    None
                }
            },
            async {
                if store {
                    None
                } else {
                    self.series(
                        token,
                        "bandwidth",
                        &[
                            ("resource", reference),
                            ("startTime", start.as_str()),
                            ("endTime", end.as_str()),
                        ],
                    )
                    .await
                }
            },
        );
        let mut missing = Vec::new();
        if cpu.is_none() {
            missing.push("CPU");
        }
        if memory.is_none() {
            missing.push("memory");
        }
        let total = |rows: Option<Vec<RenderRow>>| {
            points(
                rows.unwrap_or_default()
                    .into_iter()
                    .map(|(_, at, n)| (at, n)),
            )
        };
        let statuses =
            super::metrics::by_status_class(requests.unwrap_or_default().into_iter().filter_map(
                |(labels, at, n)| Some((label(&labels, "statusCode")?.parse().ok()?, at, n)),
            ));
        let latency = latency.unwrap_or_default();
        let quantile = |q: &str, name: &str| {
            line(
                name,
                points(
                    latency
                        .iter()
                        .filter(|(labels, _, _)| {
                            label(labels, "quantile")
                                .and_then(|value| value.parse::<f64>().ok())
                                .zip(q.parse::<f64>().ok())
                                .is_some_and(|(a, b)| (a - b).abs() < 1e-6)
                        })
                        .map(|(_, at, n)| (at.clone(), *n)),
                ),
            )
        };
        let charts = vec![
            chart("requests", "Requests", U::Count, S::Total, statuses),
            chart(
                "latency",
                "Response time",
                U::Milliseconds,
                S::Average,
                vec![
                    quantile("0.5", "p50"),
                    quantile("0.95", "p95"),
                    quantile("0.99", "p99"),
                ],
            ),
            chart(
                "cpu",
                "CPU",
                U::Cores,
                S::Average,
                vec![line("CPU", total(cpu))],
            ),
            chart(
                "memory",
                "Memory",
                U::Bytes,
                S::Average,
                vec![line("Memory", total(memory))],
            ),
            chart(
                "instances",
                "Instances",
                U::Count,
                S::Latest,
                vec![line("Instances", total(instances))],
            ),
            chart(
                "connections",
                "Active connections",
                U::Count,
                S::Average,
                vec![line("Connections", total(connections))],
            ),
            chart(
                "disk",
                "Disk used",
                U::Bytes,
                S::Latest,
                vec![line("Disk", total(disk))],
            ),
            chart(
                "bandwidth",
                "Bandwidth",
                U::Bytes,
                S::Total,
                vec![line("Bandwidth", total(bandwidth))],
            ),
        ];
        Ok(super::metrics::metrics(
            from, to, charts, &missing, "Render",
        ))
    }

    /// A service's environment variable, through Render's API. Render applies
    /// it on the service's next deploy.
    async fn set_provider_variable(
        &self,
        token: &Secret,
        _owner: &str,
        id: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        let id = service_id(id)?;
        self.http
            .json(
                Provider::Render,
                token,
                Method::PUT,
                &["services", id, "env-vars", name],
                &[],
                Some(&serde_json::json!({"value": value.expose()})),
            )
            .await?;
        Ok(())
    }

    async fn set_provider_variables(
        &self,
        token: &Secret,
        owner: &str,
        id: &str,
        values: &[(String, Secret)],
        shared: bool,
        _redeploy: bool,
    ) -> Result<(), EngineError> {
        // Render's shared variables are environment groups, not this service.
        if shared {
            return Err(crate::application::ports::providers::unsupported());
        }
        for (name, value) in values {
            self.set_provider_variable(token, owner, id, name, value)
                .await?;
        }
        Ok(())
    }

    async fn delete_provider_variable(
        &self,
        token: &Secret,
        _owner: &str,
        id: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        let id = service_id(id)?;
        self.http
            .json(
                Provider::Render,
                token,
                Method::DELETE,
                &["services", id, "env-vars", name],
                &[],
                None,
            )
            .await?;
        Ok(())
    }

    /// Deletes the service, Postgres or Key Value instance at Render.
    async fn delete(&self, token: &Secret, _owner: &str, id: &str) -> Result<(), EngineError> {
        if !plain_id(id) {
            return Err(invalid());
        }
        let kind = if id.starts_with("dpg-") {
            "postgres"
        } else if id.starts_with("red-") {
            "key-value"
        } else {
            "services"
        };
        self.http
            .json(
                Provider::Render,
                token,
                Method::DELETE,
                &[kind, id],
                &[],
                None,
            )
            .await?;
        Ok(())
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        super::manifest::builtin("render").operations()
    }

    fn prepare_tools(&self) {
        if let Some(cli) = super::manifest::builtin("render").cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        super::manifest::builtin("render").actions.clone()
    }

    /// Render's API first, its CLI for what the API can't do; both aimed at
    /// the workspace and the resource.
    async fn provider_operation(
        &self,
        token: &Secret,
        owner: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        let manifest = super::manifest::builtin("render");
        let name = for_resource(name, reference);
        let mut context = vec![("workspace", owner)];
        if let Some(id) = reference {
            context.push(("resource", id));
            // The API names each kind of resource its own way.
            context.push((
                if id.starts_with("dpg-") {
                    "postgres"
                } else if id.starts_with("red-") {
                    "key_value"
                } else {
                    "service"
                },
                id,
            ));
        }
        if manifest.api_operation(name).is_some() {
            return super::api_catalog::run_manifest(
                &self.http,
                Provider::Render,
                manifest,
                token,
                &context,
                name,
                args,
            )
            .await
            .map_err(git_access);
        }
        let cli = manifest.cli.as_ref().ok_or(EngineError::Store)?;
        if !super::cli::has(cli, name) {
            return Err(EngineError::Invalid(
                "This provider has no such operation in Hoista. Call list_provider_operations for the list.",
            ));
        }
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
        .map_err(git_access)
    }
}

/// Render's answer to a repository it can't read (a private one, with no
/// GitHub access granted to Render) says only "invalid or unfetchable".
/// Hoista says what to approve, and where: Render reads GitHub through its
/// GitHub app, which only the person can install, in a browser.
fn git_access(error: EngineError) -> EngineError {
    match &error {
        EngineError::Provider { message, .. }
            if message.to_ascii_lowercase().contains("unfetchable") =>
        {
            EngineError::Provider {
                evidence: None,
                code: "provider.git_access_needed".into(),
                message: "Render can't read this repository. It's private, and Render reads it through two links: Render's GitHub app must have this repository under Repository access (https://github.com/apps/render/installations/new), and the Render user whose API key Hoista holds needs GitHub under Account Settings → Git Deployment Credentials. Check both, then run this again.".into(),
            }
        }
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_deploy_becomes_a_provider_deployment() {
        use crate::application::ports::providers::ProviderDeployState;
        let row = serde_json::json!({
            "id": "dep-1",
            "status": "deactivated",
            "createdAt": "2026-11-01T10:00:00Z",
            "finishedAt": "2026-11-01T10:03:00Z",
            "commit": {"id": "abc123", "message": "Fix the header\n\nLonger text"},
        });
        let deployment = render_deploy("srv-1", &row).unwrap();
        assert!(matches!(deployment.state, ProviderDeployState::Succeeded));
        assert_eq!(deployment.message.as_deref(), Some("Fix the header"));
        let failed = render_deploy("srv-1", &serde_json::json!({"id":"dep-2","status":"build_failed","createdAt":"2026-11-01T11:00:00Z"})).unwrap();
        assert!(matches!(failed.state, ProviderDeployState::Failed));
    }

    #[test]
    fn a_private_repository_says_what_to_approve() {
        let said = git_access(EngineError::Provider {
            evidence: None,
            code: "provider.rejected".into(),
            message: "The provider rejected the operation: repo URL is invalid or unfetchable"
                .into(),
        });
        let EngineError::Provider { code, message, .. } = said else {
            panic!("not a provider error");
        };
        assert_eq!(code, "provider.git_access_needed");
        assert!(message.contains("https://github.com/apps/render/installations/new"));
        let other = git_access(EngineError::Invalid("nope"));
        assert!(matches!(other, EngineError::Invalid("nope")));
    }

    #[test]
    fn metrics_come_from_renders_series() {
        let value = serde_json::json!([
            {
                "labels": [{"field": "statusCode", "value": "200"}],
                "unit": "",
                "values": [{"timestamp": "2026-11-01T10:00:00Z", "value": 5}],
            },
            {
                "labels": [{"field": "statusCode", "value": "502"}],
                "unit": "",
                "values": [{"timestamp": "2026-11-01T10:00:00Z", "value": 1}],
            },
            {
                "labels": [],
                "unit": "MB",
                "values": [{"timestamp": "2026-11-01T10:00:00Z", "value": 2}],
            },
        ]);
        let rows = render_rows(&value);
        assert_eq!(rows.len(), 3);
        assert_eq!(label(&rows[1].0, "statusCode"), Some("502"));
        assert_eq!(rows[2].2, 2e6);
    }

    #[test]
    fn statuses_and_kinds_come_from_renders_own_words() {
        assert_eq!(deploy_status("live"), ResourceStatus::Live);
        assert_eq!(deploy_status("build_failed"), ResourceStatus::Failing);
        assert_eq!(
            deploy_status("build_in_progress"),
            ResourceStatus::Deploying
        );
        assert_eq!(kind_of("cron_job"), ResourceKind::Job);
        assert_eq!(kind_of("background_worker"), ResourceKind::Service);
        assert!(plain_id("srv-abc123"));
        assert!(!plain_id("srv/../x"));
    }

    #[test]
    fn database_details_read_key_value_too() {
        assert_eq!(
            for_resource("retrievePostgres", Some("red-1")),
            "retrieveKeyValue"
        );
        assert_eq!(
            for_resource("retrievePostgres", Some("dpg-1")),
            "retrievePostgres"
        );
        assert_eq!(for_resource("listLogs", Some("red-1")), "listLogs");
    }

    #[test]
    fn the_api_comes_first_and_is_aimed_at_the_resource() {
        let manifest = super::super::manifest::builtin("render");
        for name in [
            "listDeploys",
            "listLogs",
            "retrievePostgres",
            "retrieveKeyValue",
        ] {
            assert!(manifest.api_operation(name).is_some(), "{name}");
        }
        let logs = manifest.api_operation("listLogs").unwrap();
        assert!(logs.aimed_at("workspace") && logs.aimed_at("resource"));
        assert!(
            manifest
                .api_operation("listDeploys")
                .unwrap()
                .aimed_at("service")
        );
        // Values and connection strings stay out.
        for name in [
            "getEnvVarsForService",
            "retrievePostgresConnectionInfo",
            "listPostgresUsers",
            "retrieveEnvGroup",
            "deleteService",
        ] {
            assert!(manifest.api_operation(name).is_none(), "{name}");
        }
    }
}
