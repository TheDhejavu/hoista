//! The saved deploy queue and the worker that runs it, rolling back when a deploy fails.

use crate::deploys::DeployService;
use crate::engine::{Actor, Engine, now};
use crate::{
    deploys::{
        metadata::admission,
        planning::{release_order, validate_database_plan},
    },
    domain::{
        model::*,
        provenance::{SourceObservation, SourceProvenance},
    },
    environments::secrets as environment_secrets,
    ports::{
        build::BuildProgress,
        catalog::{CatalogRepository, DeploymentAdmission, EngineError},
        providers::Artifact,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// Every deploy's steps, in order. A step that doesn't apply is skipped.
const DEPLOY_STEPS: [&str; 9] = [
    "detect",
    "plan",
    "secrets",
    "leak_check",
    "build",
    "provision",
    "migrate",
    "release",
    "verify",
];
/// How often the worker looks for newly queued deploys.
const QUEUE_POLL: Duration = Duration::from_millis(250);
const SOURCE_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(10);
/// Pages of history searched for the deploy to roll back to.
const MAX_ROLLBACK_PAGES: usize = 1_000;

fn fingerprint(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

/// Saves the source snapshot on the deploy as soon as the builder has it.
struct DeploymentBuildProgress<'a> {
    repository: &'a dyn CatalogRepository,
    deploy: &'a mut Deploy,
}

#[async_trait::async_trait]
impl BuildProgress for DeploymentBuildProgress<'_> {
    async fn source_snapshot(&mut self, provenance: &SourceProvenance) -> Result<(), EngineError> {
        self.deploy.provenance = Some(provenance.clone());
        self.repository.put_deploy(self.deploy).await
    }
}

impl Engine {
    /// Checks a deploy and queues it. The caller holds the provider-change lock.
    pub(crate) async fn enqueue_deploy_locked(
        &self,
        actor: &Actor,
        project_id: &str,
        plan_id: &str,
        options: DeployOptions,
        environment_keys: Vec<String>,
        environment_admission: Option<DeploymentAdmission>,
    ) -> Result<Deploy, EngineError> {
        self.authorize(actor, "apps:write").await?;
        let expected_plan_revision = options.expected_plan_revision;
        if expected_plan_revision
            .is_some_and(|revision| !(0..=MAX_SAFE_INTEGER).contains(&revision))
        {
            return Err(EngineError::Invalid(
                "Use the nonnegative plan revision returned by Hoista.",
            ));
        }
        if options.request_key.is_some()
            && (expected_plan_revision.is_none() || !environment_keys.is_empty())
        {
            return Err(EngineError::Invalid(
                "A deployment request key requires an explicit plan revision. Secret-bearing changes must use environment admission.",
            ));
        }
        let request_key = options.request_key.clone();
        let (title, channel, started_by_name) = self.deploy_metadata(actor, options).await?;
        let admission = admission(
            actor,
            request_key.as_deref(),
            serde_json::json!([
                "deploy-admission-v1",
                project_id,
                plan_id,
                expected_plan_revision,
                title,
                channel
            ]),
        )?;
        let admission = admission.or(environment_admission);
        if let Some(admission) = &admission
            && let Some(existing) = self.repository.admitted_deploy(admission).await?
        {
            return Ok(existing);
        }
        let project = self
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if project.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        let mut plan = self
            .repository
            .plan(plan_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if expected_plan_revision.is_some_and(|revision| revision != plan.revision) {
            return Err(EngineError::Conflict(
                "The reviewed plan changed. Read and review the current plan before requesting a deployment; nothing was queued.",
            ));
        }
        if !plan.source.within(&project.source) {
            return Err(EngineError::Invalid(
                "This plan was made from a folder outside this project's source. Plan again with project_id, from the project's folder or a folder inside it.",
            ));
        }
        release_order(&plan.resources, &self.providers())?;
        validate_database_plan(&plan, &self.providers())?;
        let existing = self.projects().all_resources().await?;
        for resource in &mut plan.resources {
            let account = self
                .repository
                .account(&resource.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            if account.provider != resource.provider
                || account.status != AccountStatus::Connected
                || !self.adapter(resource.provider)?.capabilities().deploy
            {
                return Err(EngineError::Invalid(
                    "The planned provider account cannot deploy this resource.",
                ));
            }
            let matching = existing.iter().find(|old| {
                old.project_id.as_deref() == Some(project_id)
                    && old.name == resource.name
                    && old.account_id == resource.account_id
            });
            if matching.is_some_and(|old| old.mode != Mode::Managed) {
                return Err(EngineError::Watching);
            }
            // A provider may list one resource the same way whether it serves
            // HTTP or cron. Source inference cannot change an existing
            // resource's role there (including when its last cron is removed).
            if let Some(old) = matching
                && self
                    .adapter(old.provider)
                    .is_ok_and(|adapter| adapter.keeps_known_role(&old.provider_ref))
                && matches!(old.kind, ResourceKind::Web | ResourceKind::Job)
                && matches!(resource.kind, ResourceKind::Web | ResourceKind::Job)
            {
                resource.kind = old.kind;
            }
            resource.provider_ref = matching.map(|old| old.provider_ref.clone());
        }
        if plan
            .resources
            .iter()
            .any(|resource| resource.provider_ref.is_none())
        {
            self.authorize(actor, "apps:create").await?;
        }
        // Each queued run has its own plan snapshot, so editing the plan can't change a
        // pending deployment.
        plan.id = format!("queued:{}", uuid::Uuid::new_v4());
        self.repository.put_plan(&plan).await?;
        let deploy = Deploy {
            failure: None,
            environment_keys,
            title,
            channel,
            started_by_name,
            release_version: None,
            provenance: None,
            source_observation: None,
            cancel_requested: false,
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.into(),
            version: 0,
            plan_id: plan.id,
            releases: vec![],
            source: project.source,
            started_by: actor.id().into(),
            status: DeployStatus::Queued,
            steps: DEPLOY_STEPS
                .into_iter()
                .map(|name| DeployStep {
                    name: name.into(),
                    status: StepStatus::Pending,
                    message: None,
                })
                .collect(),
            url: None,
            started_at: now(),
            finished_at: None,
            duration_ms: None,
            rollback_of: None,
        };
        self.repository
            .enqueue_with_admission(&deploy, admission.as_ref())
            .await
    }

    /// Stops the deploy here when the person asked to cancel it.
    pub(crate) async fn check_deploy_cancelled(&self, id: &str) -> Result<(), EngineError> {
        if self
            .repository
            .deploy(id)
            .await?
            .ok_or(EngineError::NotFound)?
            .cancel_requested
        {
            return Err(EngineError::Provider {
                evidence: None,
                code: "deploy.cancelled".into(),
                message: "Deployment cancelled at a safe boundary. Completed provider changes are recorded; review any rollback result.".into(),
            });
        }
        Ok(())
    }

    /// Runs the saved queue one deploy at a time. On shutdown it stops taking new deploys and
    /// cancels the current one, letting its provider call, recovery and cleanup finish first.
    ///
    /// # Errors
    /// Returns state failures that prevent safe queue operation.
    pub async fn run_deployments(
        self: Arc<Self>,
        shutdown: CancellationToken,
    ) -> Result<(), EngineError> {
        self.run_deployments_until_idle(shutdown, CancellationToken::new())
            .await
    }

    /// Runs everything already queued once new deploys are no longer taken, keeping inputs
    /// held in memory. A forced shutdown still cancels the current deploy cleanly. Cancel
    /// `drain` only after every request transport has drained, so nothing is queued after.
    ///
    /// # Errors
    /// Returns state failures that prevent safe queue operation.
    pub async fn run_deployments_until_idle(
        self: Arc<Self>,
        shutdown: CancellationToken,
        drain: CancellationToken,
    ) -> Result<(), EngineError> {
        self.repository.recover_deploys().await?;
        loop {
            // Read the drain flag before the queue: a queue read before new
            // deploys stopped could miss the last one accepted.
            let draining = drain.is_cancelled();
            let pending = self.repository.queued().await?;
            if pending.is_empty() && draining {
                return Ok(());
            }
            for mut deploy in pending {
                if shutdown.is_cancelled() {
                    return Ok(());
                }
                let id = deploy.id.clone();
                let result = {
                    let execution = self.deploys().execute_deploy(&mut deploy);
                    tokio::pin!(execution);
                    tokio::select! {
                        result = &mut execution => result,
                        () = shutdown.cancelled() => {
                            // Never abandon a provider write or a build half-way.
                            // Record the cancel, and let the deploy stop where it's safe.
                            if self.repository.cancel_deploy("engine", &id).await.is_err() {
                                tracing::warn!(code = "deploy.shutdown_cancel_failed");
                            }
                            execution.await
                        },
                    }
                };
                self.environment_inputs.lock().await.remove(&id);
                if let Err(error) = result {
                    self.record_failure(&mut deploy, &error).await?;
                }
            }
            tokio::select! {
                () = shutdown.cancelled() => return Ok(()),
                () = tokio::time::sleep(QUEUE_POLL) => {}
            }
        }
    }

    /// Marks a deploy failed, rolling back what it released, and saves it.
    async fn record_failure(
        &self,
        deploy: &mut Deploy,
        error: &EngineError,
    ) -> Result<(), EngineError> {
        let failed_steps = deploy
            .steps
            .iter()
            .filter(|step| step.status == StepStatus::Running)
            .map(|step| step.name.clone())
            .collect();
        let release_started = deploy.steps.iter().any(|step| {
            step.name == "release"
                && matches!(step.status, StepStatus::Running | StepStatus::Succeeded)
        });
        let compensation_failed = self.deploys().rollback_after_failure(deploy).await.is_err();
        deploy.failure = Some(DeployFailure {
            provider_response: error.provider_response().cloned(),
            missing_secrets: error.missing_secrets().to_vec(),
            code: error.code().into(),
            message: error.to_string(),
            steps: failed_steps,
            release_started,
            compensation_failed,
        });
        if compensation_failed {
            deploy.steps.push(DeployStep {
                name: "automatic_rollback".into(),
                status: StepStatus::Failed,
                message: Some("Automatic rollback could not restore every resource. Inspect the project before retrying.".into()),
            });
            self.repository
                .event(
                    "engine",
                    Some(&deploy.project_id),
                    "deploy.rollback_failed",
                    "Automatic rollback did not restore every resource; inspect the provider state.",
                )
                .await?;
        }
        deploy.status = DeployStatus::Failed;
        deploy.finished_at = Some(now());
        for step in &mut deploy.steps {
            if step.status == StepStatus::Running {
                step.status = StepStatus::Failed;
                step.message = Some(error.to_string());
            }
        }
        self.repository.put_deploy(deploy).await?;
        if self.builder.cleanup(&deploy.id).await.is_err() {
            tracing::warn!(code = "build.cleanup_failed");
        }
        Ok(())
    }

    /// Moves one of the deploy's steps on, and saves it.
    pub(crate) async fn step(
        &self,
        deploy: &mut Deploy,
        name: &str,
        status: StepStatus,
    ) -> Result<(), EngineError> {
        self.check_deploy_cancelled(&deploy.id).await?;
        let step = deploy
            .steps
            .iter_mut()
            .find(|step| step.name == name)
            .ok_or(EngineError::Store)?;
        step.status = status;
        self.repository.put_deploy(deploy).await
    }
}

impl<'a> DeployService<'a> {
    /// Queues a checked plan. Only the worker makes the changes at the provider.
    ///
    /// # Errors
    /// Rejects watching projects, disconnected providers and stale or invalid plans.
    pub async fn enqueue_deploy(
        self,
        actor: &Actor,
        project_id: &str,
        plan_id: &str,
    ) -> Result<Deploy, EngineError> {
        self.engine
            .deploys()
            .enqueue_deploy_with_options(actor, project_id, plan_id, DeployOptions::default())
            .await
    }

    /// Queues a deployment with checked [`DeployOptions`]: a retry key, the reviewed plan
    /// revision, a title.
    ///
    /// # Errors
    /// Rejects invalid options, and the same permission and plan failures as a plain deploy.
    pub async fn enqueue_deploy_with_options(
        self,
        actor: &Actor,
        project_id: &str,
        plan_id: &str,
        options: DeployOptions,
    ) -> Result<Deploy, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine
            .enqueue_deploy_locked(actor, project_id, plan_id, options, vec![], None)
            .await
    }

    /// Asks for a deploy to be cancelled, even while the worker holds the provider-change lock.
    ///
    /// # Errors
    /// Rejects unauthorized agents, missing deployments or already-finished work.
    pub async fn cancel_deploy(self, actor: &Actor, id: &str) -> Result<(), EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.repository.cancel_deploy(actor.id(), id).await?;
        self.engine.environment_inputs.lock().await.remove(id);
        Ok(())
    }

    async fn execute_deploy(self, deploy: &mut Deploy) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.check_deploy_cancelled(&deploy.id).await?;
        if deploy.rollback_of.is_some() {
            return self.engine.execute_rollback(deploy).await;
        }
        let started = Instant::now();
        let project = self
            .engine
            .repository
            .project(&deploy.project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if project.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        let mut plan = self
            .engine
            .repository
            .plan(&deploy.plan_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        deploy.status = DeployStatus::Running;
        self.engine.repository.put_deploy(deploy).await?;
        self.engine
            .step(deploy, "detect", StepStatus::Running)
            .await?;
        deploy.source_observation = Some(
            match tokio::time::timeout(
                SOURCE_OBSERVATION_TIMEOUT,
                self.engine.inspector.observe_source(&plan.source),
            )
            .await
            {
                Ok(Ok(observation)) => observation,
                _ => SourceObservation::Unavailable,
            },
        );
        // Saved before anything below can fail. Not being able to read the
        // source never hides the deploy's own error, but not being able to
        // save does stop it.
        self.engine.repository.put_deploy(deploy).await?;
        let detected = self.engine.inspector.detect(&plan.source).await?;
        if detected.framework != plan.framework
            || detected.prisma_migration != plan.prisma_migration
            || detected.generated_secret_names != plan.generated_secret_names
        {
            return Err(EngineError::Conflict(
                "The source framework, authentication or migration configuration changed. Create a fresh deployment plan.",
            ));
        }
        self.engine
            .step(deploy, "detect", StepStatus::Succeeded)
            .await?;
        self.engine
            .step(deploy, "plan", StepStatus::Running)
            .await?;
        let existing_resources = self.engine.projects().all_resources().await?;
        resolve_queued_resources(&mut plan, &project.id, &existing_resources)?;
        self.engine.repository.put_plan(&plan).await?;
        let order = release_order(&plan.resources, &self.engine.providers())?;
        validate_database_plan(&plan, &self.engine.providers())?;
        self.engine
            .step(deploy, "plan", StepStatus::Succeeded)
            .await?;
        self.engine
            .step(deploy, "secrets", StepStatus::Running)
            .await?;
        let overrides = if deploy.environment_keys.is_empty() {
            vec![]
        } else {
            let actor = if deploy.started_by == "person" {
                Actor::Person
            } else {
                Actor::Agent {
                    id: deploy.started_by.clone(),
                }
            };
            self.engine.authorize(&actor, "apps:write").await?;
            self.engine
                .validate_environment_targets(&project.id, &plan, &deploy.environment_keys)
                .await?;
            self.engine
                .environment_inputs
                .lock()
                .await
                .remove(&deploy.id)
                .ok_or_else(|| EngineError::Provider {
                    evidence: None,
                    code: "deploy.environment_input_lost".into(),
                    message: "The in-memory environment values are unavailable after restart or cancellation. Submit the replacement again; no old value was substituted.".into(),
                })?
        };
        let mut secrets = self
            .engine
            .builder
            .secrets_with_overrides(&plan, overrides)
            .await?;
        let existing_names = self.engine.repository.secrets(&project.id).await?;
        for metadata in &existing_names {
            if !metadata.requires_source_sync || deploy.environment_keys.contains(&metadata.name) {
                continue;
            }
            let supplied = secrets.iter().find(|(name, _)| name == &metadata.name);
            let matches = supplied
                .is_none_or(|(_, value)| fingerprint(value.expose()) == metadata.fingerprint);
            if !matches || (browser_env(&metadata.name) && supplied.is_none()) {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "deploy.environment_source_stale".into(),
                    message: "Source environment inputs differ from the last explicit replacement. Update the source values or submit another environment deployment.".into(),
                });
            }
        }
        let provided_names: BTreeSet<_> = secrets.iter().map(|(name, _)| name.clone()).collect();
        environment_secrets::resolve(
            &plan,
            &project.id,
            &existing_resources,
            &existing_names,
            &mut secrets,
        )?;
        let generated_inputs: BTreeSet<_> = secrets
            .iter()
            .filter(|(name, _)| {
                !provided_names.contains(name) && plan.generated_secret_names.contains(name)
            })
            .map(|(name, _)| name.clone())
            .collect();
        // Browser variables are built into the files, and may not reach the
        // provider at all, so their fingerprints are kept from here.
        let public_inputs: Vec<_> = secrets
            .iter()
            .filter(|(name, _)| browser_env(name))
            .map(|(name, value)| (name.clone(), fingerprint(value.expose())))
            .collect();
        self.engine
            .step(deploy, "secrets", StepStatus::Succeeded)
            .await?;
        self.engine
            .step(deploy, "leak_check", StepStatus::Running)
            .await?;
        self.engine
            .step(deploy, "build", StepStatus::Running)
            .await?;
        // Project build code never runs unfenced unless the person chose that.
        if let Err(reason) = self.engine.builder.isolation()
            && !self.engine.repository.settings().await?.unisolated_builds
        {
            return Err(EngineError::Provider {
                evidence: None,
                code: "build.not_isolated".into(),
                message: format!(
                    "{reason} Nothing was built. To build without isolation anyway, turn on Build without isolation in Settings → Safety."
                ),
            });
        }
        let id = deploy.id.clone();
        let mut artifact = self
            .engine
            .builder
            .build_with_progress(
                &plan,
                &id,
                secrets,
                &mut DeploymentBuildProgress {
                    repository: self.engine.repository.as_ref(),
                    deploy,
                },
            )
            .await?;
        if artifact.provenance.is_some() {
            deploy.provenance = artifact.provenance.clone();
        }
        self.engine
            .step(deploy, "build", StepStatus::Succeeded)
            .await?;
        self.engine
            .step(deploy, "leak_check", StepStatus::Succeeded)
            .await?;
        let has_database = plan
            .resources
            .iter()
            .any(|resource| resource.kind == ResourceKind::Database);
        self.engine
            .step(
                deploy,
                "provision",
                if has_database {
                    StepStatus::Running
                } else {
                    StepStatus::Skipped
                },
            )
            .await?;
        if plan.prisma_migration.is_none() {
            self.engine
                .step(deploy, "migrate", StepStatus::Skipped)
                .await?;
        }
        let mut database_resource = None;
        for index in order {
            self.engine.check_deploy_cancelled(&deploy.id).await?;
            // Checked again right before every change at a provider: the
            // person may have stopped Hoista managing the project.
            if self
                .engine
                .repository
                .project(&project.id)
                .await?
                .is_none_or(|value| value.mode != Mode::Managed)
            {
                return Err(EngineError::Watching);
            }
            let planned = &plan.resources[index];
            if planned.kind != ResourceKind::Database {
                self.engine
                    .step(deploy, "release", StepStatus::Running)
                    .await?;
            }
            let account = self
                .engine
                .repository
                .account(&planned.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let token = self.engine.accounts().account_token(&account.id).await?;
            let adapter = self.engine.adapter(planned.provider)?;
            if deploy.started_by != "person" {
                let actor = Actor::Agent {
                    id: deploy.started_by.clone(),
                };
                self.engine.authorize(&actor, "apps:write").await?;
                if planned.provider_ref.is_none() {
                    self.engine.authorize(&actor, "apps:create").await?;
                }
            }
            let release = adapter
                .deploy(&token, &account.external_id, planned, &artifact)
                .await?;
            let previous = self
                .engine
                .projects()
                .all_resources()
                .await?
                .into_iter()
                .find(|resource| {
                    resource.project_id.as_deref() == Some(&project.id)
                        && resource.name == planned.name
                        && resource.account_id == account.id
                });
            let resource = Resource {
                id: previous
                    .map(|resource| resource.id)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                project_id: Some(project.id.clone()),
                kind: planned.kind,
                provider: planned.provider,
                account_id: account.id.clone(),
                provider_ref: release.reference.clone(),
                name: planned.name.clone(),
                region: planned.region.clone(),
                status: ResourceStatus::Unknown,
                mode: Mode::Managed,
                url: release.url.clone(),
                group: None,
            };
            self.engine.repository.put_resource(&resource).await?;
            deploy.releases.push(ResourceRelease {
                resource_id: resource.id.clone(),
                name: resource.name.clone(),
                release: release.clone(),
            });
            self.engine.repository.put_deploy(deploy).await?;
            if planned.kind == ResourceKind::Database {
                let connection = adapter
                    .connection_string(&token, &account.external_id, &release.reference)
                    .await?;
                artifact.secrets.retain(|(name, _)| name != "DATABASE_URL");
                artifact.secrets.push(("DATABASE_URL".into(), connection));
                database_resource = Some(resource.id.clone());
                self.engine
                    .step(deploy, "provision", StepStatus::Succeeded)
                    .await?;
                if plan.prisma_migration.is_some() {
                    self.engine
                        .step(deploy, "migrate", StepStatus::Running)
                        .await?;
                    let direct = adapter
                        .migration_connection_string(
                            &token,
                            &account.external_id,
                            &release.reference,
                        )
                        .await?;
                    self.engine
                        .builder
                        .migrate(&plan, &deploy.id, &direct)
                        .await?;
                    // Migration credentials are never included in the runtime artifact.
                    self.engine
                        .step(deploy, "migrate", StepStatus::Succeeded)
                        .await?;
                }
            } else {
                let managed_database = plan
                    .resources
                    .iter()
                    .find(|resource| resource.kind == ResourceKind::Database)
                    .map(|resource| resource.provider);
                record_secrets(
                    self.engine,
                    deploy,
                    &resource.id,
                    &artifact,
                    &public_inputs,
                    managed_database,
                    &generated_inputs,
                )
                .await?;
                if let Some(database) = &database_resource {
                    self.engine
                        .repository
                        .put_link(&Link {
                            id: format!("database:{}", resource.id),
                            from_resource: resource.id.clone(),
                            to_resource: database.clone(),
                            via: "DATABASE_URL".into(),
                            derived: true,
                            missing: false,
                        })
                        .await?;
                }
            }
            drop(token);
        }
        self.engine
            .step(deploy, "release", StepStatus::Succeeded)
            .await?;
        self.engine
            .step(deploy, "verify", StepStatus::Running)
            .await?;
        for released in &deploy.releases {
            let mut resource = self
                .engine
                .repository
                .resource(&released.resource_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let account = self
                .engine
                .repository
                .account(&resource.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let token = self.engine.accounts().account_token(&account.id).await?;
            self.engine
                .adapter(resource.provider)?
                .health(&token, &account.external_id, &released.release)
                .await?;
            resource.status = ResourceStatus::Live;
            self.engine.repository.put_resource(&resource).await?;
            if released.release.url.is_some() {
                deploy.url = released.release.url.clone();
            }
        }
        self.engine
            .step(deploy, "verify", StepStatus::Succeeded)
            .await?;
        deploy.status = if deploy.rollback_of.is_some() {
            DeployStatus::RolledBack
        } else {
            DeployStatus::Live
        };
        deploy.finished_at = Some(now());
        deploy.duration_ms = i64::try_from(started.elapsed().as_millis()).ok();
        self.engine.repository.put_deploy(deploy).await?;
        if self.engine.builder.cleanup(&deploy.id).await.is_err() {
            tracing::warn!(code = "build.cleanup_failed");
        }
        Ok(())
    }

    async fn rollback_after_failure(self, failed: &Deploy) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        if !failed.steps.iter().any(|step| {
            step.name == "release"
                && matches!(step.status, StepStatus::Running | StepStatus::Succeeded)
        }) {
            return Ok::<_, EngineError>(());
        }
        let plan = self
            .engine
            .repository
            .plan(&failed.plan_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let mut cursor = None;
        let mut previous = None;
        for _ in 0..MAX_ROLLBACK_PAGES {
            let page = self
                .engine
                .repository
                .deploys(&failed.project_id, cursor.as_deref())
                .await?;
            previous = page.items.into_iter().find(|value| {
                value.id != failed.id
                    && matches!(value.status, DeployStatus::Live | DeployStatus::RolledBack)
            });
            cursor = page.next_cursor;
            if previous.is_some() || cursor.is_none() {
                break;
            }
        }
        let Some(previous) = previous else {
            if cursor.is_some() {
                return Err(EngineError::Conflict(
                    "Recovery history exceeds the supported limit.",
                ));
            }
            return Ok(());
        };
        let mut recovery_failed = false;
        for released in previous.releases.iter().rev() {
            if !plan
                .resources
                .iter()
                .any(|planned| planned.name == released.name)
            {
                continue;
            }
            if let Err(error) = self.engine.deploys().restore_release(released).await {
                recovery_failed = true;
                self.engine
                    .repository
                    .event(
                        "engine",
                        Some(&failed.project_id),
                        "deploy.resource_recovery_failed",
                        error.code(),
                    )
                    .await?;
            }
        }
        if recovery_failed {
            return Err(EngineError::Conflict(
                "One or more resources could not be restored.",
            ));
        }
        self.engine
            .repository
            .event(
                "engine",
                Some(&failed.project_id),
                "deploy.recovered",
                "Restored previous releases where the provider supports rollback.",
            )
            .await
    }

    async fn restore_release(self, released: &ResourceRelease) -> Result<(), EngineError> {
        let mut resource = self
            .engine
            .repository
            .resource(&released.resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        // Database contents are never reverted as part of a code rollback.
        if resource.kind == ResourceKind::Database {
            return Ok(());
        }
        if resource.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        let adapter = self.engine.adapter(resource.provider)?;
        if !adapter.capabilities().rollback {
            return Err(crate::ports::providers::unsupported());
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let restored = adapter
            .rollback(&token, &account.external_id, &released.release)
            .await?;
        adapter
            .health(&token, &account.external_id, &restored)
            .await?;
        resource.status = ResourceStatus::Live;
        resource.url = restored.url;
        self.engine.repository.put_resource(&resource).await
    }
}

/// Records which variables a resource was given and their fingerprints,
/// never their values.
async fn record_secrets(
    engine: &Engine,
    deploy: &Deploy,
    resource_id: &str,
    artifact: &Artifact,
    public_inputs: &[(String, String)],
    managed_database: Option<crate::Provider>,
    generated_inputs: &BTreeSet<String>,
) -> Result<(), EngineError> {
    let mut values: BTreeMap<String, String> = artifact
        .secrets
        .iter()
        .map(|(name, value)| (name.clone(), fingerprint(value.expose())))
        .collect();
    values.extend(public_inputs.iter().cloned());
    for (name, fingerprint) in values {
        let managed_by = if name == "DATABASE_URL" {
            managed_database
        } else {
            None
        };
        let metadata = SecretMetadata {
            requires_source_sync: deploy.environment_keys.contains(&name),
            set_by: Some(
                if managed_by.is_some() || generated_inputs.contains(&name) {
                    "hoista".into()
                } else {
                    deploy.started_by.clone()
                },
            ),
            managed_by,
            project_id: deploy.project_id.clone(),
            resource_id: resource_id.into(),
            name,
            fingerprint,
            set_at: now(),
        };
        engine.repository.put_secret(&metadata).await?;
    }
    Ok(())
}

/// Points the plan at resources already recorded in this project. A queued
/// plan can be older than a deploy queued before it, so this happens when it
/// runs. A name Hoista doesn't know is never matched to something at the
/// provider.
fn resolve_queued_resources(
    plan: &mut Plan,
    project_id: &str,
    resources: &[Resource],
) -> Result<(), EngineError> {
    for planned in &mut plan.resources {
        let existing = resources.iter().find(|resource| {
            resource.project_id.as_deref() == Some(project_id)
                && resource.name == planned.name
                && resource.account_id == planned.account_id
        });
        match existing {
            Some(resource) => {
                if resource.mode != Mode::Managed {
                    return Err(EngineError::Watching);
                }
                if resource.provider != planned.provider
                    || resource.kind != planned.kind
                    || planned
                        .provider_ref
                        .as_ref()
                        .is_some_and(|reference| reference != &resource.provider_ref)
                {
                    return Err(EngineError::Conflict(
                        "The planned resource identity changed. Create a fresh plan.",
                    ));
                }
                planned.provider_ref = Some(resource.provider_ref.clone());
            }
            None if planned.provider_ref.is_some() => {
                return Err(EngineError::Conflict(
                    "The planned resource is no longer owned by this project. Create a fresh plan.",
                ));
            }
            None => {}
        }
    }
    Ok(())
}
