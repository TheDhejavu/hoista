//! Suggested project groupings from resource names and shared hostnames. Suggestions only;
//! nothing is grouped.

use crate::accounts::AccountService;
use crate::engine::Actor;
use crate::{
    domain::model::*,
    ports::catalog::{EngineError, Page},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

const PAGE_SIZE: usize = 100;
const MAX_NAME_BYTES: usize = 128;
const MAX_GROUP_MEMBERS: usize = 100;
const MAX_CURSOR_BYTES: usize = 512;

/// Why resources were suggested together. Not proof that one depends on another.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ImportEvidenceKind {
    /// Names share a normalized stem after removing one common role suffix.
    SharedName,
    /// Recorded public URLs or domain names have exactly the same hostname.
    SharedHostname,
    /// Services in the same Railway project; the value is the provider project id.
    SameProject,
}

/// One reason to group resources, for the person to review.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct ImportEvidence {
    /// What matched.
    pub kind: ImportEvidenceKind,
    /// Shared name stem or exact hostname; never a URL, connection string or secret.
    pub value: String,
}

/// A suggested project. Reading it groups or changes nothing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct ImportProposal {
    /// Id derived from the evidence and the exact members, so it is the same each time.
    pub id: String,
    /// Suggested name; never used without the person's choice.
    pub label: String,
    /// Unassigned resource ids to review. Importing checks the selection again.
    pub resource_ids: Vec<String>,
    /// Evidence explaining this proposal.
    pub evidence: ImportEvidence,
    /// True when suggestions overlap or two members have the same provider and kind; ask the
    /// person to choose.
    pub ambiguous: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    snapshot: String,
    after: String,
}

impl<'a> AccountService<'a> {
    /// Suggests groups. Reads no secrets and changes nothing, locally or at a provider.
    ///
    /// # Errors
    /// Rejects unauthorized reads, stale or invalid cursors, and catalogs or groups too large.
    pub async fn import_suggestions(
        self,
        actor: &Actor,
        cursor: Option<&str>,
    ) -> Result<Page<ImportProposal>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        if cursor.is_none() {
            self.engine.look_at_new_accounts().await?;
        }
        let resources = self.engine.visible_import_resources().await?;
        let provider_project = |resource: &Resource| {
            self.engine
                .adapter(resource.provider)
                .ok()
                .and_then(|adapter| adapter.provider_project(&resource.provider_ref))
        };
        let page = select(&resources, &provider_project, cursor)?;
        self.engine.authorize(actor, "apps:read").await?;
        Ok(page)
    }
}

pub(crate) fn stem(name: &str) -> Option<String> {
    if name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return None;
    }
    let mut value = name.trim().to_ascii_lowercase().replace('_', "-");
    for suffix in [
        "-database",
        "-uploads",
        "-bucket",
        "-worker",
        "-web",
        "-api",
        "-db",
    ] {
        if let Some(base) = value.strip_suffix(suffix) {
            value = base.to_owned();
            break;
        }
    }
    if value.len() < 3
        || !valid_slug(&value)
        || [
            "project",
            "api",
            "web",
            "database",
            "default",
            "prod",
            "production",
            "staging",
            "service",
            "bucket",
            "uploads",
            "worker",
        ]
        .contains(&value.as_str())
    {
        None
    } else {
        Some(value)
    }
}

pub(crate) fn host(resource: &Resource) -> Option<String> {
    if resource.kind == ResourceKind::Domain
        && crate::domain::domains::valid_hostname(&resource.name)
    {
        return Some(resource.name.clone());
    }
    let url = url::Url::parse(resource.url.as_deref()?).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    crate::domain::domains::valid_hostname(host).then(|| host.to_owned())
}

fn digest(value: &impl Serialize) -> Result<String, EngineError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| EngineError::Store)?)
    ))
}

/// Groups resources by name stem, address and `provider_project` (the provider
/// project each belongs to, where its provider has projects).
fn select(
    resources: &[Resource],
    provider_project: &dyn Fn(&Resource) -> Option<String>,
    cursor: Option<&str>,
) -> Result<Page<ImportProposal>, EngineError> {
    if resources.len() > MAX_DISCOVERED_RESOURCES {
        return Err(EngineError::Conflict(
            "Import suggestions exceed the 10,000-resource catalog limit.",
        ));
    }
    let mut groups: BTreeMap<(u8, String), BTreeMap<String, &Resource>> = BTreeMap::new();
    for resource in resources
        .iter()
        .filter(|resource| resource.project_id.is_none())
    {
        let project = provider_project(resource);
        for (kind, key) in [(0, stem(&resource.name)), (1, host(resource)), (2, project)] {
            if let Some(key) = key {
                groups
                    .entry((kind, key))
                    .or_default()
                    .insert(resource.id.clone(), resource);
            }
        }
    }
    let mut proposals = Vec::new();
    let mut memberships: BTreeMap<String, usize> = BTreeMap::new();
    for ((kind, value), members) in groups {
        if members.len() < 2 {
            continue;
        }
        if members.len() > MAX_GROUP_MEMBERS {
            return Err(EngineError::Conflict(
                "Too many resources share one import hint. Use manual selection to narrow the group.",
            ));
        }
        let resource_ids: Vec<_> = members.keys().cloned().collect();
        let roles: BTreeSet<_> = members
            .values()
            .map(|resource| format!("{}:{:?}", resource.provider.slug(), resource.kind))
            .collect();
        for id in &resource_ids {
            *memberships.entry(id.clone()).or_default() += 1;
        }
        // A provider project id is not a name; label the group after its first code-running member.
        let label = if kind == 2 {
            members
                .values()
                .find(|resource| resource.kind != ResourceKind::Database)
                .or_else(|| members.values().next())
                .map_or_else(|| value.clone(), |resource| resource.name.clone())
        } else {
            value.clone()
        };
        proposals.push(ImportProposal {
            id: digest(&(kind, &value, &resource_ids))?,
            label,
            resource_ids,
            evidence: ImportEvidence {
                kind: match kind {
                    0 => ImportEvidenceKind::SharedName,
                    1 => ImportEvidenceKind::SharedHostname,
                    _ => ImportEvidenceKind::SameProject,
                },
                value,
            },
            ambiguous: roles.len() < members.len(),
        });
    }
    for proposal in &mut proposals {
        proposal.ambiguous |= proposal.resource_ids.iter().any(|id| memberships[id] > 1);
    }
    proposals.sort_by(|a, b| a.id.cmp(&b.id));
    let snapshot = digest(&proposals)?;
    let after = if let Some(cursor) = cursor {
        if cursor.len() > MAX_CURSOR_BYTES {
            return Err(EngineError::Invalid("Invalid import suggestion cursor."));
        }
        let cursor: Cursor = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(cursor)
                .map_err(|_| EngineError::Invalid("Invalid import suggestion cursor."))?,
        )
        .map_err(|_| EngineError::Invalid("Invalid import suggestion cursor."))?;
        if cursor.snapshot != snapshot {
            return Err(EngineError::Conflict(
                "Import candidates changed. Refresh the suggestions before continuing.",
            ));
        }
        if !proposals.iter().any(|proposal| proposal.id == cursor.after) {
            return Err(EngineError::Invalid("Invalid import suggestion cursor."));
        }
        cursor.after
    } else {
        String::new()
    };
    let mut items: Vec<_> = proposals
        .into_iter()
        .filter(|proposal| proposal.id > after)
        .take(PAGE_SIZE + 1)
        .collect();
    let more = items.len() > PAGE_SIZE;
    items.truncate(PAGE_SIZE);
    let next_cursor = if more {
        Some(
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    snapshot,
                    after: items.last().ok_or(EngineError::Store)?.id.clone(),
                })
                .map_err(|_| EngineError::Store)?,
            ),
        )
    } else {
        None
    };
    Ok(Page { items, next_cursor })
}

#[cfg(test)]
mod tests {
    use crate::Provider;
    use crate::accounts::import_suggestions::*;
    fn resource(id: &str, name: &str, kind: ResourceKind, provider: Provider) -> Resource {
        Resource {
            id: id.into(),
            name: name.into(),
            kind,
            provider,
            account_id: format!("{}-account", provider.slug()),
            project_id: None,
            provider_ref: id.into(),
            region: None,
            status: ResourceStatus::Unknown,
            mode: Mode::Watching,
            url: None,
            group: None,
        }
    }

    fn no_projects(_: &Resource) -> Option<String> {
        None
    }

    #[test]
    fn resources_in_one_provider_project_are_suggested_together() {
        let api = resource("p1/api", "api", ResourceKind::Service, Provider::Railway);
        let cache = resource(
            "p1/cache",
            "cache",
            ResourceKind::Database,
            Provider::Railway,
        );
        let by_prefix = |resource: &Resource| {
            resource
                .provider_ref
                .split_once('/')
                .map(|(project, _)| project.to_owned())
        };
        let values = select(&[api.clone(), cache.clone()], &by_prefix, None)
            .unwrap()
            .items;
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].resource_ids.len(), 2);
        assert!(
            select(&[api, cache], &no_projects, None)
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[test]
    fn names_and_exact_hosts_produce_reviewable_alternatives_without_transitive_merging() {
        let mut web = resource(
            "web",
            "moodboard-web",
            ResourceKind::Web,
            Provider::Cloudflare,
        );
        web.url = Some("https://moodboard.example.com/path?unrelated=value".into());
        let database = resource(
            "db",
            "moodboard_database",
            ResourceKind::Database,
            Provider::Neon,
        );
        let zone = resource(
            "zone",
            "moodboard.example.com",
            ResourceKind::Domain,
            Provider::Cloudflare,
        );
        let values = select(&[web.clone(), database, zone], &no_projects, None)
            .unwrap()
            .items;
        assert_eq!(values.len(), 2);
        assert!(
            values
                .iter()
                .all(|value| value.ambiguous && value.resource_ids.len() == 2)
        );
        assert!(
            values
                .iter()
                .all(|value| !value.evidence.value.contains("unrelated"))
        );
        web.url = Some("https://private:credential@moodboard.example.com".into());
        assert!(host(&web).is_none());
        assert!(stem("production").is_none());
        assert!(stem("api").is_none());
    }

    #[test]
    fn duplicate_roles_are_ambiguous_and_assigned_resources_are_excluded() {
        let first = resource(
            "first",
            "moodboard",
            ResourceKind::Web,
            Provider::Cloudflare,
        );
        let mut other = first.clone();
        other.id = "second".into();
        other.account_id = "another-account".into();
        assert!(
            select(&[first.clone(), other.clone()], &no_projects, None)
                .unwrap()
                .items[0]
                .ambiguous
        );
        other.project_id = Some("existing".into());
        assert!(
            select(&[first, other], &no_projects, None)
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[test]
    fn pages_are_deterministic_and_reject_cursors_after_candidate_changes() {
        let mut resources = Vec::new();
        for index in 0..101 {
            resources.push(resource(
                &format!("web-{index}"),
                &format!("project-{index}-web"),
                ResourceKind::Web,
                Provider::Cloudflare,
            ));
            resources.push(resource(
                &format!("db-{index}"),
                &format!("project-{index}-db"),
                ResourceKind::Database,
                Provider::Neon,
            ));
        }
        let first = select(&resources, &no_projects, None).unwrap();
        assert_eq!(first.items.len(), 100);
        resources.reverse();
        let second = select(&resources, &no_projects, first.next_cursor.as_deref()).unwrap();
        assert_eq!(second.items.len(), 1);
        assert!(second.next_cursor.is_none());
        resources[0].project_id = Some("assigned".into());
        assert!(matches!(
            select(&resources, &no_projects, first.next_cursor.as_deref()),
            Err(EngineError::Conflict(_))
        ));
        assert!(select(&resources, &no_projects, Some("invalid")).is_err());
    }
}
