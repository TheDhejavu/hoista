//! A resource's detail, built from what Hoista has recorded about it and its
//! links.

use crate::engine::{Actor, Engine};
use crate::projects::ProjectService;
use crate::{
    domain::{domains::CustomDomain, model::*},
    ports::catalog::EngineError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// One recorded link to or from a resource, including links no longer seen.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ResourceRelationship {
    /// The link: direction, binding, where it came from, and whether it is still seen.
    pub link: Link,
    /// The resource at the other end.
    pub resource: Resource,
    /// Its project, if it is in one.
    pub project: Option<Project>,
}

/// The latest recorded release of one resource. Says nothing about its health now.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct RecordedResourceRelease {
    /// The deployment that made this release.
    pub deploy_id: String,
    /// The project's deployment number, counting failed ones.
    #[ts(type = "number")]
    pub attempt_version: i64,
    /// How the deployment ended; it can have failed after this resource was released.
    pub status: DeployStatus,
    /// Person or agent that started the attempt.
    pub started_by: String,
    /// RFC 3339 start time.
    pub started_at: String,
    /// RFC 3339 finish time, if finished.
    pub finished_at: Option<String>,
    /// The provider's release recorded for this resource.
    pub release: ResourceRelease,
}

/// What the resource drawer shows. Unknown status and missing metrics stay null.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ResourceDetail {
    /// The resource, with the status the provider last reported.
    pub resource: Resource,
    /// Owning project; null for an ungrouped discovery candidate.
    pub project: Option<Project>,
    /// Its account, without credentials.
    pub account: Account,
    /// Outgoing edges: resources this resource depends on.
    pub dependencies: Vec<ResourceRelationship>,
    /// Incoming edges: recorded consumers of this resource.
    pub used_by: Vec<ResourceRelationship>,
    /// Recorded custom domains attached to this resource.
    pub domains: Vec<CustomDomain>,
    /// The latest recorded day of usage (UTC); not a month-to-date total.
    pub latest_usage: Option<Usage>,
    /// The latest deployment that released this resource.
    pub latest_recorded_release: Option<RecordedResourceRelease>,
    /// Always false: links at the provider are not all discovered yet.
    pub relationships_complete: bool,
}

impl<'a> ProjectService<'a> {
    /// Reads a resource's detail from saved state, without contacting providers. It includes
    /// links no longer seen, reads every page of domains, and never presents a deployment
    /// record as current health.
    ///
    /// # Errors
    /// Rejects missing state, invalid recorded links, missing permission and catalog limits.
    pub async fn resource_detail(
        self,
        actor: &Actor,
        id: &str,
    ) -> Result<ResourceDetail, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let resource = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let project = match &resource.project_id {
            Some(project_id) => Some(
                self.engine
                    .repository
                    .project(project_id)
                    .await?
                    .ok_or(EngineError::NotFound)?,
            ),
            None => None,
        };
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let mut dependencies = vec![];
        let mut used_by = vec![];
        for link in self.engine.repository.resource_links(id).await? {
            let outgoing = link.from_resource == id;
            let other = if outgoing {
                &link.to_resource
            } else if link.to_resource == id {
                &link.from_resource
            } else {
                return Err(EngineError::Store);
            };
            if other == id {
                return Err(EngineError::Store);
            }
            let related = self
                .engine
                .repository
                .resource(other)
                .await?
                .ok_or(EngineError::NotFound)?;
            let related_project = match &related.project_id {
                Some(project_id) if Some(project_id) == resource.project_id.as_ref() => {
                    project.clone()
                }
                Some(project_id) => Some(
                    self.engine
                        .repository
                        .project(project_id)
                        .await?
                        .ok_or(EngineError::NotFound)?,
                ),
                None => None,
            };
            let relationship = ResourceRelationship {
                link,
                resource: related,
                project: related_project,
            };
            if outgoing {
                dependencies.push(relationship)
            } else {
                used_by.push(relationship)
            };
        }
        let latest_recorded_release = if let Some(project_id) = &resource.project_id {
            self.engine
                .repository
                .latest_resource_deploy(project_id, id)
                .await?
                .map(|deploy| {
                    let mut releases = deploy
                        .releases
                        .into_iter()
                        .filter(|release| release.resource_id == id);
                    let release = releases.next().ok_or(EngineError::Store)?;
                    if releases.next().is_some() {
                        return Err(EngineError::Store);
                    }
                    Ok(RecordedResourceRelease {
                        deploy_id: deploy.id,
                        attempt_version: deploy.version,
                        status: deploy.status,
                        started_by: deploy.started_by,
                        started_at: deploy.started_at,
                        finished_at: deploy.finished_at,
                        release,
                    })
                })
                .transpose()?
        } else {
            None
        };
        let domains = self
            .engine
            .environments()
            .all_domains()
            .await?
            .into_iter()
            .filter(|domain| domain.resource_id == id)
            .collect();
        let latest_usage = self.engine.repository.latest_resource_usage(id).await?;
        self.engine.authorize(actor, "apps:read").await?;
        Ok(ResourceDetail {
            resource,
            project,
            account,
            dependencies,
            used_by,
            domains,
            latest_usage,
            latest_recorded_release,
            relationships_complete: false,
        })
    }
}

impl<'a> ProjectService<'a> {
    /// Reads a database's structure through the provider's API, for watched resources too.
    /// At most eight reads run at once; each times out after sixty seconds.
    ///
    /// # Errors
    /// Rejects non-databases, unavailable credentials or providers, a changed project and revoked
    /// access.
    pub async fn database_observation(
        self,
        actor: &Actor,
        id: &str,
    ) -> Result<crate::domain::database_observation::DatabaseObservation, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let _permit = self.engine.resource_reads.try_acquire().map_err(|_| {
            EngineError::Conflict("Eight resource observations are already running. Retry shortly.")
        })?;
        let resource = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.kind != ResourceKind::Database {
            return Err(EngineError::Invalid("Select a database resource."));
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let result = tokio::time::timeout(
            READ_TIMEOUT,
            self.engine
                .adapter(resource.provider)?
                .database_observation(&token, &account.external_id, &resource.provider_ref),
        )
        .await
        .map_err(|_| EngineError::Provider {
            evidence: None,
            code: "database.observation_timeout".into(),
            message: "The database inventory took too long. Retry the observation.".into(),
        })??;
        self.engine.authorize(actor, "apps:read").await?;
        let current = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let current_account = self
            .engine
            .repository
            .account(&account.id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if current_account.status != AccountStatus::Connected
            || current_account.external_id != account.external_id
            || current_account.provider != account.provider
        {
            return Err(EngineError::Conflict(
                "The account changed during observation. Reconnect and retry.",
            ));
        }
        if current.kind != resource.kind
            || current.project_id != resource.project_id
            || current.account_id != resource.account_id
            || current.provider_ref != resource.provider_ref
            || current.provider != resource.provider
        {
            return Err(EngineError::Conflict(
                "The resource changed during observation. Retry.",
            ));
        }
        Ok(result)
    }
}

impl Engine {
    /// Whether a resource in another project depends on this one, directly or indirectly.
    /// Checked before disruptive actions.
    pub(crate) async fn has_external_consumers(
        &self,
        resource: &Resource,
    ) -> Result<bool, EngineError> {
        let resources = self.projects().all_resources().await?;
        let links = self.repository.all_links().await?;
        let mut affected = std::collections::BTreeSet::from([resource.id.clone()]);
        loop {
            let previous = affected.len();
            for link in &links {
                if affected.contains(&link.to_resource) {
                    affected.insert(link.from_resource.clone());
                }
            }
            if affected.len() == previous {
                break;
            }
        }
        for id in affected {
            let consumer = resources
                .iter()
                .find(|resource| resource.id == id)
                .ok_or(EngineError::Store)?;
            if consumer.project_id != resource.project_id {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl<'a> ProjectService<'a> {
    /// Reads a project's owned resources and directly connected shared resources.
    ///
    /// # Errors
    /// Rejects missing projects, dangling relationships and graph bounds.
    pub async fn project_map_resources(
        self,
        id: &str,
    ) -> Result<(Vec<Resource>, Vec<Link>), EngineError> {
        self.engine
            .repository
            .project(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let resources = self.engine.projects().all_resources().await?;
        let owned: std::collections::BTreeSet<_> = resources
            .iter()
            .filter(|resource| resource.project_id.as_deref() == Some(id))
            .map(|resource| resource.id.clone())
            .collect();
        let links: Vec<_> = self
            .engine
            .repository
            .all_links()
            .await?
            .into_iter()
            .filter(|link| owned.contains(&link.from_resource) || owned.contains(&link.to_resource))
            .collect();
        let mut visible = owned;
        for link in &links {
            visible.insert(link.from_resource.clone());
            visible.insert(link.to_resource.clone());
        }
        let nodes: Vec<_> = resources
            .into_iter()
            .filter(|resource| visible.contains(&resource.id))
            .collect();
        if nodes.len() != visible.len() {
            return Err(EngineError::Store);
        }
        Ok((nodes, links))
    }
}

impl<'a> ProjectService<'a> {
    /// Reads one page of the provider's operations on a database, for watched resources too.
    /// At most eight reads run at once; each times out after sixty seconds.
    ///
    /// # Errors
    /// Rejects non-databases, unavailable credentials or providers, a changed project and revoked
    /// access.
    pub async fn database_operations(
        self,
        actor: &Actor,
        id: &str,
        cursor: Option<&str>,
    ) -> Result<crate::domain::database_observation::DatabaseOperationHistory, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let _permit = self.engine.resource_reads.try_acquire().map_err(|_| {
            EngineError::Conflict("Eight resource observations are already running. Retry shortly.")
        })?;
        let resource = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.kind != ResourceKind::Database {
            return Err(EngineError::Invalid("Select a database resource."));
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let result = tokio::time::timeout(
            READ_TIMEOUT,
            self.engine.adapter(resource.provider)?.database_operations(
                &token,
                &account.external_id,
                &resource.provider_ref,
                cursor,
            ),
        )
        .await
        .map_err(|_| EngineError::Provider {
            evidence: None,
            code: "database.observation_timeout".into(),
            message: "The database inventory took too long. Retry the observation.".into(),
        })??;
        self.engine.authorize(actor, "apps:read").await?;
        let current = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let current_account = self
            .engine
            .repository
            .account(&account.id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if current_account.status != AccountStatus::Connected
            || current_account.external_id != account.external_id
            || current_account.provider != account.provider
        {
            return Err(EngineError::Conflict(
                "The account changed during observation. Reconnect and retry.",
            ));
        }
        if current.kind != resource.kind
            || current.project_id != resource.project_id
            || current.account_id != resource.account_id
            || current.provider_ref != resource.provider_ref
            || current.provider != resource.provider
        {
            return Err(EngineError::Conflict(
                "The resource changed during observation. Retry.",
            ));
        }
        Ok(result)
    }
}
