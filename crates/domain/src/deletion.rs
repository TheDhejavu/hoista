//! What deleting a project or a resource would remove, shown to the person
//! before they confirm. The confirmation is tied to exactly what they saw.

use super::model::{GatedAction, Link, ResourceKind};
use crate::Provider;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A resource at a provider that the delete would touch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct DeletionResource {
    /// Its id in Hoista.
    pub id: String,
    /// Its project, which can be another project when the resource is shared.
    pub project_id: Option<String>,
    /// Its name.
    pub name: String,
    /// What it is. A database's data is gone for good unless it's copied first.
    pub kind: ResourceKind,
    /// Its provider.
    pub provider: Provider,
    /// The account it's in.
    pub account_id: String,
    /// What the provider calls it: exactly what will be deleted there.
    pub provider_ref: String,
    /// Its public address, so the person sees what stops answering.
    pub url: Option<String>,
}

/// A custom domain that would stop pointing at the deleted resource. The
/// domain itself and its DNS zone stay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct DeletionDomain {
    /// Its id in Hoista.
    pub id: String,
    /// The domain.
    pub hostname: String,
    /// The resource it points at now.
    pub resource_id: String,
    /// Its DNS zone at the provider, to check the domain is still the person's.
    pub zone_id: String,
}

/// Everything Hoista knows a delete would affect. It covers what Hoista has
/// recorded, not everything in the provider account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct DeletionPreview {
    /// `DeleteProject` or `DeleteResource`.
    pub action: GatedAction,
    /// The project or resource being deleted.
    pub target: String,
    /// Its project.
    pub project_id: String,
    /// The newest deploy when this was shown. A newer deploy means showing it again.
    pub latest_deploy_id: Option<String>,
    /// The name the person types to confirm.
    pub confirmation_name: String,
    /// What gets deleted, in the order it will be: databases last.
    pub resources: Vec<DeletionResource>,
    /// Resources that stay but depend on something being deleted.
    pub affected_resources: Vec<DeletionResource>,
    /// Every database on each database resource being deleted.
    pub databases: Vec<super::database_export::DatabaseTarget>,
    /// Custom domains that stop pointing anywhere. The domains themselves stay.
    pub domains: Vec<DeletionDomain>,
    /// The map links that touch what's being deleted, including ones marked missing.
    pub links: Vec<Link>,
    /// Why it can't be deleted right now. None doesn't mean the data is saved anywhere.
    pub blockers: Vec<String>,
    /// Whether this build of Hoista can copy a database before deleting it.
    pub database_export_available: bool,
    /// A hash of all of the above. The delete is refused if anything has changed since.
    pub fingerprint: String,
}

/// The person's confirmation, for exactly the preview they saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DeletionConfirmation {
    /// Copy the databases before deleting. On unless the person turns it off,
    /// which only they can, and only when deleting a whole project.
    #[serde(default = "copy_databases_first")]
    pub backup_databases: bool,
    /// The name they typed, exactly as shown.
    pub confirmation_name: String,
    /// The preview's `fingerprint`.
    pub fingerprint: String,
}

fn copy_databases_first() -> bool {
    true
}
