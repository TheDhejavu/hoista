//! A backup of what Hoista knows about your projects. It has no field for a
//! token, an agent or a secret value, so none can end up in one.

use super::model::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The backup format this build writes and reads.
pub const BACKUP_VERSION: u32 = 1;

/// The most accounts one backup can hold.
pub const MAX_ACCOUNTS: usize = 1_000;
/// The most projects one backup can hold.
pub const MAX_PROJECTS: usize = 10_000;
/// The most resources one backup can hold.
pub const MAX_RESOURCES: usize = 10_000;
/// The most map links one backup can hold.
pub const MAX_LINKS: usize = 50_000;
/// The most custom domains one backup can hold.
pub const MAX_CUSTOM_DOMAINS: usize = 10_000;

/// Everything a backup holds. Restored resources always start out watched,
/// not managed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    /// The backup format's version.
    pub version: u32,
    /// Connected accounts, without their tokens. Each one is reconnected after a restore.
    pub accounts: Vec<Account>,
    /// Projects and where their source is.
    pub projects: Vec<Project>,
    /// The resources in those projects.
    pub resources: Vec<Resource>,
    /// The links between resources on the project map.
    pub links: Vec<Link>,
    /// Custom domains and the resources they point at.
    pub custom_domains: Vec<super::domains::CustomDomain>,
    /// Settings.
    pub settings: Settings,
    /// Environments other than `default`.
    pub environments: Vec<Environment>,
}
