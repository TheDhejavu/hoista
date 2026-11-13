//! Management operations shared by the desktop and agent transports.

use crate::deploys::DeployService;
use crate::engine::{Actor, Engine, now};
use crate::{deploys::planning::release_order, domain::model::*, ports::catalog::EngineError};

impl Engine {
    pub(crate) async fn remove_managed_resource(
        &self,
        resource: &Resource,
    ) -> Result<(), EngineError> {
        for domain in self
            .environments()
            .all_domains()
            .await?
            .into_iter()
            .filter(|domain| domain.resource_id == resource.id)
        {
            self.detach_saved_domain(resource, &domain).await?;
        }
        let account = self
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.accounts().account_token(&account.id).await?;
        self.adapter(resource.provider)?
            .delete(&token, &account.external_id, &resource.provider_ref)
            .await?;
        self.repository.remove_resource(&resource.id).await
    }

    pub(crate) async fn execute_rollback(&self, deploy: &mut Deploy) -> Result<(), EngineError> {
        deploy.status = DeployStatus::Running;
        self.step(deploy, "restore", StepStatus::Running).await?;
        for index in 0..deploy.releases.len() {
            self.check_deploy_cancelled(&deploy.id).await?;
            let released = &mut deploy.releases[index];
            let resource = self
                .repository
                .resource(&released.resource_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let project = self
                .repository
                .project(&deploy.project_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            if resource.mode != Mode::Managed || project.mode != Mode::Managed {
                return Err(EngineError::Watching);
            }
            if resource.kind == ResourceKind::Database {
                continue;
            }
            let account = self
                .repository
                .account(&resource.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let token = self.accounts().account_token(&account.id).await?;
            released.release = self
                .adapter(resource.provider)?
                .rollback(&token, &account.external_id, &released.release)
                .await?;
            self.repository.put_deploy(deploy).await?;
        }
        self.step(deploy, "restore", StepStatus::Succeeded).await?;
        self.step(deploy, "verify", StepStatus::Running).await?;
        for released in &deploy.releases {
            let mut resource = self
                .repository
                .resource(&released.resource_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let account = self
                .repository
                .account(&resource.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let token = self.accounts().account_token(&account.id).await?;
            self.adapter(resource.provider)?
                .health(&token, &account.external_id, &released.release)
                .await?;
            resource.status = ResourceStatus::Live;
            resource.url = released.release.url.clone();
            self.repository.put_resource(&resource).await?;
        }
        self.step(deploy, "verify", StepStatus::Succeeded).await?;
        deploy.status = DeployStatus::RolledBack;
        deploy.finished_at = Some(now());
        self.repository.put_deploy(deploy).await
    }
}

impl<'a> DeployService<'a> {
    /// Deletes an approved project, retaining failed resources for a safe retry.
    ///
    /// # Errors
    /// Rejects watching projects, active deployments, missing approval or failed deletions.
    pub async fn delete_project(
        self,
        actor: &Actor,
        id: &str,
        approval: Option<&str>,
        confirmation: Option<&crate::domain::deletion::DeletionConfirmation>,
    ) -> Result<Vec<crate::domain::database_export::DatabaseArchive>, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let project = self
            .engine
            .repository
            .project(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if project.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        if self.engine.repository.has_pending_deploy(id).await? {
            return Err(EngineError::Conflict(
                "Wait for pending deployments before deleting this project.",
            ));
        }
        let resources = self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| resource.project_id.as_deref() == Some(id))
            .collect::<Vec<_>>();
        if resources
            .iter()
            .any(|resource| resource.mode != Mode::Managed)
        {
            return Err(EngineError::Watching);
        }
        let links = self
            .engine
            .repository
            .links(id)
            .await?
            .into_iter()
            .filter(|link| {
                resources
                    .iter()
                    .any(|resource| resource.id == link.to_resource)
            })
            .collect::<Vec<_>>();
        let order = deletion_order(&resources, &links)?;
        let (archives, _) = self
            .engine
            .confirm_deletion(
                actor,
                GatedAction::DeleteProject,
                id,
                approval,
                confirmation,
            )
            .await?;
        // Check the whole graph before changing anything at a provider.
        for index in order {
            self.engine
                .remove_managed_resource(&resources[index])
                .await?;
        }
        self.engine.repository.remove_project(id).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                Some(id),
                "app.deleted",
                "Deleted approved project resources.",
            )
            .await?;
        Ok(archives)
    }

    /// Pauses or resumes one managed resource. If it partly fails, the status is left unknown.
    ///
    /// # Errors
    /// Rejects watching/unsupported resources, queued deployments and provider conflicts.
    pub async fn control_resource(
        self,
        actor: &Actor,
        id: &str,
        resume: bool,
    ) -> Result<Resource, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let mut resource = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        if let Some(project_id) = &resource.project_id {
            let project = self
                .engine
                .repository
                .project(project_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            if project.mode != Mode::Managed {
                return Err(EngineError::Watching);
            }
            if self
                .engine
                .repository
                .has_pending_deploy(project_id)
                .await?
            {
                return Err(EngineError::Conflict(
                    "Wait for pending deployments before pausing or resuming.",
                ));
            }
        }
        if !resume && self.engine.has_external_consumers(&resource).await? {
            return Err(EngineError::Conflict(
                "Other projects depend on this resource. Move their dependencies before pausing it.",
            ));
        }
        let adapter = self.engine.adapter(resource.provider)?;
        if !adapter.capabilities().pause {
            return Err(crate::ports::providers::unsupported());
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        // An interrupted multi-Machine operation must not leave a stale Live/Paused label.
        resource.status = ResourceStatus::Unknown;
        self.engine.repository.put_resource(&resource).await?;
        if resume {
            resource.status = adapter
                .resume(&token, &account.external_id, &resource.provider_ref)
                .await?;
        } else {
            adapter
                .pause(&token, &account.external_id, &resource.provider_ref)
                .await?;
            resource.status = ResourceStatus::Paused;
        }
        self.engine.repository.put_resource(&resource).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                resource.project_id.as_deref(),
                if resume {
                    "resource.resumed"
                } else {
                    "resource.paused"
                },
                if resume {
                    "Restored saved startup controls and observed provider runtime state."
                } else {
                    "Paused supported compute; storage and other provider fees may continue."
                },
            )
            .await?;
        Ok(resource)
    }

    /// Validates an edited plan without changing any queued plan snapshot.
    ///
    /// # Errors
    /// Rejects changes to source identity, invalid dependencies or mismatched accounts.
    pub async fn edit_plan(self, actor: &Actor, mut value: Plan) -> Result<Plan, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        if value.id.starts_with("queued:") {
            return Err(EngineError::Conflict(
                "A queued plan snapshot cannot be edited.",
            ));
        }
        let old = self
            .engine
            .repository
            .plan(&value.id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if old.revision != value.revision {
            return Err(EngineError::Conflict(
                "The plan changed since you read it. Run hoist plan show, review the new version and reapply your edit.",
            ));
        }
        if old.source != value.source
            || old.framework != value.framework
            || old.prisma_migration != value.prisma_migration
            || old.generated_secret_names != value.generated_secret_names
        {
            return Err(EngineError::Invalid(
                "Create a new plan when changing source, framework, authentication or migration configuration.",
            ));
        }
        release_order(&value.resources, &self.engine.providers())?;
        crate::deploys::planning::validate_database_plan(&value, &self.engine.providers())?;
        for resource in &value.resources {
            let account = self
                .engine
                .repository
                .account(&resource.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            if account.provider != resource.provider || account.status != AccountStatus::Connected {
                return Err(EngineError::Invalid(
                    "Choose a connected account matching each provider.",
                ));
            }
        }
        let expected_revision = value.revision;
        value.revision = expected_revision
            .checked_add(1)
            .ok_or(EngineError::Invalid(
                "Plan revision is exhausted. Create a fresh plan.",
            ))?;
        self.engine
            .repository
            .update_plan(&value, expected_revision)
            .await?;
        Ok(value)
    }

    /// Queues a rollback to earlier releases; database schemas are left as they are.
    ///
    /// # Errors
    /// Rejects incomplete versions, watching resources and unavailable rollback capabilities.
    pub async fn enqueue_rollback(
        self,
        actor: &Actor,
        deploy_id: &str,
    ) -> Result<Deploy, EngineError> {
        self.engine
            .deploys()
            .enqueue_rollback_with_options(actor, deploy_id, DeployOptions::default())
            .await
    }

    /// Queues a rollback with the current caller's presentation metadata.
    ///
    /// # Errors
    /// Rejects invalid metadata or an unavailable/unauthorized rollback target.
    pub async fn enqueue_rollback_with_options(
        self,
        actor: &Actor,
        deploy_id: &str,
        options: DeployOptions,
    ) -> Result<Deploy, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        if options.expected_plan_revision.is_some() {
            return Err(EngineError::Invalid(
                "Rollback selects a recorded release, not an editable plan revision.",
            ));
        }
        self.engine.authorize(actor, "apps:write").await?;
        let request_key = options.request_key.clone();
        let (title, channel, started_by_name) = self.engine.deploy_metadata(actor, options).await?;
        let admission = crate::deploys::metadata::admission(
            actor,
            request_key.as_deref(),
            serde_json::json!(["rollback-admission-v1", deploy_id, title, channel]),
        )?;
        if let Some(admission) = &admission
            && let Some(existing) = self.engine.repository.admitted_deploy(admission).await?
        {
            return Ok(existing);
        }
        let old = self
            .engine
            .repository
            .deploy(deploy_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if !matches!(old.status, DeployStatus::Live | DeployStatus::RolledBack)
            || old.releases.is_empty()
        {
            return Err(EngineError::Invalid(
                "Choose a successfully released version to restore.",
            ));
        }
        let project = self
            .engine
            .repository
            .project(&old.project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if project.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        for release in &old.releases {
            let resource = self
                .engine
                .repository
                .resource(&release.resource_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            if resource.mode != Mode::Managed {
                return Err(EngineError::Watching);
            }
            if resource.kind != ResourceKind::Database
                && !self
                    .engine
                    .adapter(resource.provider)?
                    .capabilities()
                    .rollback
            {
                return Err(crate::ports::providers::unsupported());
            }
        }
        let mut next = old;
        next.title = title;
        next.channel = channel;
        next.started_by_name = started_by_name;
        next.environment_keys.clear();
        next.rollback_of = Some(next.id.clone());
        next.id = uuid::Uuid::new_v4().to_string();
        next.version = 0;
        next.cancel_requested = false;
        next.started_by = actor.id().into();
        next.started_at = now();
        next.finished_at = None;
        next.duration_ms = None;
        next.status = DeployStatus::Queued;
        next.steps = vec![
            DeployStep {
                name: "restore".into(),
                status: StepStatus::Pending,
                message: None,
            },
            DeployStep {
                name: "verify".into(),
                status: StepStatus::Pending,
                message: None,
            },
        ];
        self.engine
            .repository
            .enqueue_with_admission(&next, admission.as_ref())
            .await
    }
}

/// Remove runtimes before databases, following consumer edges where possible.
/// Cycles inside the reviewed deletion set are broken in a fixed order; a resource outside
/// that set is never quietly skipped.
pub(crate) fn deletion_order(
    resources: &[Resource],
    links: &[Link],
) -> Result<Vec<usize>, EngineError> {
    let indexes: std::collections::BTreeMap<_, _> = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| (resource.id.as_str(), index))
        .collect();
    if links.iter().any(|link| {
        !indexes.contains_key(link.from_resource.as_str())
            || !indexes.contains_key(link.to_resource.as_str())
    }) {
        return Err(EngineError::Conflict(
            "Reconcile missing or foreign resources before deleting this project.",
        ));
    }
    let mut remaining: std::collections::BTreeSet<_> = (0..resources.len()).collect();
    let mut order = Vec::with_capacity(resources.len());
    while !remaining.is_empty() {
        let databases = remaining
            .iter()
            .all(|index| resources[*index].kind == ResourceKind::Database);
        let candidates: Vec<_> = remaining
            .iter()
            .copied()
            .filter(|index| (resources[*index].kind == ResourceKind::Database) == databases)
            .collect();
        let next = candidates
            .iter()
            .copied()
            .find(|index| {
                !links.iter().any(|link| {
                    link.to_resource == resources[*index].id
                        && indexes
                            .get(link.from_resource.as_str())
                            .is_some_and(|source| {
                                remaining.contains(source)
                                    && (resources[*source].kind == ResourceKind::Database)
                                        == databases
                            })
                })
            })
            .or_else(|| candidates.first().copied())
            .ok_or(EngineError::Store)?;
        remaining.remove(&next);
        order.push(next);
    }
    Ok(order)
}
