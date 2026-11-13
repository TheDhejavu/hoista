//! Changing a project's variables, deployed through the normal deploy
//! pipeline. Values are only ever held in memory.

use crate::engine::{Actor, Engine};
use crate::environments::{EnvironmentService, admission_keys};
use crate::{
    domain::{
        Secret,
        environment::{
            PublicEnvironmentPrefill, PublicEnvironmentRequest, PublicEnvironmentUnavailable,
            PublicEnvironmentUnavailableReason, PublicEnvironmentValue,
        },
        leaks::LeakScanner,
        model::*,
    },
    ports::{
        catalog::{DeploymentAdmission, EngineError},
        providers::remove_variable,
    },
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, time::Duration};

const MAX_CHANGES: usize = 64;
const MAX_VALUE_BYTES: usize = 16 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024;
/// Changes waiting for the deploy worker, across all projects.
const MAX_QUEUED_CHANGES: usize = 100;
const MAX_PUBLIC_NAMES: usize = 256;
const PREFILL_TIMEOUT: Duration = Duration::from_secs(30);

impl Engine {
    /// Refuses a change that has nothing to run it, or that sets a variable
    /// Hoista manages itself (a database address, or a link's binding).
    pub(crate) async fn validate_environment_targets(
        &self,
        project_id: &str,
        plan: &Plan,
        names: &[String],
    ) -> Result<(), EngineError> {
        if !plan
            .resources
            .iter()
            .any(|resource| resource.kind != ResourceKind::Database)
        {
            return Err(EngineError::Invalid(
                "An environment deployment needs a runtime resource.",
            ));
        }
        let metadata = self.repository.secrets(project_id).await?;
        let links = self.repository.links(project_id).await?;
        let database = plan
            .resources
            .iter()
            .any(|resource| resource.kind == ResourceKind::Database);
        if names.iter().any(|name| {
            (database && matches!(name.as_str(), "DATABASE_URL" | "DIRECT_URL"))
                || metadata
                    .iter()
                    .any(|value| value.name == *name && value.managed_by.is_some())
                || links.iter().any(|link| link.via == *name && link.derived)
        }) {
            return Err(EngineError::Invalid(
                "A requested binding is managed by Hoista. Change its resource connection instead.",
            ));
        }
        Ok(())
    }
}

impl<'a> EnvironmentService<'a> {
    /// Queues new values for the project's variables, deployed with `plan_id`.
    /// The values stay in this process's memory until the deploy uses them.
    ///
    /// # Errors
    /// Fails without access, for bad names or values, a secret in a variable
    /// the browser can see, a variable Hoista manages, a plan with nothing to
    /// run it, or a full queue.
    pub async fn replace_environment(
        self,
        actor: &Actor,
        project_id: &str,
        plan_id: &str,
        values: Vec<(String, Secret)>,
        mut options: DeployOptions,
    ) -> Result<Deploy, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let names = check_values(&values)?;
        let admission = match options.request_key.take() {
            Some(request_key) => {
                let receipt = self
                    .admission(actor, project_id, plan_id, &request_key, &options, &values)
                    .await?;
                if let Some(existing) = self.engine.repository.admitted_deploy(&receipt).await? {
                    // A retry: the first attempt's values and run stand as they are.
                    return Ok(existing);
                }
                Some(receipt)
            }
            None => None,
        };
        let plan = self
            .engine
            .repository
            .plan(plan_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        self.engine
            .validate_environment_targets(project_id, &plan, &names)
            .await?;
        if self.engine.environment_inputs.lock().await.len() >= MAX_QUEUED_CHANGES {
            return Err(EngineError::Conflict(
                "The environment deployment queue is full.",
            ));
        }
        let deploy = self
            .engine
            .enqueue_deploy_locked(actor, project_id, plan_id, options, names, admission)
            .await?;
        self.engine
            .environment_inputs
            .lock()
            .await
            .insert(deploy.id.clone(), values);
        Ok(deploy)
    }

    /// The identity a retried change is recognised by: the request key, and
    /// a keyed fingerprint of everything the person reviewed, values included.
    async fn admission(
        self,
        actor: &Actor,
        project_id: &str,
        plan_id: &str,
        request_key: &str,
        options: &DeployOptions,
        values: &[(String, Secret)],
    ) -> Result<DeploymentAdmission, EngineError> {
        if options
            .expected_plan_revision
            .is_none_or(|revision| !(0..=MAX_SAFE_INTEGER).contains(&revision))
        {
            return Err(EngineError::Invalid(
                "A keyed environment deployment requires its reviewed plan revision.",
            ));
        }
        let request_key = DeploymentAdmission::request_key(actor.id(), request_key)?;
        let (title, channel, _) = self.engine.deploy_metadata(actor, options.clone()).await?;
        let metadata = serde_json::to_vec(&serde_json::json!([
            "environment-admission-v1",
            actor.id(),
            request_key,
            project_id,
            plan_id,
            options.expected_plan_revision,
            title,
            channel,
        ]))
        .map_err(|_| EngineError::Store)?;
        let key = admission_keys::load(self.engine.repository.as_ref(), self.engine.vault.as_ref())
            .await?;
        let fingerprint = admission_keys::fingerprint(&key, &metadata, values)?;
        DeploymentAdmission::new(actor.id(), &request_key, &fingerprint)
    }
}

/// Checks the names and values of a change, and returns the names in order.
fn check_values(values: &[(String, Secret)]) -> Result<Vec<String>, EngineError> {
    if values.is_empty() || values.len() > MAX_CHANGES {
        return Err(EngineError::Invalid(
            "Replace between 1 and 64 project environment variables per deployment.",
        ));
    }
    let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
    let mut names = BTreeSet::new();
    let mut total = 0usize;
    for (name, value) in values {
        total = total
            .checked_add(value.expose().len())
            .ok_or(EngineError::Store)?;
        if !valid_env_name(name)
            || !names.insert(name.clone())
            || value.expose().len() > MAX_VALUE_BYTES
            || total > MAX_TOTAL_BYTES
            || value.expose().contains('\0')
        {
            return Err(EngineError::Invalid(
                "Use unique valid names, values up to 16 KiB without NUL, and at most 64 KiB total.",
            ));
        }
        if !scanner.public_env(name, value).is_empty() {
            return Err(EngineError::Invalid(
                "A browser-visible environment variable cannot contain a secret.",
            ));
        }
    }
    Ok(names.into_iter().collect())
}

impl<'a> EnvironmentService<'a> {
    /// Removes a secret from a resource at its provider. The source and any
    /// files already built stay as they are.
    ///
    /// # Errors
    /// Fails without access, for a bad name, a resource that's watched or in
    /// another project, a variable Hoista manages, a deploy still waiting, a
    /// provider that can't remove secrets, or when the provider call fails.
    pub async fn delete_secret(
        self,
        actor: &Actor,
        project_id: &str,
        resource_id: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        if !valid_env_name(name) {
            return Err(EngineError::Invalid(
                "Use a valid environment variable name.",
            ));
        }
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.project_id.as_deref() != Some(project_id) {
            return Err(EngineError::NotFound);
        }
        if project.mode != Mode::Managed || resource.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        if self
            .engine
            .repository
            .has_pending_deploy(project_id)
            .await?
        {
            return Err(EngineError::Conflict(
                "Wait for queued or active deployments before removing a hosted secret.",
            ));
        }
        if self
            .engine
            .repository
            .secrets(project_id)
            .await?
            .iter()
            .any(|secret| {
                secret.resource_id == resource_id
                    && secret.name == name
                    && secret.managed_by.is_some()
            })
            || self
                .engine
                .repository
                .links(project_id)
                .await?
                .iter()
                .any(|link| link.from_resource == resource_id && link.via == name && link.derived)
        {
            return Err(EngineError::Invalid(
                "This binding is managed by Hoista. Change its resource connection instead.",
            ));
        }
        let adapter = self.engine.adapter(resource.provider)?;
        if !adapter.capabilities().secrets || resource.kind == ResourceKind::Database {
            return Err(EngineError::Unsupported(format!(
                "Hoista can't remove secrets from this resource at {}.",
                resource.provider.display_name()
            )));
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                Some(project_id),
                "secret.delete_requested",
                &format!("Requested hosted secret removal: {name} on resource {resource_id}. Completion is not yet confirmed."),
            )
            .await?;
        remove_variable(
            adapter.as_ref(),
            &token,
            &account.external_id,
            &resource.provider_ref,
            name,
        )
        .await?;
        self.engine
            .repository
            .delete_secret_metadata(actor.id(), project_id, resource_id, name)
            .await
    }
}

impl<'a> EnvironmentService<'a> {
    /// The values of the browser-visible variables as the project's folder
    /// has them, to fill in for the person. Only the person can ask.
    ///
    /// # Errors
    /// Fails for an agent, a plan that isn't for this project's folder or
    /// has changed, a secret in one of the values, too much data, or after
    /// 30 seconds.
    pub async fn public_environment(
        self,
        actor: &Actor,
        project_id: &str,
        request: PublicEnvironmentRequest,
    ) -> Result<PublicEnvironmentPrefill, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Invalid(
                "Public source prefill is available only to the local owner.",
            ));
        }
        tokio::time::timeout(
            PREFILL_TIMEOUT,
            self.read_public_values(project_id, request),
        )
        .await
        .map_err(|_| {
            EngineError::Conflict("Public source inspection timed out. Wait for active work to finish, then inspect the source again.")
        })?
    }

    async fn read_public_values(
        self,
        project_id: &str,
        request: PublicEnvironmentRequest,
    ) -> Result<PublicEnvironmentPrefill, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let plan = self
            .engine
            .repository
            .plan(&request.plan_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if plan.revision != request.expected_plan_revision {
            return Err(EngineError::Conflict(
                "The plan changed. Refresh it before reading public source values.",
            ));
        }
        if !matches!(plan.source, Source::Folder { .. }) || !plan.source.within(&project.source) {
            return Err(EngineError::Invalid(
                "Public prefill requires a plan matching this project's linked local folder.",
            ));
        }
        let names: BTreeSet<_> = plan
            .env_names
            .iter()
            .filter(|name| browser_env(name))
            .cloned()
            .collect();
        if names.len() > MAX_PUBLIC_NAMES || names.iter().any(|name| !valid_env_name(name)) {
            return Err(EngineError::Invalid(
                "Invalid public environment names in the plan.",
            ));
        }
        let source = if names.is_empty() {
            Vec::new()
        } else {
            self.engine.builder.secrets(&plan).await?
        };
        let recorded_secrets = self.engine.repository.secrets(project_id).await?;
        let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
        let mut result = PublicEnvironmentPrefill {
            plan_id: plan.id,
            plan_revision: plan.revision,
            source_only: true,
            values: Vec::new(),
            unavailable: Vec::new(),
        };
        let mut total = 0usize;
        for name in names {
            let recorded: Vec<_> = recorded_secrets
                .iter()
                .filter(|secret| secret.name == name)
                .collect();
            let value = source
                .iter()
                .find(|(key, _)| key == &name)
                .map(|(_, value)| value);
            let reason = if recorded.iter().any(|secret| secret.managed_by.is_some()) {
                Some(PublicEnvironmentUnavailableReason::ManagedBinding)
            } else if let Some(value) = value {
                if value.expose().len() > MAX_VALUE_BYTES
                    || value.expose().contains('\0')
                    || !scanner.public_env(&name, value).is_empty()
                {
                    return Err(EngineError::Invalid(
                        "A public source value is oversized or contains a suspected server secret.",
                    ));
                }
                let fingerprint = format!("{:x}", Sha256::digest(value.expose().as_bytes()));
                if recorded
                    .iter()
                    .any(|secret| secret.fingerprint != fingerprint)
                {
                    Some(PublicEnvironmentUnavailableReason::SourceDiffersFromRecorded)
                } else {
                    total += value.expose().len();
                    if total > MAX_TOTAL_BYTES {
                        return Err(EngineError::Invalid(
                            "Public source values exceed the 64 KiB response limit.",
                        ));
                    }
                    result.values.push(PublicEnvironmentValue {
                        name: name.clone(),
                        value: value.expose().into(),
                        matches_recorded_value: !recorded.is_empty(),
                    });
                    None
                }
            } else {
                Some(PublicEnvironmentUnavailableReason::MissingFromSource)
            };
            if let Some(reason) = reason {
                result
                    .unavailable
                    .push(PublicEnvironmentUnavailable { name, reason });
            }
        }
        Ok(result)
    }
}
