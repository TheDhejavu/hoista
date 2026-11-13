//! Neon: each Postgres database is a Neon project of its own. Connection
//! strings are fetched when needed and never stored in a release.

use super::{
    answers::{field, invalid},
    http::ProviderHttp,
};
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{Artifact, ProviderAdapter, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::{Value, json};

/// Neon's API adapter. An account is one Neon organization.
pub struct Neon {
    http: ProviderHttp,
}

const MAX_ID_CHARS: usize = 60;
/// Ranges up to this many days are charted by the hour, longer ones by the day.
const MAX_HOURLY_DAYS: i64 = 7;
/// The most databases an export inventory lists.
const MAX_DATABASES: usize = 10_000;

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M24 0V24l-9.365-8.045V24H0V0ZM2.942 21.087h8.751V9.563l9.365 8.204V2.919L2.942 2.914Z" fill="#34D59A"/></svg>"##;

/// Neon's entry in the provider list.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Neon,
    logo: LOGO,
    capabilities: Capabilities {
        deploy: true,
        discover: true,
        metrics: true,
        provider_operations: true,
        delete: true,
        new_database: true,
        project_database: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://console.neon.tech/api/v2/",
        hosts: &["console.neon.tech"],
        explains_refusals: false,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["users", "me", "organizations"],
            query: &[],
            page: None,
            cursor: None,
        },
        rows: &["organizations"],
        item: &[],
        id: "id",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(Neon::new(http)),
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
            path: "/users/me/organizations",
            body: "",
            answer: r#"{"organizations":[{"id":"acct-1","name":"Sam"}]}"#,
        },
        super::conformance::Route {
            path: "/projects?",
            body: "",
            answer: r#"{"projects":[{"id":"res-1","name":"db","region_id":"aws-us-east-2"}],"pagination":{}}"#,
        },
    ],
    empty: r#"{}"#,
};

impl Neon {
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
            .json(Provider::Neon, token, Method::GET, path, query, None)
            .await
    }

    /// Makes a Neon project with one Postgres database (`neondb`), scaling
    /// to zero when idle. Returns its id.
    async fn create_project(
        &self,
        token: &Secret,
        org: &str,
        name: &str,
        region: Option<&str>,
    ) -> Result<String, EngineError> {
        let mut project = json!({
            "name": name,
            "branch": {"database_name": "neondb", "role_name": "neondb_owner"},
            "default_endpoint_settings": {
                "autoscaling_limit_min_cu": 0.25,
                "autoscaling_limit_max_cu": 1,
                "suspend_timeout_seconds": 0,
            },
        });
        if !org.is_empty() {
            project["org_id"] = org.into();
        }
        if let Some(region) = region {
            project["region_id"] = region.into();
        }
        let value = self
            .http
            .json(
                Provider::Neon,
                token,
                Method::POST,
                &["projects"],
                &[],
                Some(&json!({"project":project})),
            )
            .await?;
        field(&value["project"], "id")
    }

    async fn ready(&self, token: &Secret, reference: &str) -> Result<(), EngineError> {
        for _ in 0..60 {
            let value = self
                .get(token, &["projects", reference, "endpoints"], &[])
                .await?;
            let endpoints = value["endpoints"].as_array().ok_or_else(invalid)?;
            if endpoints.iter().any(|endpoint| {
                endpoint["type"] == "read_write"
                    && matches!(endpoint["current_state"].as_str(), Some("active" | "idle"))
                    && endpoint["pending_state"].is_null()
            }) {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Err(EngineError::Provider {
            evidence: None,
            code: "deploy.database_timeout".into(),
            message:
                "Neon is still provisioning the database. Inspect the project before retrying."
                    .into(),
        })
    }
}

/// Neon's consumption rows as charts: compute used and data sent out.
fn consumption_charts(
    value: &Value,
) -> Vec<Option<crate::application::ports::providers::ProviderChart>> {
    use super::metrics::{chart, line, number, points, time_of};
    use crate::application::ports::providers::{MetricSummary as S, MetricUnit as U};
    let records: Vec<&Value> = value["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|project| project["periods"].as_array().into_iter().flatten())
        .flat_map(|period| period["consumption"].as_array().into_iter().flatten())
        .collect();
    let series = |metric: &str| {
        points(records.iter().filter_map(|record| {
            let at = time_of(&record["timeframe_start"])?;
            let value = record["metrics"]
                .as_array()?
                .iter()
                .find(|item| item["metric_name"].as_str() == Some(metric))
                .and_then(|item| number(&item["value"]))?;
            Some((at, value))
        }))
    };
    vec![
        chart(
            "compute",
            "Compute used",
            U::Seconds,
            S::Total,
            vec![line("Compute unit seconds", series("compute_unit_seconds"))],
        ),
        chart(
            "transfer",
            "Data sent out",
            U::Bytes,
            S::Total,
            vec![line(
                "Public network",
                series("public_network_transfer_bytes"),
            )],
        ),
    ]
}

#[async_trait::async_trait]
impl ProviderAdapter for Neon {
    fn provider(&self) -> Provider {
        Provider::Neon
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// A Postgres database as a Neon project of its own.
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
            return Err(EngineError::Invalid("Neon makes Postgres databases only."));
        }
        let id = self.create_project(token, org, name, region).await?;
        Ok((
            vec![id],
            format!("Made the Postgres database {name} as a Neon project"),
        ))
    }

    /// What the project used over time, from Neon's consumption history:
    /// compute and data sent out. Neon keeps this on paid plans only.
    async fn provider_metrics(
        &self,
        token: &Secret,
        org: &str,
        project: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        _step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::stamp;
        let plain = |id: &str| {
            !id.is_empty()
                && id.len() <= MAX_ID_CHARS
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        };
        if !plain(project) {
            return Err(invalid());
        }
        let granularity = if to - from <= chrono::Duration::days(MAX_HOURLY_DAYS) {
            "hourly"
        } else {
            "daily"
        };
        let (start, end) = (stamp(from), stamp(to));
        let mut query = vec![
            ("project_ids", project),
            ("from", start.as_str()),
            ("to", end.as_str()),
            ("granularity", granularity),
            ("metrics", "compute_unit_seconds"),
            ("metrics", "public_network_transfer_bytes"),
        ];
        if plain(org) && org.starts_with("org-") {
            query.push(("org_id", org));
        }
        let value = self
            .get(token, &["consumption_history", "v2", "projects"], &query)
            .await
            .map_err(|error| match error {
                EngineError::Provider { code, .. }
                    if code == "provider.token_scope" || code == "provider.rejected" =>
                {
                    EngineError::Provider {
                        evidence: None,
                        code: "provider.plan".into(),
                        message: "Neon keeps usage history on paid plans (Launch and up), \
                                  from when the plan started."
                            .into(),
                    }
                }
                other => other,
            })?;
        Ok(super::metrics::metrics(
            from,
            to,
            consumption_charts(&value),
            &[],
            "Neon",
        ))
    }

    /// A project Neon lists is there and answers (a sleeping compute wakes
    /// on the first query); a deleted one is gone.
    async fn observe_resource(
        &self,
        token: &Secret,
        _org: &str,
        resource: &Resource,
    ) -> Result<crate::application::ports::providers::Observed, EngineError> {
        let reference = resource.provider_ref.as_str();
        let status = match self.get(token, &["projects", reference], &[]).await {
            Ok(_) => ResourceStatus::Live,
            Err(EngineError::Provider { code, .. }) if code == "provider.not_found" => {
                ResourceStatus::Stopped
            }
            Err(error) => return Err(error),
        };
        Ok(crate::application::ports::providers::Observed {
            status,
            url: None,
            kind: Some(ResourceKind::Database),
        })
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        super::manifest::builtin("neon").operations()
    }

    fn prepare_tools(&self) {
        if let Some(cli) = super::manifest::builtin("neon").cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        super::manifest::builtin("neon").actions.clone()
    }

    /// Neon's API first, `neonctl` for what it can't do; both aimed at the
    /// organization and the project the resource is.
    async fn provider_operation(
        &self,
        token: &Secret,
        external_id: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        let manifest = super::manifest::builtin("neon");
        let mut context = vec![("org", external_id)];
        context.extend(reference.map(|project| ("project", project)));
        if manifest.api_operation(name).is_some() {
            return super::api_catalog::run_manifest(
                &self.http,
                Provider::Neon,
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

    /// What the organization's projects used this month, as Neon counts it
    /// (paid plans): compute hours, storage and data transfer.
    async fn provider_plan_usage(
        &self,
        token: &Secret,
        org: &str,
    ) -> Result<Vec<crate::application::ports::providers::ProviderPlanUsage>, EngineError> {
        use crate::application::ports::providers::ProviderPlanUsage;
        let now = chrono::Utc::now();
        let from =
            super::runpod::month_start(now).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let to = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let answer = self
            .http
            .json(
                Provider::Neon,
                token,
                Method::GET,
                &["consumption_history", "v2", "projects"],
                &[
                    ("from", from.as_str()),
                    ("to", to.as_str()),
                    ("granularity", "monthly"),
                    ("org_id", org),
                    ("limit", "100"),
                    (
                        "metrics",
                        "compute_unit_seconds,root_branch_bytes_month,public_network_transfer_bytes",
                    ),
                ],
                None,
            )
            .await?;
        let mut totals = std::collections::BTreeMap::<String, f64>::new();
        for project in answer["projects"].as_array().into_iter().flatten() {
            for period in project["periods"].as_array().into_iter().flatten() {
                for slot in period["consumption"].as_array().into_iter().flatten() {
                    // Named metrics, or the older fixed fields.
                    if let Some(metrics) = slot["metrics"].as_array() {
                        for metric in metrics {
                            if let (Some(name), Some(value)) =
                                (metric["metric_name"].as_str(), metric["value"].as_f64())
                            {
                                *totals.entry(name.to_owned()).or_default() += value;
                            }
                        }
                    } else {
                        for name in [
                            "compute_unit_seconds",
                            "root_branch_bytes_month",
                            "public_network_transfer_bytes",
                        ] {
                            if let Some(value) = slot[name].as_f64() {
                                *totals.entry(name.to_owned()).or_default() += value;
                            }
                        }
                        if let Some(value) = slot["compute_time_seconds"].as_f64() {
                            *totals.entry("compute_unit_seconds".into()).or_default() += value;
                        }
                    }
                }
            }
        }
        let gb = 1_000_000_000.0;
        Ok([
            (
                "compute_unit_seconds",
                "Compute this month",
                "CU-hours",
                3600.0,
            ),
            (
                "root_branch_bytes_month",
                "Storage this month",
                "GB-month",
                gb,
            ),
            (
                "public_network_transfer_bytes",
                "Public data transfer",
                "GB",
                gb,
            ),
        ]
        .into_iter()
        .filter_map(|(key, label, unit, per)| {
            totals.get(key).map(|value| ProviderPlanUsage {
                label: label.into(),
                used: (value / per * 100.0).round() / 100.0,
                limit: None,
                unit: unit.into(),
            })
        })
        .collect())
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        super::accounts::list(&self.http, Provider::Neon, token).await
    }

    async fn verify(&self, token: &Secret, org: &str) -> Result<Vec<String>, EngineError> {
        let value = self
            .get(token, &["projects"], &[("limit", "1"), ("org_id", org)])
            .await?;
        if !value["projects"].is_array() {
            return Err(invalid());
        }
        Ok(vec!["projects:read (verified by request)".into()])
    }

    async fn discover(
        &self,
        token: &Secret,
        org: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        let mut cursor = String::new();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let mut query = vec![("limit", "100"), ("org_id", org)];
            if !cursor.is_empty() {
                query.push(("cursor", &cursor));
            }
            let value = self.get(token, &["projects"], &query).await?;
            if value.get("unavailable").is_some_and(|v| {
                v.as_array().is_some_and(|a| !a.is_empty())
                    || v.as_object().is_some_and(|m| !m.is_empty())
            }) {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "provider.partial_response".into(),
                    message: "Neon returned an incomplete project list. Retry discovery later."
                        .into(),
                });
            }
            let projects = value["projects"].as_array().ok_or_else(invalid)?;
            for project in projects {
                resources.push(RemoteResource {
                    reference: field(project, "id")?,
                    name: field(project, "name")?,
                    kind: ResourceKind::Database,
                    region: project["region_id"].as_str().map(str::to_owned),
                    // Listed means it's there; a compute asleep still answers.
                    status: ResourceStatus::Live,
                    url: None,
                    group: None,
                });
            }
            let next = value["pagination"]["cursor"]
                .as_str()
                .filter(|s| !s.is_empty());
            if projects.is_empty() || next.is_none() {
                return Ok(resources);
            }
            cursor = next.ok_or_else(invalid)?.into();
            if !seen.insert(cursor.clone()) {
                return Err(invalid());
            }
        }
        Err(EngineError::Conflict(
            "Neon discovery exceeded 10,000 projects.",
        ))
    }

    async fn deploy(
        &self,
        token: &Secret,
        org: &str,
        resource: &ResourcePlan,
        _: &Artifact,
    ) -> Result<Release, EngineError> {
        if resource.kind != ResourceKind::Database {
            return Err(crate::application::ports::providers::unsupported());
        }
        let reference = if let Some(reference) = &resource.provider_ref {
            let project = self.get(token, &["projects", reference], &[]).await?;
            if !org.is_empty() && project["project"]["org_id"].as_str() != Some(org) {
                return Err(EngineError::Invalid(
                    "The Neon project belongs to a different organization.",
                ));
            }
            reference.clone()
        } else {
            self.create_project(token, org, &resource.name, resource.region.as_deref())
                .await?
        };
        self.ready(token, &reference).await?;
        Ok(Release {
            schedules: None,
            reference,
            version: "database-existing-schema".into(),
            url: None,
        })
    }

    async fn health(&self, token: &Secret, _: &str, release: &Release) -> Result<(), EngineError> {
        self.ready(token, &release.reference).await
    }

    async fn connection_string(
        &self,
        token: &Secret,
        _: &str,
        reference: &str,
    ) -> Result<Secret, EngineError> {
        let mut value = self
            .get(
                token,
                &["projects", reference, "connection_uri"],
                &[
                    ("database_name", "neondb"),
                    ("role_name", "neondb_owner"),
                    ("pooled", "true"),
                ],
            )
            .await?;
        let Value::String(uri) = value["uri"].take() else {
            return Err(invalid());
        };
        Ok(Secret::new(uri))
    }

    async fn migration_connection_string(
        &self,
        token: &Secret,
        _: &str,
        reference: &str,
    ) -> Result<Secret, EngineError> {
        let mut value = self
            .get(
                token,
                &["projects", reference, "connection_uri"],
                &[
                    ("database_name", "neondb"),
                    ("role_name", "neondb_owner"),
                    ("pooled", "false"),
                ],
            )
            .await?;
        let Value::String(uri) = value["uri"].take() else {
            return Err(invalid());
        };
        Ok(Secret::new(uri))
    }

    async fn database_observation(
        &self,
        token: &Secret,
        org: &str,
        reference: &str,
    ) -> Result<crate::domain::database_observation::DatabaseObservation, EngineError> {
        observation::read(self, token, org, reference).await
    }

    async fn database_operations(
        &self,
        token: &Secret,
        org: &str,
        reference: &str,
        cursor: Option<&str>,
    ) -> Result<crate::domain::database_observation::DatabaseOperationHistory, EngineError> {
        observation::operations(self, token, org, reference, cursor).await
    }

    async fn database_inventory(
        &self,
        token: &Secret,
        org: &str,
        reference: &str,
    ) -> Result<Vec<crate::domain::database_export::DatabaseIdentity>, EngineError> {
        let project = self.get(token, &["projects", reference], &[]).await?;
        if !org.is_empty() && project["project"]["org_id"].as_str() != Some(org) {
            return Err(EngineError::Invalid(
                "The Neon project belongs to a different organization.",
            ));
        }
        let mut inventory = Vec::new();
        let mut cursor = String::new();
        let mut cursors = std::collections::BTreeSet::new();
        let mut branches_seen = std::collections::BTreeSet::new();
        for _ in 0..100 {
            let mut query = vec![("limit", "100")];
            if !cursor.is_empty() {
                query.push(("cursor", &cursor));
            }
            let response = self
                .get(token, &["projects", reference, "branches"], &query)
                .await?;
            reject_incomplete(&response)?;
            let branches = response["branches"].as_array().ok_or_else(invalid)?;
            if branches.len() > 100 {
                return Err(invalid());
            }
            for branch in branches {
                let branch_id = field(branch, "id")?;
                if branch["project_id"].as_str() != Some(reference)
                    || !branches_seen.insert(branch_id.clone())
                {
                    return Err(invalid());
                }
                let response = self
                    .get(
                        token,
                        &["projects", reference, "branches", &branch_id, "databases"],
                        &[],
                    )
                    .await?;
                reject_incomplete(&response)?;
                if response
                    .get("pagination")
                    .is_some_and(|value| !value.is_null())
                {
                    return Err(invalid());
                }
                let databases = response["databases"].as_array().ok_or_else(invalid)?;
                let mut names = std::collections::BTreeSet::new();
                for database in databases {
                    let name = field(database, "name")?;
                    if database["branch_id"].as_str() != Some(branch_id.as_str())
                        || !names.insert(name.clone())
                    {
                        return Err(invalid());
                    }
                    inventory.push(crate::domain::database_export::DatabaseIdentity {
                        branch_id: branch_id.clone(),
                        database_name: name,
                        owner_name: field(database, "owner_name")?,
                    });
                    if inventory.len() > MAX_DATABASES {
                        return Err(EngineError::Conflict(
                            "This account has more databases than an export can list.",
                        ));
                    }
                }
            }
            let next = &response["pagination"]["next"];
            if next.is_null() || next.as_str() == Some("") {
                if branches_seen.is_empty() {
                    return Err(invalid());
                }
                inventory.sort();
                return Ok(inventory);
            }
            let next = next.as_str().ok_or_else(invalid)?;
            if branches.is_empty() || !cursors.insert(next.to_owned()) {
                return Err(invalid());
            }
            cursor = next.into();
        }
        Err(EngineError::Conflict(
            "Database export inventory exceeded its branch pagination limit.",
        ))
    }

    async fn database_export_connection(
        &self,
        token: &Secret,
        _: &str,
        reference: &str,
        database: &crate::domain::database_export::DatabaseIdentity,
    ) -> Result<Secret, EngineError> {
        let mut value = self
            .get(
                token,
                &["projects", reference, "connection_uri"],
                &[
                    ("branch_id", &database.branch_id),
                    ("database_name", &database.database_name),
                    ("role_name", &database.owner_name),
                    ("pooled", "false"),
                ],
            )
            .await?;
        let Value::String(uri) = value["uri"].take() else {
            return Err(invalid());
        };
        Ok(Secret::new(uri))
    }

    async fn delete(&self, token: &Secret, _: &str, reference: &str) -> Result<(), EngineError> {
        self.http
            .json(
                Provider::Neon,
                token,
                Method::DELETE,
                &["projects", reference],
                &[],
                None,
            )
            .await?;
        Ok(())
    }
}

fn reject_incomplete(response: &Value) -> Result<(), EngineError> {
    if response.get("unavailable").is_some_and(|value| {
        !value.is_null() && value.as_array().is_none_or(|array| !array.is_empty())
    }) {
        return Err(invalid());
    }
    if response["pagination"]
        .get("cursor")
        .is_some_and(|value| !value.is_null())
    {
        return Err(invalid());
    }
    Ok(())
}

mod observation;

#[cfg(test)]
mod metrics_tests {
    #[test]
    fn consumption_becomes_charts() {
        let value = serde_json::json!({
            "projects": [
                {
                    "project_id": "p",
                    "periods": [
                        {
                            "consumption": [
                                {
                                    "timeframe_start": "2026-11-01T10:00:00Z",
                                    "metrics": [
                                        {
                                            "metric_name": "compute_unit_seconds",
                                            "value": 30,
                                        },
                                        {
                                            "metric_name": "public_network_transfer_bytes",
                                            "value": 2048,
                                        },
                                    ],
                                },
                            ],
                        },
                    ],
                },
            ],
        });
        let charts: Vec<_> = super::consumption_charts(&value)
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(charts.len(), 2);
        assert_eq!(charts[0].series[0].points[0].value, 30.0);
    }
}
