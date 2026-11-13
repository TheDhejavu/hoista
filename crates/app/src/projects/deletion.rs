//! Reviewing and approving deletions. Review, confirmation and the delete
//! itself all use the same recorded impact.

use crate::projects::ProjectService;
use crate::{
    deploys::operations::deletion_order,
    engine::{Actor, Engine, now},
};
use crate::{
    domain::{deletion::*, model::*},
    ports::catalog::EngineError,
};
use sha2::{Digest, Sha256};

/// Why a deletion can't go ahead, in words that say what to do instead.
const DEPENDED_ON: &str = "Other projects depend on these resources. Move their dependencies or delete those consuming projects first.";
const ON_ITS_PROVIDER: &str = "This project runs on its provider, so Hoista doesn't delete it as a whole. Remove each of its services with remove_service (the person approves each one in Hoista); a watched project leaves Hoista when its last service is gone.";
const DEPLOYING: &str = "A deploy is still running. Wait for it to finish, then try again.";
const UNMAPPED: &str = "Some of this project's map links point at resources Hoista no longer sees. Refresh the project's map, then try again.";

pub(crate) fn require_executable(preview: &DeletionPreview) -> Result<(), EngineError> {
    match preview.blockers.first() {
        None => Ok(()),
        Some(first) => Err(EngineError::Conflict(
            [DEPENDED_ON, ON_ITS_PROVIDER, DEPLOYING, UNMAPPED]
                .into_iter()
                .find(|known| first == known)
                .unwrap_or(
                    "This can't be deleted yet. Read the deletion preview in Hoista for why.",
                ),
        )),
    }
}

impl Engine {
    pub(crate) async fn deletion_impact(
        &self,
        action: GatedAction,
        id: &str,
    ) -> Result<DeletionPreview, EngineError> {
        let selected = if action == GatedAction::DeleteResource {
            Some(
                self.repository
                    .resource(id)
                    .await?
                    .ok_or(EngineError::NotFound)?,
            )
        } else if action == GatedAction::DeleteProject {
            None
        } else {
            return Err(EngineError::Invalid(
                "Use a project or resource deletion action.",
            ));
        };
        let project_id = match &selected {
            Some(resource) => resource.project_id.clone().ok_or(EngineError::Watching)?,
            None => id.to_owned(),
        };
        let project = self
            .repository
            .project(&project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let all_resources = self.projects().all_resources().await?;
        let project_resources: Vec<_> = all_resources
            .iter()
            .filter(|resource| resource.project_id.as_deref() == Some(&project_id))
            .cloned()
            .collect();
        let mut resources = if let Some(resource) = selected {
            vec![resource]
        } else {
            project_resources.clone()
        };
        resources.sort_by(|left, right| left.id.cmp(&right.id));
        let all_links = self.repository.all_links().await?;
        if all_links.iter().any(|link| {
            !all_resources
                .iter()
                .any(|resource| resource.id == link.from_resource && resource.project_id.is_some())
                || !all_resources.iter().any(|resource| {
                    resource.id == link.to_resource && resource.project_id.is_some()
                })
        }) {
            return Err(EngineError::Conflict(
                "Reconcile missing or unassigned map resources before deletion.",
            ));
        }
        let deleting: std::collections::BTreeSet<_> = resources
            .iter()
            .map(|resource| resource.id.clone())
            .collect();
        let mut affected = deleting.clone();
        loop {
            let previous = affected.len();
            for link in &all_links {
                if affected.contains(&link.to_resource) {
                    affected.insert(link.from_resource.clone());
                }
            }
            if affected.len() == previous {
                break;
            }
        }
        let mut affected_resources: Vec<_> = all_resources
            .iter()
            .filter(|resource| affected.contains(&resource.id) && !deleting.contains(&resource.id))
            .collect();
        affected_resources.sort_by(|left, right| left.id.cmp(&right.id));
        let external_consumers = affected_resources
            .iter()
            .any(|resource| resource.project_id.as_deref() != Some(&project_id));
        let affected_resources = affected_resources.into_iter().map(resource).collect();
        let mut links: Vec<_> = all_links
            .into_iter()
            .filter(|link| {
                affected.contains(&link.from_resource) || affected.contains(&link.to_resource)
            })
            .collect();
        links.sort_by(|left, right| left.id.cmp(&right.id));
        let mut blockers = vec![];
        if external_consumers {
            blockers.push(DEPENDED_ON.into());
        }
        if project.mode != Mode::Managed
            || resources
                .iter()
                .any(|resource| resource.mode != Mode::Managed)
        {
            blockers.push(ON_ITS_PROVIDER.into());
        }
        if self.repository.has_pending_deploy(&project_id).await? {
            blockers.push(DEPLOYING.into());
        }
        let order = if action == GatedAction::DeleteResource {
            vec![0]
        } else {
            let internal_links: Vec<_> = links
                .iter()
                .filter(|link| {
                    deleting.contains(&link.from_resource) && deleting.contains(&link.to_resource)
                })
                .cloned()
                .collect();
            match deletion_order(&resources, &internal_links) {
                Ok(order) => order,
                Err(_) => {
                    blockers.push(UNMAPPED.into());
                    (0..resources.len()).collect()
                }
            }
        };
        let mut domains: Vec<_> = self
            .environments()
            .all_domains()
            .await?
            .into_iter()
            .filter(|domain| {
                resources
                    .iter()
                    .any(|resource| resource.id == domain.resource_id)
            })
            .map(|domain| DeletionDomain {
                id: domain.id,
                hostname: domain.hostname,
                resource_id: domain.resource_id,
                zone_id: domain.observed.zone_id,
            })
            .collect();
        domains.sort_by(|left, right| left.id.cmp(&right.id));
        let mut preview = DeletionPreview {
            action,
            target: id.into(),
            project_id: project.id.clone(),
            latest_deploy_id: self
                .repository
                .deploys(&project.id, None)
                .await?
                .items
                .first()
                .map(|deploy| deploy.id.clone()),
            confirmation_name: if action == GatedAction::DeleteResource {
                resources[0].name.clone()
            } else {
                project.name
            },
            resources: order
                .into_iter()
                .map(|index| resource(&resources[index]))
                .collect(),
            affected_resources,
            domains,
            links,
            blockers,
            database_export_available: self
                .database_exporter
                .as_ref()
                .is_some_and(|exporter| exporter.available()),
            databases: self.database_targets(&resources).await?,
            fingerprint: String::new(),
        };
        preview.fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&preview).map_err(|_| EngineError::Store)?)
        );
        Ok(preview)
    }

    pub(crate) async fn confirm_deletion(
        &self,
        actor: &Actor,
        action: GatedAction,
        id: &str,
        approval: Option<&str>,
        confirmation: Option<&DeletionConfirmation>,
    ) -> Result<
        (
            Vec<crate::domain::database_export::DatabaseArchive>,
            Vec<DeletionResource>,
        ),
        EngineError,
    > {
        let preview = self.deletion_impact(action, id).await?;
        require_executable(&preview)?;
        match actor {
            Actor::Person => check_confirmation(&preview, confirmation)?,
            Actor::Agent { id: agent } => {
                let approved = self
                    .repository
                    .approval(approval.ok_or(EngineError::ApprovalRequired)?)
                    .await?
                    .ok_or(EngineError::ApprovalRequired)?;
                if approved.decision != ApprovalDecision::Allowed
                    || chrono::DateTime::parse_from_rfc3339(&approved.expires_at)
                        .map_or(true, |expiry| expiry <= chrono::Utc::now())
                    || approved.agent_id != *agent
                    || approved.action != action
                    || approved.target != id
                    || approved
                        .deletion
                        .as_ref()
                        .is_none_or(|saved| saved.fingerprint != preview.fingerprint)
                {
                    return Err(EngineError::Conflict(
                        "Deletion impact changed or was not reviewed; request a new approval.",
                    ));
                }
            }
        }
        let backup = match actor {
            Actor::Agent { .. } => true,
            Actor::Person => confirmation.is_none_or(|confirmation| confirmation.backup_databases),
        };
        if !backup && action != GatedAction::DeleteProject {
            return Err(EngineError::Invalid(
                "Individual database deletion always requires a backup.",
            ));
        }
        let archives = if backup {
            self.export_targets(actor, &preview.databases).await?
        } else {
            vec![]
        };
        // The provider can change outside Hoista while an archive is written.
        if self.deletion_impact(action, id).await?.fingerprint != preview.fingerprint {
            return Err(EngineError::Conflict(
                "Deletion impact changed during export; no provider deletion was started.",
            ));
        }
        self.gate(actor, action, id, approval).await?;
        Ok((archives, preview.affected_resources))
    }

    /// Records an answer that is already the person's: their owner check
    /// passed, or the access they set says "without asking".
    pub(crate) async fn record_decision(
        &self,
        id: &str,
        allow: bool,
        confirmation: Option<&DeletionConfirmation>,
    ) -> Result<(), EngineError> {
        let _mutation = self.mutation.lock().await;
        let approval = self
            .repository
            .approval(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if allow && approval.action == GatedAction::ConnectAgent {
            self.accounts().connection_review(&approval).await?;
        }
        if allow
            && matches!(
                approval.action,
                GatedAction::DeleteProject | GatedAction::DeleteResource
            )
        {
            let preview = self
                .deletion_impact(approval.action, &approval.target)
                .await?;
            require_executable(&preview)?;
            if approval
                .deletion
                .as_ref()
                .is_none_or(|saved| saved.fingerprint != preview.fingerprint)
            {
                return Err(EngineError::Conflict(
                    "Deletion impact changed; request a new approval.",
                ));
            }
            check_confirmation(&preview, confirmation)?;
        }
        self.repository.decide_approval(id, allow, &now()).await
    }
}

impl<'a> ProjectService<'a> {
    /// Reviews what a delete would remove, from recorded resources and domains. Changes
    /// nothing at the provider and uses up no approval.
    ///
    /// # Errors
    /// Rejects inaccessible or missing targets and incomplete local maps.
    pub async fn preview_deletion(
        self,
        actor: &Actor,
        action: GatedAction,
        id: &str,
    ) -> Result<DeletionPreview, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let _mutation = self.engine.mutation.lock().await;
        self.engine.deletion_impact(action, id).await
    }

    /// Records the person's decision, requiring typed confirmation for a deletion approval.
    ///
    /// # Errors
    /// Rejects changed impact, wrong confirmation, unknown or expired approvals.
    pub async fn decide_approval(
        self,
        id: &str,
        allow: bool,
        confirmation: Option<&DeletionConfirmation>,
    ) -> Result<(), EngineError> {
        if allow {
            // An agent's key or a script running as the person can reach
            // this; the computer's own owner prompt can't be answered by them.
            let approval = self
                .engine
                .repository
                .approval(id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let name = self
                .engine
                .repository
                .agent(&approval.agent_id)
                .await?
                .map_or_else(|| "an agent".to_owned(), |agent| agent.name);
            self.engine
                .confirm_owner(&format!("allow {name}: {}", approval.reason))
                .await?;
        }
        self.engine.record_decision(id, allow, confirmation).await
    }

    /// Allows what the agent asked for when the person set its access to
    /// "without asking". Connecting agents and rotating the link stay with
    /// the person. Returns false when the access says to ask. Recorded like
    /// any answer.
    ///
    /// # Errors
    /// Reports state that can't be saved and a deletion whose impact changed.
    pub async fn allow_by_access(self, actor: &Actor, id: &str) -> Result<bool, EngineError> {
        let Actor::Agent { id: agent_id } = actor else {
            return Ok(false);
        };
        let Some(agent) = self.engine.repository.agent(agent_id).await? else {
            return Ok(false);
        };
        if !agent.grant.without_asking {
            return Ok(false);
        }
        let Some(approval) = self
            .engine
            .repository
            .approval(id)
            .await?
            .filter(|approval| approval.agent_id == *agent_id)
        else {
            return Ok(false);
        };
        let access_only = matches!(approval.action, GatedAction::ConnectAgent);
        if access_only || approval.decision != ApprovalDecision::Pending {
            return Ok(false);
        }
        let confirmation = approval
            .deletion
            .as_ref()
            .map(|preview| DeletionConfirmation {
                backup_databases: true,
                confirmation_name: preview.confirmation_name.clone(),
                fingerprint: preview.fingerprint.clone(),
            });
        self.engine
            .record_decision(id, true, confirmation.as_ref())
            .await?;
        self.engine
            .repository
            .event(
                actor.id(),
                None,
                "approval.allowed_by_access",
                &format!(
                    "Allowed by its access (without asking): {}",
                    approval.reason
                ),
            )
            .await?;
        Ok(true)
    }

    /// Records the person's answer given in the agent's own client: the
    /// client asked them (MCP elicitation) and they allowed or declined. Only
    /// the agent that asked can pass on the answer, and a deletion still
    /// needs the exact name the person typed.
    ///
    /// # Errors
    /// Rejects other agents' approvals, unknown or expired approvals, and a
    /// deletion whose typed name doesn't match.
    pub async fn answer_in_client(
        self,
        actor: &Actor,
        id: &str,
        allow: bool,
        typed_name: Option<&str>,
    ) -> Result<Approval, EngineError> {
        let Actor::Agent { id: agent } = actor else {
            return Err(EngineError::Invalid(
                "Only an agent's client passes on an answer this way.",
            ));
        };
        let approval = self
            .engine
            .repository
            .approval(id)
            .await?
            .filter(|approval| approval.agent_id == *agent)
            .ok_or(EngineError::NotFound)?;
        if approval.decision != ApprovalDecision::Pending {
            return Err(EngineError::Conflict("This approval was already answered."));
        }
        let confirmation = approval
            .deletion
            .as_ref()
            .map(|preview| DeletionConfirmation {
                backup_databases: true,
                confirmation_name: typed_name.unwrap_or_default().to_owned(),
                fingerprint: preview.fingerprint.clone(),
            });
        if allow
            && let Some(preview) = &approval.deletion
            && typed_name != Some(preview.confirmation_name.as_str())
        {
            return Err(EngineError::Conflict(
                "The name typed doesn't match, so nothing was deleted.",
            ));
        }
        self.engine
            .projects()
            .decide_approval(id, allow, confirmation.as_ref())
            .await?;
        self.engine
            .repository
            .event(
                actor.id(),
                None,
                if allow {
                    "approval.allowed_in_client"
                } else {
                    "approval.declined_in_client"
                },
                &format!(
                    "{} in the agent: {}",
                    if allow { "Allowed" } else { "Declined" },
                    approval.reason
                ),
            )
            .await?;
        self.engine
            .repository
            .approval(id)
            .await?
            .ok_or(EngineError::NotFound)
    }

    /// Saves the person's map edit under the same lock as deletion review.
    ///
    /// # Errors
    /// Rejects invalid resource relationships or unavailable storage.
    pub async fn save_link(self, link: &Link) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.repository.put_link(link).await
    }
}

fn check_confirmation(
    preview: &DeletionPreview,
    confirmation: Option<&DeletionConfirmation>,
) -> Result<(), EngineError> {
    if confirmation.is_none_or(|value| {
        value.confirmation_name != preview.confirmation_name
            || value.fingerprint != preview.fingerprint
    }) {
        return Err(EngineError::Conflict(
            "Type the exact displayed name and confirm the current deletion preview.",
        ));
    }
    Ok(())
}

fn resource(resource: &Resource) -> DeletionResource {
    DeletionResource {
        id: resource.id.clone(),
        project_id: resource.project_id.clone(),
        name: resource.name.clone(),
        kind: resource.kind,
        provider: resource.provider,
        account_id: resource.account_id.clone(),
        provider_ref: resource.provider_ref.clone(),
        url: resource.url.clone(),
    }
}
