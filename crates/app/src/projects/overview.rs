//! Project overviews for the desktop app. Missing figures are never turned into zeros.

use crate::engine::Engine;
use crate::projects::ProjectService;
use crate::{domain::model::*, ports::catalog::EngineError};
use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One UTC day in the overview chart.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct DailyTraffic {
    /// UTC calendar day.
    pub day: String,
    /// Sum across request-serving resources, or null when coverage is incomplete.
    #[ts(type = "number | null")]
    pub requests: Option<i64>,
    /// Sum across the same resources, or null when unavailable.
    #[ts(type = "number | null")]
    pub errors: Option<i64>,
}

/// A project's measured figures, with gaps left null.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct ProjectStats {
    /// Seven UTC days ending today, including today's partial day.
    pub traffic: Vec<DailyTraffic>,
    /// Seven-day request total; null if any day or request-serving resource is missing.
    #[ts(type = "number | null")]
    pub requests: Option<i64>,
    /// Errors divided by requests, as a fraction. Null for missing data or zero requests.
    pub error_rate: Option<f64>,
    /// Milliseconds. Never inferred from a median or averaged daily percentiles.
    pub p99: Option<f64>,
    /// UTC date of the latest recorded usage observation.
    pub last_observed_day: Option<String>,
}

/// A project's overview for the desktop app, from what providers last reported.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct ProjectOverview {
    /// Project identity and source.
    pub project: Project,
    /// Last observed aggregate resource state, independently of deploy failures.
    pub status: ResourceStatus,
    /// All associated resources.
    pub resources: Vec<Resource>,
    /// Newest finished deployment, including failures that left the live version
    /// unchanged. A deploy still running doesn't hide the last result.
    pub last_deploy: Option<Deploy>,
    /// Usage as reported, with gaps left null.
    pub stats: ProjectStats,
    /// For a folder source that is a clone of a GitHub repository: the
    /// repository, branch and commit, read from the folder itself.
    pub source_git: Option<SourceGit>,
}

/// Where a source folder's code lives on GitHub, read from its `origin` remote.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct SourceGit {
    /// `owner/name` on GitHub.
    pub repository: String,
    /// The branch checked out, when one is.
    pub branch: Option<String>,
    /// The commit checked out.
    pub commit: Option<String>,
    /// True for a folder the system clears on its own, such as /tmp.
    pub temporary: bool,
}

impl Engine {
    pub(crate) async fn overview_for(
        &self,
        project: Project,
        resources: Vec<Resource>,
    ) -> Result<ProjectOverview, EngineError> {
        let id = project.id.clone();
        let today = chrono::Utc::now().date_naive();
        let week = today - chrono::Duration::days(6);
        let usage = self.repository.usage(&id, &week.to_string()).await?;
        let status = aggregate_status(&resources);
        let stats = summarize(&resources, &usage, today);
        let source_git = self.inspector.github_remote(&project.source).await;
        Ok(ProjectOverview {
            source_git,
            project,
            status,
            resources,
            stats,
            last_deploy: self
                .repository
                .deploys(&id, None)
                .await?
                .items
                .into_iter()
                .find(|deploy| {
                    !matches!(deploy.status, DeployStatus::Queued | DeployStatus::Running)
                }),
        })
    }
}

impl<'a> ProjectService<'a> {
    /// Reads the overview from saved state, without contacting providers or filling in figures
    /// they don't report.
    ///
    /// # Errors
    /// Reports missing projects or unavailable persistent state.
    pub async fn project_overview(self, id: &str) -> Result<ProjectOverview, EngineError> {
        let project = self
            .engine
            .repository
            .project(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let resources: Vec<_> = self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| resource.project_id.as_deref() == Some(id))
            .collect();
        self.engine.overview_for(project, resources).await
    }
}

/// A folder the system empties on its own: code there can vanish.
pub fn temporary_folder(path: &str) -> bool {
    [
        "/tmp/",
        "/private/tmp/",
        "/var/tmp/",
        "/private/var/folders/",
        "/var/folders/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix) || path == prefix.trim_end_matches('/'))
}

pub(crate) fn aggregate_status(resources: &[Resource]) -> ResourceStatus {
    // A single failed dependency must not be hidden by a live web resource.
    // Many pieces have no status a provider reports (a bucket, a Worker, an email
    // domain), so a piece not checked yet doesn't hide the live ones: a
    // project of ten pieces would otherwise read "checking" forever. A
    // resource that was never deployed, or was stopped, doesn't hide the live
    // ones either; a project with nothing in it yet is "not deployed".
    for status in [
        ResourceStatus::Failing,
        ResourceStatus::Deploying,
        ResourceStatus::Paused,
        ResourceStatus::Live,
        ResourceStatus::Sleeping,
        ResourceStatus::Idle,
        ResourceStatus::Unknown,
        ResourceStatus::Stopped,
        ResourceStatus::NotDeployed,
    ] {
        if resources.iter().any(|resource| resource.status == status) {
            return status;
        }
    }
    ResourceStatus::NotDeployed
}

pub(crate) fn summarize(resources: &[Resource], rows: &[Usage], today: NaiveDate) -> ProjectStats {
    let traffic_resources: Vec<_> = resources
        .iter()
        .filter(|p| {
            matches!(
                p.kind,
                ResourceKind::Web | ResourceKind::Service | ResourceKind::Job
            )
        })
        .collect();
    let traffic: Vec<_> = (0..7)
        .rev()
        .map(|offset| {
            let day = (today - chrono::Duration::days(offset)).to_string();
            let sum = |errors: bool| {
                if traffic_resources.is_empty() {
                    return None;
                }
                traffic_resources.iter().try_fold(0_i64, |total, resource| {
                    let row = rows
                        .iter()
                        .find(|row| row.resource_id == resource.id && row.day == day)?;
                    let value = if errors { row.errors? } else { row.requests? };
                    if value < 0 {
                        return None;
                    }
                    total.checked_add(value)
                })
            };
            DailyTraffic {
                requests: sum(false),
                errors: sum(true),
                day,
            }
        })
        .collect();
    let requests = traffic
        .iter()
        .try_fold(0_i64, |sum, day| sum.checked_add(day.requests?));
    let errors = traffic
        .iter()
        .try_fold(0_i64, |sum, day| sum.checked_add(day.errors?));
    let error_rate = requests
        .zip(errors)
        .filter(|(requests, errors)| *requests > 0 && errors <= requests)
        .map(|(requests, errors)| errors as f64 / requests as f64);
    let end = today.to_string();
    ProjectStats {
        requests,
        error_rate,
        p99: None,
        last_observed_day: rows
            .iter()
            .filter(|row| row.day <= end)
            .map(|row| row.day.clone())
            .max(),
        traffic,
    }
}

#[cfg(test)]
mod tests {
    use crate::projects::overview::*;
    fn resource() -> Resource {
        Resource {
            id: "web".into(),
            project_id: Some("project".into()),
            kind: ResourceKind::Web,
            provider: crate::Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:web".into(),
            name: "web".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Managed,
            url: None,
            group: None,
        }
    }

    #[test]
    fn missing_days_are_unknown() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 7).expect("date");
        let resources = vec![resource()];
        let mut rows: Vec<_> = (1..=7)
            .map(|day| Usage {
                resource_id: "web".into(),
                day: format!("2026-11-{day:02}"),
                requests: Some(10),
                errors: Some(1),
                compute_seconds: None,
                storage_bytes: None,
                egress_bytes: None,
            })
            .collect();
        let complete = summarize(&resources, &rows, today);
        assert_eq!(complete.requests, Some(70));
        assert_eq!(complete.error_rate, Some(0.1));
        rows.remove(2);
        let partial = summarize(&resources, &rows, today);
        assert_eq!(partial.requests, None);
        assert_eq!(partial.traffic[2].requests, None);
        assert_eq!(partial.p99, None);
    }

    #[test]
    fn unobserved_resources_do_not_produce_totals() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 1).expect("date");
        let empty = summarize(&[resource()], &[], today);
        assert_eq!(empty.requests, None);
        let mut failed = resource();
        failed.status = ResourceStatus::Failing;
        assert_eq!(
            aggregate_status(&[resource(), failed]),
            ResourceStatus::Failing
        );
        assert_eq!(aggregate_status(&[]), ResourceStatus::NotDeployed);
    }

    #[test]
    fn pieces_without_a_status_dont_hide_a_live_project() {
        let mut bucket = resource();
        bucket.status = ResourceStatus::Unknown;
        assert_eq!(
            aggregate_status(&[resource(), bucket.clone()]),
            ResourceStatus::Live
        );
        assert_eq!(aggregate_status(&[bucket]), ResourceStatus::Unknown);
    }
}
