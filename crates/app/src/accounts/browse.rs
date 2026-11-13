//! Project search and filters for the desktop app, in a stable order and with
//! limits.

use crate::accounts::AccountService;
use crate::{
    Provider,
    domain::model::*,
    ports::catalog::{EngineError, Page},
};
use crate::{engine::ProjectOverview, projects::overview::aggregate_status};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use ts_rs::TS;

const PAGE_SIZE: usize = 100;
/// The most projects one search looks through.
const MAX_PROJECTS: usize = 10_000;
const MAX_SEARCH_CHARS: usize = 80;
const MAX_CURSOR_BYTES: usize = 2048;

/// How projects are sorted; ties go by project id, so the order never shifts.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSort {
    /// Most recently created first.
    #[default]
    Newest,
    /// Project name in ascending order.
    Name,
}

/// Optional project filters. A cursor works only with the same filters and sort.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectListQuery {
    /// Case-insensitive substring of the project name, at most 80 characters.
    pub q: Option<String>,
    /// Include projects with a resource on this provider.
    pub provider: Option<Provider>,
    /// Filter by observed aggregate state.
    pub status: Option<ResourceStatus>,
    /// Managed or watching projects.
    pub mode: Option<Mode>,
    /// Only projects in this environment (its id).
    pub environment: Option<String>,
    /// Ordering; newest by default.
    #[serde(default)]
    pub sort: ProjectSort,
    /// The `next_cursor` of the page before, passed back as it is.
    pub cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    filter: String,
    key: String,
    id: String,
}

/// Catalog-wide counts for the Projects screen header, computed from the same
/// overviews as the project list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct ProjectTotals {
    /// Projects in the catalog.
    pub projects: u32,
    /// Projects whose aggregate resource status is live.
    pub live: u32,
    /// Projects whose aggregate resource status is failing.
    pub failing: u32,
    /// Distinct providers hosting at least one grouped resource.
    pub providers: u32,
}

impl<'a> AccountService<'a> {
    /// Totals over every project, with the same limit as project search.
    ///
    /// # Errors
    /// Rejects catalogs over 10,000 projects or unavailable state.
    pub async fn project_totals(self) -> Result<ProjectTotals, EngineError> {
        self.engine.accounts().project_totals_in(None).await
    }

    /// Totals over the projects of one environment, or every project when None.
    ///
    /// # Errors
    /// Rejects catalogs over 10,000 projects or unavailable state.
    pub async fn project_totals_in(
        self,
        environment: Option<&str>,
    ) -> Result<ProjectTotals, EngineError> {
        let mut projects = self.engine.accounts().all_projects().await?;
        if let Some(id) = environment {
            projects.retain(|project| project.environment_id == id);
        }
        let resources = self.engine.projects().all_resources().await?;
        let mut totals = ProjectTotals {
            projects: u32::try_from(projects.len()).map_err(|_| EngineError::Store)?,
            live: 0,
            failing: 0,
            providers: 0,
        };
        let mut providers = std::collections::BTreeSet::new();
        for project in projects {
            let owned: Vec<Resource> = resources
                .iter()
                .filter(|resource| resource.project_id.as_deref() == Some(&project.id))
                .cloned()
                .collect();
            providers.extend(owned.iter().map(|resource| resource.provider.slug()));
            let overview = self.engine.overview_for(project, owned).await?;
            match overview.status {
                ResourceStatus::Live => totals.live += 1,
                ResourceStatus::Failing => totals.failing += 1,
                _ => {}
            }
        }
        totals.providers = u32::try_from(providers.len()).map_err(|_| EngineError::Store)?;
        Ok(totals)
    }

    /// Every project, reading the catalog a page at a time.
    ///
    /// # Errors
    /// Returns an error when the store cannot be read.
    pub async fn all_projects(self) -> Result<Vec<Project>, EngineError> {
        let mut projects = vec![];
        let mut cursor = None;
        loop {
            let page = self.engine.repository.projects(cursor.as_deref()).await?;
            projects.extend(page.items);
            if projects.len() > MAX_PROJECTS {
                return Err(EngineError::Conflict(
                    "Project search exceeds 10,000 projects.",
                ));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(projects);
            }
        }
    }

    /// Returns up to 100 matching overviews; usage the provider didn't report stays null.
    ///
    /// # Errors
    /// Rejects invalid or reused cursors, oversized searches or unavailable state.
    pub async fn project_summaries(
        self,
        query: ProjectListQuery,
    ) -> Result<Page<ProjectOverview>, EngineError> {
        let projects = self.engine.accounts().all_projects().await?;
        let resources = self.engine.projects().all_resources().await?;
        let selected = select(projects, &resources, query)?;
        let mut items = Vec::with_capacity(selected.items.len());
        for project in selected.items {
            let owned = resources
                .iter()
                .filter(|resource| resource.project_id.as_deref() == Some(&project.id))
                .cloned()
                .collect();
            items.push(self.engine.overview_for(project, owned).await?);
        }
        Ok(Page {
            items,
            next_cursor: selected.next_cursor,
        })
    }
}

fn select(
    mut projects: Vec<Project>,
    resources: &[Resource],
    mut query: ProjectListQuery,
) -> Result<Page<Project>, EngineError> {
    let supplied = query.cursor.take();
    if query
        .q
        .as_ref()
        .is_some_and(|q| q.chars().count() > MAX_SEARCH_CHARS)
    {
        return Err(EngineError::Invalid(
            "Search text is limited to 80 characters.",
        ));
    }
    query.q = query
        .q
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty());
    let filter = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&query).map_err(|_| EngineError::Store)?)
    );
    let cursor = supplied
        .map(|value| {
            if value.len() > MAX_CURSOR_BYTES {
                return Err(EngineError::Invalid("Invalid project search cursor."));
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| EngineError::Invalid("Invalid project search cursor."))?;
            let cursor: Cursor = serde_json::from_slice(&bytes)
                .map_err(|_| EngineError::Invalid("Invalid project search cursor."))?;
            if cursor.filter != filter {
                return Err(EngineError::Invalid(
                    "Start a new page when changing project filters.",
                ));
            }
            Ok(cursor)
        })
        .transpose()?;
    projects.retain(|project| {
        let owned: Vec<_> = resources
            .iter()
            .filter(|p| p.project_id.as_deref() == Some(&project.id))
            .cloned()
            .collect();
        query
            .q
            .as_ref()
            .is_none_or(|q| project.name.to_lowercase().contains(q))
            && query.mode.is_none_or(|mode| mode == project.mode)
            && query
                .environment
                .as_ref()
                .is_none_or(|id| *id == project.environment_id)
            && query
                .provider
                .is_none_or(|provider| owned.iter().any(|p| p.provider == provider))
            && query
                .status
                .is_none_or(|status| aggregate_status(&owned) == status)
    });
    let key = |project: &Project| match query.sort {
        ProjectSort::Name => project.name.clone(),
        ProjectSort::Newest => project.created_at.clone(),
    };
    let compare = |a_key: &str, a_id: &str, b_key: &str, b_id: &str| {
        let order = match query.sort {
            ProjectSort::Name => a_key.cmp(b_key),
            ProjectSort::Newest => b_key.cmp(a_key),
        };
        order.then_with(|| a_id.cmp(b_id))
    };
    projects.sort_by(|a, b| compare(&key(a), &a.id, &key(b), &b.id));
    if let Some(cursor) = cursor {
        projects.retain(|project| {
            compare(&key(project), &project.id, &cursor.key, &cursor.id) == Ordering::Greater
        });
    }
    let more = projects.len() > PAGE_SIZE;
    projects.truncate(PAGE_SIZE);
    let next_cursor = if more {
        projects
            .last()
            .map(|last| {
                serde_json::to_vec(&Cursor {
                    filter,
                    key: key(last),
                    id: last.id.clone(),
                })
                .map(|value| URL_SAFE_NO_PAD.encode(value))
                .map_err(|_| EngineError::Store)
            })
            .transpose()?
    } else {
        None
    };
    Ok(Page {
        items: projects,
        next_cursor,
    })
}

#[cfg(test)]
mod tests {
    use crate::accounts::browse::*;
    #[test]
    fn search_pagination_is_stable_and_cannot_reuse_another_filters_cursor() {
        let projects: Vec<_> = (0..205)
            .map(|i| Project {
                id: format!("{i:03}"),
                name: format!("project-{i:03}"),
                source: Source::Folder {
                    path: "/fixture".into(),
                },
                mode: Mode::Watching,
                created_at: "2026-11-02T00:00:00Z".into(),
                environment_id: "env_default".into(),
            })
            .collect();
        let first = select(projects.clone(), &[], ProjectListQuery::default()).expect("first");
        assert_eq!(first.items.len(), 100);
        let query = ProjectListQuery {
            cursor: first.next_cursor,
            ..Default::default()
        };
        let second = select(projects.clone(), &[], query.clone()).expect("second");
        assert_eq!(second.items[0].id, "100");
        let third = select(
            projects.clone(),
            &[],
            ProjectListQuery {
                cursor: second.next_cursor,
                ..Default::default()
            },
        )
        .expect("third");
        assert_eq!(third.items.len(), 5);
        assert!(third.next_cursor.is_none());
        assert!(
            select(
                projects.clone(),
                &[],
                ProjectListQuery {
                    q: Some("project".into()),
                    ..query
                }
            )
            .is_err()
        );
        let filtered = select(
            projects,
            &[],
            ProjectListQuery {
                q: Some("PROJECT-204".into()),
                status: Some(ResourceStatus::NotDeployed),
                ..Default::default()
            },
        )
        .expect("filter");
        assert_eq!(filtered.items.len(), 1);
    }
}
