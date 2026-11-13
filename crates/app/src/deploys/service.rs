//! The deploys service: plans, deploys and the provider's own operations.

use crate::engine::*;
use crate::ports::catalog::EngineError;
use crate::{Provider, domain::model::*};

const MAX_MESSAGE_BYTES: usize = 200;

/// Plans, deploys and the provider's own operations.
#[derive(Clone, Copy)]
pub struct DeployService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Plans, deploys and the provider's own operations.
    #[must_use]
    pub fn deploys(&self) -> DeployService<'_> {
        DeployService { engine: self }
    }
}

impl<'a> DeployService<'a> {
    /// Inspects the source and saves an editable deploy plan. Nothing changes at any provider.
    ///
    /// # Errors
    /// Reports unsupported source, disconnected accounts or invalid planning choices.
    pub async fn plan(
        self,
        actor: &Actor,
        source: Source,
        name: &str,
        pinned: Option<Provider>,
        existing: Option<Provider>,
    ) -> Result<Plan, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        let plan = self
            .engine
            .plan_unsaved(source, name, pinned, existing)
            .await?;
        self.engine.save_plan(actor, plan).await
    }

    /// Uploads the project's linked folder (or a folder inside it) to a watched
    /// resource's provider and deploys it, as the provider's own CLI does. Only the
    /// folder the person linked to the project can be sent.
    ///
    /// # Errors
    /// Rejects unauthorized callers, projects without a linked folder, paths that
    /// leave that folder, managed resources and provider failures.
    pub async fn provider_upload(
        self,
        actor: &Actor,
        resource_id: &str,
        subfolder: Option<&str>,
        message: Option<&str>,
    ) -> Result<String, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        if message.is_some_and(|text| {
            text.len() > MAX_MESSAGE_BYTES || text.chars().any(char::is_control)
        }) {
            return Err(EngineError::Invalid(
                "Keep the deploy message to one line of 200 characters.",
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
                "Hoista manages this resource; deploy it with a plan instead.",
            ));
        }
        let project = self
            .engine
            .repository
            .project(
                resource
                    .project_id
                    .as_deref()
                    .ok_or(EngineError::NotFound)?,
            )
            .await?
            .ok_or(EngineError::NotFound)?;
        let Source::Folder { path } = &project.source else {
            return Err(EngineError::Conflict(
                "Link this project to its folder in Hoista (Settings → Code) before deploying from it.",
            ));
        };
        let root = std::path::PathBuf::from(path);
        let folder = match subfolder.filter(|sub| !sub.is_empty() && *sub != ".") {
            Some(sub) => {
                let sub = std::path::Path::new(sub);
                if sub.is_absolute()
                    || sub
                        .components()
                        .any(|resource| !matches!(resource, std::path::Component::Normal(_)))
                {
                    return Err(EngineError::Invalid(
                        "Give the subfolder relative to the project's folder, such as backend.",
                    ));
                }
                root.join(sub)
            }
            None => root.clone(),
        };
        // A symlink inside the folder must not lead outside it.
        let real = std::fs::canonicalize(&folder)
            .map_err(|_| EngineError::Invalid("That folder doesn't exist on this computer."))?;
        let real_root = std::fs::canonicalize(&root)
            .map_err(|_| EngineError::Invalid("The project's linked folder no longer exists."))?;
        if !real.starts_with(&real_root) {
            return Err(EngineError::Invalid(
                "That folder is outside the project's folder.",
            ));
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let done = self
            .engine
            .adapter(resource.provider)?
            .provider_upload(
                &token,
                &account.external_id,
                &resource.provider_ref,
                &real,
                message,
            )
            .await?;
        let summary = format!("{done} on {}.", resource.name);
        if let Some(mut current) = self.engine.repository.resource(&resource.id).await? {
            current.status = ResourceStatus::Deploying;
            self.engine.repository.put_resource(&current).await?;
        }
        self.engine
            .repository
            .event(
                actor.id(),
                resource.project_id.as_deref(),
                "provider.action",
                &summary,
            )
            .await?;
        Ok(summary)
    }

    /// Asks a watched resource's provider to run one action (redeploy, restart,
    /// roll back and the rest), and records it in the project's activity.
    ///
    /// # Errors
    /// Rejects unauthorized callers, managed resources (deploy those with plans),
    /// unsupported providers and provider refusals.
    pub async fn provider_action(
        self,
        actor: &Actor,
        resource_id: &str,
        action: crate::ports::providers::ProviderAction<'_>,
    ) -> Result<String, EngineError> {
        self.engine
            .deploys()
            .provider_action_approved(actor, resource_id, action, None)
            .await
    }

    /// [`Engine::provider_action`], with the approval an agent got for a
    /// destructive action (see [`Engine::provider_action_gate`]).
    ///
    /// # Errors
    /// As [`Engine::provider_action`], plus a missing or unmatched approval.
    pub async fn provider_action_approved(
        self,
        actor: &Actor,
        resource_id: &str,
        action: crate::ports::providers::ProviderAction<'_>,
        approval: Option<&str>,
    ) -> Result<String, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        if let Some((gated, target)) = Engine::provider_action_gate(resource_id, &action) {
            self.engine.gate(actor, gated, &target, approval).await?;
        }
        if let crate::ports::providers::ProviderAction::Rollback { deployment_id } = action
            && (deployment_id.is_empty()
                || deployment_id.len() > 128
                || !deployment_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        {
            return Err(EngineError::Invalid(
                "Name the deployment to roll back to by its id.",
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
                "Hoista manages this resource; deploy or roll it back with a plan instead.",
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
        // Removing it: the provider's own delete, where it has one; otherwise
        // the provider's action of that name (which may work per environment).
        let summary = if action == crate::ports::providers::ProviderAction::RemoveService
            && adapter.capabilities().delete
            && !adapter.removes_services_by_action()
        {
            adapter
                .delete(&token, &account.external_id, &resource.provider_ref)
                .await?;
            format!(
                "Deleted {} at {}.",
                resource.name,
                resource.provider.display_name()
            )
        } else {
            let done = adapter
                .provider_action(
                    &token,
                    &account.external_id,
                    &resource.provider_ref,
                    &action,
                )
                .await?;
            format!("{done} on {}.", resource.name)
        };
        // Say what's happening now rather than wait for the next check: the
        // monitor looks again within a minute and records how it ended.
        let now = match &action {
            crate::ports::providers::ProviderAction::Redeploy
            | crate::ports::providers::ProviderAction::Restart
            | crate::ports::providers::ProviderAction::Rollback { .. }
            | crate::ports::providers::ProviderAction::DeployLatest
            | crate::ports::providers::ProviderAction::DeployCommit { .. } => {
                Some(ResourceStatus::Deploying)
            }
            crate::ports::providers::ProviderAction::Stop => Some(ResourceStatus::Stopped),
            _ => None,
        };
        if let Some(status) = now
            && let Some(mut current) = self.engine.repository.resource(&resource.id).await?
        {
            current.status = status;
            self.engine.repository.put_resource(&current).await?;
        }
        if action == crate::ports::providers::ProviderAction::RemoveService {
            // Gone at the provider, so gone here. A watched project mirrors one
            // at the provider, so left empty it goes too; a project made in
            // Hoista stays, empty, with its id, for what's deployed next.
            self.engine.repository.remove_resource(&resource.id).await?;
            if let Some(project_id) = resource.project_id.as_deref()
                && self
                    .engine
                    .repository
                    .project(project_id)
                    .await?
                    .is_some_and(|project| project.mode == Mode::Watching)
                && !self
                    .engine
                    .projects()
                    .all_resources()
                    .await?
                    .iter()
                    .any(|other| other.project_id.as_deref() == Some(project_id))
            {
                self.engine.repository.remove_project(project_id).await?;
            }
        }
        self.engine
            .repository
            .event(
                actor.id(),
                resource.project_id.as_deref(),
                "provider.action",
                &summary,
            )
            .await?;
        Ok(summary)
    }

    /// Creates a service, a template such as a database, or a bucket next to
    /// a watched project's resources at their provider, and adds what it created to
    /// the project.
    ///
    /// # Errors
    /// Rejects unauthorized callers, projects with no watched resource at a provider
    /// that supports it, invalid input and provider failures.
    pub async fn provider_create(
        self,
        actor: &Actor,
        project_id: &str,
        what: crate::ports::providers::ProviderCreate<'_>,
    ) -> Result<(String, Vec<Resource>), EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        // The project's watched resources at providers that make things next to
        // one another; the first provider that takes this kind does it, so a
        // project with pieces on several providers creates at the right one.
        let candidates: Vec<Resource> = self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| {
                resource.project_id.as_deref() == Some(project_id)
                    && resource.mode == Mode::Watching
                    && self.engine.adapter(resource.provider).is_ok_and(|adapter| {
                        let can = adapter.capabilities();
                        can.new_project
                            && (can.new_database || can.new_bucket || can.new_from_image)
                    })
            })
            .collect();
        let mut tried = std::collections::BTreeSet::new();
        let mut made = None;
        for beside in candidates {
            if !tried.insert(beside.account_id.clone()) {
                continue;
            }
            let Some(account) = self.engine.repository.account(&beside.account_id).await? else {
                continue;
            };
            let token = self.engine.accounts().account_token(&account.id).await?;
            let result = self
                .engine
                .adapter(beside.provider)?
                .provider_create(&token, &account.external_id, &beside.provider_ref, &what)
                .await;
            drop(token);
            match result {
                Err(EngineError::Provider { ref code, .. }) if code == "provider.not_supported" => {
                }
                result => {
                    made = Some((account, result?));
                    break;
                }
            }
        }
        let (account, (references, done)) = made.ok_or(EngineError::Conflict(
            "Nothing in this project is at a provider that makes services next to each other. Use add_to_project, naming the account.",
        ))?;
        let mut added = Vec::new();
        if !references.is_empty() {
            let _mutation = self.engine.mutation.lock().await;
            self.engine.discover_locked(actor, &account.id).await?;
            for mut resource in self.engine.projects().all_resources().await? {
                if resource.account_id == account.id
                    && references.contains(&resource.provider_ref)
                    && resource.project_id.is_none()
                {
                    resource.project_id = Some(project_id.to_owned());
                    self.engine.repository.put_resource(&resource).await?;
                    added.push(resource);
                }
            }
        }
        let summary = format!("{done}.");
        self.engine
            .repository
            .event(actor.id(), Some(project_id), "provider.action", &summary)
            .await?;
        Ok((summary, added))
    }

    /// Creates a new project at a provider that builds and runs it: a provider
    /// project of its own with one service deploying the repository, and a
    /// public address. The provider starts the first deploy; Hoista watches the
    /// result like any project on that provider.
    ///
    /// # Errors
    /// Rejects callers without write and create access, invalid or taken
    /// names, accounts that aren't connected, providers that can't do this and
    /// provider failures.
    pub async fn new_project_at_provider(
        self,
        actor: &Actor,
        account_id: &str,
        name: String,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<(ImportedProject, String), EngineError> {
        let account = self
            .engine
            .new_project_account(actor, account_id, &name)
            .await?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let made = self
            .engine
            .adapter(account.provider)?
            .provider_new_project(&token, &account.external_id, &name, repo, branch)
            .await;
        drop(token);
        let (references, done) = made?;
        self.engine
            .adopt_new_project(actor, &account, name, None, &references, &done)
            .await
    }

    /// Creates a new project at a provider from a folder on this computer, with
    /// the provider's own command-line tool (Modal's `modal deploy`), and links
    /// the project to that folder. Hoista watches the result.
    ///
    /// # Errors
    /// As [`Self::new_project_at_provider`], and a folder that isn't on this
    /// computer.
    pub async fn new_project_from_folder(
        self,
        actor: &Actor,
        account_id: &str,
        name: String,
        folder: &str,
    ) -> Result<(ImportedProject, String), EngineError> {
        let account = self
            .engine
            .new_project_account(actor, account_id, &name)
            .await?;
        let path = std::path::Path::new(folder);
        if !path.is_absolute() {
            return Err(EngineError::Invalid(
                "Give the folder as a full path on this computer.",
            ));
        }
        let real = std::fs::canonicalize(path)
            .ok()
            .filter(|real| real.is_dir())
            .ok_or(EngineError::Invalid(
                "That folder doesn't exist on this computer.",
            ))?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let made = self
            .engine
            .adapter(account.provider)?
            .provider_new_from_folder(&token, &account.external_id, &name, &real)
            .await;
        drop(token);
        let (references, done) = made?;
        let source = Source::Folder {
            path: real.to_string_lossy().into_owned(),
        };
        self.engine
            .adopt_new_project(actor, &account, name, Some(source), &references, &done)
            .await
    }

    /// Deploys an empty project's linked folder as a new project at the provider,
    /// with the provider's own command-line tool, under the project's name. The
    /// project then watches what the provider made, like an imported one.
    ///
    /// # Errors
    /// Rejects callers without write and create access, projects that
    /// aren't linked to a folder or already have resources, accounts that
    /// aren't connected, providers that can't do this and provider failures.
    pub async fn deploy_project_folder(
        self,
        actor: &Actor,
        account_id: &str,
        project_id: &str,
    ) -> Result<(ImportedProject, String), EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.authorize(actor, "apps:create").await?;
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let Source::Folder { path } = &project.source else {
            return Err(EngineError::Conflict(
                "Link this project to its folder in Hoista (Settings → Code) first.",
            ));
        };
        if self
            .engine
            .projects()
            .all_resources()
            .await?
            .iter()
            .any(|resource| resource.project_id.as_deref() == Some(project_id))
        {
            return Err(EngineError::Conflict(
                "This project already has resources. Deploy its folder to one of them with deploy_folder.",
            ));
        }
        let real = std::fs::canonicalize(path)
            .ok()
            .filter(|real| real.is_dir())
            .ok_or(EngineError::Invalid(
                "The project's linked folder no longer exists.",
            ))?;
        let account = self
            .engine
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if account.status != AccountStatus::Connected {
            return Err(EngineError::Conflict(
                "This account isn't connected. Reconnect it in Hoista first.",
            ));
        }
        let token = self.engine.accounts().account_token(&account.id).await?;
        let made = self
            .engine
            .adapter(account.provider)?
            .provider_new_from_folder(&token, &account.external_id, &project.name, &real)
            .await;
        drop(token);
        let (references, done) = made?;
        let resource_ids = self
            .engine
            .made_resources(actor, &account, &references)
            .await?;
        // The provider builds and runs it now, so Hoista watches it.
        let project = if project.mode == Mode::Watching {
            project
        } else {
            self.engine
                .repository
                .set_project_mode(project_id, Mode::Watching)
                .await?
        };
        let resources = self
            .engine
            .projects()
            .group(actor, project_id, &resource_ids)
            .await?;
        self.engine.mark_deploying(&resource_ids).await?;
        let summary = format!("{done}.");
        self.engine
            .repository
            .event(actor.id(), Some(project_id), "provider.action", &summary)
            .await?;
        Ok((ImportedProject { project, resources }, summary))
    }
}
