//! Finding resources from a plain description. A description only suggests candidates;
//! grouping needs the chosen ids.

use crate::engine::Actor;
use crate::projects::ProjectService;
use crate::{
    Provider,
    domain::model::{Resource, ResourceKind},
    ports::catalog::EngineError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use ts_rs::TS;

const MAX_DESCRIPTION_BYTES: usize = 4096;
/// The most resources one description can match.
const MAX_CANDIDATES: usize = 100;
/// One matching resource, with its account and project so similar matches can be told apart.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct DescriptionCandidate {
    /// Resource id, to select it.
    pub id: String,
    /// Name as the provider shows it.
    pub name: String,
    /// The provider.
    pub provider: Provider,
    /// Resource role.
    pub kind: ResourceKind,
    /// Local account id, to tell apart resources on different accounts.
    pub account_id: String,
    /// The project it is in now; grouping never moves it out of another project. `app_id` is
    /// the older name.
    pub project_id: Option<String>,
    /// The provider's reference for it; part of what the review token covers.
    pub provider_ref: String,
    /// The id, name, provider project name or hostname that matched. Not proof of a dependency.
    pub matched: String,
}

/// Resources matching a description. Reading it changes nothing.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DescriptionResolution {
    /// Up to 100 candidates to review and choose from.
    pub candidates: Vec<DescriptionCandidate>,
    /// True when matches on different providers or of different kinds overlap; nothing is chosen
    /// automatically.
    pub ambiguous: bool,
    /// Token for exactly this result, to pass back when grouping. Grants no access.
    pub review_token: String,
    /// Always true: a description never groups resources without a choice.
    pub selection_required: bool,
}

fn mentions(text: &str, name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    text.match_indices(name).any(|(start, _)| {
        let end = start + name.len();
        let boundary = |c: char| !c.is_alphanumeric() && c != '_' && c != '-';
        text[..start].chars().next_back().is_none_or(boundary)
            && text[end..].chars().next().is_none_or(boundary)
    })
}

fn resolve(
    resources: &[Resource],
    description: &str,
) -> Result<DescriptionResolution, EngineError> {
    if description.is_empty()
        || description.len() > MAX_DESCRIPTION_BYTES
        || description.chars().any(char::is_control)
    {
        return Err(EngineError::Invalid(
            "Use a description of at most 4096 bytes without control characters.",
        ));
    }
    let description = description.to_lowercase();
    let providers: HashSet<_> = Provider::ALL
        .into_iter()
        .filter(|p| {
            mentions(&description, p.slug())
                || mentions(&description, &p.display_name().to_lowercase())
        })
        .collect();
    let mut candidates = Vec::new();
    for resource in resources {
        if !providers.is_empty() && !providers.contains(&resource.provider) {
            continue;
        }
        let names = [
            Some(resource.id.clone()),
            Some(resource.name.clone()),
            crate::accounts::import_suggestions::stem(&resource.name),
            crate::accounts::import_suggestions::host(resource),
        ];
        let Some(matched) = names
            .into_iter()
            .flatten()
            .find(|name| mentions(&description, &name.to_lowercase()))
        else {
            continue;
        };
        candidates.push(DescriptionCandidate {
            id: resource.id.clone(),
            name: resource.name.clone(),
            provider: resource.provider,
            kind: resource.kind,
            account_id: resource.account_id.clone(),
            project_id: resource.project_id.clone(),
            provider_ref: resource.provider_ref.clone(),
            matched,
        });
        if candidates.len() > MAX_CANDIDATES {
            return Err(EngineError::Conflict(
                "The description matches more than 100 resources. Use exact names, IDs or provider names to narrow it.",
            ));
        }
    }
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    let mut roles = HashMap::new();
    let ambiguous = candidates.iter().any(|c| {
        let count = roles
            .entry((c.provider, format!("{:?}", c.kind)))
            .or_insert(0);
        *count += 1;
        *count > 1
    });
    let encoded = serde_json::to_vec(&("description-resolution-v1", &description, &candidates))
        .map_err(|_| EngineError::Store)?;
    Ok(DescriptionResolution {
        candidates,
        ambiguous,
        review_token: format!("{:x}", Sha256::digest(encoded)),
        selection_required: true,
    })
}

impl<'a> ProjectService<'a> {
    /// Finds resources whose id, name, provider project name or hostname the description
    /// mentions. Groups nothing and reads no secrets.
    /// # Errors
    /// Rejects unauthorized reads, invalid descriptions and catalogs too large to search.
    pub async fn resolve_description(
        self,
        actor: &Actor,
        description: &str,
    ) -> Result<DescriptionResolution, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        self.engine.look_at_new_accounts().await?;
        let result = resolve(&self.engine.visible_import_resources().await?, description)?;
        self.engine.authorize(actor, "apps:read").await?;
        Ok(result)
    }

    /// Groups the chosen candidates from a reviewed [`Engine::resolve_description`] result.
    /// The usual project checks apply; no dependency is assumed and nothing changes at the provider.
    /// # Errors
    /// Rejects a stale review token, unauthorized writes, invalid choices and resources in
    /// another project.
    pub async fn group_description(
        self,
        actor: &Actor,
        description: &str,
        review: &str,
        project_id: &str,
        ids: &[String],
    ) -> Result<Vec<Resource>, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:read").await?;
        self.engine.authorize(actor, "apps:write").await?;
        let current = resolve(&self.engine.visible_import_resources().await?, description)?;
        if review != current.review_token {
            return Err(EngineError::Conflict(
                "Matching resources changed. Resolve and review the description again.",
            ));
        }
        let known: HashSet<_> = current.candidates.iter().map(|c| c.id.as_str()).collect();
        let selected: HashSet<_> = ids.iter().map(String::as_str).collect();
        if ids.is_empty()
            || ids.len() > 100
            || selected.len() != ids.len()
            || !selected.is_subset(&known)
        {
            return Err(EngineError::Invalid(
                "Select distinct resource IDs from the reviewed candidates.",
            ));
        }
        self.engine
            .repository
            .group_resources(actor.id(), project_id, ids)
            .await
    }
}
