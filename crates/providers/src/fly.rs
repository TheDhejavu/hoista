//! Fly.io: apps and their Machines. Reads go through the Machines and GraphQL
//! APIs; deploys and rollbacks run Hoista's pinned flyctl.

use super::{
    answers::{field, invalid},
    http::ProviderHttp,
};
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{Artifact, Observed, ProviderAdapter, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

/// Fly.io's adapter. The token goes only to Fly's APIs and into the
/// environment of the flyctl run that needs it.
pub struct Fly {
    http: ProviderHttp,
}

/// Fly's macaroon tokens (`FlyV1 fm2_…`) name their own scheme and go as they
/// are; older personal tokens are Bearer tokens.
fn sign_in(token: &Secret) -> Result<super::SignInHeaders, EngineError> {
    let value = if token.expose().starts_with("FlyV1 ") {
        token.expose().to_owned()
    } else {
        format!("Bearer {}", token.expose())
    };
    Ok(vec![("authorization", value)])
}

/// The port an app listens on when nothing says otherwise.
const DEFAULT_PORT: u16 = 8080;
/// How long setting up a certificate may take, and a flyctl deploy or rollback.
const CERTIFICATE_TIMEOUT: Duration = Duration::from_secs(45);
const FLYCTL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Fly's finest chart resolution.
const MIN_STEP_SECS: u32 = 15;
const MB_PER_GB: u64 = 1024;
/// The largest `fly.toml` a rollback rewrites.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
/// The most machines one app may have for Hoista to read it.
const MAX_MACHINES: usize = 10_000;

/// Fly's Machines API, where apps and machines are managed.
const MACHINES_API: &str = "https://api.machines.dev/v1/";
/// Fly's own API, for logs.
const FLY_API: &str = "https://api.fly.io";
/// Fly's GraphQL API: organizations and app details.
const GRAPHQL: &str = "https://api.fly.io/graphql";
/// Fly's Prometheus API, for metrics.
const PROMETHEUS: &str = "https://api.fly.io/prometheus";

/// An app's public address.
fn app_url(app: &str) -> String {
    format!("https://{app}.fly.dev")
}

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M11.987 0c-2.45-.01-5.002.925-6.541 2.897-1.17 1.502-1.664 3.474-1.49 5.356.29 2.112 1.476 3.96 2.676 5.672a41.5 41.5 0 0 0 4.216 4.831c-1.063.832-1.943 2.286-1.357 3.644.821 2.32 4.665 2.05 5.122-.372.39-1.288-.694-2.533-1.428-3.309 2.388-2.431 4.706-5.036 6.17-8.145.595-1.32.902-2.802.614-4.24-.28-2.341-1.823-4.473-3.967-5.46C14.76.266 13.364.016 11.987 0m-.236 1.577v15.534C9.881 13.483 7.724 9.266 8.73 5.069c.35-1.539 1.253-3.309 3.02-3.492m1.996.04c1.534.357 3.031 1.096 3.906 2.48 1.3 1.93 1.318 4.55.1 6.521-1.268 2.395-3.06 4.463-4.916 6.415 1.472-2.974 3.074-6.106 3.182-9.5-.043-2.08-.438-4.612-2.272-5.916M11.97 20.103c.848.342 1.597 1.983.153 2.173-.664.15-1.367-.599-.995-1.222.213-.355.488-.73.842-.95" fill="#A78BFA"/></svg>"##;

/// Fly.io's entry in the provider list. Accounts are listed from its GraphQL
/// API beside the Machines API.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Fly,
    logo: LOGO,
    capabilities: Capabilities {
        provider_variables: true,
        deploy: true,
        logs: true,
        metrics: true,
        rollback: true,
        pause: true,
        domains: true,
        secrets: true,
        discover: true,
        server: true,
        provider_operations: true,
        provider_details: true,
        delete: true,
        new_from_folder: true,
        new_from_image: true,
        plan_deploy: true,
        containers: true,
        ..Capabilities::NONE
    },
    cli: Some(super::ProviderCli {
        program: "flyctl",
        token_env: "FLY_API_TOKEN",
        env: &[
            ("CI", "true"),
            ("FLY_NO_UPDATE_CHECK", "1"),
            ("FLY_SEND_METRICS", "0"),
            ("DO_NOT_TRACK", "1"),
        ],
        keeper_commands: &["version", "deploy", "secrets", "releases", "image", "certs"],
    }),
    api: super::ProviderApi {
        base: MACHINES_API,
        hosts: &["api.machines.dev", "api.fly.io"],
        explains_refusals: true,
        sign_in: super::SignIn::Headers(sign_in),
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Graphql {
            url: Some(GRAPHQL),
            query: "query HoistaAccountChoices($after:String) { organizations(first:100,after:$after) { nodes { slug name } pageInfo { hasNextPage endCursor } } }",
            cursor: Some("after"),
        },
        rows: &["data", "organizations", "nodes"],
        item: &[],
        id: "slug",
        pages: super::accounts::Pages::Cursor {
            more: &["data", "organizations", "pageInfo", "hasNextPage"],
            end: &["data", "organizations", "pageInfo", "endCursor"],
        },
    },
    adapter: |http| std::sync::Arc::new(Fly::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of the provider for the conformance suite: enough to connect and
/// find one resource.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "acct-1",
    resource: "res-1",
    kind: crate::domain::model::ResourceKind::Service,
    routes: &[
        super::conformance::Route {
            path: "/graphql",
            body: "HoistaAccountChoices",
            answer: r#"{"data":{"organizations":{"nodes":[{"slug":"acct-1","name":"Sam"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#,
        },
        super::conformance::Route {
            path: "/apps?org_slug",
            body: "",
            answer: r#"{"apps":[{"id":"res-1","name":"res-1","machine_count":1}],"total_apps":1}"#,
        },
    ],
    empty: r#"{}"#,
};

impl Fly {
    /// Shares the allow-listed HTTP executor.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    async fn machines(&self, token: &Secret, app: &str) -> Result<Vec<Value>, EngineError> {
        let value = self
            .http
            .json(
                Provider::Fly,
                token,
                Method::GET,
                &["apps", app, "machines"],
                &[],
                None,
            )
            .await?;
        let machines = value.as_array().ok_or_else(invalid)?;
        if machines.len() > MAX_MACHINES {
            return Err(EngineError::Conflict(
                "This Fly app exceeds the machine limit.",
            ));
        }
        Ok(machines.clone())
    }

    async fn release(&self, token: &Secret, app: &str) -> Result<Release, EngineError> {
        let machines = self.machines(token, app).await?;
        let machine = newest_machine(&machines).ok_or_else(invalid)?;
        let image = &machine["image_ref"];
        let registry = field(image, "registry")?;
        let repository = field(image, "repository")?;
        let digest = field(image, "digest")?;
        if !digest.starts_with("sha256:") {
            return Err(invalid());
        }
        Ok(Release {
            schedules: None,
            reference: app.into(),
            version: format!("{registry}/{repository}@{digest}"),
            url: Some(app_url(app)),
        })
    }
}

/// The machine running the newest release. An app that stops its machines
/// when idle can have none started a moment after a deploy, so a stopped or
/// suspended machine counts too; one being destroyed or that failed doesn't.
/// The most recently updated wins (a deploy updates the machines it
/// releases to), then a started one.
fn newest_machine(machines: &[Value]) -> Option<&Value> {
    machines
        .iter()
        .filter(|machine| {
            matches!(
                machine["state"].as_str(),
                Some(
                    "started"
                        | "starting"
                        | "stopped"
                        | "stopping"
                        | "suspended"
                        | "created"
                        | "replacing"
                        | "updating"
                )
            ) && machine["image_ref"]["digest"].is_string()
        })
        .max_by(|a, b| {
            let started = |machine: &Value| machine["state"] == "started";
            let updated = |machine: &Value| machine["updated_at"].as_str().unwrap_or("").to_owned();
            updated(a)
                .cmp(&updated(b))
                .then_with(|| started(a).cmp(&started(b)))
        })
}

/// An app's state from its machines (see `observe_resource`).
fn machines_status(machines: &[Value]) -> ResourceStatus {
    let state = |machine: &Value| machine["state"].as_str().unwrap_or("").to_owned();
    // Destroyed machines are gone; they say nothing about the app now.
    let states: Vec<String> = machines
        .iter()
        .map(state)
        .filter(|s| !matches!(s.as_str(), "destroyed" | "destroying"))
        .collect();
    let any = |names: &[&str]| states.iter().any(|s| names.contains(&s.as_str()));
    if states.is_empty() {
        ResourceStatus::NotDeployed
    } else if any(&["started"]) {
        ResourceStatus::Live
    } else if any(&["failed"]) {
        ResourceStatus::Failing
    } else if any(&["starting", "replacing", "updating"]) {
        ResourceStatus::Deploying
    } else if any(&["stopped", "stopping", "suspended", "suspending", "created"]) {
        // `created` is a machine made but never started (a standby, or one a
        // deploy left for the first request): not a deploy in progress.
        // Fly starts a stopped machine on a request when its service says so.
        let wakes = machines.iter().any(|machine| {
            machine["config"]["services"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|service| service["autostart"].as_bool().unwrap_or(false))
        });
        if wakes {
            ResourceStatus::Sleeping
        } else {
            ResourceStatus::Stopped
        }
    } else {
        ResourceStatus::Unknown
    }
}

/// One Fly release as a provider deployment.
fn fly_release(
    app: &str,
    row: &Value,
) -> Option<crate::application::ports::providers::ProviderDeployment> {
    use crate::application::ports::providers::{ProviderDeployState, ProviderDeployment};
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let status = row["status"]
        .as_str()
        .unwrap_or("pending")
        .to_ascii_lowercase();
    let state = match status.as_str() {
        "complete" | "completed" | "succeeded" | "success" => ProviderDeployState::Succeeded,
        "failed" | "failure" => ProviderDeployState::Failed,
        "interrupted" | "cancelled" | "canceled" | "skipped" => ProviderDeployState::Stopped,
        _ => ProviderDeployState::InProgress,
    };
    Some(ProviderDeployment {
        id: text(&row["id"])?,
        status: status.clone(),
        state,
        created_at: text(&row["createdAt"])?,
        url: None,
        commit: None,
        message: text(&row["description"])
            .filter(|said| !said.is_empty())
            .or_else(|| {
                row["version"]
                    .as_i64()
                    .map(|version| format!("Release v{version}"))
            }),
        branch: None,
        repo: None,
        finished_at: None,
        can_roll_back: None,
        author: text(&row["user"]["email"]),
        provider_page: Some(format!("https://fly.io/apps/{app}/releases")),
    })
}

/// One point of a Prometheus series: its labels, time and value.
type PromRow = (serde_json::Map<String, Value>, String, f64);

/// Prometheus's range answer (`data.result[].{metric, values}`) as
/// `(labels, time, value)` rows.
fn prometheus_rows(value: &Value) -> Vec<PromRow> {
    let mut rows = Vec::new();
    for series in value["data"]["result"].as_array().into_iter().flatten() {
        let labels = series["metric"].as_object().cloned().unwrap_or_default();
        for pair in series["values"].as_array().into_iter().flatten() {
            let at = pair[0].as_f64().and_then(crate::units::from_unix_seconds);
            if let (Some(at), Some(n)) = (at, super::metrics::number(&pair[1])) {
                rows.push((labels.clone(), super::metrics::stamp(at), n));
            }
        }
    }
    rows
}

impl Fly {
    /// One PromQL range query against Fly's managed Prometheus for the org.
    async fn prometheus(
        &self,
        token: &Secret,
        org: &str,
        query: &str,
        window: (&str, &str, &str),
    ) -> Result<Vec<PromRow>, EngineError> {
        let (start, end, step) = window;
        self.http
            .get_at(
                Provider::Fly,
                token,
                PROMETHEUS,
                &[org, "api", "v1", "query_range"],
                &[
                    ("query", query),
                    ("start", start),
                    ("end", end),
                    ("step", step),
                ],
            )
            .await
            .and_then(|value| {
                if value["status"].as_str() == Some("success") {
                    Ok(prometheus_rows(&value))
                } else {
                    Err(invalid())
                }
            })
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Fly {
    fn provider(&self) -> Provider {
        Provider::Fly
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// A new Fly app, built from a folder with a Dockerfile (or the folder's
    /// own fly.toml) on Fly's remote builder, or running an image. Fly gives
    /// it public addresses on its first deploy.
    async fn provider_add(
        &self,
        token: &Secret,
        org: &str,
        _project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        use crate::application::ports::providers::ProviderNew;
        let (app, folder, image, port) = match *what {
            ProviderNew::Folder { name, folder } => (name, Some(folder), None, None),
            ProviderNew::Image {
                name,
                image,
                gpu: None,
                cpu: None,
                port,
                args: None,
            } => (name, None, Some(image), port),
            ProviderNew::Image { .. } => {
                return Err(EngineError::Invalid(
                    "Fly runs an image as it is here: leave out gpu, cpu and args.",
                ));
            }
            _ => return Err(crate::application::ports::providers::unsupported()),
        };
        if !valid_slug(app) {
            return Err(EngineError::Invalid(
                "Use a lowercase name with letters, digits and hyphens.",
            ));
        }
        let own_config = folder.is_some_and(|folder| folder.join("fly.toml").is_file());
        if let Some(folder) = folder
            && !own_config
            && !folder.join("Dockerfile").is_file()
        {
            return Err(EngineError::Invalid(
                "Fly builds a folder from its Dockerfile or fly.toml. Add one, then try again.",
            ));
        }
        // The pinned flyctl first, before anything billable is made.
        command(&self.http, token, &["version"], None, None).await?;
        self.http
            .json(
                Provider::Fly,
                token,
                Method::POST,
                &["apps"],
                &[],
                Some(&json!({"app_name":app,"org_slug":org})),
            )
            .await?;
        // A config of Hoista's own, outside the person's folder, unless the
        // folder has its own.
        let workspace = tempfile::tempdir().map_err(|_| EngineError::Store)?;
        let config_path = workspace.path().join("fly.toml");
        let port = port.unwrap_or(DEFAULT_PORT);
        let config = json!({
            "app": app,
            "primary_region": "iad",
            "env": {"PORT": port.to_string()},
            "http_service": {
                "internal_port": port,
                "force_https": true,
                "auto_stop_machines": "stop",
                "auto_start_machines": true,
                "min_machines_running": 0,
            },
            "vm": [{"size": "shared-cpu-1x", "memory": "256mb"}],
        });
        let configuration = toml::to_string(&config).map_err(|_| EngineError::Store)?;
        tokio::fs::write(&config_path, configuration)
            .await
            .map_err(|_| EngineError::Store)?;
        let config_arg = config_path.to_string_lossy().into_owned();
        let mut args = vec![
            "deploy",
            "--app",
            app,
            "--remote-only",
            "--ha=false",
            "--now",
        ];
        if !own_config {
            args.extend(["--config", config_arg.as_str()]);
        }
        if let Some(image) = image {
            args.extend(["--image", image]);
        }
        let working = folder.unwrap_or(workspace.path());
        // With Hoista's config elsewhere, the folder is named as what to
        // build, or flyctl would build the config's own folder.
        let working_arg = working.to_string_lossy().into_owned();
        if folder.is_some() && !own_config {
            args.push(working_arg.as_str());
        }
        command(&self.http, token, &args, Some(working), None).await?;
        Ok((
            vec![app.to_owned()],
            match (folder, image) {
                (_, Some(image)) => format!("Made the Fly app {app} running {image}"),
                _ => format!("Made the Fly app {app} from its folder"),
            },
        ))
    }

    /// A new project from a folder: the same as adding one to a project.
    async fn provider_new_from_folder(
        &self,
        token: &Secret,
        org: &str,
        name: &str,
        folder: &std::path::Path,
    ) -> Result<(Vec<String>, String), EngineError> {
        self.provider_add(
            token,
            org,
            name,
            None,
            &crate::application::ports::providers::ProviderNew::Folder { name, folder },
        )
        .await
    }

    /// The app's numbers from Fly's managed Prometheus: requests through
    /// Fly's edge by status, response time, and CPU and memory in use.
    async fn provider_metrics(
        &self,
        token: &Secret,
        org: &str,
        app: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::{by_status_class, chart, line, points};
        use crate::application::ports::providers::{MetricSummary as S, MetricUnit as U};
        if !valid_slug(app) || !valid_slug(org) {
            return Err(EngineError::Invalid("A Fly app name is required."));
        }
        let (start, end) = (from.timestamp().to_string(), to.timestamp().to_string());
        let step = step_seconds.max(MIN_STEP_SECS).to_string();
        let window = (start.as_str(), end.as_str(), step.as_str());
        let range = format!("{step}s");
        let requests = format!(
            "sum by (status) (increase(fly_edge_http_responses_count{{app=\"{app}\"}}[{range}]))"
        );
        let quantile = |q: &str| {
            format!(
                "histogram_quantile({q}, sum by (le) (rate(fly_edge_http_response_time_seconds_bucket{{app=\"{app}\"}}[{range}]))) * 1000"
            )
        };
        let cpu =
            format!("sum(rate(fly_instance_cpu{{app=\"{app}\",mode!=\"idle\"}}[{range}])) / 100");
        let memory = format!(
            "sum(fly_instance_memory_mem_total{{app=\"{app}\"}} - fly_instance_memory_mem_available{{app=\"{app}\"}})"
        );
        let (p50, p95) = (quantile("0.5"), quantile("0.95"));
        let (requests, p50, p95, cpu, memory) = tokio::join!(
            self.prometheus(token, org, &requests, window),
            self.prometheus(token, org, &p50, window),
            self.prometheus(token, org, &p95, window),
            self.prometheus(token, org, &cpu, window),
            self.prometheus(token, org, &memory, window),
        );
        // With nothing answered, Fly's own reason says why (an app with no
        // machines, a token without access to its metrics).
        if let (Err(error), Err(_), Err(_)) = (&requests, &cpu, &memory) {
            return Err(match error {
                EngineError::Provider { code, message, .. }
                    if code == "provider.rejected" || code.starts_with("provider.token") =>
                {
                    EngineError::Provider {
                        evidence: None,
                        code: code.clone(),
                        message: format!(
                            "Fly's metrics refused the request: {}",
                            message.trim_start_matches("The provider rejected the operation: ")
                        ),
                    }
                }
                _ => EngineError::Provider {
                    evidence: None,
                    code: "provider.unavailable".into(),
                    message: "Fly's metrics didn't answer. Try again in a moment.".into(),
                },
            });
        }
        let (requests, p50, p95, cpu, memory) =
            (requests.ok(), p50.ok(), p95.ok(), cpu.ok(), memory.ok());
        let mut missing = Vec::new();
        if cpu.is_none() {
            missing.push("CPU");
        }
        if memory.is_none() {
            missing.push("memory");
        }
        let flat = |rows: Option<Vec<PromRow>>| {
            // Prometheus answers NaN for a quantile with no requests.
            points(
                rows.unwrap_or_default()
                    .into_iter()
                    .filter(|(_, _, n)| n.is_finite())
                    .map(|(_, at, n)| (at, n)),
            )
        };
        let statuses = by_status_class(requests.unwrap_or_default().into_iter().filter_map(
            |(labels, at, n)| Some((labels.get("status")?.as_str()?.parse().ok()?, at, n.round())),
        ));
        let charts = vec![
            chart("requests", "Requests", U::Count, S::Total, statuses),
            chart(
                "latency",
                "Response time",
                U::Milliseconds,
                S::Average,
                vec![line("p50", flat(p50)), line("p95", flat(p95))],
            ),
            chart(
                "cpu",
                "CPU",
                U::Cores,
                S::Average,
                vec![line("CPU", flat(cpu))],
            ),
            chart(
                "memory",
                "Memory",
                U::Bytes,
                S::Average,
                vec![line("Memory", flat(memory))],
            ),
        ];
        Ok(super::metrics::metrics(from, to, charts, &missing, "Fly"))
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        manifest().operations()
    }

    fn prepare_tools(&self) {
        if let Some(cli) = super::manifest::builtin("fly").cli.as_ref() {
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
        org: &str,
        reference: Option<&str>,
        folder: Option<&Path>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        // The organization and app the account and resource name.
        if manifest().api_operation(name).is_some() {
            let mut context = vec![("org", org)];
            context.extend(reference.map(|app| ("app", app)));
            return super::api_catalog::run_manifest(
                &self.http,
                Provider::Fly,
                manifest(),
                token,
                &context,
                name,
                args,
            )
            .await;
        }
        let cli = manifest().cli.as_ref().ok_or(EngineError::Store)?;
        if !super::cli::has(cli, name) {
            return Err(EngineError::Invalid(
                "Fly.io has no such operation in Hoista. Call list_provider_operations for the list.",
            ));
        }
        let app = reference.ok_or(EngineError::Invalid(
            "Fly.io commands run on one app: give the resource_id of one Hoista watches.",
        ))?;
        super::cli::run(
            cli,
            &self.http,
            token,
            super::cli::Aim {
                context: &[("app", app)],
                folder,
            },
            name,
            args,
        )
        .await
    }

    async fn configure_domain(
        &self,
        token: &Secret,
        org: &str,
        app: &str,
        hostname: &str,
        zone: &str,
        create_zone: bool,
    ) -> Result<crate::domain::domains::DomainObservation, EngineError> {
        if create_zone {
            return Err(EngineError::Invalid(
                "Fly does not manage DNS zones. Add records at your DNS provider without --create-zone.",
            ));
        }
        tokio::time::timeout(
            CERTIFICATE_TIMEOUT,
            self.configure_certificate(token, org, app, hostname, zone, true),
        )
        .await
        .map_err(|_| {
            EngineError::Conflict(
                "Fly certificate operation timed out. Inspect the hostname in Fly before retrying.",
            )
        })?
    }

    async fn observe_domain(
        &self,
        token: &Secret,
        org: &str,
        app: &str,
        hostname: &str,
        zone: &str,
    ) -> Result<crate::domain::domains::DomainObservation, EngineError> {
        tokio::time::timeout(
            CERTIFICATE_TIMEOUT,
            self.configure_certificate(token, org, app, hostname, zone, false),
        )
        .await
        .map_err(|_| {
            EngineError::Conflict("Fly certificate check timed out. Inspect the hostname in Fly.")
        })?
    }

    async fn detach_domain(
        &self,
        token: &Secret,
        org: &str,
        app: &str,
        domain_id: Option<&str>,
        hostname: &str,
    ) -> Result<(), EngineError> {
        if domain_id.is_some_and(|id| id != hostname) {
            return Err(EngineError::Conflict(
                "The saved Fly hostname identifier does not match. Refresh the domain before removal.",
            ));
        }
        tokio::time::timeout(
            CERTIFICATE_TIMEOUT,
            self.detach_certificate(token, org, app, hostname),
        )
        .await
        .map_err(|_| {
            EngineError::Conflict(
                "Fly certificate removal timed out. Inspect the hostname in Fly before retrying.",
            )
        })?
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        super::accounts::list(&self.http, Provider::Fly, token).await
    }

    /// An app is as its machines are: any started is live, any being made or
    /// replaced is deploying, stopped ones that start on a request are
    /// sleeping, and none at all means nothing is deployed yet.
    async fn observe_resource(
        &self,
        token: &Secret,
        _org: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let app = resource.provider_ref.as_str();
        let machines = match self.machines(token, app).await {
            Ok(machines) => machines,
            Err(EngineError::Provider { code, .. }) if code == "provider.not_found" => {
                return Ok(Observed {
                    status: ResourceStatus::Stopped,
                    url: None,
                    kind: None,
                });
            }
            Err(error) => return Err(error),
        };
        Ok(Observed {
            status: machines_status(&machines),
            url: Some(app_url(app)),
            kind: None,
        })
    }

    /// The app as Fly keeps it: releases, variable and secret names, its
    /// addresses and certificates, machines' image, region and size, and disks.
    async fn provider_details(
        &self,
        token: &Secret,
        _org: &str,
        app: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        use crate::application::ports::providers::{
            ProviderConfig, ProviderDomain, ProviderDomainState, ProviderVolume,
        };
        if app.is_empty() || !app.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(invalid());
        }
        let body = serde_json::json!({
            "query": "query HoistaApp($name:String!) { app(name:$name) { releasesUnprocessed(first:25) { nodes { id version status description createdAt user { email } } } certificates(first:50) { nodes { hostname clientStatus } } } }",
            "variables": {"name": app},
        });
        // Releases and certificates come from Fly's GraphQL API; when it
        // won't answer, the rest still shows.
        let graph = self
            .http
            .post_at(Provider::Fly, token, GRAPHQL, &body, true)
            .await
            .unwrap_or_default();
        let deployments = graph["data"]["app"]["releasesUnprocessed"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| fly_release(app, row))
            .collect();
        let machines = self.machines(token, app).await.unwrap_or_default();
        let running: Vec<&Value> = machines
            .iter()
            .filter(|machine| machine["state"].as_str() != Some("destroyed"))
            .collect();
        let newest = newest_machine(&machines);
        let config = &newest.map_or(Value::Null, |machine| machine["config"].clone());
        // Plain variables live on each machine; secrets on the app, names only.
        let mut names: Vec<String> = config["env"]
            .as_object()
            .map(|env| env.keys().cloned().collect())
            .unwrap_or_default();
        if let Ok(secrets) = self
            .http
            .json(
                Provider::Fly,
                token,
                Method::GET,
                &["apps", app, "secrets"],
                &[],
                None,
            )
            .await
        {
            names.extend(
                secrets
                    .as_array()
                    .or_else(|| secrets["secrets"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|row| row["name"].as_str().or_else(|| row["label"].as_str()))
                    .map(str::to_owned),
            );
        }
        names.sort();
        names.dedup();
        let mut domains = vec![ProviderDomain {
            name: format!("{app}.fly.dev"),
            custom: false,
            state: Some(ProviderDomainState::Working),
            records: vec![],
            port: None,
            certificate: None,
            dns: vec![],
        }];
        domains.extend(
            graph["data"]["app"]["certificates"]["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    let ready = row["clientStatus"].as_str().unwrap_or("");
                    Some(ProviderDomain {
                        name: row["hostname"].as_str()?.to_owned(),
                        custom: true,
                        state: Some(if ready.eq_ignore_ascii_case("ready") {
                            ProviderDomainState::Working
                        } else {
                            ProviderDomainState::Pending
                        }),
                        records: vec![],
                        port: None,
                        certificate: Some(ready.to_owned()).filter(|said| !said.is_empty()),
                        dns: vec![],
                    })
                }),
        );
        let mounts: Vec<(String, String)> = running
            .iter()
            .flat_map(|machine| {
                machine["config"]["mounts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|mount| {
                Some((
                    mount["volume"].as_str()?.to_owned(),
                    mount["path"].as_str()?.to_owned(),
                ))
            })
            .collect();
        let volumes = self
            .http
            .json(
                Provider::Fly,
                token,
                Method::GET,
                &["apps", app, "volumes"],
                &[],
                None,
            )
            .await
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter(|volume| volume["state"].as_str() != Some("destroyed"))
            .filter_map(|volume| {
                let id = volume["id"].as_str()?.to_owned();
                Some(ProviderVolume {
                    mount_path: mounts
                        .iter()
                        .find(|(volume, _)| *volume == id)
                        .map(|(_, path)| path.clone()),
                    name: volume["name"].as_str().unwrap_or(&id).to_owned(),
                    instance_id: None,
                    used_mb: None,
                    size_mb: volume["size_gb"].as_u64().map(|gb| gb * MB_PER_GB),
                    region: volume["region"].as_str().map(str::to_owned),
                    id,
                })
            })
            .collect();
        let services = config["services"].as_array().cloned().unwrap_or_default();
        let sleeps = (!services.is_empty()).then(|| {
            services.iter().any(|service| {
                !matches!(service["autostop"].as_str(), None | Some("off"))
                    || service["autostop"].as_bool() == Some(true)
            })
        });
        let health = config["checks"]
            .as_object()
            .and_then(|checks| checks.values().find_map(|check| check["path"].as_str()))
            .or_else(|| {
                services.iter().find_map(|service| {
                    service["checks"]
                        .as_array()
                        .and_then(|checks| checks.iter().find_map(|check| check["path"].as_str()))
                })
            })
            .map(str::to_owned);
        let config = newest.map(|machine| ProviderConfig {
            image: config["image"].as_str().map(str::to_owned),
            start_command: config["init"]["cmd"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|command| !command.is_empty()),
            healthcheck_path: health,
            replicas: u32::try_from(running.len()).ok(),
            region: machine["region"].as_str().map(str::to_owned),
            restart_policy: config["restart"]["policy"].as_str().map(str::to_owned),
            ..ProviderConfig::default()
        });
        Ok(crate::application::ports::providers::ProviderDetails {
            deployments,
            variable_names: Some(names),
            shared_variable_names: None,
            domains,
            references: vec![],
            sleeps,
            config,
            volumes,
            tcp_proxies: vec![],
            addresses: vec![],
            private_domain: Some(format!("{app}.internal")),
        })
    }

    async fn verify(&self, token: &Secret, org: &str) -> Result<Vec<String>, EngineError> {
        self.discover(token, org).await?;
        Ok(vec!["apps:read (verified by request)".into()])
    }

    async fn discover(
        &self,
        token: &Secret,
        org: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        // Without pagination parameters Fly returns the whole list; a short one
        // (fewer than `total_apps`) is refused.
        let value = self
            .http
            .json(
                Provider::Fly,
                token,
                Method::GET,
                &["apps"],
                &[("org_slug", org)],
                None,
            )
            .await?;
        let apps = value["apps"].as_array().ok_or_else(invalid)?;
        if apps.len() > MAX_DISCOVERED_RESOURCES
            || value["total_apps"]
                .as_u64()
                .is_some_and(|total| total != apps.len() as u64)
        {
            return Err(invalid());
        }
        apps.iter()
            .map(|app| {
                let name = field(app, "name")?;
                Ok(RemoteResource {
                    reference: name.clone(),
                    name: name.clone(),
                    kind: ResourceKind::Service,
                    region: None,
                    status: ResourceStatus::Unknown,
                    url: Some(app_url(&name)),
                    group: None,
                })
            })
            .collect()
    }

    /// Fly.io secrets are write-only already: its API and CLI show only a digest.
    async fn provider_hide_server_key(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<bool, EngineError> {
        Ok(true)
    }

    /// A new Fly app running the published image without stopping, with a volume
    /// at `/data` and the app's own `fly.dev` address. The variables go in as
    /// Fly secrets before the first Machine starts.
    async fn provider_new_hoista_server(
        &self,
        token: &Secret,
        org: &str,
        source: &crate::application::ports::providers::ServerSource,
        variables: &std::collections::BTreeMap<String, Secret>,
    ) -> Result<(String, String), EngineError> {
        let crate::application::ports::providers::ServerSource::Image(image) = source else {
            return Err(EngineError::Invalid(
                "On Fly.io Hoista runs the published image. Leave out the repository.",
            ));
        };
        if !crate::is_pinned_image(image) {
            return Err(EngineError::Invalid(
                "The server image must be pinned by its sha256 digest.",
            ));
        }
        if org.is_empty() {
            return Err(EngineError::Conflict(
                "This Fly.io account has no organization Hoista can create apps in. Connect Fly.io again from Accounts.",
            ));
        }
        // Check flyctl runs before making anything Fly bills for.
        command(&self.http, token, &["version"], None, None).await?;
        let app = format!(
            "hoista-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        );
        let host = format!("{app}.fly.dev");
        self.http
            .json(
                Provider::Fly,
                token,
                Method::POST,
                &["apps"],
                &[],
                Some(&json!({"app_name": app, "org_slug": org})),
            )
            .await?;
        let mut secrets: Vec<(String, Secret)> = variables
            .iter()
            .map(|(name, value)| (name.clone(), Secret::new(value.expose().to_owned())))
            .collect();
        secrets.push((
            "HOISTA_PUBLIC_URL".into(),
            Secret::new(format!("https://{host}")),
        ));
        command(
            &self.http,
            token,
            &["secrets", "import", "--app", &app, "--stage"],
            None,
            Some(&secret_input(&secrets)?),
        )
        .await?;
        drop(secrets);
        let config = json!({
            "app": app,
            "primary_region": "iad",
            "env": {"PORT": "8080"},
            "http_service": {
                "internal_port": DEFAULT_PORT,
                "force_https": true,
                // Agents reach it any time, so it never sleeps.
                "auto_stop_machines": "off",
                "auto_start_machines": true,
                "min_machines_running": 1
            },
            "mounts": {"source": "hoista_data", "destination": "/data", "initial_size": "1gb"},
            "vm": [{"size": "shared-cpu-1x", "memory": "512mb"}]
        });
        let configuration = toml::to_string(&config).map_err(|_| EngineError::Store)?;
        let folder = tempfile::tempdir().map_err(|_| EngineError::Store)?;
        tokio::fs::write(folder.path().join("fly.toml"), configuration)
            .await
            .map_err(|_| EngineError::Store)?;
        command(
            &self.http,
            token,
            &[
                "deploy",
                "--app",
                &app,
                "--config",
                "fly.toml",
                "--image",
                image,
                "--ha=false",
                "--now",
            ],
            Some(folder.path()),
            None,
        )
        .await?;
        Ok((app, host))
    }

    async fn deploy(
        &self,
        token: &Secret,
        org: &str,
        resource: &ResourcePlan,
        artifact: &Artifact,
    ) -> Result<Release, EngineError> {
        if resource.kind != ResourceKind::Service || !valid_slug(&resource.name) {
            return Err(crate::application::ports::providers::unsupported());
        }
        let directory = Path::new(artifact.image.as_deref().ok_or(EngineError::Invalid(
            "Fly needs a prepared Docker build context.",
        ))?);
        let app = resource.provider_ref.as_deref().unwrap_or(&resource.name);
        // Check flyctl runs before making anything Fly bills for.
        command(&self.http, token, &["version"], None, None).await?;
        if resource.provider_ref.is_none() {
            self.http
                .json(
                    Provider::Fly,
                    token,
                    Method::POST,
                    &["apps"],
                    &[],
                    Some(&json!({"app_name":app,"org_slug":org})),
                )
                .await?;
        }
        let config = json!({
            "app": app,
            "primary_region": resource.region.as_deref().unwrap_or("iad"),
            "env": {"PORT": "8080", "HOST": "0.0.0.0"},
            "http_service": {
                "internal_port": DEFAULT_PORT,
                "force_https": true,
                "auto_stop_machines": "stop",
                "auto_start_machines": true,
                "min_machines_running": 0,
            },
            "vm": [{"size": "shared-cpu-1x", "memory": "256mb"}],
        });
        let configuration = toml::to_string(&config).map_err(|_| EngineError::Store)?;
        tokio::fs::write(directory.join("hoista.fly.toml"), configuration)
            .await
            .map_err(|_| EngineError::Store)?;
        if !artifact.secrets.is_empty() {
            let payload = secret_input(&artifact.secrets)?;
            command(
                &self.http,
                token,
                &["secrets", "import", "--app", app, "--stage"],
                Some(directory),
                Some(&payload),
            )
            .await?;
        }
        command(
            &self.http,
            token,
            &[
                "deploy",
                "--app",
                app,
                "--config",
                "hoista.fly.toml",
                "--remote-only",
                "--ha=false",
                "--now",
            ],
            Some(directory),
            None,
        )
        .await?;
        self.release(token, app).await
    }

    async fn rollback(
        &self,
        token: &Secret,
        _: &str,
        release: &Release,
    ) -> Result<Release, EngineError> {
        if !valid_slug(&release.reference) || !release.version.contains("@sha256:") {
            return Err(EngineError::Invalid(
                "Rollback requires a saved immutable Fly image digest.",
            ));
        }
        // Save the app's config into an empty temporary folder, so no other
        // fly.toml (the daemon's folder, another project's) affects the rollback.
        let workspace = tempfile::tempdir().map_err(|_| EngineError::Store)?;
        command(
            &self.http,
            token,
            &["config", "save", "--app", &release.reference, "--yes"],
            Some(workspace.path()),
            None,
        )
        .await?;
        let path = workspace.path().join("fly.toml");
        if tokio::fs::metadata(&path)
            .await
            .map_err(|_| invalid())?
            .len()
            > MAX_CONFIG_BYTES
        {
            return Err(invalid());
        }
        let config = tokio::fs::read_to_string(&path)
            .await
            .map_err(|_| invalid())?;
        let config = rollback_configuration(&config, &release.reference)?;
        tokio::fs::write(&path, config)
            .await
            .map_err(|_| EngineError::Store)?;
        command(
            &self.http,
            token,
            &[
                "deploy",
                "--app",
                &release.reference,
                "--config",
                "fly.toml",
                "--image",
                &release.version,
                "--ha=false",
                "--now",
            ],
            Some(workspace.path()),
            None,
        )
        .await?;
        self.release(token, &release.reference).await
    }

    async fn set_secret(
        &self,
        token: &Secret,
        _: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        let values = vec![(name.into(), Secret::new(value.expose().into()))];
        let payload = secret_input(&values)?;
        command(
            &self.http,
            token,
            &["secrets", "import", "--app", reference],
            None,
            Some(&payload),
        )
        .await
    }
    // A watched Fly app's variables are its secrets.
    async fn set_provider_variable(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        self.set_secret(token, account, reference, name, value)
            .await
    }

    async fn set_provider_variables(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        values: &[(String, Secret)],
        shared: bool,
        _redeploy: bool,
    ) -> Result<(), EngineError> {
        if shared {
            return Err(crate::application::ports::providers::unsupported());
        }
        for (name, value) in values {
            self.set_secret(token, account, reference, name, value)
                .await?;
        }
        Ok(())
    }

    async fn delete_provider_variable(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        self.delete_secret(token, account, reference, name).await
    }

    async fn delete_secret(
        &self,
        token: &Secret,
        _: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        if !crate::domain::model::valid_env_name(name) {
            return Err(EngineError::Invalid(
                "Use a valid environment variable name.",
            ));
        }
        command(
            &self.http,
            token,
            &["secrets", "unset", name, "--app", reference],
            None,
            None,
        )
        .await
    }

    async fn health(&self, _: &Secret, _: &str, release: &Release) -> Result<(), EngineError> {
        self.http
            .health(release.url.as_deref().ok_or_else(invalid)?)
            .await
    }

    /// The app's recent log lines, as `fly logs --no-tail` reads them:
    /// Fly keeps a short window, read a page at a time from the oldest.
    async fn log_history(
        &self,
        token: &Secret,
        _org: &str,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        use crate::domain::logs::{LogEntry, LogHistory, LogLevel};
        query.milliseconds().map_err(EngineError::Invalid)?;
        if !valid_slug(reference) {
            return Err(EngineError::Invalid("A Fly app name is required."));
        }
        let scanner = crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
        let observed = chrono::Utc::now().to_rfc3339();
        let wanted = query.query.trim().to_lowercase();
        let mut items = Vec::new();
        let mut next = String::new();
        for _ in 0..5 {
            let mut params = vec![];
            if !next.is_empty() {
                params.push(("next_token", next.as_str()));
            }
            let page = self
                .http
                .get_at(
                    Provider::Fly,
                    token,
                    FLY_API,
                    &["api", "v1", "apps", reference, "logs"],
                    &params,
                )
                .await?;
            let rows = page["data"].as_array().cloned().unwrap_or_default();
            for row in &rows {
                let at = &row["attributes"];
                let Some(text) = at["message"]
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                else {
                    continue;
                };
                let timestamp = at["timestamp"].as_str().map(str::to_owned);
                if let Some(when) = timestamp.as_deref()
                    && (when < query.from.as_str() || when > query.to.as_str())
                {
                    continue;
                }
                if !wanted.is_empty() && !text.to_lowercase().contains(&wanted) {
                    continue;
                }
                let message = crate::domain::logs::clipped(
                    scanner.redact(text.trim_end(), std::slice::from_ref(token)),
                );
                let level = match at["level"].as_str().unwrap_or_default() {
                    "error" | "fatal" => LogLevel::Error,
                    "warn" | "warning" => LogLevel::Warn,
                    _ => LogLevel::Info,
                };
                let source = at["instance"]
                    .as_str()
                    .filter(|instance| !instance.is_empty())
                    .map_or_else(
                        || "app".to_owned(),
                        |instance| format!("machine {instance}"),
                    );
                items.push(LogEntry {
                    timestamp,
                    observed_at: observed.clone(),
                    level,
                    source,
                    message,
                });
            }
            let token_next = page["meta"]["next_token"].as_str().unwrap_or_default();
            if rows.is_empty() || token_next.is_empty() || token_next == next {
                break;
            }
            next = token_next.to_owned();
        }
        items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        items.truncate(100);
        items.reverse();
        Ok(LogHistory {
            items,
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: false,
        })
    }

    async fn delete(&self, token: &Secret, _: &str, reference: &str) -> Result<(), EngineError> {
        self.http
            .json(
                Provider::Fly,
                token,
                Method::DELETE,
                &["apps", reference],
                &[("force", "true")],
                None,
            )
            .await?;
        Ok(())
    }

    async fn pause(&self, token: &Secret, _: &str, reference: &str) -> Result<(), EngineError> {
        self.control(token, reference, false).await
    }

    async fn resume(
        &self,
        token: &Secret,
        _: &str,
        reference: &str,
    ) -> Result<ResourceStatus, EngineError> {
        self.control(token, reference, true).await?;
        let machines = self.machines(token, reference).await?;
        Ok(
            if machines.iter().any(|machine| machine["state"] == "started") {
                ResourceStatus::Live
            } else {
                ResourceStatus::Sleeping
            },
        )
    }
}

fn secret_input(values: &[(String, Secret)]) -> Result<Secret, EngineError> {
    let mut text = String::new();
    for (name, value) in values {
        // Refuse values dotenv would read differently, rather than change them.
        if !valid_env_name(name)
            || value.expose().contains(['\r', '\n', '"', '\'', '#'])
            || value.expose().trim() != value.expose()
        {
            return Err(EngineError::Invalid(
                "This Fly secret needs multiline or quoted encoding. Set it using the Fly dashboard before deploying.",
            ));
        }
        text.push_str(name);
        text.push('=');
        text.push_str(value.expose());
        text.push('\n');
    }
    Ok(Secret::new(text))
}

async fn command(
    http: &ProviderHttp,
    token: &Secret,
    args: &[&str],
    directory: Option<&Path>,
    input: Option<&Secret>,
) -> Result<(), EngineError> {
    // On a server the credential keeper runs it, with the token there.
    if let Some(key) = token.handle_key() {
        let keeper = http.keeper().ok_or(EngineError::Store)?;
        return if keeper
            .provider_cli(Provider::Fly, key, args, directory, input)
            .await?
        {
            Ok(())
        } else {
            Err(EngineError::Provider {
                evidence: None,
                code: "provider.cli_failed".into(),
                message: "Fly could not complete this operation. Inspect the app in Fly before retrying; command output was not retained because it may contain secrets.".into(),
            })
        };
    }
    // Hoista's own pinned flyctl, so nothing has to be installed first.
    let pinned = match manifest().cli.as_ref() {
        Some(cli) if http.tools().is_some() => Some(super::cli::binary(cli, http).await?),
        _ => None,
    };
    let cli = ENTRY.cli.as_ref().ok_or(EngineError::Store)?;
    let mut command = match &pinned {
        Some(path) => tokio::process::Command::new(path),
        None => hoista_system::tool_paths::command(cli.program),
    };
    command.args(args);
    cli.prepare(&mut command, token);
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let status = hoista_system::process::run(&mut command, input, FLYCTL_TIMEOUT)
        .await
        .map_err(|error| match error {
            hoista_system::process::ProcessError::Spawn(_) => {
                EngineError::Invalid("Install flyctl before deploying or rolling back Fly apps. If already installed, refresh hoist tools configure and restart the daemon.")
            }
            hoista_system::process::ProcessError::Timeout => EngineError::Provider {
                evidence: None,
                code: "provider.timeout".into(),
                message: "Fly deployment timed out. Its local process was stopped; inspect the remote release before retrying.".into(),
            },
            hoista_system::process::ProcessError::Cleanup => {
                EngineError::Invalid("Fly process cleanup could not be confirmed. Inspect local processes and the remote release before retrying.")
            }
            hoista_system::process::ProcessError::Io(_) => invalid(),
        })?;
    if !status.success() {
        return Err(EngineError::Provider {
            evidence: None,
            code: "provider.cli_failed".into(),
            message: "Fly could not complete this operation. Inspect the app in Fly before retrying; command output was not retained because it may contain secrets.".into(),
        });
    }
    Ok(())
}

// Image rollback must never rerun a schema migration or another release command.
fn rollback_configuration(config: &str, app: &str) -> Result<String, EngineError> {
    let mut config: toml::Value = toml::from_str(config).map_err(|_| invalid())?;
    if config.get("app").and_then(toml::Value::as_str) != Some(app) {
        return Err(invalid());
    }
    if let Some(deploy) = config.get_mut("deploy") {
        deploy
            .as_table_mut()
            .ok_or_else(invalid)?
            .remove("release_command");
    }
    toml::to_string(&config).map_err(|_| invalid())
}

/// Fly.io's manifest: its pinned CLI and the desktop app's buttons.
fn manifest() -> &'static super::manifest::Manifest {
    super::manifest::builtin("fly")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_app_is_as_its_machines_are() {
        let of = |states: &[&str], autostart: bool| {
            let machines: Vec<Value> = states
                .iter()
                .map(|state| json!({"state": state, "config": {"services": [{"autostart": autostart}]}}))
                .collect();
            machines_status(&machines)
        };
        assert_eq!(of(&[], false), ResourceStatus::NotDeployed);
        assert_eq!(of(&["stopped", "started"], false), ResourceStatus::Live);
        assert_eq!(of(&["replacing"], false), ResourceStatus::Deploying);
        assert_eq!(of(&["created", "stopped"], true), ResourceStatus::Sleeping);
        assert_eq!(of(&["destroyed"], false), ResourceStatus::NotDeployed);
        assert_eq!(of(&["failed", "stopped"], false), ResourceStatus::Failing);
        assert_eq!(of(&["stopped"], true), ResourceStatus::Sleeping);
        assert_eq!(of(&["suspended"], false), ResourceStatus::Stopped);
    }

    #[test]
    fn a_release_becomes_a_provider_deployment() {
        let row = json!({
            "id": "rel_1",
            "version": 7,
            "status": "complete",
            "description": "",
            "createdAt": "2026-11-01T10:00:00Z",
            "user": {"email": "sam@example.com"},
        });
        let deployment = fly_release("sam-app", &row).unwrap();
        assert_eq!(deployment.message.as_deref(), Some("Release v7"));
        assert!(matches!(
            deployment.state,
            crate::application::ports::providers::ProviderDeployState::Succeeded
        ));
        assert_eq!(deployment.author.as_deref(), Some("sam@example.com"));
    }

    #[test]
    fn prometheus_ranges_become_rows() {
        let value = json!({
            "status": "success",
            "data": {
                "resultType": "matrix",
                "result": [
                    {
                        "metric": {"status": "200"},
                        "values": [[1762000000, "4"], [1762000060, "NaN"]],
                    },
                ],
            },
        });
        let rows = prometheus_rows(&value);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0["status"], "200");
        assert_eq!(rows[0].1, "2025-11-01T12:26:40Z");
    }

    #[test]
    fn rollback_preserves_target_configuration_without_database_release_commands() {
        let original = "app = 'ours'\n[deploy]\nrelease_command = 'npx prisma migrate deploy'\nstrategy = 'rolling'\n[http_service]\ninternal_port = 8080\n";
        let saved = rollback_configuration(original, "ours").expect("config");
        let parsed: toml::Value = toml::from_str(&saved).expect("toml");
        assert!(parsed["deploy"].get("release_command").is_none());
        assert_eq!(parsed["deploy"]["strategy"].as_str(), Some("rolling"));
        assert_eq!(
            parsed["http_service"]["internal_port"].as_integer(),
            Some(8080)
        );
        assert!(rollback_configuration(original, "other").is_err());
    }
}

mod control;

mod domains;

#[cfg(test)]
mod release_tests {
    use super::newest_machine;
    use serde_json::json;

    fn machine(id: &str, state: &str, updated: &str) -> serde_json::Value {
        json!({
            "id": id,
            "state": state,
            "updated_at": updated,
            "image_ref": {
                "registry": "registry.fly.io",
                "repository": "app",
                "digest": format!("sha256:{id}"),
            },
        })
    }

    #[test]
    fn a_stopped_machine_still_shows_the_release() {
        let machines = [machine("a", "stopped", "2026-11-07T09:00:00Z")];
        assert_eq!(
            newest_machine(&machines).map(|m| &m["id"]),
            Some(&json!("a"))
        );
    }

    #[test]
    fn the_newest_machine_wins_then_a_started_one() {
        let machines = [
            machine("new", "stopped", "2026-11-07T10:00:00Z"),
            machine("old", "started", "2026-11-07T09:00:00Z"),
            machine("gone", "destroyed", "2026-11-07T11:00:00Z"),
        ];
        assert_eq!(
            newest_machine(&machines).map(|m| &m["id"]),
            Some(&json!("new"))
        );
        let same = [
            machine("idle", "stopped", "2026-11-07T10:00:00Z"),
            machine("live", "started", "2026-11-07T10:00:00Z"),
        ];
        assert_eq!(
            newest_machine(&same).map(|m| &m["id"]),
            Some(&json!("live"))
        );
    }
}
