//! Supabase: the projects in one organization, each found with its
//! database, edge functions and storage buckets, plus their logs, secrets,
//! the Management API and the Supabase CLI.

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

const MAX_NAME_CHARS: usize = 128;
const MAX_FUNCTIONS: usize = 500;
/// Supabase's log queries cover at most a day.
const LOG_SPAN: chrono::Duration = chrono::Duration::hours(24);
/// Identical log queries within this long share one answer, up to
/// `MAX_LOG_ANSWERS` of them; Supabase rate-limits its log endpoint.
const LOG_ANSWER_LIFETIME: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_LOG_ANSWERS: usize = 256;
/// How much of the token's digest keys a shared answer.
const TOKEN_KEY_BYTES: usize = 12;
/// How far into a line its level is looked for.
const LEVEL_PREFIX_CHARS: usize = 40;
const MAX_TRAFFIC_DAYS: i64 = 31;

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M11.9 1.036c-.015-.986-1.26-1.41-1.874-.637L.764 12.05C-.33 13.427.65 15.455 2.409 15.455h9.579l.113 7.51c.014.985 1.259 1.408 1.873.636l9.262-11.653c1.093-1.375.113-3.403-1.645-3.403h-9.642z" fill="#3FCF8E"/></svg>"##;

/// Supabase's entry in the provider list.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Supabase,
    logo: LOGO,
    capabilities: Capabilities {
        discover: true,
        provider_details: true,
        provider_operations: true,
        provider_variables: true,
        logs: true,
        metrics: true,
        delete: true,
        new_database: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://api.supabase.com/v1/",
        hosts: &["api.supabase.com"],
        // Supabase's refusals are sentences for its users, such as
        // "Project is paused"; they're shown with the token removed.
        explains_refusals: true,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["organizations"],
            query: &[],
            page: None,
            cursor: None,
        },
        rows: &[],
        item: &[],
        id: "id",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(Supabase::new(http)),
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
    kind: crate::domain::model::ResourceKind::Database,
    routes: &[
        super::conformance::Route {
            path: "/organizations",
            body: "",
            answer: r#"[{"id":"acct-1","name":"Sam"}]"#,
        },
        super::conformance::Route {
            path: "/projects",
            body: "",
            answer: r#"[{"id":"res-1","name":"db","organization_id":"acct-1","status":"ACTIVE_HEALTHY"}]"#,
        },
    ],
    empty: r#"{}"#,
};

/// Supabase's Management API adapter, for one organization.
pub struct Supabase {
    provider: Provider,
    http: ProviderHttp,
}

impl Supabase {
    /// Shares the allow-listed HTTP boundary.
    #[must_use]
    pub fn new(http: ProviderHttp) -> Self {
        Self {
            provider: Provider::Supabase,
            http,
        }
    }

    /// Every project in the organization, each with what is in it: its
    /// Primary Database, storage buckets and edge functions. The project is
    /// the group; the database is named the way Supabase names it.
    async fn projects(
        &self,
        token: &Secret,
        org: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let value = self
            .http
            .json(self.provider, token, Method::GET, &["projects"], &[], None)
            .await?;
        let projects = value.as_array().ok_or_else(invalid)?;
        if projects.len() > MAX_DISCOVERED_RESOURCES {
            return Err(EngineError::Conflict(
                "Supabase discovery found more projects than Hoista keeps.",
            ));
        }
        let mut found = Vec::new();
        for project in projects.iter().filter(|p| {
            org.is_empty()
                || p["organization_id"].as_str() == Some(org)
                || p["organization_slug"].as_str() == Some(org)
        }) {
            let reference = field(project, "id")?;
            let name = field(project, "name")?;
            let region = project["region"].as_str().map(str::to_owned);
            let status = project_status(project["status"].as_str());
            found.push(RemoteResource {
                // The database's reference is the project's id.
                reference: reference.clone(),
                name: "Primary Database".into(),
                kind: ResourceKind::Database,
                region: region.clone(),
                status,
                url: None,
                group: Some(name.clone()),
            });
            // A paused project answers nothing else until it's restored.
            if status != ResourceStatus::Live {
                continue;
            }
            for function in self.list(token, &reference, &["functions"]).await {
                let Some(slug) = function["slug"].as_str() else {
                    continue;
                };
                found.push(RemoteResource {
                    reference: format!("{reference}/function:{slug}"),
                    name: function["name"].as_str().unwrap_or(slug).to_owned(),
                    kind: ResourceKind::Service,
                    region: region.clone(),
                    status: match function["status"].as_str() {
                        Some("ACTIVE") => ResourceStatus::Live,
                        Some("THROTTLED") => ResourceStatus::Failing,
                        _ => ResourceStatus::Unknown,
                    },
                    url: Some(format!(
                        "https://{reference}.supabase.co/functions/v1/{slug}"
                    )),
                    group: Some(name.clone()),
                });
            }
            for bucket in self.list(token, &reference, &["storage", "buckets"]).await {
                let Some(id) = bucket["id"].as_str() else {
                    continue;
                };
                found.push(RemoteResource {
                    reference: format!("{reference}/bucket:{id}"),
                    name: bucket["name"].as_str().unwrap_or(id).to_owned(),
                    kind: ResourceKind::Bucket,
                    region: region.clone(),
                    status: ResourceStatus::Live,
                    url: None,
                    group: Some(name.clone()),
                });
            }
        }
        Ok(found)
    }

    /// A resource's stored logs, newest 100 in the range: Postgres for the
    /// database, an edge function's own logs, storage for a bucket. The
    /// search text is matched here, never sent as part of the query.
    async fn logs(
        &self,
        token: &Secret,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        use crate::domain::logs::{LogEntry, LogHistory, LogLevel};
        query.milliseconds().map_err(EngineError::Invalid)?;
        let project = project_of(reference);
        let rest = reference.split_once('/').map(|(_, rest)| rest);
        let (sql, source) = match rest {
            None => (
                "SELECT timestamp, event_message, log_attributes['parsed.error_severity'] AS level FROM logs WHERE source = 'postgres_logs' ORDER BY timestamp DESC LIMIT 100".to_owned(),
                "postgres",
            ),
            Some(rest) if rest.starts_with("function:") => {
                let slug = &rest["function:".len()..];
                if !plain(slug) {
                    return Err(invalid());
                }
                let function = self
                    .http
                    .json(
                        self.provider,
                        token,
                        Method::GET,
                        &["projects", project, "functions", slug],
                        &[],
                        None,
                    )
                    .await?;
                let id = function["id"].as_str().unwrap_or_default();
                if !plain(id) {
                    return Err(invalid());
                }
                (
                    format!("SELECT timestamp, event_message, log_attributes['level'] AS level FROM logs WHERE source = 'function_logs' AND (log_attributes['function_id'] = '{id}' OR log_attributes['metadata.function_id'] = '{id}') ORDER BY timestamp DESC LIMIT 100"),
                    "edge function",
                )
            }
            Some(rest) if rest.starts_with("bucket:") => {
                let bucket = &rest["bucket:".len()..];
                if !plain(bucket) {
                    return Err(invalid());
                }
                (
                    format!("SELECT timestamp, event_message FROM logs WHERE source = 'storage_logs' AND event_message LIKE '%/{bucket}/%' ORDER BY timestamp DESC LIMIT 100"),
                    "storage",
                )
            }
            Some(_) => return Err(invalid()),
        };
        let from = chrono::DateTime::parse_from_rfc3339(&query.from)
            .map_err(|_| EngineError::Invalid("Give the start as an RFC 3339 time."))?
            .with_timezone(&chrono::Utc);
        let to = chrono::DateTime::parse_from_rfc3339(&query.to)
            .map_err(|_| EngineError::Invalid("Give the end as an RFC 3339 time."))?
            .with_timezone(&chrono::Utc);
        let value = self.log_query(token, project, &sql, from, to).await?;
        let scanner = crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
        let needle = query.query.to_lowercase();
        let mut items: Vec<LogEntry> = value["result"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let text = row["event_message"].as_str()?;
                if !needle.is_empty() && !text.to_lowercase().contains(&needle) {
                    return None;
                }
                let stated = row["level"]
                    .as_str()
                    .filter(|level| !level.is_empty())
                    .unwrap_or(text);
                let level = level_of(stated);
                let message =
                    crate::domain::logs::clipped(scanner.redact(text, std::slice::from_ref(token)));
                Some(LogEntry {
                    timestamp: row["timestamp"].as_i64().and_then(micros).or_else(|| {
                        // `2026-08-22T00:13:07.000000`, in UTC without saying so.
                        row["timestamp"].as_str().map(|text| {
                            if text.ends_with('Z') || text.contains('+') {
                                text.to_owned()
                            } else {
                                format!("{}Z", text.replace(' ', "T"))
                            }
                        })
                    }),
                    observed_at: chrono::Utc::now().to_rfc3339(),
                    level: match level {
                        Some(level) => level,
                        None => LogLevel::Info,
                    },
                    source: source.into(),
                    message,
                })
            })
            .collect();
        // Newest first from Supabase; oldest first like every other provider.
        items.reverse();
        Ok(LogHistory {
            items,
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: false,
        })
    }

    /// One question to Supabase's unified logs (ClickHouse SQL over one
    /// `logs` table, at most 24 hours, about ten questions a minute). A range
    /// over a day keeps its last 24 hours. The same question within 30
    /// seconds is answered from the last answer.
    async fn log_query(
        &self,
        token: &Secret,
        project: &str,
        sql: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<serde_json::Value, EngineError> {
        use chrono::SecondsFormat;
        let from = from.max(to - LOG_SPAN);
        let start = from.to_rfc3339_opts(SecondsFormat::Secs, true);
        let end = to.to_rfc3339_opts(SecondsFormat::Secs, true);
        let key = format!(
            "{}\u{0}{project}\u{0}{sql}\u{0}{start}\u{0}{end}",
            token_key(token)
        );
        if let Some(answer) = recent_log_answer(&key) {
            return Ok(answer);
        }
        let value = self
            .http
            .json(
                self.provider,
                token,
                Method::GET,
                &["projects", project, "analytics", "endpoints", "logs"],
                &[
                    ("sql", sql),
                    ("iso_timestamp_start", start.as_str()),
                    ("iso_timestamp_end", end.as_str()),
                ],
                None,
            )
            .await?;
        // A query Supabase can't run still answers 200, with its reason.
        if value["result"].is_null()
            && let Some(said) = value["error"]
                .as_str()
                .or_else(|| value["error"]["message"].as_str())
        {
            let scanner =
                crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.rejected".into(),
                message: format!(
                    "Supabase couldn't read its logs: {}",
                    scanner.redact(said, std::slice::from_ref(token))
                ),
            });
        }
        keep_log_answer(key, &value);
        Ok(value)
    }

    /// The names of a project's secrets, sorted; never their values.
    async fn secret_names(
        &self,
        token: &Secret,
        project: &str,
    ) -> Result<Vec<String>, EngineError> {
        let mut names: Vec<String> = self
            .try_list(token, project, &["secrets"])
            .await?
            .iter()
            .filter_map(|item| item["name"].as_str().map(str::to_owned))
            .take(MAX_FUNCTIONS)
            .collect();
        names.sort();
        Ok(names)
    }

    /// One list inside a project, such as its edge functions. Empty when the
    /// project can't answer it (a token limited to some features), so the
    /// rest of the project still shows.
    async fn list(&self, token: &Secret, project: &str, path: &[&str]) -> Vec<serde_json::Value> {
        self.try_list(token, project, path)
            .await
            .unwrap_or_default()
    }

    async fn try_list(
        &self,
        token: &Secret,
        project: &str,
        path: &[&str],
    ) -> Result<Vec<serde_json::Value>, EngineError> {
        let mut segments = vec!["projects", project];
        segments.extend_from_slice(path);
        let value = self
            .http
            .json(self.provider, token, Method::GET, &segments, &[], None)
            .await?;
        let items = value.as_array().ok_or_else(invalid)?;
        if items.len() > 1000 {
            return Err(EngineError::Conflict(
                "A Supabase project has more than 1,000 functions or buckets.",
            ));
        }
        Ok(items.clone())
    }
}

type LogAnswers =
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, serde_json::Value)>>;

fn log_answers() -> &'static LogAnswers {
    static ANSWERS: std::sync::OnceLock<LogAnswers> = std::sync::OnceLock::new();
    ANSWERS.get_or_init(Default::default)
}

/// A digest of the token, so one account never sees another's answer.
fn token_key(token: &Secret) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(token.expose().as_bytes());
    digest
        .iter()
        .take(TOKEN_KEY_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn recent_log_answer(key: &str) -> Option<serde_json::Value> {
    if cfg!(test) {
        return None;
    }
    let answers = log_answers().lock().ok()?;
    let (at, value) = answers.get(key)?;
    (at.elapsed() < LOG_ANSWER_LIFETIME).then(|| value.clone())
}

fn keep_log_answer(key: String, value: &serde_json::Value) {
    if let Ok(mut answers) = log_answers().lock() {
        answers.retain(|_, (at, _)| at.elapsed() < LOG_ANSWER_LIFETIME);
        if answers.len() < MAX_LOG_ANSWERS {
            answers.insert(key, (std::time::Instant::now(), value.clone()));
        }
    }
}

/// Letters, digits, `-`, `_` and `.` only: safe to place in a log query.
fn plain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME_CHARS
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Supabase's log time, microseconds since 1970, as RFC 3339.
fn micros(value: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_micros(value).map(|at| at.to_rfc3339())
}

/// The level a Supabase log line states: Postgres's `ERROR:` and `FATAL:`,
/// `WARNING:`, or an edge function's `error` and `warn`.
fn level_of(text: &str) -> Option<crate::domain::logs::LogLevel> {
    use crate::domain::logs::LogLevel;
    let head: String = text
        .chars()
        .take(LEVEL_PREFIX_CHARS)
        .collect::<String>()
        .to_ascii_lowercase();
    if head.starts_with("error")
        || head.starts_with("fatal")
        || head.starts_with("panic")
        || head.contains(" error:")
        || head.contains("fatal:")
    {
        Some(LogLevel::Error)
    } else if head.starts_with("warn") || head.contains("warning:") {
        Some(LogLevel::Warn)
    } else if head.starts_with("debug") {
        Some(LogLevel::Debug)
    } else {
        None
    }
}

/// The project a reference belongs to: `ref`, `ref/function:slug` or `ref/bucket:id`.
fn project_of(reference: &str) -> &str {
    reference.split('/').next().unwrap_or(reference)
}

/// A Supabase project's status as Hoista shows it.
fn project_status(status: Option<&str>) -> ResourceStatus {
    match status {
        Some("ACTIVE_HEALTHY") => ResourceStatus::Live,
        Some("INACTIVE" | "PAUSING") => ResourceStatus::Paused,
        Some("ACTIVE_UNHEALTHY" | "INIT_FAILED" | "RESTORE_FAILED" | "PAUSE_FAILED") => {
            ResourceStatus::Failing
        }
        _ => ResourceStatus::Unknown,
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Supabase {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// A Supabase project: Postgres with auth, storage and realtime. Its
    /// database password is made here and not kept: the project's API keys
    /// are what apps use, and the password can be reset in Supabase.
    async fn provider_add(
        &self,
        token: &Secret,
        org: &str,
        _project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let crate::application::ports::providers::ProviderNew::Database {
            name,
            engine,
            region,
        } = *what
        else {
            return Err(crate::application::ports::providers::unsupported());
        };
        if engine != "postgres" {
            return Err(EngineError::Invalid(
                "Supabase makes Postgres databases only.",
            ));
        }
        if org.is_empty() {
            return Err(EngineError::Conflict(
                "This Supabase account isn't tied to one organization. Reconnect it and pick one.",
            ));
        }
        let password = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let value = self
            .http
            .json(
                self.provider,
                token,
                Method::POST,
                &["projects"],
                &[],
                Some(&serde_json::json!({
                    "name": name,
                    "organization_id": org,
                    "region": region.unwrap_or("us-east-1"),
                    "db_pass": password,
                })),
            )
            .await;
        drop(password);
        // A token scoped to chosen projects can't make new ones.
        let value = value.map_err(|error| match error {
            EngineError::Provider { code, .. } if code == "provider.token_scope" => {
                EngineError::Provider {
                    evidence: None,
                    code: "supabase.token_cannot_create".into(),
                    message: "This Supabase token can't create projects: it is limited to chosen projects. In Supabase, open Account, then Access Tokens, generate a token with access to all projects in the organization and Projects set to read-write, then reconnect Supabase in Hoista with it.".into(),
                }
            }
            other => other,
        });
        let reference = field(&value?, "id")?;
        Ok((
            vec![reference],
            format!(
                "Made the Supabase project {name} (Postgres with auth, storage and realtime; its database password wasn't kept, reset it in Supabase for direct access)"
            ),
        ))
    }

    /// The project's public key, the one a browser uses with Row Level
    /// Security: its publishable key, or the legacy anon key. Never the
    /// secret or service-role key.
    async fn issue_key(
        &self,
        token: &Secret,
        _org: &str,
        reference: &str,
        _name: &str,
    ) -> Result<Secret, EngineError> {
        let project = reference
            .split_once('/')
            .map_or(reference, |(project, _)| project);
        let keys = self
            .http
            .json(
                self.provider,
                token,
                Method::GET,
                &["projects", project, "api-keys"],
                &[],
                None,
            )
            .await?;
        let rows = keys.as_array().cloned().unwrap_or_default();
        let public = |row: &&serde_json::Value| {
            row["type"].as_str() == Some("publishable")
                || (row["type"].as_str() == Some("legacy") || row["type"].is_null())
                    && row["name"].as_str() == Some("anon")
        };
        rows.iter()
            .filter(public)
            // The newer publishable key first.
            .max_by_key(|row| row["type"].as_str() == Some("publishable"))
            .and_then(|row| row["api_key"].as_str())
            .filter(|key| !key.is_empty())
            .map(|key| Secret::new(key.to_owned()))
            .ok_or(EngineError::Provider {
                evidence: None,
                code: "supabase.no_public_key".into(),
                message: "Supabase didn't return a public key for this project. Check it has an anon or publishable key under Project Settings, API Keys.".into(),
            })
    }

    fn key_variable(&self) -> Option<&'static str> {
        Some("SUPABASE_ANON_KEY")
    }

    async fn provider_details(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        let (project, function) = reference
            .split_once('/')
            .map_or((reference, None), |(project, rest)| {
                (project, rest.strip_prefix("function:"))
            });
        let value = self
            .http
            .json(
                self.provider,
                token,
                Method::GET,
                &["projects", project],
                &[],
                None,
            )
            .await?;
        let region = value["region"].as_str().map(str::to_owned);
        // The project's secrets (what its edge functions read), names only,
        // shown once: on its database, the project's own resource.
        let variable_names = match function.or_else(|| reference.contains('/').then_some("")) {
            Some(_) => None,
            None => self.secret_names(token, project).await.ok(),
        };
        let database_host = value["database"]["host"].as_str().map(str::to_owned);
        let domains = match function {
            Some(slug) => vec![crate::application::ports::providers::ProviderDomain {
                name: format!("{project}.supabase.co/functions/v1/{slug}"),
                custom: false,
                state: None,
                records: vec![],
                port: None,
                certificate: None,
                dns: vec![],
            }],
            None => vec![],
        };
        Ok(crate::application::ports::providers::ProviderDetails {
            deployments: vec![],
            variable_names,
            shared_variable_names: None,
            domains,
            references: vec![],
            sleeps: None,
            config: Some(crate::application::ports::providers::ProviderConfig {
                region,
                ..crate::application::ports::providers::ProviderConfig::default()
            }),
            volumes: vec![],
            tcp_proxies: vec![],
            addresses: vec![],
            private_domain: function.is_none().then_some(database_host).flatten(),
        })
    }

    async fn provider_traffic(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<crate::application::ports::providers::ProviderTraffic, EngineError> {
        if to <= from || to - from > chrono::Duration::days(MAX_TRAFFIC_DAYS) {
            return Err(EngineError::Invalid("Choose a range of at most 31 days."));
        }
        // Every request the project served passes Supabase's edge, which logs
        // its status: count them for the range.
        let sql = "SELECT count() AS requests, countIf(toInt32OrZero(log_attributes['response.status_code']) >= 400 AND toInt32OrZero(log_attributes['response.status_code']) < 500) AS client_errors, countIf(toInt32OrZero(log_attributes['response.status_code']) >= 500) AS server_errors FROM logs WHERE source = 'edge_logs'";
        // Supabase answers at most a day at a time: say which day was counted.
        let from = from.max(to - LOG_SPAN);
        let value = self
            .log_query(token, project_of(reference), sql, from, to)
            .await?;
        let (start, end) = (from.to_rfc3339(), to.to_rfc3339());
        let row = &value["result"][0];
        let count = |field: &str| {
            row[field]
                .as_u64()
                .or_else(|| row[field].as_str().and_then(|text| text.parse().ok()))
                .and_then(|n| u32::try_from(n).ok())
        };
        Ok(crate::application::ports::providers::ProviderTraffic {
            from: start,
            to: end,
            requests: count("requests"),
            client_errors: count("client_errors"),
            server_errors: count("server_errors"),
            p50_ms: None,
            p95_ms: None,
            p99_ms: None,
            top_errors: vec![],
            recent: vec![],
        })
    }

    async fn log_history(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        self.logs(token, reference, query).await
    }

    /// Deletes a project (with its database), or one of its edge functions.
    /// Buckets are emptied and removed in Supabase itself.
    async fn delete(&self, token: &Secret, _org: &str, reference: &str) -> Result<(), EngineError> {
        let project = project_of(reference);
        if !plain(project) {
            return Err(invalid());
        }
        let path: Vec<&str> = match reference.split_once('/') {
            None => vec!["projects", project],
            Some((_, rest)) => match rest.strip_prefix("function:") {
                Some(slug) if plain(slug) => vec!["projects", project, "functions", slug],
                _ => return Err(crate::application::ports::providers::unsupported()),
            },
        };
        self.http
            .json(Provider::Supabase, token, Method::DELETE, &path, &[], None)
            .await?;
        Ok(())
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        manifest().operations()
    }

    fn prepare_tools(&self) {
        if let Some(cli) = super::manifest::builtin("supabase").cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        manifest().actions.clone()
    }

    async fn set_provider_variable(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        let values = [(name.to_owned(), Secret::new(value.expose().into()))];
        self.set_provider_variables(token, external_id, reference, &values, false, false)
            .await
    }

    async fn set_provider_variables(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        values: &[(String, Secret)],
        _shared: bool,
        _redeploy: bool,
    ) -> Result<(), EngineError> {
        if values.is_empty() || values.len() > 100 {
            return Err(EngineError::Invalid(
                "Set between 1 and 100 secrets at once.",
            ));
        }
        let body: Vec<serde_json::Value> = values
            .iter()
            .map(|(name, value)| serde_json::json!({ "name": name, "value": value.expose() }))
            .collect();
        self.http
            .json(
                self.provider,
                token,
                Method::POST,
                &["projects", project_of(reference), "secrets"],
                &[],
                Some(&serde_json::Value::Array(body)),
            )
            .await
            .map(|_| ())
    }

    async fn delete_provider_variable(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        self.http
            .json(
                self.provider,
                token,
                Method::DELETE,
                &["projects", project_of(reference), "secrets"],
                &[],
                Some(&serde_json::json!([name])),
            )
            .await
            .map(|_| ())
    }

    async fn provider_operation(
        &self,
        token: &Secret,
        _external_id: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        if let Some(cli) = manifest().cli.as_ref()
            && super::cli::has(cli, name)
        {
            let (project, function) = reference
                .map(super::supabase_catalog::context)
                .unwrap_or((None, None));
            let project = project.ok_or(EngineError::Invalid(
                "Supabase commands run on one project: give the resource_id of one of its resources.",
            ))?;
            let mut context = vec![("project", project)];
            context.extend(function.map(|function| ("function", function)));
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
        super::supabase_catalog::run(&self.http, token, reference, name, args).await
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

/// Supabase's manifest: its API, its pinned CLI and the desktop app's buttons.
pub(super) fn manifest() -> &'static super::manifest::Manifest {
    super::manifest::builtin("supabase")
}
