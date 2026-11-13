//! The projects service: projects and their resources: creating, importing, linking sources, the map, moving, deleting.

use crate::domain::model::*;
use crate::engine::*;
use crate::ports::catalog::EngineError;

/// The most resources Hoista keeps across every account.
/// A commit id, short or full (SHA-1 or SHA-256).
const COMMIT_ID_CHARS: std::ops::RangeInclusive<usize> = 7..=64;
const MAX_MOVED_RESOURCES: usize = 100;

/// Projects and their resources: creating, importing, linking sources, the map, moving, deleting.
#[derive(Clone, Copy)]
pub struct ProjectService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Projects and their resources: creating, importing, linking sources, the map, moving, deleting.
    #[must_use]
    pub fn projects(&self) -> ProjectService<'_> {
        ProjectService { engine: self }
    }
}

impl<'a> ProjectService<'a> {
    /// Reads every saved resource, up to 10,000.
    ///
    /// # Errors
    /// Reports a storage failure or more than `MAX_DISCOVERED_RESOURCES`.
    pub async fn all_resources(self) -> Result<Vec<Resource>, EngineError> {
        let mut all = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.engine.repository.resources(cursor.as_deref()).await?;
            all.extend(page.items);
            if all.len() > MAX_DISCOVERED_RESOURCES {
                return Err(EngineError::Conflict(
                    "This machine exceeds 10,000 discovered resources.",
                ));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(all);
            }
        }
    }

    /// Creates a project locally; nothing changes at any provider.
    ///
    /// # Errors
    /// Rejects invalid names or unavailable local state.
    pub async fn create_project(
        self,
        actor: &Actor,
        name: String,
        source: Source,
        mode: Mode,
    ) -> Result<Project, EngineError> {
        self.engine
            .projects()
            .create_project_in(actor, None, name, source, mode)
            .await
    }

    /// Creates local project metadata in an environment (`default` when None).
    ///
    /// # Errors
    /// Rejects invalid names, an unknown environment or unavailable local state.
    pub async fn create_project_in(
        self,
        actor: &Actor,
        environment_id: Option<&str>,
        name: String,
        source: Source,
        mode: Mode,
    ) -> Result<Project, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.authorize(actor, "apps:create").await?;
        if !valid_slug(&name) {
            return Err(EngineError::Invalid(
                "Use a lowercase project name with letters, digits and hyphens.",
            ));
        }
        let environment_id = self.engine.known_environment(environment_id).await?;
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            source,
            mode,
            created_at: now(),
            environment_id,
        };
        self.engine
            .repository
            .create_project(actor.id(), &project)
            .await?;
        Ok(project)
    }

    /// Switches a project and its resources between watching and managed.
    ///
    /// # Errors
    /// Rejects unknown projects or state write failures.
    pub async fn set_mode(self, project_id: &str, mode: Mode) -> Result<Project, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine
            .repository
            .set_project_mode(project_id, mode)
            .await
    }

    /// Creates an imported project and groups selected resources in one local transaction.
    /// Nothing changes at the provider; without a source the project stays unlinked.
    ///
    /// # Errors
    /// Rejects invalid selections, missing scopes or agent attempts to opt into management.
    pub async fn import_project(
        self,
        actor: &Actor,
        name: String,
        source: Option<Source>,
        mode: Mode,
        resource_ids: &[String],
    ) -> Result<ImportedProject, EngineError> {
        self.engine
            .projects()
            .import_project_in(actor, None, name, source, mode, resource_ids)
            .await
    }

    /// Imports into an environment (`default` when None).
    ///
    /// # Errors
    /// As [`Self::import_project`], and an unknown environment.
    pub async fn import_project_in(
        self,
        actor: &Actor,
        environment_id: Option<&str>,
        name: String,
        source: Option<Source>,
        mode: Mode,
        resource_ids: &[String],
    ) -> Result<ImportedProject, EngineError> {
        let environment_id = self.engine.known_environment(environment_id).await?;
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.authorize(actor, "apps:create").await?;
        if matches!(actor, Actor::Agent { .. }) && mode == Mode::Managed {
            return Err(EngineError::ApprovalRequired);
        }
        if !valid_slug(&name) {
            return Err(EngineError::Invalid(
                "Use a lowercase project name with letters, digits and hyphens.",
            ));
        }
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            source: source.unwrap_or(Source::Unlinked),
            mode,
            created_at: now(),
            environment_id,
        };
        let resources = self
            .engine
            .repository
            .import_project(actor.id(), &project, resource_ids)
            .await?;
        Ok(ImportedProject { project, resources })
    }

    /// Validates and links (or changes) a project's deployment source.
    /// Linking does not deploy or change its watching/managed choice.
    ///
    /// # Errors
    /// Rejects invalid sources, missing permissions or already-linked projects.
    pub async fn link_source(
        self,
        actor: &Actor,
        project_id: &str,
        source: Source,
    ) -> Result<Project, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        // A managed project's next deploy builds from the new source; one
        // already under way keeps the source it started with.
        if project.mode == Mode::Managed
            && self
                .engine
                .repository
                .has_pending_deploy(project_id)
                .await?
        {
            return Err(EngineError::Conflict(
                "Wait for the running deploy to finish, then change the source.",
            ));
        }
        // A watched project can be disconnected; Hoista only stops pointing at the code.
        if source == Source::Unlinked && project.mode == Mode::Managed {
            return Err(EngineError::Invalid(
                "Choose a folder or pinned GitHub source.",
            ));
        }
        match (&source, project.mode) {
            (Source::Unlinked, _) => {}
            // The provider builds a watched project from its repository, so Hoista
            // doesn't fetch the code (a private one needs no local sign-in):
            // the names only have to be a repository and a commit.
            (Source::Github { repository, commit }, Mode::Watching) => {
                if !github_repository(repository)
                    || !COMMIT_ID_CHARS.contains(&commit.len())
                    || !commit.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(EngineError::Invalid(
                        "Name the repository as owner/name, with a commit id.",
                    ));
                }
            }
            // A watched project's provider builds it; the folder is where you and
            // your agents work, so it only has to be a folder.
            (Source::Folder { .. }, Mode::Watching) => {
                match self.engine.inspector.detect(&source).await {
                    Ok(_) => {}
                    Err(EngineError::Invalid(message))
                        if message == crate::deploys::planning::NO_SUPPORTED_PROJECT => {}
                    Err(error) => return Err(error),
                }
            }
            _ => {
                self.engine.inspector.detect(&source).await?;
            }
        }
        self.engine
            .repository
            .link_source(actor.id(), project_id, &source)
            .await
    }

    /// What's in a folder, before connecting it to a project. Reads only.
    ///
    /// # Errors
    /// Rejects a missing project or a path that isn't a readable folder.
    pub async fn preview_source(
        self,
        actor: &Actor,
        project_id: &str,
        path: String,
    ) -> Result<crate::deploys::planning::SourcePreview, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        self.engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let source = Source::Folder { path: path.clone() };
        let empty = crate::deploys::planning::SourcePreview {
            path,
            framework: None,
            shape: None,
            package_manager: None,
            build_command: Vec::new(),
            env_names: Vec::new(),
            warnings: Vec::new(),
            unrecognised: None,
        };
        match self.engine.inspector.detect(&source).await {
            Ok(found) => Ok(crate::deploys::planning::SourcePreview {
                framework: Some(found.framework),
                shape: Some(found.shape),
                package_manager: found.package_manager,
                build_command: found.build_command,
                env_names: found.env_names,
                warnings: found.warnings,
                ..empty
            }),
            Err(EngineError::Invalid(message))
                if message == crate::deploys::planning::NO_SUPPORTED_PROJECT =>
            {
                Ok(crate::deploys::planning::SourcePreview {
                    unrecognised: Some(message.into()),
                    ..empty
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Groups discovered resources into a project. Nothing changes at the provider.
    ///
    /// # Errors
    /// Rejects missing resources, duplicate ids or resources belonging to another project.
    pub async fn group(
        self,
        actor: &Actor,
        project_id: &str,
        resource_ids: &[String],
    ) -> Result<Vec<Resource>, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let grouped = self
            .engine
            .repository
            .group_resources(actor.id(), project_id, resource_ids)
            .await?;
        // A project holding only what runs on its providers is a watched project.
        if let Some(project) = self.engine.repository.project(project_id).await?
            && project.mode == Mode::Managed
            && self
                .engine
                .projects()
                .all_resources()
                .await?
                .iter()
                .filter(|resource| resource.project_id.as_deref() == Some(project_id))
                .all(|resource| resource.mode == Mode::Watching)
        {
            self.engine
                .repository
                .set_project_mode(project_id, Mode::Watching)
                .await?;
        }
        Ok(grouped)
    }

    /// Stops watching a project: Hoista forgets the project and its resources go back
    /// to Found on your providers. Nothing changes at the provider.
    ///
    /// # Errors
    /// Rejects agents, managed projects (delete those instead) and unknown projects.
    pub async fn stop_watching(self, actor: &Actor, project_id: &str) -> Result<(), EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the person can stop watching a project, in the Hoista app.",
            ));
        }
        let _mutation = self.engine.mutation.lock().await;
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if project.mode != Mode::Watching {
            return Err(EngineError::Conflict(
                "Hoista manages this project; delete it from its settings instead.",
            ));
        }
        for mut resource in self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| resource.project_id.as_deref() == Some(project_id))
        {
            resource.project_id = None;
            self.engine.repository.put_resource(&resource).await?;
        }
        self.engine.repository.remove_project(project_id).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                None,
                "app.unwatched",
                &format!(
                    "Stopped watching {}. Nothing changed at its provider.",
                    project.name
                ),
            )
            .await
    }

    /// Takes one watched resource out of its project; it goes back to Found on
    /// your providers. A project left with nothing is removed. Nothing changes at
    /// the provider.
    ///
    /// # Errors
    /// Rejects agents, managed resources and resources that aren't in a project.
    pub async fn stop_watching_resource(
        self,
        actor: &Actor,
        resource_id: &str,
    ) -> Result<(), EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the person can take a resource out of a project, in the Hoista app.",
            ));
        }
        let _mutation = self.engine.mutation.lock().await;
        let mut resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let Some(project_id) = resource.project_id.clone() else {
            return Err(EngineError::Invalid("This resource isn't in a project."));
        };
        if resource.mode != Mode::Watching {
            return Err(EngineError::Conflict(
                "Hoista manages this resource; delete it from its settings instead.",
            ));
        }
        let project = self.engine.repository.project(&project_id).await?;
        resource.project_id = None;
        self.engine.repository.put_resource(&resource).await?;
        let emptied = !self
            .engine
            .projects()
            .all_resources()
            .await?
            .iter()
            .any(|other| other.project_id.as_deref() == Some(project_id.as_str()));
        if emptied {
            self.engine.repository.remove_project(&project_id).await?;
        }
        let project_name = project.map(|project| project.name).unwrap_or_default();
        self.engine
            .repository
            .event(
                actor.id(),
                if emptied {
                    None
                } else {
                    Some(project_id.as_str())
                },
                "app.resource_unwatched",
                &format!(
                    "Removed {} from {}. Nothing changed at its provider.",
                    resource.name, project_name
                ),
            )
            .await
    }

    /// Whether a resource is managed or watched, or None when it doesn't exist.
    ///
    /// # Errors
    /// Reports state that can't be saved.
    pub async fn resource_mode(self, resource_id: &str) -> Result<Option<Mode>, EngineError> {
        Ok(self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .map(|resource| resource.mode))
    }

    /// Makes something new at a provider and adds it to an existing project, so
    /// one project can have pieces on every provider: code from a repository or
    /// a folder, a database, a bucket, a domain to send email from, or an
    /// image to run. Where the project already has a resource on the same
    /// account, the provider makes the new one next to it.
    ///
    /// # Errors
    /// Rejects callers without write and create access, a missing project,
    /// accounts that aren't connected, providers that don't take this kind,
    /// invalid names or folders, and provider failures.
    pub async fn add_to_project(
        self,
        actor: &Actor,
        project_id: &str,
        account_id: &str,
        what: crate::ports::providers::ProviderNew<'_>,
    ) -> Result<(String, Vec<Resource>), EngineError> {
        use crate::ports::providers::ProviderNew;
        self.engine.authorize(actor, "apps:write").await?;
        self.engine.authorize(actor, "apps:create").await?;
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
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
        let adapter = self.engine.adapter(account.provider)?;
        if !what.claimed_by(&adapter.capabilities()) {
            return Err(EngineError::Conflict(match what {
                ProviderNew::Repo { .. } => {
                    "This provider can't build a repository. Pick an account on a provider whose capabilities include new_project."
                }
                ProviderNew::Folder { .. } => {
                    "This provider can't deploy from a folder. Pick an account on a provider whose capabilities include new_from_folder."
                }
                ProviderNew::Database { .. } => {
                    "This provider can't make a database. Pick an account on a provider whose capabilities include new_database."
                }
                ProviderNew::Bucket { .. } => {
                    "This provider can't make a bucket. Pick an account on a provider whose capabilities include new_bucket."
                }
                ProviderNew::EmailDomain { .. } => {
                    "This provider doesn't send email. Pick an account on a provider whose capabilities include new_email_domain."
                }
                ProviderNew::Image { .. } => {
                    "This provider can't run an image. Pick an account on a provider whose capabilities include new_from_image."
                }
            }));
        }
        if let Some(missing) = adapter.missing_grant(&account.scopes, &what) {
            return Err(EngineError::Conflict(missing));
        }
        let name = match what {
            ProviderNew::Repo { name, .. }
            | ProviderNew::Folder { name, .. }
            | ProviderNew::Database { name, .. }
            | ProviderNew::Bucket { name }
            | ProviderNew::Image { name, .. } => Some(name),
            ProviderNew::EmailDomain { .. } => None,
        };
        if name.is_some_and(|name| !valid_slug(name)) {
            return Err(EngineError::Invalid(
                "Use a lowercase name with letters, digits and hyphens.",
            ));
        }
        let folder;
        let what = if let ProviderNew::Folder {
            name,
            folder: given,
        } = what
        {
            if !given.is_absolute() {
                return Err(EngineError::Invalid(
                    "Give the folder as a full path on this computer.",
                ));
            }
            folder = std::fs::canonicalize(given)
                .ok()
                .filter(|real| real.is_dir())
                .ok_or(EngineError::Invalid(
                    "That folder doesn't exist on this computer.",
                ))?;
            ProviderNew::Folder {
                name,
                folder: &folder,
            }
        } else {
            what
        };
        let beside = self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .find(|resource| {
                resource.project_id.as_deref() == Some(project_id)
                    && resource.account_id == account.id
                    && resource.mode == Mode::Watching
            })
            .map(|resource| resource.provider_ref);
        let token = self.engine.accounts().account_token(&account.id).await?;
        let made = adapter
            .provider_add(
                &token,
                &account.external_id,
                &project.name,
                beside.as_deref(),
                &what,
            )
            .await;
        drop(token);
        let (references, done) = made?;
        let resource_ids = self
            .engine
            .made_resources(actor, &account, &references)
            .await?;
        if resource_ids.is_empty() && !references.is_empty() {
            return Err(EngineError::Conflict(
                "The provider made it, but Hoista couldn't find it there yet. Run discovery on the account in a minute, then add it to the project from Found on your providers.",
            ));
        }
        // The project keeps its mode: a managed project (one an agent made
        // with create_project) stays managed, so Hoista can still deploy to
        // it, with the new piece watched beside the rest.
        let resources = {
            let _mutation = self.engine.mutation.lock().await;
            self.engine
                .repository
                .group_resources(actor.id(), project_id, &resource_ids)
                .await?
        };
        if what.runs_code() {
            self.engine.mark_deploying(&resource_ids).await?;
        }
        let summary = format!("{done}.");
        self.engine
            .repository
            .event(actor.id(), Some(project_id), "provider.action", &summary)
            .await?;
        Ok((summary, resources))
    }

    /// The account's projects at its provider that hold nothing yet, so they can
    /// be seen (and deleted there) though discovery has nothing to add.
    ///
    /// # Errors
    /// Missing permission, an unknown account or provider failure.
    pub async fn empty_provider_projects(
        self,
        actor: &Actor,
        account_id: &str,
    ) -> Result<Vec<crate::ports::providers::ProviderEmptyProject>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let account = self
            .engine
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .adapter(account.provider)?
            .provider_empty_projects(&token, &account.external_id)
            .await
    }

    /// Reads what the provider reports about one resource right now: its own
    /// deployment history, variable names and domains. Nothing is stored.
    ///
    /// # Errors
    /// Rejects unauthorized reads, missing resources or accounts, and providers that
    /// don't report these (`provider.not_supported`).
    pub async fn provider_details(
        self,
        actor: &Actor,
        resource_id: &str,
    ) -> Result<crate::ports::providers::ProviderDetails, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let resource = self
            .engine
            .repository
            .resource(resource_id)
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
            .provider_details(&token, &account.external_id, &resource.provider_ref)
            .await
    }

    /// Links a project to a repository a code account reaches, at the newest
    /// commit of `branch` (the repository's default when absent).
    ///
    /// # Errors
    /// Reports missing permission, unknown accounts, repositories or
    /// branches, or provider failure.
    pub async fn link_repository(
        self,
        actor: &Actor,
        project_id: &str,
        account_id: &str,
        repository: &str,
        branch: Option<&str>,
    ) -> Result<Project, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        let account = self
            .engine
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let adapter = self.engine.adapter(account.provider)?;
        // Listing needs only the token's metadata access; the newest commit
        // needs its code access too. Knowing which one failed says what to fix.
        let listed = adapter
            .repositories(&token, &account.external_id)
            .await
            .map_err(|error| unreachable_repository(error, repository, false))?
            .into_iter()
            .find(|repo| repo.full_name.eq_ignore_ascii_case(repository));
        let branch = match (branch, &listed) {
            (Some(branch), _) => branch.to_owned(),
            (None, Some(repo)) => repo.default_branch.clone(),
            (None, None) => {
                return Err(unreachable_repository(
                    EngineError::Invalid("This GitHub account can't reach that repository."),
                    repository,
                    false,
                ));
            }
        };
        let commit = adapter
            .branch_head(&token, &account.external_id, repository, &branch)
            .await
            .map_err(|error| unreachable_repository(error, repository, listed.is_some()))?;
        drop(token);
        self.engine
            .projects()
            .link_source(
                actor,
                project_id,
                Source::Github {
                    repository: repository.to_owned(),
                    commit,
                },
            )
            .await
    }

    /// Renames a project in Hoista only. Providers keep their own names.
    ///
    /// # Errors
    /// Rejects invalid names, a name another project already uses, or a missing project.
    pub async fn rename_project(
        self,
        actor: &Actor,
        id: &str,
        name: &str,
    ) -> Result<Project, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        if !valid_slug(name) {
            return Err(EngineError::Invalid(
                "Use a lowercase project name with letters, digits and hyphens.",
            ));
        }
        let mut project = self
            .engine
            .repository
            .project(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if self
            .engine
            .accounts()
            .all_projects()
            .await?
            .iter()
            .any(|other| {
                other.id != id
                    && other.name == name
                    && other.environment_id == project.environment_id
            })
        {
            return Err(EngineError::Conflict(
                "Another project in this environment already has that name.",
            ));
        }
        let before = std::mem::replace(&mut project.name, name.to_owned());
        self.engine.repository.put_project(&project).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                Some(id),
                "app.renamed",
                &format!("Renamed {before} to {name}."),
            )
            .await?;
        Ok(project)
    }

    /// Moves a project into another environment. Nothing changes at the provider.
    ///
    /// # Errors
    /// Rejects unknown projects or environments, and a name already used there.
    pub async fn move_project(
        self,
        actor: &Actor,
        project_id: &str,
        environment_id: &str,
    ) -> Result<Project, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        self.engine
            .repository
            .move_project(actor.id(), project_id, environment_id)
            .await
    }

    /// Moves one resource of a watched project into a new watched project of its own,
    /// removing the old project if that leaves it empty. Nothing changes at the provider.
    ///
    /// # Errors
    /// Rejects managed resources, invalid or taken names, and missing resources.
    pub async fn split_resource(
        self,
        actor: &Actor,
        resource_id: &str,
        name: &str,
    ) -> Result<Project, EngineError> {
        let mut resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let from = resource.project_id.clone().ok_or(EngineError::Invalid(
            "This resource isn't in a project yet; add it from Found on your providers.",
        ))?;
        if resource.mode != Mode::Watching {
            return Err(EngineError::Watching);
        }
        let source = self
            .engine
            .repository
            .project(&from)
            .await?
            .ok_or(EngineError::NotFound)?;
        if self
            .engine
            .accounts()
            .all_projects()
            .await?
            .iter()
            .any(|other| other.name == name && other.environment_id == source.environment_id)
        {
            return Err(EngineError::Conflict(
                "Another project in this environment already has that name.",
            ));
        }
        let created = self
            .engine
            .projects()
            .create_project_in(
                actor,
                Some(&source.environment_id),
                name.to_owned(),
                source.source.clone(),
                Mode::Watching,
            )
            .await?;
        let _mutation = self.engine.mutation.lock().await;
        resource.project_id = Some(created.id.clone());
        self.engine.repository.put_resource(&resource).await?;
        if !self
            .engine
            .projects()
            .all_resources()
            .await?
            .iter()
            .any(|other| other.project_id.as_deref() == Some(from.as_str()))
        {
            self.engine.repository.remove_project(&from).await?;
        }
        self.engine
            .repository
            .event(
                actor.id(),
                Some(&created.id),
                "app.split",
                &format!(
                    "Moved {} out of {} into its own project.",
                    resource.name, source.name
                ),
            )
            .await?;
        Ok(created)
    }

    /// Moves resources into another project, whichever project they are in
    /// now: for something that landed in the wrong project, such as a
    /// project Hoista made on its own for a resource created at a provider.
    /// Nothing changes at the provider, and the target keeps its mode. A project
    /// left empty with no deploys of its own is removed.
    ///
    /// # Errors
    /// Rejects 0 or over 100 resources, missing resources or projects, and a
    /// resource Hoista deployed in a project with deploy history, which stays
    /// with that history.
    pub async fn move_resources(
        self,
        actor: &Actor,
        resource_ids: &[String],
        into: &str,
    ) -> Result<(Vec<Resource>, Vec<String>), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        if resource_ids.is_empty() || resource_ids.len() > MAX_MOVED_RESOURCES {
            return Err(EngineError::Invalid("Move between 1 and 100 resources."));
        }
        let target = self
            .engine
            .repository
            .project(into)
            .await?
            .ok_or(EngineError::NotFound)?;
        let mut moving = Vec::new();
        for id in resource_ids {
            let resource = self
                .engine
                .repository
                .resource(id)
                .await?
                .ok_or(EngineError::NotFound)?;
            if resource.project_id.as_deref() == Some(into) {
                continue;
            }
            if resource.mode == Mode::Managed
                && let Some(from) = resource.project_id.as_deref()
                && !self
                    .engine
                    .repository
                    .deploys(from, None)
                    .await?
                    .items
                    .is_empty()
            {
                return Err(EngineError::Conflict(
                    "Hoista deploys this resource and its deploy history is in its project; it stays there.",
                ));
            }
            moving.push(resource);
        }
        let mut left: Vec<String> = moving
            .iter()
            .filter_map(|resource| resource.project_id.clone())
            .collect();
        left.sort();
        left.dedup();
        for resource in &mut moving {
            resource.project_id = Some(into.to_owned());
            self.engine.repository.put_resource(resource).await?;
        }
        // Projects the move emptied, and that have no history of their own, go.
        let everything = self.engine.projects().all_resources().await?;
        let mut removed = Vec::new();
        for from in left {
            let empty = !everything
                .iter()
                .any(|resource| resource.project_id.as_deref() == Some(from.as_str()));
            if empty
                && self
                    .engine
                    .repository
                    .deploys(&from, None)
                    .await?
                    .items
                    .is_empty()
            {
                if let Some(project) = self.engine.repository.project(&from).await? {
                    removed.push(project.name);
                }
                self.engine.repository.remove_project(&from).await?;
            }
        }
        if !moving.is_empty() {
            let names: Vec<&str> = moving
                .iter()
                .map(|resource| resource.name.as_str())
                .collect();
            self.engine
                .repository
                .event(
                    actor.id(),
                    Some(into),
                    "app.resources_added",
                    &format!("Moved {} into {}.", names.join(", "), target.name),
                )
                .await?;
        }
        Ok((moving, removed))
    }

    /// Moves every resource of one watched project into another watched project and removes
    /// the emptied project, for resources that belong together (for example a
    /// Railway service and its database). Nothing changes at the provider. Usage,
    /// logs and status follow the resources.
    ///
    /// # Errors
    /// Rejects merging a project into itself, missing projects, managed projects or
    /// managed resources (those carry deploy history and stay where they are).
    pub async fn merge_projects(
        self,
        actor: &Actor,
        from: &str,
        into: &str,
    ) -> Result<Project, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        if from == into {
            return Err(EngineError::Invalid(
                "Choose a different project to merge into.",
            ));
        }
        let source = self
            .engine
            .repository
            .project(from)
            .await?
            .ok_or(EngineError::NotFound)?;
        let target = self
            .engine
            .repository
            .project(into)
            .await?
            .ok_or(EngineError::NotFound)?;
        if source.mode != Mode::Watching || target.mode != Mode::Watching {
            return Err(EngineError::Conflict(
                "Only projects Hoista watches can be merged; managed projects keep their deploy history.",
            ));
        }
        let resources: Vec<Resource> = self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| resource.project_id.as_deref() == Some(from))
            .collect();
        if resources
            .iter()
            .any(|resource| resource.mode != Mode::Watching)
        {
            return Err(EngineError::Watching);
        }
        for mut resource in resources {
            resource.project_id = Some(into.to_owned());
            self.engine.repository.put_resource(&resource).await?;
        }
        self.engine.repository.remove_project(from).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                Some(into),
                "app.merged",
                &format!("Moved {}'s resources into {}.", source.name, target.name),
            )
            .await?;
        Ok(target)
    }

    /// Deletes a managed resource after checking agent approval.
    ///
    /// # Errors
    /// Rejects watching resources, missing approval or provider deletion failure.
    pub async fn delete_resource(
        self,
        actor: &Actor,
        id: &str,
        approval: Option<&str>,
        confirmation: Option<&crate::domain::deletion::DeletionConfirmation>,
    ) -> Result<Vec<crate::domain::database_export::DatabaseArchive>, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let resource = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let project = self
            .engine
            .repository
            .project(
                resource
                    .project_id
                    .as_deref()
                    .ok_or(EngineError::Watching)?,
            )
            .await?
            .ok_or(EngineError::NotFound)?;
        if project.mode != Mode::Managed || resource.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        if self
            .engine
            .repository
            .has_pending_deploy(&project.id)
            .await?
        {
            return Err(EngineError::Conflict(
                "Wait for pending deployments before deleting this resource.",
            ));
        }
        let (archives, affected) = self
            .engine
            .confirm_deletion(
                actor,
                GatedAction::DeleteResource,
                id,
                approval,
                confirmation,
            )
            .await?;
        // Once the delete may have started, resources that depend on it must stop showing
        // Live, even if the outcome turns out unclear.
        for impact in &affected {
            let mut consumer = self
                .engine
                .repository
                .resource(&impact.id)
                .await?
                .ok_or(EngineError::NotFound)?;
            consumer.status = ResourceStatus::Unknown;
            self.engine.repository.put_resource(&consumer).await?;
        }
        for domain in self
            .engine
            .environments()
            .all_domains()
            .await?
            .into_iter()
            .filter(|domain| domain.resource_id == resource.id)
        {
            self.engine.detach_saved_domain(&resource, &domain).await?;
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .adapter(resource.provider)?
            .delete(&token, &account.external_id, &resource.provider_ref)
            .await?;
        self.engine.repository.remove_resource(id).await?;
        if !affected.is_empty() {
            self.engine
                .repository
                .event(
                    actor.id(),
                    Some(&project.id),
                    "app.dependency_removed",
                    "Deleted a reviewed dependency; retained dependent resources need attention.",
                )
                .await?;
        }
        self.engine
            .repository
            .event(
                actor.id(),
                resource.project_id.as_deref(),
                "resource.deleted",
                "Deleted an approved provider resource.",
            )
            .await?;
        Ok(archives)
    }
}
