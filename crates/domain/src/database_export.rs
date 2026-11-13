//! Copies of a PostgreSQL database saved on this computer, and which database
//! a resource is. Never a connection string or password.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A finished, checked copy of a database (`pg_dump`'s custom format), saved
/// where only this user can read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseArchive {
    /// The copy's id.
    pub id: String,
    /// The database resource it was taken from.
    pub resource_id: String,
    /// The provider's branch the database is on.
    pub branch_id: String,
    /// The database's name.
    pub database_name: String,
    /// Where the file is on this computer. It holds the app's data, so treat it like the database.
    pub path: String,
    /// Its size in bytes.
    #[ts(type = "number")]
    pub bytes: u64,
    /// SHA-256 of the whole file.
    pub sha256: String,
    /// When it finished (RFC 3339).
    pub completed_at: String,
}

/// Which database a resource is, at its provider.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseIdentity {
    /// The provider's branch.
    pub branch_id: String,
    /// The database's name on that branch.
    pub database_name: String,
    /// The role that owns it, which Hoista connects as to make a copy.
    pub owner_name: String,
}

/// A database a delete would remove, with the resource it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseTarget {
    /// The resource.
    pub resource_id: String,
    /// The database at the provider.
    pub database: DatabaseIdentity,
}
