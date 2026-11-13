//! The environments service: environments, variables, secrets and domains.

use crate::domain::{Secret, model::*};
use crate::engine::*;
use crate::ports::catalog::EngineError;
use sha2::{Digest, Sha256};

/// Variables set at the provider in one go.
const MAX_VARIABLES: usize = 200;
const MAX_VALUE_BYTES: usize = 32 * 1024;
const MAX_ENVIRONMENTS: usize = 20;

/// Environments, variables, secrets and domains.
#[derive(Clone, Copy)]
pub struct EnvironmentService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Environments, variables, secrets and domains.
    #[must_use]
    pub fn environments(&self) -> EnvironmentService<'_> {
        EnvironmentService { engine: self }
    }
}

impl<'a> EnvironmentService<'a> {
    /// Sets or deletes (`value` None) one variable of a watched resource at its
    /// provider, at the person's request. Only the name is recorded. Agents can't
    /// do this to watched resources: they ask the person to do it in Hoista.
    ///
    /// # Errors
    /// Rejects agents, invalid names, managed resources and provider failures.
    pub async fn set_provider_variable(
        self,
        actor: &Actor,
        resource_id: &str,
        name: &str,
        value: Option<Secret>,
    ) -> Result<(), EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || name.as_bytes()[0].is_ascii_digit()
        {
            return Err(EngineError::Invalid(
                "Use a variable name of letters, digits and underscores, not starting with a digit.",
            ));
        }
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.mode != Mode::Watching {
            return Err(EngineError::Conflict(
                "Set this project's secrets from its Environment tab; Hoista manages them.",
            ));
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let adapter = self.engine.adapter(resource.provider)?;
        let (kind, summary) = match &value {
            Some(value) => {
                crate::ports::providers::put_variable(
                    adapter.as_ref(),
                    &token,
                    &account.external_id,
                    &resource.provider_ref,
                    name,
                    value,
                )
                .await?;
                ("secret.set", format!("Set {name} on {}.", resource.name))
            }
            None => {
                crate::ports::providers::remove_variable(
                    adapter.as_ref(),
                    &token,
                    &account.external_id,
                    &resource.provider_ref,
                    name,
                )
                .await?;
                (
                    "secret.deleted",
                    format!("Deleted {name} from {}.", resource.name),
                )
            }
        };
        self.engine
            .repository
            .event(actor.id(), resource.project_id.as_deref(), kind, &summary)
            .await
    }

    /// Sets many variables of a watched resource at once, or its provider project's
    /// shared variables (`shared`). Only the names are recorded.
    ///
    /// # Errors
    /// Rejects invalid names, too many at once, managed resources and provider failures.
    pub async fn set_provider_variables(
        self,
        actor: &Actor,
        resource_id: &str,
        values: Vec<(String, Secret)>,
        shared: bool,
        redeploy: bool,
    ) -> Result<(), EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        if values.is_empty() || values.len() > MAX_VARIABLES {
            return Err(EngineError::Invalid("Set 1 to 200 variables at a time."));
        }
        for (name, value) in &values {
            if !variable_name_ok(name) {
                return Err(EngineError::Invalid(
                    "Use variable names of letters, digits and underscores, not starting with a digit.",
                ));
            }
            if value.expose().is_empty() || value.expose().len() > MAX_VALUE_BYTES {
                return Err(EngineError::Invalid(
                    "Variable values are 1 byte to 32 KiB.",
                ));
            }
        }
        let (resource, account) = self.engine.watched_resource(resource_id).await?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let adapter = self.engine.adapter(resource.provider)?;
        match adapter
            .set_provider_variables(
                &token,
                &account.external_id,
                &resource.provider_ref,
                &values,
                shared,
                redeploy,
            )
            .await
        {
            // A provider without a batch call takes them one at a time, as
            // variables or as secrets.
            Err(error) if crate::ports::providers::is_unsupported(&error) && !shared => {
                for (name, value) in &values {
                    crate::ports::providers::put_variable(
                        adapter.as_ref(),
                        &token,
                        &account.external_id,
                        &resource.provider_ref,
                        name,
                        value,
                    )
                    .await?;
                }
            }
            other => other?,
        }
        let mut names: Vec<&str> = values.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        let mut list = names.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        if names.len() > 8 {
            list.push_str(&format!(" and {} more", names.len() - 8));
        }
        let summary = if shared {
            format!("Set shared variables {list} for {}.", resource.name)
        } else {
            format!("Set {list} on {}.", resource.name)
        };
        self.engine
            .repository
            .event(
                actor.id(),
                resource.project_id.as_deref(),
                "secret.set",
                &summary,
            )
            .await
    }

    /// Deletes one of a watched resource's provider project's shared variables.
    ///
    /// # Errors
    /// Rejects invalid names, managed resources and provider failures.
    pub async fn delete_shared_variable(
        self,
        actor: &Actor,
        resource_id: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        if !variable_name_ok(name) {
            return Err(EngineError::Invalid(
                "Use a variable name of letters, digits and underscores, not starting with a digit.",
            ));
        }
        let (resource, account) = self.engine.watched_resource(resource_id).await?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .adapter(resource.provider)?
            .delete_shared_variable(&token, &account.external_id, &resource.provider_ref, name)
            .await?;
        self.engine
            .repository
            .event(
                actor.id(),
                resource.project_id.as_deref(),
                "secret.deleted",
                &format!(
                    "Deleted shared variable {name} from {}’s provider project.",
                    resource.name
                ),
            )
            .await
    }

    /// Every environment, `default` first.
    ///
    /// # Errors
    /// Reports unavailable local state.
    pub async fn list(self, actor: &Actor) -> Result<Vec<Environment>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        self.engine.repository.environments().await
    }

    /// Adds an empty environment. Nothing changes at any provider.
    ///
    /// # Errors
    /// Rejects invalid or taken names, and more than 20 environments.
    pub async fn create_environment(
        self,
        actor: &Actor,
        name: &str,
    ) -> Result<Environment, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.authorize(actor, "apps:create").await?;
        if !valid_slug(name) {
            return Err(EngineError::Invalid(
                "Use a lowercase environment name with letters, digits and hyphens.",
            ));
        }
        if self.engine.repository.environments().await?.len() >= MAX_ENVIRONMENTS {
            return Err(EngineError::Conflict(
                "Hoista keeps at most 20 environments.",
            ));
        }
        let environment = Environment {
            id: format!("env_{}", uuid::Uuid::new_v4().simple()),
            name: name.to_owned(),
            created_at: now(),
        };
        self.engine
            .repository
            .create_environment(actor.id(), &environment)
            .await?;
        Ok(environment)
    }

    /// Renames an environment.
    ///
    /// # Errors
    /// Rejects invalid or taken names and unknown environments.
    pub async fn rename_environment(
        self,
        actor: &Actor,
        id: &str,
        name: &str,
    ) -> Result<Environment, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        if !valid_slug(name) {
            return Err(EngineError::Invalid(
                "Use a lowercase environment name with letters, digits and hyphens.",
            ));
        }
        self.engine
            .repository
            .rename_environment(actor.id(), id, name)
            .await
    }

    /// Removes an empty environment other than `default`.
    ///
    /// # Errors
    /// Rejects `default`, environments with projects, and unknown ones.
    pub async fn remove_environment(self, actor: &Actor, id: &str) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        if matches!(actor, Actor::Agent { .. }) {
            return Err(EngineError::ApprovalRequired);
        }
        self.engine
            .repository
            .remove_environment(actor.id(), id)
            .await
    }

    /// Gives one service a key to another: makes the key at the provider it
    /// unlocks (a Resend key that can only send from that domain) and puts
    /// it straight into the other service's variables. The key is never
    /// kept, logged or returned; only the variable name is.
    ///
    /// # Errors
    /// Rejects a provider that can't make keys, an invalid name, missing
    /// resources, and the provider's own refusals.
    pub async fn give_key(
        self,
        actor: &Actor,
        from_resource: &str,
        to_resource: &str,
        variable: Option<&str>,
    ) -> Result<String, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        let from = self
            .engine
            .repository
            .resource(from_resource)
            .await?
            .ok_or(EngineError::NotFound)?;
        let to = self
            .engine
            .repository
            .resource(to_resource)
            .await?
            .ok_or(EngineError::NotFound)?;
        let adapter = self.engine.adapter(from.provider)?.clone();
        let name = variable
            .map(str::to_owned)
            .or_else(|| adapter.key_variable().map(str::to_owned))
            .ok_or(EngineError::Invalid("Name the variable the key goes in."))?;
        if !valid_env_name(&name) {
            return Err(EngineError::Invalid(
                "Use a variable name of letters, digits and underscores.",
            ));
        }
        let account = self
            .engine
            .repository
            .account(&from.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let key = adapter
            .issue_key(
                &token,
                &account.external_id,
                &from.provider_ref,
                &format!("hoista {} for {}", from.name, to.name),
            )
            .await?;
        drop(token);
        match (to.mode, to.project_id.as_deref()) {
            (Mode::Managed, Some(project)) => {
                self.engine
                    .environments()
                    .set_secret(actor, project, &to.id, &name, key)
                    .await?;
            }
            _ => {
                self.engine
                    .environments()
                    .set_provider_variable(actor, &to.id, &name, Some(key))
                    .await?;
            }
        }
        self.engine
            .repository
            .event(
                actor.id(),
                to.project_id.as_deref(),
                "secret.changed",
                &format!(
                    "Made a {} key for {} and set it as {name} on {}; the key wasn't kept.",
                    from.provider.display_name(),
                    from.name,
                    to.name
                ),
            )
            .await?;
        Ok(name)
    }

    /// Writes a project secret to its provider only; Hoista keeps just its name and fingerprint.
    ///
    /// # Errors
    /// Rejects watched-only resources, invalid names, or provider or keychain errors.
    pub async fn set_secret(
        self,
        actor: &Actor,
        project_id: &str,
        resource_id: &str,
        name: &str,
        value: Secret,
    ) -> Result<SecretMetadata, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
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
        if project.mode != Mode::Managed || resource.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        if resource.project_id.as_deref() != Some(project_id) || !valid_env_name(name) {
            return Err(EngineError::Invalid(
                "Choose a resource belonging to this project and a valid environment name.",
            ));
        }
        let managed = self
            .engine
            .repository
            .secrets(project_id)
            .await?
            .iter()
            .any(|metadata| {
                metadata.resource_id == resource_id
                    && metadata.name == name
                    && metadata.managed_by.is_some()
            });
        let derived = self
            .engine
            .repository
            .links(project_id)
            .await?
            .iter()
            .any(|link| link.from_resource == resource_id && link.via == name && link.derived);
        if managed || derived {
            return Err(EngineError::Invalid(
                "This binding is managed by Hoista. Change its resource connection instead.",
            ));
        }
        if !crate::domain::leaks::LeakScanner::new()
            .map_err(|_| EngineError::Store)?
            .public_env(name, &value)
            .is_empty()
        {
            return Err(EngineError::Invalid(
                "A browser-visible environment variable cannot contain a secret.",
            ));
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let adapter = self.engine.adapter(resource.provider)?;
        match adapter
            .set_secret(
                &token,
                &account.external_id,
                &resource.provider_ref,
                name,
                &value,
            )
            .await
        {
            // Providers that keep variables on the service rather than as secrets.
            Err(error) if crate::ports::providers::is_unsupported(&error) => {
                adapter
                    .set_provider_variable(
                        &token,
                        &account.external_id,
                        &resource.provider_ref,
                        name,
                        &value,
                    )
                    .await?;
            }
            other => other?,
        }
        let metadata = SecretMetadata {
            requires_source_sync: false,
            set_by: Some(actor.id().into()),
            managed_by: None,
            project_id: project_id.into(),
            resource_id: resource_id.into(),
            name: name.into(),
            fingerprint: format!("{:x}", Sha256::digest(value.expose().as_bytes())),
            set_at: now(),
        };
        drop(value);
        drop(token);
        self.engine.repository.put_secret(&metadata).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                Some(project_id),
                "secret.changed",
                "Changed a provider secret without retaining its value.",
            )
            .await?;
        Ok(metadata)
    }
}
