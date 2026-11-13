//! Approvals as the desktop app shows them. Display only: a decision still
//! goes through the engine's check of the exact impact that was reviewed.

use super::catalog::ApiFault;
use crate::{
    Provider,
    application::{engine::Engine, ports::catalog::EngineError},
    domain::{deletion::DeletionConfirmation, model::*},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// Display-only resource target; operations without resources have an empty target list.
#[derive(Serialize, JsonSchema, TS)]
pub struct ApprovalDisplayTarget {
    /// Kind of effect, such as deletion or disruption.
    pub label: String,
    /// Display name from Hoista's own records, not from the agent.
    pub name: String,
    /// Provider serving this resource.
    pub provider: Provider,
}

/// One exact setting shown in a non-resource approval review.
#[derive(Serialize, JsonSchema, TS)]
pub struct ApprovalDisplayDetail {
    /// Human-readable setting label.
    pub label: String,
    /// The value as saved when the approval was asked for.
    pub value: String,
}

/// Desktop projection preserving terminal states and exact deletion confirmation material.
#[derive(Serialize, JsonSchema, TS)]
pub struct ApprovalDisplay {
    /// Approval identity used for the decision request.
    pub id: String,
    /// Requesting agent identity.
    pub agent_id: String,
    /// Registered display name, or identity when unavailable.
    pub agent_name: String,
    /// Execution semantics shown alongside the agent identity.
    pub agent_detail: String,
    /// Exact server-side action this request authorizes.
    pub action: GatedAction,
    /// Human-readable summary.
    pub title: String,
    /// Original request explanation, rendered as text.
    pub reason: String,
    /// Reviewed resource and hostname effects.
    pub targets: Vec<ApprovalDisplayTarget>,
    /// Exact before/after settings for non-resource changes.
    pub details: Vec<ApprovalDisplayDetail>,
    /// Scope and execution behavior after permission is granted.
    pub if_allowed: String,
    /// Effect of denying this request.
    pub if_declined: String,
    /// Exact name the UI requires before permitting Allow.
    pub confirm_text: Option<String>,
    /// Saved deletion fingerprint and backup policy for the decision request.
    pub confirmation: Option<DeletionConfirmation>,
    /// Reason Desktop cannot offer Allow; decline remains possible.
    pub allow_blocked: Option<String>,
    /// RFC3339 creation timestamp.
    pub created_at: String,
    /// RFC3339 expiry timestamp.
    pub expires_at: String,
    /// Null for pending; otherwise allow, decline, expired or consumed.
    pub decision: Option<String>,
}

#[derive(Serialize)]
struct DisplayPage {
    items: Vec<ApprovalDisplay>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct Cursor {
    cursor: Option<String>,
}

pub(super) fn router() -> Router<Arc<Engine>> {
    Router::new()
        .route("/desktop/approvals", get(list))
        .route("/desktop/approvals/{id}", get(one))
}

async fn one(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<ApprovalDisplay>, ApiFault> {
    let approval = engine
        .repository
        .approval(&id)
        .await?
        .ok_or(EngineError::NotFound)?;
    Ok(Json(display(&engine, approval).await?))
}

async fn list(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<DisplayPage>, ApiFault> {
    let page = engine.repository.approvals(query.cursor.as_deref()).await?;
    let mut items = Vec::new();
    for approval in page.items {
        // The list shows what's waiting; a decided approval is still readable by id.
        if approval.decision == ApprovalDecision::Pending {
            items.push(display(&engine, approval).await?);
        }
    }
    Ok(Json(DisplayPage {
        items,
        cursor: page.next_cursor,
    }))
}

async fn display(engine: &Engine, approval: Approval) -> Result<ApprovalDisplay, EngineError> {
    let agent = engine.repository.agent(&approval.agent_id).await?;
    let agent_name = agent
        .as_ref()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| approval.agent_id.clone());
    let mut view = ApprovalDisplay {
        id: approval.id.clone(),
        agent_id: approval.agent_id.clone(),
        agent_name: agent_name.clone(),
        agent_detail: "Approval permits one agent retry; it does not execute the action.".into(),
        action: approval.action,
        title: format!("{agent_name} requests approval"),
        reason: approval.reason.clone(),
        targets: vec![],
        details: vec![],
        if_allowed: "The agent may retry the reviewed operation before approval expires.".into(),
        if_declined: "No permission is granted. The agent cannot use this request.".into(),
        confirm_text: None,
        confirmation: None,
        allow_blocked: None,
        created_at: approval.created_at.clone(),
        expires_at: approval.expires_at.clone(),
        decision: match approval.decision {
            ApprovalDecision::Pending => None,
            ApprovalDecision::Allowed => Some("allow".into()),
            ApprovalDecision::Declined => Some("decline".into()),
            ApprovalDecision::Expired => Some("expired".into()),
            ApprovalDecision::Consumed => Some("consumed".into()),
        },
    };
    match approval.action {
        GatedAction::DeleteDomain => {
            view.title = format!("{agent_name} wants to remove a hostname");
            view.if_allowed = "The agent may detach this hostname and all its certificates, including custom Fly certificates. DNS records and the project remain.".into();
            // Only what Hoista already knows: showing an approval never asks a provider.
            let resources: std::collections::HashMap<_, _> = engine
                .projects()
                .all_resources()
                .await?
                .into_iter()
                .map(|p| (p.id.clone(), p))
                .collect();
            let mut found = false;
            for domain in engine.environments().all_domains().await? {
                if let Some(resource) = resources.get(&domain.resource_id)
                    && engine
                        .environments()
                        .domain_approval_target(&domain, resource)
                        == approval.target
                {
                    view.targets.push(ApprovalDisplayTarget {
                        label: "Hostname".into(),
                        name: domain.hostname.clone(),
                        provider: resource.provider,
                    });
                    view.confirm_text = Some(domain.hostname);
                    found = true;
                    break;
                }
            }
            if !found {
                view.allow_blocked = Some(
                    "The hostname assignment changed or disappeared. Request a new approval."
                        .into(),
                );
            }
        }
        GatedAction::DeleteProject | GatedAction::DeleteResource => {
            view.title = format!("{agent_name} wants to delete resources");
            if let Some(preview) = &approval.deletion {
                view.confirm_text = Some(preview.confirmation_name.clone());
                view.confirmation = Some(DeletionConfirmation {
                    backup_databases: true,
                    confirmation_name: preview.confirmation_name.clone(),
                    fingerprint: preview.fingerprint.clone(),
                });
                for resource in &preview.resources {
                    view.targets.push(ApprovalDisplayTarget {
                        label: "Delete resource".into(),
                        name: resource.name.clone(),
                        provider: resource.provider,
                    });
                }
                for resource in &preview.affected_resources {
                    view.targets.push(ApprovalDisplayTarget {
                        label: "May be disrupted".into(),
                        name: resource.name.clone(),
                        provider: resource.provider,
                    });
                }
                for domain in &preview.domains {
                    if let Some(resource) = preview
                        .resources
                        .iter()
                        .find(|r| r.id == domain.resource_id)
                    {
                        view.targets.push(ApprovalDisplayTarget {
                            label: "Detach hostname".into(),
                            name: domain.hostname.clone(),
                            provider: resource.provider,
                        });
                    }
                }
                view.if_allowed =
                    "The agent may retry deletion of the listed resources and hostname attachments. Database backups remain required; changed impact is rejected by the daemon.".into();
                if !preview.blockers.is_empty() {
                    view.allow_blocked = Some(preview.blockers.join(" "));
                }
            } else {
                view.allow_blocked = Some(
                    "This request has no reviewed deletion impact. Request a new approval.".into(),
                );
            }
        }
        GatedAction::ConnectAgent => {
            view.title = "Allow a remote agent to connect?".into();
            view.agent_detail = "Client names are supplied by the requester; verify the client ID and callback before allowing access.".into();
            match engine.accounts().connection_review(&approval).await {
                Ok((pending, client, requester)) => {
                    view.details.push(ApprovalDisplayDetail {
                        label: "Client name (self-reported)".into(),
                        value: client.name,
                    });
                    view.details.push(ApprovalDisplayDetail {
                        label: "Client ID".into(),
                        value: client.id,
                    });
                    view.details.push(ApprovalDisplayDetail {
                        label: "Callback URL".into(),
                        value: pending.redirect_uri,
                    });
                    view.details.push(ApprovalDisplayDetail {
                        label: "Hoista endpoint".into(),
                        value: pending.resource,
                    });
                    view.details.push(ApprovalDisplayDetail {
                        label: "Requested permissions".into(),
                        value: requester
                            .scopes
                            .iter()
                            .map(|scope| match scope.as_str() {
                                "apps:read" => {
                                    "apps:read — read project metadata, logs and other permitted observations"
                                }
                                "apps:write" => {
                                    "apps:write — change managed projects; gated actions still require separate approval"
                                }
                                "apps:create" => {
                                    "apps:create — permit creation workflows when paired with their other required scopes"
                                }
                                _ => "unsupported permission",
                            })
                            .collect::<Vec<_>>()
                            .join("; "),
                    });
                    view.if_allowed =
                        "This client may complete its one-use PKCE code exchange before expiry and receive access with exactly the listed permissions. Allowing does not deploy a project or execute another operation. Disconnect the agent to revoke its access.".into();
                }
                Err(
                    EngineError::NotFound
                    | EngineError::ApprovalRequired
                    | EngineError::AgentsDisabled,
                ) => {
                    view.allow_blocked =
                        Some("This connection request expired, changed, is missing review details, or agents are disabled. Restart the connection flow.".into());
                }
                Err(error) => return Err(error),
            }
        }
        GatedAction::ProviderChange => {
            // The target is `{resource or account}:{what}:{which}`.
            let mut pieces = approval.target.splitn(3, ':');
            let on = pieces.next().unwrap_or_default();
            let what = pieces.next().unwrap_or_default();
            let mut place = None;
            if let Some(resource) = engine.repository.resource(on).await? {
                place = Some(resource.provider.display_name());
                view.targets.push(ApprovalDisplayTarget {
                    label: "On".into(),
                    name: resource.name.clone(),
                    provider: resource.provider,
                });
            } else if let Some(account) = engine.repository.account(on).await? {
                place = Some(account.provider.display_name());
                view.targets.push(ApprovalDisplayTarget {
                    label: "Account".into(),
                    name: account.label.clone(),
                    provider: account.provider,
                });
            } else {
                view.allow_blocked =
                    Some("That resource is gone from Hoista. Nothing to allow.".into());
            }
            let place = place.unwrap_or("the provider");
            view.title =
                format!("{agent_name} wants to make a change at {place} that can’t be undone");
            view.agent_detail = String::new();
            if what.chars().any(|c| c.is_ascii_uppercase()) {
                view.details.push(ApprovalDisplayDetail {
                    label: "Operation".into(),
                    value: what.to_owned(),
                });
            }
            view.if_allowed = "The agent can do exactly this, once.".into();
            view.if_declined = "Nothing changes.".into();
        }
        _ => {
            view.allow_blocked =
                Some("This action's detailed Desktop review is not available yet. Review it through the CLI.".into());
        }
    }
    if agent.as_ref().is_none_or(|a| a.revoked) {
        view.allow_blocked = Some("The requesting agent is missing or revoked.".into());
    }
    Ok(view)
}
