//! What needs the person's attention. Read-only; approvals and projects are paged separately.

use crate::projects::ProjectService;
use crate::{domain::model::*, ports::catalog::EngineError};
use chrono::{Duration, NaiveDate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Separate cursors for the approvals list and the project scan.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AttentionQuery {
    /// Approvals cursor.
    pub cursor: Option<String>,
    /// Project scan cursor. An empty page can still have one.
    pub project_cursor: Option<String>,
}

/// An issue or suggestion. Reading it changes nothing at any provider.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttentionNotice {
    /// The newest deployment attempt failed; live resources may still be healthy.
    DeploymentFailed {
        /// Owning project.
        project_id: String,
        /// Failed attempt.
        deploy_id: String,
    },
    /// A resource's provider reports it failing (a crashed or failed deployment).
    ResourceFailing {
        /// Owning project.
        project_id: String,
        /// The failing resource.
        resource_id: String,
    },
    /// A resource that serves requests got none on every day checked; worth a look.
    IdleResource {
        /// Owning project.
        project_id: String,
        /// Resource to review; this is not permission to pause it.
        resource_id: String,
        /// First full UTC day checked.
        since: String,
        /// Last full UTC day checked.
        through: String,
        /// Number of full days with usage reported.
        days: u16,
    },
}

/// Two lists, each paged on its own. Follow both cursors to read the whole feed.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct AttentionFeed {
    /// Pending approvals from this approval page.
    pub approvals: Vec<Approval>,
    /// Approvals cursor, even when the filtered page is empty.
    pub next_cursor: Option<String>,
    /// Notices for this page of projects; they don't claim savings or current health.
    pub notices: Vec<AttentionNotice>,
    /// Project scan cursor, even for pages without notices.
    pub project_next_cursor: Option<String>,
}

impl<'a> ProjectService<'a> {
    /// Reads the attention feed from saved state, without calling providers or changing anything.
    ///
    /// # Errors
    /// Reports invalid cursors, reads too large to finish, and storage errors.
    pub async fn attention(self, query: AttentionQuery) -> Result<AttentionFeed, EngineError> {
        let approvals = self
            .engine
            .repository
            .approvals(query.cursor.as_deref())
            .await?;
        let projects = self
            .engine
            .repository
            .projects(query.project_cursor.as_deref())
            .await?;
        let resources = self.engine.projects().all_resources().await?;
        let days = self.engine.repository.settings().await?.idle_days;
        let today = chrono::Utc::now().date_naive();
        let since = (today - Duration::days(i64::from(days))).to_string();
        let through = (today - Duration::days(1)).to_string();
        let mut notices = Vec::new();
        for project in projects.items {
            let project_id = project.id.clone();
            let owned: Vec<_> = resources
                .iter()
                .filter(|p| p.project_id.as_deref() == Some(&project_id))
                .cloned()
                .collect();
            let overview = self.engine.overview_for(project, owned).await?;
            if let Some(deploy) = overview
                .last_deploy
                .filter(|d| d.status == DeployStatus::Failed)
            {
                notices.push(AttentionNotice::DeploymentFailed {
                    project_id: project_id.clone(),
                    deploy_id: deploy.id,
                });
            }
            let usage = self.engine.repository.usage(&project_id, &since).await?;
            for resource in overview.resources {
                if resource.status == ResourceStatus::Failing {
                    notices.push(AttentionNotice::ResourceFailing {
                        project_id: project_id.clone(),
                        resource_id: resource.id.clone(),
                    });
                    continue;
                }
                if matches!(resource.kind, ResourceKind::Web | ResourceKind::Service)
                    && matches!(
                        resource.status,
                        ResourceStatus::Live | ResourceStatus::Sleeping | ResourceStatus::Idle
                    )
                    && idle_window(&resource.id, &usage, today, days)
                {
                    notices.push(AttentionNotice::IdleResource {
                        project_id: project_id.clone(),
                        resource_id: resource.id,
                        since: since.clone(),
                        through: through.clone(),
                        days,
                    });
                }
            }
        }
        Ok(AttentionFeed {
            approvals: approvals
                .items
                .into_iter()
                .filter(|a| a.decision == ApprovalDecision::Pending)
                .collect(),
            next_cursor: approvals.next_cursor,
            notices,
            project_next_cursor: projects.next_cursor,
        })
    }
}

pub(crate) fn idle_window(resource_id: &str, rows: &[Usage], today: NaiveDate, days: u16) -> bool {
    !rows.iter().any(|row| {
        row.resource_id == resource_id
            && row.day == today.to_string()
            && row.requests.is_some_and(|count| count > 0)
    }) && SETTING_DAYS.contains(&days)
        && (1..=days).all(|offset| {
            let day = (today - Duration::days(i64::from(offset))).to_string();
            let mut matching = rows
                .iter()
                .filter(|r| r.resource_id == resource_id && r.day == day);
            matches!(matching.next(), Some(row) if row.requests == Some(0))
                && matching.next().is_none()
        })
}

#[cfg(test)]
mod tests {
    use crate::projects::attention::*;
    #[test]
    fn idle_requires_each_completed_day_and_never_uses_missing_or_duplicate_data() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let row = |day: &str, requests| Usage {
            resource_id: "p".into(),
            day: day.into(),
            requests,
            errors: None,
            compute_seconds: None,
            storage_bytes: None,
            egress_bytes: None,
        };
        let mut rows = vec![row("2026-11-01", Some(0)), row("2026-10-31", Some(0))];
        assert!(idle_window("p", &rows, today, 2));
        assert!(!idle_window("other", &rows, today, 2));
        assert!(!idle_window("p", &rows, today, 3));
        assert!(!idle_window("p", &rows, today, 0));
        for value in [None, Some(-1), Some(1)] {
            rows[0].requests = value;
            assert!(!idle_window("p", &rows, today, 2));
        }
        rows[0].requests = Some(0);
        rows.push(row("2026-11-02", Some(10)));
        assert!(!idle_window("p", &rows, today, 2));
        rows.pop();
        rows.push(row("2026-11-01", Some(0)));
        assert!(!idle_window("p", &rows, today, 2));
    }
}
