//! Listing a provider's operations and running them on a watched resource or a
//! connected account.
//!
//! Reads need `apps:read` and changes `apps:write`; destructive changes also
//! need the person's approval for that exact operation and arguments.

use crate::deploys::DeployService;
use crate::engine::{Actor, Engine};
use crate::{
    domain::model::*,
    ports::{
        catalog::EngineError,
        providers::{ProviderOperationInfo, ProviderRisk},
    },
};
use sha2::{Digest, Sha256};

const MAX_SLUG_CHARS: usize = 50;

/// Whether an operation's name says it makes something: `createPod`,
/// `modal deploy`, `railway add`.
fn creates(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| ["create", "deploy", "add", "new", "up", "launch"].contains(&word))
        || ["create", "deploy", "launch"]
            .iter()
            .any(|start| lower.starts_with(start))
}

/// A project name from a resource's name: lowercase letters, digits and
/// hyphens.
fn slug_of(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug
        .trim_end_matches('-')
        .chars()
        .take(MAX_SLUG_CHARS)
        .collect();
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.is_empty() {
        "resource".into()
    } else {
        slug
    }
}

/// What a provider operation runs on.
#[derive(Debug, Clone)]
pub enum OperationTarget {
    /// A resource Hoista watches, such as one Railway service.
    Resource(String),
    /// A connected account, such as a Railway workspace.
    Account(String),
}

impl OperationTarget {
    fn id(&self) -> &str {
        match self {
            Self::Resource(id) | Self::Account(id) => id,
        }
    }
}

impl Engine {
    /// The approval target for one destructive operation with these
    /// arguments: an approval covers exactly this, once.
    #[must_use]
    pub fn provider_operation_target(
        target: &OperationTarget,
        name: &str,
        args: &serde_json::Value,
    ) -> String {
        let mut digest = Sha256::new();
        digest.update(args.to_string().as_bytes());
        let hash: String = digest
            .finalize()
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("{}:{name}:{hash}", target.id())
    }
}

impl<'a> DeployService<'a> {
    /// Adds what a creating operation made at the provider to Hoista, so it
    /// doesn't wait under Found on your providers: into the project of the
    /// resource the operation ran on, or as a watched project of its own
    /// when it ran on the account. Returns the new resources' ids.
    async fn adopt_created(
        self,
        actor: &Actor,
        account: &Account,
        on: Option<&Resource>,
        into: Option<&str>,
        before: &[String],
    ) -> Result<Vec<String>, EngineError> {
        let made: Vec<Resource> = {
            let _mutation = self.engine.mutation.lock().await;
            self.engine.discover_locked(actor, &account.id).await?;
            self.engine
                .projects()
                .all_resources()
                .await?
                .into_iter()
                .filter(|found| {
                    found.account_id == account.id
                        && found.project_id.is_none()
                        && !before.contains(&found.provider_ref)
                })
                .collect()
        };
        if made.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = made.iter().map(|found| found.id.clone()).collect();
        // The project the caller named, else the one the operation ran in.
        if let Some(project) =
            into.or_else(|| on.and_then(|resource| resource.project_id.as_deref()))
        {
            // Joining keeps the project's mode: a project Hoista deploys stays one.
            self.engine
                .repository
                .group_resources(actor.id(), project, &ids)
                .await?;
            let names: Vec<&str> = made.iter().map(|found| found.name.as_str()).collect();
            self.engine
                .repository
                .event(
                    actor.id(),
                    Some(project),
                    "app.resources_added",
                    &format!(
                        "Added {} to this project after creating it.",
                        names.join(", ")
                    ),
                )
                .await?;
            return Ok(ids);
        }
        let mut taken = std::collections::BTreeSet::new();
        let mut cursor = None;
        loop {
            let page = self.engine.repository.projects(cursor.as_deref()).await?;
            taken.extend(page.items.into_iter().map(|project| project.name));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        for found in &made {
            let base = slug_of(&found.name);
            let mut name = base.clone();
            let mut n = 2;
            while taken.contains(&name) {
                name = format!("{base}-{n}");
                n += 1;
            }
            taken.insert(name.clone());
            let imported = self
                .engine
                .projects()
                .import_project(
                    actor,
                    name,
                    None,
                    Mode::Watching,
                    std::slice::from_ref(&found.id),
                )
                .await?;
            self.engine
                .repository
                .event(
                    actor.id(),
                    Some(&imported.project.id),
                    "app.imported",
                    &format!(
                        "Added {} from {} after creating it there.",
                        found.name,
                        account.provider.display_name()
                    ),
                )
                .await?;
        }
        Ok(ids)
    }

    async fn operation_owner(
        self,
        target: &OperationTarget,
    ) -> Result<(Option<Resource>, Account), EngineError> {
        match target {
            OperationTarget::Resource(id) => {
                let resource = self
                    .engine
                    .repository
                    .resource(id)
                    .await?
                    .ok_or(EngineError::NotFound)?;
                let account = self
                    .engine
                    .repository
                    .account(&resource.account_id)
                    .await?
                    .ok_or(EngineError::NotFound)?;
                Ok((Some(resource), account))
            }
            OperationTarget::Account(id) => Ok((
                None,
                self.engine
                    .repository
                    .account(id)
                    .await?
                    .ok_or(EngineError::NotFound)?,
            )),
        }
    }

    /// Everything the provider behind a resource or account can do.
    ///
    /// # Errors
    /// Unknown targets or missing permission.
    pub async fn provider_operations(
        self,
        actor: &Actor,
        target: &OperationTarget,
    ) -> Result<Vec<ProviderOperationInfo>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let (_, account) = self.engine.deploys().operation_owner(target).await?;
        Ok(self.engine.adapter(account.provider)?.provider_operations())
    }

    /// The project's buttons for a resource, from its provider's manifest: only
    /// those for its kind, and only operations the provider still offers.
    ///
    /// # Errors
    /// Unknown targets or missing permission.
    pub async fn provider_quick_actions(
        self,
        actor: &Actor,
        target: &OperationTarget,
    ) -> Result<Vec<crate::ports::providers::ProviderQuickAction>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let (resource, account) = self.engine.deploys().operation_owner(target).await?;
        let adapter = self.engine.adapter(account.provider)?;
        let offered = adapter.provider_operations();
        Ok(adapter
            .provider_quick_actions()
            .into_iter()
            .filter(|action| {
                action.on.is_none_or(|kind| {
                    resource
                        .as_ref()
                        .is_some_and(|resource| resource.kind == kind)
                }) && (action.refs.is_empty()
                    || resource.as_ref().is_some_and(|resource| {
                        action
                            .refs
                            .iter()
                            .any(|prefix| resource.provider_ref.starts_with(prefix.as_str()))
                    }))
                    && offered.iter().any(|op| op.name == action.operation)
            })
            .collect())
    }

    /// How risky an operation is, or None when the provider has no such operation.
    ///
    /// # Errors
    /// Unknown targets or unsupported providers.
    pub async fn provider_operation_risk(
        self,
        target: &OperationTarget,
        name: &str,
    ) -> Result<Option<ProviderRisk>, EngineError> {
        let (_, account) = self.engine.deploys().operation_owner(target).await?;
        Ok(self
            .engine
            .adapter(account.provider)?
            .provider_operations()
            .into_iter()
            .find(|op| op.name == name)
            .map(|op| op.risk))
    }

    /// Runs one provider operation and returns the provider's answer, secrets removed.
    ///
    /// # Errors
    /// Missing permission or approval, Hoista-managed resources, unknown
    /// operations, invalid arguments and provider failures.
    pub async fn provider_operation(
        self,
        actor: &Actor,
        target: OperationTarget,
        name: &str,
        args: serde_json::Value,
        approval: Option<&str>,
    ) -> Result<serde_json::Value, EngineError> {
        self.engine
            .deploys()
            .provider_operation_into(actor, target, name, args, approval, None)
            .await
    }

    /// [`provider_operation`](Self::provider_operation), with what a creating
    /// operation makes joining the project `into` instead of a project of its own.
    ///
    /// # Errors
    /// As `provider_operation`, and a project that doesn't exist.
    pub async fn provider_operation_into(
        self,
        actor: &Actor,
        target: OperationTarget,
        name: &str,
        args: serde_json::Value,
        approval: Option<&str>,
        into: Option<&str>,
    ) -> Result<serde_json::Value, EngineError> {
        if let Some(project) = into
            && self.engine.repository.project(project).await?.is_none()
        {
            return Err(EngineError::NotFound);
        }
        let (resource, account) = self.engine.deploys().operation_owner(&target).await?;
        let adapter = self.engine.adapter(account.provider)?.clone();
        let risk = adapter
            .provider_operations()
            .into_iter()
            .find(|op| op.name == name)
            .map(|op| op.risk)
            .ok_or(EngineError::Invalid(
                "This provider has no such operation in Hoista. Call list_provider_operations for the list.",
            ))?;
        if risk == ProviderRisk::Read {
            self.engine.authorize(actor, "apps:read").await?;
        } else {
            self.engine.authorize(actor, "apps:write").await?;
            if resource
                .as_ref()
                .is_some_and(|resource| resource.mode != Mode::Watching)
            {
                return Err(EngineError::Conflict(
                    "Hoista manages this resource; change it with a plan instead.",
                ));
            }
        }
        if risk == ProviderRisk::Destroy {
            let gate = Engine::provider_operation_target(&target, name, &args);
            self.engine
                .gate(actor, GatedAction::ProviderChange, &gate, approval)
                .await?;
        }
        // Command-line tools that work on files use the project's own folder,
        // the one the person linked in Hoista, never one a caller names.
        let folder = match resource
            .as_ref()
            .and_then(|resource| resource.project_id.as_deref())
        {
            Some(project) => match self.engine.repository.project(project).await? {
                Some(Project {
                    source: Source::Folder { path },
                    ..
                }) => Some(std::path::PathBuf::from(path)),
                _ => None,
            },
            None => None,
        };
        // What the account had before, to find what a creating operation made.
        let creates = risk == ProviderRisk::Change && creates(name);
        let before: Vec<String> = if creates {
            self.engine
                .projects()
                .all_resources()
                .await?
                .into_iter()
                .filter(|known| known.account_id == account.id)
                .map(|known| known.provider_ref)
                .collect()
        } else {
            Vec::new()
        };
        let token = self.engine.accounts().account_token(&account.id).await?;
        let answer = adapter
            .provider_operation(
                &token,
                &account.external_id,
                resource
                    .as_ref()
                    .map(|resource| resource.provider_ref.as_str()),
                folder.as_deref(),
                name,
                args,
            )
            .await?;
        let mut answer = answer;
        if creates
            && let Ok(added) = self
                .engine
                .deploys()
                .adopt_created(actor, &account, resource.as_ref(), into, &before)
                .await
            && !added.is_empty()
            && let Some(object) = answer.as_object_mut()
        {
            object.insert("added_to_hoista".into(), serde_json::json!(added));
            object.insert(
                "next".into(),
                serde_json::json!("If you made it from a folder or repository, call link_folder or link_repository on its project so the person sees which code it runs."),
            );
        }
        if risk != ProviderRisk::Read {
            // What the change made shows up in Hoista without a manual scan.
            self.engine.rescans.after_change(&account.id);
            let on = resource
                .as_ref()
                .map_or_else(|| account.label.clone(), |resource| resource.name.clone());
            let summary = format!("Ran {name} at {} on {on}.", account.provider.display_name());
            self.engine
                .repository
                .event(
                    actor.id(),
                    resource
                        .as_ref()
                        .and_then(|resource| resource.project_id.as_deref()),
                    "provider.operation",
                    &summary,
                )
                .await?;
        }
        Ok(answer)
    }
}
