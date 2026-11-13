//! Vercel: a team's projects, deployments, variables and domains. Logs,
//! metrics and anything else the REST API lacks go through the Vercel CLI.

use super::{
    answers::{field, invalid},
    http::ProviderHttp,
};
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{ProviderAdapter, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;

/// Vercel's entry in the provider list.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Vercel,
    logo: LOGO,
    capabilities: Capabilities {
        bills: true,
        discover: true,
        browser_sign_in: CLIENT_ID.is_some(),
        provider_operations: true,
        provider_details: true,
        provider_variables: true,
        logs: true,
        metrics: true,
        delete: true,
        new_project: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://api.vercel.com/",
        hosts: &["api.vercel.com"],
        explains_refusals: false,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["v2", "teams"],
            query: &[("limit", "100")],
            page: None,
            cursor: Some("until"),
        },
        rows: &["teams"],
        item: &[],
        id: "id",
        pages: super::accounts::Pages::Next(&["pagination", "next"]),
    },
    adapter: |http| std::sync::Arc::new(Vercel::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// Hoista's "Sign in with Vercel" app: a public client (auth method `none`)
/// with PKCE. The person picks the team and the API permissions on Vercel's
/// page. Access tokens last an hour; `offline_access` gives a 30-day refresh
/// token that rotates on every use. Registered with
/// `http://127.0.0.1:{47133,47134}/callback`.
const CLIENT_ID: Option<&str> = option_env!("HOISTA_VERCEL_CLIENT_ID");

fn oauth_app() -> Option<crate::application::ports::providers::ProviderOAuth> {
    CLIENT_ID.map(
        |client_id| crate::application::ports::providers::ProviderOAuth {
            authorize_url: "https://vercel.com/oauth/authorize",
            client_id,
            scopes: "openid email profile offline_access",
            ports: &[47133, 47134],
            extra: &[],
            redirect_host: "127.0.0.1",
            logo: LOGO,
        },
    )
}

const OAUTH_TOKEN_URL: &str = "https://api.vercel.com/login/oauth/token";

/// Vercel's mark, for Hoista's sign-in pages.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M24 22.525H0l12-21.05 12 21.05z" fill="currentColor"/></svg>"##;

/// A fake of the provider for the conformance suite: enough to connect and
/// find one resource.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "acct-1",
    resource: "res-1",
    kind: crate::domain::model::ResourceKind::Web,
    routes: &[
        super::conformance::Route {
            path: "/v2/teams",
            body: "",
            answer: r#"{"teams":[{"id":"acct-1","name":"Sam"}],"pagination":{"next":null}}"#,
        },
        super::conformance::Route {
            path: "/v10/projects",
            body: "",
            answer: r#"{"projects":[{"id":"res-1","name":"web"}],"pagination":{"next":null}}"#,
        },
    ],
    empty: r#"{}"#,
};

/// Vercel's API adapter, for one team.
pub struct Vercel {
    provider: Provider,
    http: ProviderHttp,
}

impl Vercel {
    /// Shares the allow-listed HTTP boundary.
    #[must_use]
    pub fn new(http: ProviderHttp) -> Self {
        Self {
            provider: Provider::Vercel,
            http,
        }
    }

    async fn projects(
        &self,
        token: &Secret,
        team: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut result = Vec::new();
        let mut cursor = String::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let mut query = vec![("teamId", team), ("limit", "100")];
            if !cursor.is_empty() {
                query.push(("from", &cursor));
            }
            let value = self
                .http
                .json(
                    self.provider,
                    token,
                    Method::GET,
                    &["v10", "projects"],
                    &query,
                    None,
                )
                .await?;
            let projects = value["projects"].as_array().ok_or_else(invalid)?;
            for p in projects {
                let deployment = &p["targets"]["production"];
                result.push(RemoteResource {
                    reference: field(p, "id")?,
                    name: field(p, "name")?,
                    kind: ResourceKind::Web,
                    region: None,
                    status: match deployment["readyState"].as_str() {
                        Some("READY") => ResourceStatus::Live,
                        Some("ERROR" | "CANCELED") => ResourceStatus::Failing,
                        _ => ResourceStatus::Unknown,
                    },
                    url: production_url(deployment),
                    group: None,
                });
            }
            let next = &value["pagination"]["next"];
            if next.is_null() {
                return Ok(result);
            }
            cursor = if let Some(s) = next.as_str() {
                s.into()
            } else if let Some(n) = next.as_u64() {
                n.to_string()
            } else {
                return Err(invalid());
            };
            if cursor.is_empty() || !seen.insert(cursor.clone()) || projects.is_empty() {
                return Err(invalid());
            }
        }
        Err(EngineError::Conflict(
            "Vercel discovery exceeded 10,000 projects.",
        ))
    }

    async fn get(
        &self,
        token: &Secret,
        team: &str,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<serde_json::Value, EngineError> {
        let mut query = query.to_vec();
        query.push(("teamId", team));
        self.http
            .json(self.provider, token, Method::GET, path, &query, None)
            .await
    }

    /// The project's newest deployments, at most 50.
    async fn deployments(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
    ) -> Result<Vec<serde_json::Value>, EngineError> {
        let value = self
            .get(
                token,
                team,
                &["v6", "deployments"],
                &[("projectId", project), ("limit", "50")],
            )
            .await?;
        Ok(value["deployments"].as_array().cloned().unwrap_or_default())
    }

    /// The project's variables as Vercel lists them: names, targets and ids,
    /// never their values (Hoista doesn't ask Vercel to decrypt them).
    async fn variables(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
    ) -> Result<Vec<serde_json::Value>, EngineError> {
        let value = self
            .get(token, team, &["v10", "projects", project, "env"], &[])
            .await?;
        Ok(value["envs"].as_array().cloned().unwrap_or_default())
    }
}

impl Vercel {
    /// Runtime and request logs from `vercel logs`, oldest first, redacted.
    async fn runtime_logs(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<Vec<crate::domain::logs::LogEntry>, EngineError> {
        let cli = super::manifest::builtin("vercel")
            .cli
            .as_ref()
            .ok_or(EngineError::Store)?;
        let mut args = serde_json::json!({
            "since": query.from,
            "until": query.to,
            "limit": "100",
        });
        if !query.query.trim().is_empty() {
            args["query"] = serde_json::Value::String(query.query.clone());
        }
        let answer = super::cli::run(
            cli,
            &self.http,
            token,
            super::cli::Aim {
                context: &[("team", team), ("project", project)],
                folder: None,
            },
            "vercel logs",
            args,
        )
        .await?;
        let lines = match &answer["output"] {
            serde_json::Value::Array(lines) => lines.clone(),
            other => vec![other.clone()],
        };
        let mut items: Vec<_> = lines.iter().filter_map(log_line).collect();
        items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(items)
    }
}

/// The address people use for a project's production deployment: its own
/// domain when it has one, then its lasting `.vercel.app` name, and only
/// then the deployment's one-off address.
fn production_url(deployment: &serde_json::Value) -> Option<String> {
    let aliases: Vec<&str> = deployment["alias"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|host| !host.is_empty() && !host.contains('/'))
        .collect();
    let custom = aliases.iter().find(|host| !host.ends_with(".vercel.app"));
    // The shortest `.vercel.app` alias is the project's own name; longer
    // ones carry the team or branch.
    let lasting = aliases
        .iter()
        .filter(|host| host.ends_with(".vercel.app"))
        .min_by_key(|host| host.len());
    custom
        .or(lasting)
        .copied()
        .or_else(|| deployment["url"].as_str())
        .map(|host| format!("https://{host}"))
}

/// One line of `vercel logs` JSON (`{"type": …, "data": {…}}`) as a log
/// entry; warnings and summaries are skipped.
fn log_line(line: &serde_json::Value) -> Option<crate::domain::logs::LogEntry> {
    use crate::domain::logs::LogLevel;
    let kind = line["type"].as_str().unwrap_or_default();
    if matches!(kind, "warning" | "summary" | "error") {
        return None;
    }
    let data = if line["data"].is_object() {
        &line["data"]
    } else {
        line
    };
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| data[*key].as_str().filter(|value| !value.is_empty()))
            .map(str::to_owned)
    };
    let status = data["statusCode"]
        .as_i64()
        .or_else(|| data["responseStatusCode"].as_i64())
        .or_else(|| data["status"].as_i64());
    let request = match (
        text(&["requestMethod", "method"]),
        text(&["requestPath", "path"]),
    ) {
        (Some(method), Some(path)) => Some(match status {
            Some(code) => format!("{method} {path} {code}"),
            None => format!("{method} {path}"),
        }),
        _ => None,
    };
    let message = match (text(&["message", "msg", "text"]), request) {
        (Some(message), Some(request)) => format!("{request} · {message}"),
        (Some(message), None) => message,
        (None, Some(request)) => request,
        (None, None) => return None,
    };
    let timestamp = data["timestamp"]
        .as_i64()
        .or_else(|| data["date"].as_i64())
        .and_then(|ms| at(Some(ms)))
        .or_else(|| text(&["timestamp", "time", "date"]));
    let stated = text(&["level", "severity"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let level = if stated.contains("error")
        || stated == "fatal"
        || status.is_some_and(|code| code >= 500)
    {
        LogLevel::Error
    } else if stated.contains("warn") || status.is_some_and(|code| code >= 400) {
        LogLevel::Warn
    } else if stated.contains("debug") {
        LogLevel::Debug
    } else {
        LogLevel::Info
    };
    let message = crate::domain::logs::clipped(message);
    Some(crate::domain::logs::LogEntry {
        timestamp,
        observed_at: chrono::Utc::now().to_rfc3339(),
        level,
        source: if kind.is_empty() {
            "runtime".into()
        } else {
            kind.replace('_', " ")
        },
        message,
    })
}

/// A Vercel deployment's state as Hoista reads it.
fn deploy_state(state: &str) -> crate::application::ports::providers::ProviderDeployState {
    use crate::application::ports::providers::ProviderDeployState;
    match state {
        "READY" => ProviderDeployState::Succeeded,
        "ERROR" => ProviderDeployState::Failed,
        "CANCELED" | "DELETED" => ProviderDeployState::Stopped,
        _ => ProviderDeployState::InProgress,
    }
}

/// Milliseconds since the epoch as RFC 3339.
fn at(ms: Option<i64>) -> Option<String> {
    ms.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn deployment(
    row: &serde_json::Value,
) -> Option<crate::application::ports::providers::ProviderDeployment> {
    let meta = &row["meta"];
    let text = |value: &serde_json::Value| value.as_str().map(str::to_owned);
    let state = row["state"]
        .as_str()
        .or_else(|| row["readyState"].as_str())
        .unwrap_or("QUEUED");
    let repo = match (meta["githubOrg"].as_str(), meta["githubRepo"].as_str()) {
        (Some(org), Some(name)) => Some(format!("{org}/{name}")),
        _ => None,
    };
    Some(crate::application::ports::providers::ProviderDeployment {
        id: text(&row["uid"])?,
        status: state.to_owned(),
        state: deploy_state(state),
        created_at: at(row["created"]
            .as_i64()
            .or_else(|| row["createdAt"].as_i64()))?,
        url: text(&row["url"]).map(|host| format!("https://{host}")),
        commit: text(&meta["githubCommitSha"]),
        message: text(&meta["githubCommitMessage"]),
        branch: text(&meta["githubCommitRef"]),
        repo,
        finished_at: matches!(state, "READY" | "ERROR")
            .then(|| at(row["ready"].as_i64()))
            .flatten(),
        can_roll_back: None,
        author: text(&meta["githubCommitAuthorName"]),
        // Vercel gives each deployment's page in its dashboard.
        provider_page: text(&row["inspectorUrl"])
            .filter(|url| url.starts_with("https://vercel.com/")),
    })
}

/// A build log line's level. Build tools print progress on stderr too
/// ("Build Completed in /vercel/output"), so stderr alone isn't an error
/// here: only a fatal event, or text that says something failed.
fn build_level(kind: &str, text: &str) -> crate::domain::logs::LogLevel {
    use crate::domain::logs::LogLevel;
    let lower = text.to_ascii_lowercase();
    if kind == "fatal"
        || lower.contains("error")
        || lower.contains("failed")
        || lower.contains("err!")
    {
        LogLevel::Error
    } else if lower.contains("warn") {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

impl Vercel {
    /// One of Vercel's observability metrics for the project's production
    /// traffic, from `vercel metrics`, as `{timestamp, <group>, <value>}` rows.
    async fn metric(
        &self,
        token: &Secret,
        (team, project): (&str, &str),
        window: (&str, &str, &str),
        metric: &str,
        aggregation: &str,
        group_by: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, EngineError> {
        let cli = super::manifest::builtin("vercel")
            .cli
            .as_ref()
            .ok_or(EngineError::Store)?;
        let (since, until, granularity) = window;
        // Vercel picks each metric's own aggregation when none is named
        // (counts take `count`, not `sum`), so one is named only for
        // percentiles. A refused query is asked once more with only the
        // metric and range, as Vercel's own defaults.
        let mut full = serde_json::json!({
            "metric_id": metric,
            "environment": "production",
            "since": since,
            "until": until,
            "granularity": granularity,
        });
        if !aggregation.is_empty() {
            full["aggregation"] = serde_json::Value::String(aggregation.to_owned());
        }
        let mut plain = serde_json::json!({"metric_id": metric, "since": since, "until": until});
        if !aggregation.is_empty() {
            plain["aggregation"] = serde_json::Value::String(aggregation.to_owned());
        }
        if let Some(group) = group_by {
            full["group-by"] = serde_json::Value::String(group.to_owned());
            plain["group-by"] = serde_json::Value::String(group.to_owned());
        }
        let context = [("team", team), ("project", project)];
        let ask = |args: serde_json::Value| {
            super::cli::run(
                cli,
                &self.http,
                token,
                super::cli::Aim {
                    context: &context,
                    folder: None,
                },
                "vercel metrics",
                args,
            )
        };
        let answer = match ask(full).await {
            Ok(answer) => answer,
            Err(EngineError::Provider { .. }) => ask(plain).await.map_err(plan_needed)?,
            Err(error) => return Err(error),
        };
        Ok(metric_rows(&answer["output"]))
    }
}

/// Vercel's own refusal when the team's plan lacks the metrics, as a plan
/// limit rather than a failure, so it reads as what to change.
fn plan_needed(error: EngineError) -> EngineError {
    match error {
        EngineError::Provider { message, .. } if message.contains("Observability Plus") => {
            EngineError::Provider {
                evidence: None,
                code: "provider.plan".into(),
                message: "Vercel shows these numbers with Observability Plus, an add-on to its \
                          Pro plan."
                    .into(),
            }
        }
        other => other,
    }
}

/// The rows of `vercel metrics --format json` (`{query, summary, data}`).
fn metric_rows(output: &serde_json::Value) -> Vec<serde_json::Value> {
    let output = match output {
        serde_json::Value::Array(lines) => lines
            .iter()
            .find(|line| line.get("data").is_some())
            .unwrap_or(&serde_json::Value::Null),
        other => other,
    };
    output["data"].as_array().cloned().unwrap_or_default()
}

/// A row's value: the one number that isn't the time or a group.
fn metric_value(row: &serde_json::Value, group: Option<&str>) -> Option<f64> {
    row.as_object()?
        .iter()
        .filter(|(key, _)| key.as_str() != "timestamp" && Some(key.as_str()) != group)
        .find_map(|(_, value)| value.as_f64())
}

/// Rows as `(time, value)` pairs.
fn metric_pairs(rows: &[serde_json::Value], group: Option<&str>) -> Vec<(String, f64)> {
    rows.iter()
        .filter_map(|row| {
            Some((
                super::metrics::time_of(&row["timestamp"])?,
                metric_value(row, group)?,
            ))
        })
        .collect()
}

/// Vercel's bucket sizes: the one that keeps a range to a few hundred points.
fn vercel_granularity(span: chrono::Duration) -> &'static str {
    if span <= chrono::Duration::hours(3) {
        "5m"
    } else if span <= chrono::Duration::days(2) {
        "15m"
    } else if span <= chrono::Duration::days(8) {
        "1h"
    } else {
        "1d"
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Vercel {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// A Vercel project from a repository: a site or web app, built by
    /// Vercel from its framework.
    async fn provider_add(
        &self,
        token: &Secret,
        team: &str,
        _project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        match *what {
            crate::application::ports::providers::ProviderNew::Repo {
                name,
                repo,
                branch,
                runs_as: None | Some("web" | "static"),
                schedule: None,
                ..
            } => {
                self.provider_new_project(token, team, name, repo, branch)
                    .await
            }
            crate::application::ports::providers::ProviderNew::Repo { .. } => {
                Err(EngineError::Invalid(
                    "Vercel runs a repository as a site or web app: runs_as web or static, without a schedule.",
                ))
            }
            _ => Err(crate::application::ports::providers::unsupported()),
        }
    }

    /// A Vercel project linked to the GitHub repository, with its first
    /// production deployment started. Vercel builds every push after that.
    /// Vercel's GitHub app must reach the repository.
    async fn provider_new_project(
        &self,
        token: &Secret,
        team: &str,
        name: &str,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let valid_repo = repo.split_once('/').is_some_and(|(owner, rest)| {
            !owner.is_empty()
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
        let project = self
            .http
            .json(
                self.provider,
                token,
                Method::POST,
                &["v11", "projects"],
                &[("teamId", team)],
                Some(&serde_json::json!({
                    "name": name,
                    "gitRepository": {"type": "github", "repo": repo},
                })),
            )
            .await?;
        let id = field(&project, "id")?;
        let link = &project["link"];
        let branch = branch
            .map(str::to_owned)
            .or_else(|| link["productionBranch"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "main".into());
        // The first deployment; Vercel only builds on the next push otherwise.
        let started = match link["repoId"].as_i64() {
            Some(repo_id) => self
                .http
                .json(
                    self.provider,
                    token,
                    Method::POST,
                    &["v13", "deployments"],
                    &[("teamId", team)],
                    Some(&serde_json::json!({
                        "name": name,
                        "project": id,
                        "target": "production",
                        "gitSource": {"type": "github", "repoId": repo_id, "ref": branch},
                    })),
                )
                .await
                .is_ok(),
            None => false,
        };
        Ok((
            vec![id],
            if started {
                format!(
                    "Created the Vercel project {name} from {repo} and started its first deployment from {branch}"
                )
            } else {
                format!(
                    "Created the Vercel project {name} from {repo}; it deploys on the next push to {branch}"
                )
            },
        ))
    }

    /// The project's deployments, variable names and domains, from Vercel.
    async fn provider_details(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        use crate::application::ports::providers::{
            ProviderDnsRecord, ProviderDomain, ProviderDomainState,
        };
        let deployments = self
            .deployments(token, team, project)
            .await?
            .iter()
            .filter_map(deployment)
            .collect();
        let mut names: Vec<String> = self
            .variables(token, team, project)
            .await?
            .iter()
            .filter_map(|env| env["key"].as_str().map(str::to_owned))
            .collect();
        names.sort();
        names.dedup();
        let listed = self
            .get(token, team, &["v9", "projects", project, "domains"], &[])
            .await?;
        let domains = listed["domains"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let name = row["name"].as_str()?.to_owned();
                let verified = row["verified"].as_bool().unwrap_or(true);
                Some(ProviderDomain {
                    custom: !name.ends_with(".vercel.app"),
                    state: Some(if verified {
                        ProviderDomainState::Working
                    } else {
                        ProviderDomainState::Pending
                    }),
                    records: row["verification"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|record| {
                            Some(ProviderDnsRecord {
                                kind: record["type"].as_str()?.to_owned(),
                                name: record["domain"].as_str()?.to_owned(),
                                value: record["value"].as_str()?.to_owned(),
                            })
                        })
                        .collect(),
                    port: None,
                    certificate: None,
                    dns: vec![],
                    name,
                })
            })
            .collect();
        Ok(crate::application::ports::providers::ProviderDetails {
            deployments,
            variable_names: Some(names),
            shared_variable_names: None,
            domains,
            references: vec![],
            sleeps: None,
            config: None,
            volumes: vec![],
            tcp_proxies: vec![],
            addresses: vec![],
            private_domain: None,
        })
    }

    /// What Vercel has billed the team so far this month, from its billing
    /// charges (FOCUS rows, one per product and day).
    async fn provider_bill(
        &self,
        token: &Secret,
        team: &str,
    ) -> Result<Option<crate::application::ports::providers::ProviderBill>, EngineError> {
        let now = chrono::Utc::now();
        let from =
            super::runpod::month_start(now).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let to = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let rows = self
            .get(
                token,
                team,
                &["v1", "billing", "charges"],
                &[("from", &from), ("to", &to)],
            )
            .await?;
        let rows = match rows {
            serde_json::Value::Array(rows) => rows,
            serde_json::Value::Null => vec![],
            // One row comes back as a single object.
            row => vec![row],
        };
        Ok(super::http::focus_bill(&rows, "BilledCost").or_else(|| {
            // No charges yet this month: nothing billed.
            Some(crate::application::ports::providers::ProviderBill {
                billed: Money {
                    cents: 0,
                    currency: "USD".into(),
                },
                period_start: Some(from.clone()),
                period_end: None,
                soft_limit: None,
                hard_limit: None,
                over_limit: false,
                customer_id: None,
            })
        }))
    }

    /// The project's production traffic as Vercel's observability measures
    /// it: requests by status, function invocations and how long they took.
    async fn provider_metrics(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        _step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::{by_status_class, chart, line, points, stamp};
        use crate::application::ports::providers::{MetricSummary as S, MetricUnit as U};
        let (since, until) = (stamp(from), stamp(to));
        let window = (
            since.as_str(),
            until.as_str(),
            vercel_granularity(to - from),
        );
        let (requests, invocations, p50, p95) = tokio::join!(
            self.metric(
                token,
                (team, project),
                window,
                "vercel.request.count",
                "",
                Some("httpStatus")
            ),
            self.metric(
                token,
                (team, project),
                window,
                "vercel.function_invocation.count",
                "",
                None
            ),
            self.metric(
                token,
                (team, project),
                window,
                "vercel.function_invocation.function_duration_ms",
                "p50",
                None
            ),
            self.metric(
                token,
                (team, project),
                window,
                "vercel.function_invocation.function_duration_ms",
                "p95",
                None
            ),
        );
        // A project with nothing measured yet answers nothing at all: say why
        // rather than showing an empty page.
        if requests.is_err() && invocations.is_err() && p50.is_err() && p95.is_err() {
            return Err(requests.err().unwrap_or(EngineError::Store));
        }
        let mut missing = Vec::new();
        let mut rows = |result: Result<Vec<serde_json::Value>, EngineError>, what: &'static str| {
            result.unwrap_or_else(|_| {
                missing.push(what);
                Vec::new()
            })
        };
        let requests = rows(requests, "requests");
        let invocations = rows(invocations, "function invocations");
        let p50 = rows(p50, "function duration");
        let p95 = rows(p95, "function duration");
        missing.dedup();
        let statuses = by_status_class(requests.iter().filter_map(|row| {
            let status = row["httpStatus"]
                .as_u64()
                .or_else(|| row["httpStatus"].as_str()?.parse().ok())?;
            Some((
                u16::try_from(status).ok()?,
                super::metrics::time_of(&row["timestamp"])?,
                metric_value(row, Some("httpStatus"))?,
            ))
        }));
        let charts = vec![
            chart("requests", "Requests", U::Count, S::Total, statuses),
            chart(
                "invocations",
                "Function invocations",
                U::Count,
                S::Total,
                vec![line(
                    "Invocations",
                    points(metric_pairs(&invocations, None)),
                )],
            ),
            chart(
                "duration",
                "Function duration",
                U::Milliseconds,
                S::Average,
                vec![
                    line("p50", points(metric_pairs(&p50, None))),
                    line("p95", points(metric_pairs(&p95, None))),
                ],
            ),
        ];
        Ok(super::metrics::metrics(
            from, to, charts, &missing, "Vercel",
        ))
    }

    /// Adds or removes one of the project's domains; Vercel's own
    /// `.vercel.app` address stays.
    async fn provider_action(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
        action: &crate::application::ports::providers::ProviderAction<'_>,
    ) -> Result<String, EngineError> {
        use crate::application::ports::providers::ProviderAction;
        let domain_name = |name: &str| {
            !name.is_empty()
                && name.len() <= crate::domain::domains::MAX_HOSTNAME_CHARS
                && name.contains('.')
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
        };
        match action {
            ProviderAction::AddDomain {
                domain: Some(domain),
                ..
            } if domain_name(domain) => {
                self.http
                    .json(
                        self.provider,
                        token,
                        Method::POST,
                        &["v10", "projects", project, "domains"],
                        &[("teamId", team)],
                        Some(&serde_json::json!({ "name": domain.to_ascii_lowercase() })),
                    )
                    .await?;
                Ok(format!("Added {domain}"))
            }
            ProviderAction::RemoveDomain { domain } if domain_name(domain) => {
                if domain.ends_with(".vercel.app") {
                    return Err(EngineError::Invalid(
                        "Vercel's own address stays with the project.",
                    ));
                }
                self.http
                    .json(
                        self.provider,
                        token,
                        Method::DELETE,
                        &["v9", "projects", project, "domains", domain],
                        &[("teamId", team)],
                        None,
                    )
                    .await?;
                Ok(format!("Removed {domain}"))
            }
            _ => Err(crate::application::ports::providers::unsupported()),
        }
    }

    /// The project's runtime and request logs in the time range, as
    /// `vercel logs` reads them; when the command-line tool can't, the newest
    /// deployment's build output from the API.
    async fn log_history(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        use crate::domain::logs::{LogEntry, LogHistory};
        let (from, to) = query.milliseconds().map_err(EngineError::Invalid)?;
        if let Ok(items) = self.runtime_logs(token, team, project, query).await {
            return Ok(LogHistory {
                items,
                next_cursor: None,
                query_sampling_level: None,
                coverage_complete: false,
            });
        }
        let newest = self.deployments(token, team, project).await?;
        let mut items = Vec::new();
        if let Some(id) = newest.first().and_then(|row| row["uid"].as_str()) {
            let events = self
                .get(
                    token,
                    team,
                    &["v3", "deployments", id, "events"],
                    &[("limit", "100"), ("direction", "backward")],
                )
                .await?;
            let scanner =
                crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
            let needle = query.query.to_lowercase();
            for event in events.as_array().into_iter().flatten() {
                let created = event["created"].as_i64().unwrap_or_default();
                if created < from || created > to {
                    continue;
                }
                let kind = event["type"].as_str().unwrap_or_default();
                let Some(text) = event["payload"]["text"]
                    .as_str()
                    .or_else(|| event["text"].as_str())
                    .filter(|text| !text.trim().is_empty())
                else {
                    continue;
                };
                if !needle.is_empty() && !text.to_lowercase().contains(&needle) {
                    continue;
                }
                let message =
                    crate::domain::logs::clipped(scanner.redact(text, std::slice::from_ref(token)));
                items.push(LogEntry {
                    timestamp: at(Some(created)),
                    observed_at: chrono::Utc::now().to_rfc3339(),
                    level: build_level(kind, text),
                    source: "deployment".into(),
                    message,
                });
            }
        }
        items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(LogHistory {
            items,
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: false,
        })
    }

    /// Sets one variable for Production, Preview and Development.
    async fn set_provider_variable(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        let values = [(name.to_owned(), Secret::new(value.expose().into()))];
        self.set_provider_variables(token, team, project, &values, false, false)
            .await
    }

    /// Sets variables, encrypted, for Production, Preview and Development,
    /// replacing ones of the same name. Vercel applies them on the next deploy.
    async fn set_provider_variables(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
        values: &[(String, Secret)],
        _shared: bool,
        _redeploy: bool,
    ) -> Result<(), EngineError> {
        if values.is_empty() || values.len() > 100 {
            return Err(EngineError::Invalid(
                "Set between 1 and 100 variables at once.",
            ));
        }
        let body: Vec<serde_json::Value> = values
            .iter()
            .map(|(name, value)| {
                serde_json::json!({
                    "key": name,
                    "value": value.expose(),
                    "type": "encrypted",
                    "target": ["production", "preview", "development"],
                })
            })
            .collect();
        self.http
            .json(
                self.provider,
                token,
                Method::POST,
                &["v10", "projects", project, "env"],
                &[("upsert", "true"), ("teamId", team)],
                Some(&serde_json::Value::Array(body)),
            )
            .await
            .map(|_| ())
    }

    /// Deletes every variable of that name, whatever environments it's for.
    async fn delete_provider_variable(
        &self,
        token: &Secret,
        team: &str,
        project: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        let ids: Vec<String> = self
            .variables(token, team, project)
            .await?
            .iter()
            .filter(|env| env["key"].as_str() == Some(name))
            .filter_map(|env| env["id"].as_str().map(str::to_owned))
            .collect();
        if ids.is_empty() {
            return Err(EngineError::NotFound);
        }
        for id in ids {
            self.http
                .json(
                    self.provider,
                    token,
                    Method::DELETE,
                    &["v9", "projects", project, "env", &id],
                    &[("teamId", team)],
                    None,
                )
                .await?;
        }
        Ok(())
    }

    /// Deletes the project at Vercel, with its deployments and domains.
    async fn delete(&self, token: &Secret, team: &str, project: &str) -> Result<(), EngineError> {
        if project.is_empty()
            || !project
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(EngineError::Invalid("A Vercel project ID is required."));
        }
        self.http
            .json(
                self.provider,
                token,
                Method::DELETE,
                &["v9", "projects", project],
                &[("teamId", team)],
                None,
            )
            .await?;
        Ok(())
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        super::manifest::builtin("vercel").operations()
    }

    fn prepare_tools(&self) {
        if let Some(cli) = super::manifest::builtin("vercel").cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        super::manifest::builtin("vercel").actions.clone()
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
        let manifest = super::manifest::builtin("vercel");
        // The project and team the resource and account name: the API's
        // `idOrName` and `teamId`, the CLI's VERCEL_PROJECT_ID and VERCEL_ORG_ID.
        let mut context = vec![("team", external_id)];
        context.extend(reference.map(|project| ("project", project)));
        if manifest.api_operation(name).is_some() {
            return super::api_catalog::run_manifest(
                &self.http,
                self.provider,
                manifest,
                token,
                &context,
                name,
                args,
            )
            .await;
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
    }

    fn oauth(&self) -> Option<crate::application::ports::providers::ProviderOAuth> {
        oauth_app()
    }

    async fn oauth_token(
        &self,
        grant: crate::application::ports::providers::OAuthGrant<'_>,
    ) -> Result<crate::application::ports::providers::OAuthTokens, EngineError> {
        let client_id = CLIENT_ID.ok_or_else(super::oauth::not_set_up)?;
        super::oauth::public_token(&self.http, OAUTH_TOKEN_URL, client_id, grant).await
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        super::accounts::list(&self.http, self.provider, token).await
    }

    async fn verify(&self, token: &Secret, external_id: &str) -> Result<Vec<String>, EngineError> {
        self.discover(token, external_id).await?;
        Ok(vec!["resources:read (verified by request)".into()])
    }

    async fn discover(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        self.projects(token, external_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_known_by_its_own_domain() {
        let deployment = serde_json::json!({
            "url": "portfolio-v21-bb6kmjeya-team.vercel.app",
            "alias": [
                "portfolio-v21.vercel.app",
                "portfolio-v21-team.vercel.app",
                "www.example.com",
            ],
        });
        assert_eq!(
            production_url(&deployment).as_deref(),
            Some("https://www.example.com")
        );
        let plain = serde_json::json!({"url": "x-1.vercel.app", "alias": ["x.vercel.app", "x-team.vercel.app"]});
        assert_eq!(
            production_url(&plain).as_deref(),
            Some("https://x.vercel.app")
        );
        assert_eq!(
            production_url(&serde_json::json!({"url": "x-1.vercel.app"})).as_deref(),
            Some("https://x-1.vercel.app")
        );
    }

    #[test]
    fn metric_rows_read_vercels_json() {
        let output = serde_json::json!({
            "query": {},
            "summary": [],
            "data": [
                {
                    "timestamp": "2026-11-01T10:00:00.000Z",
                    "httpStatus": 200,
                    "vercel_request_count_sum": 12,
                },
                {
                    "timestamp": "2026-11-01T10:00:00.000Z",
                    "httpStatus": 500,
                    "vercel_request_count_sum": 1,
                },
            ],
        });
        let rows = metric_rows(&output);
        assert_eq!(rows.len(), 2);
        assert_eq!(metric_value(&rows[1], Some("httpStatus")), Some(1.0));
        assert_eq!(
            metric_pairs(&rows, Some("httpStatus"))[0].0,
            "2026-11-01T10:00:00Z"
        );
        assert_eq!(vercel_granularity(chrono::Duration::days(30)), "1d");
    }

    #[test]
    fn deployments_read_as_hoista_reads_them() {
        let row = serde_json::json!({
            "uid": "dpl_1",
            "state": "READY",
            "created": 1_762_400_000_000_i64,
            "ready": 1_762_400_060_000_i64,
            "url": "web-abc.vercel.app",
            "meta": {
                "githubCommitSha": "abc",
                "githubCommitRef": "main",
                "githubOrg": "sam",
                "githubRepo": "web",
                "githubCommitAuthorName": "Sam",
            },
        });
        let deployment = deployment(&row).unwrap();
        assert_eq!(
            deployment.state,
            crate::application::ports::providers::ProviderDeployState::Succeeded
        );
        assert_eq!(
            deployment.url.as_deref(),
            Some("https://web-abc.vercel.app")
        );
        assert_eq!(deployment.repo.as_deref(), Some("sam/web"));
        assert!(deployment.finished_at.is_some());
        assert_eq!(
            deploy_state("BUILDING"),
            crate::application::ports::providers::ProviderDeployState::InProgress
        );
        assert_eq!(
            build_level("stderr", "Build Completed in /vercel/output [234ms]"),
            crate::domain::logs::LogLevel::Info
        );
        assert_eq!(
            build_level("stderr", "Error: Command \"npm run build\" exited with 1"),
            crate::domain::logs::LogLevel::Error
        );
        let request = log_line(&serde_json::json!({
            "type": "request",
            "data": {
                "timestamp": 1_762_400_000_000_i64,
                "requestMethod": "GET",
                "requestPath": "/api",
                "statusCode": 502,
            },
        }))
        .unwrap();
        assert_eq!(request.message, "GET /api 502");
        assert_eq!(request.level, crate::domain::logs::LogLevel::Error);
        assert!(log_line(&serde_json::json!({"type": "warning", "message": "x"})).is_none());
    }
}
