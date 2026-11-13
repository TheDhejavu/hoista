//! A provider database's branches and computes, as its management API
//! reports them. Reading these never connects to the database or wakes it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One branch of a database.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseBranch {
    /// The provider's id for it.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The branch it was made from, which may since have been deleted.
    pub parent_id: Option<String>,
    /// The provider's word for its state. Whether it's running is on the compute.
    pub state: String,
    /// Whether the provider marks it as the default branch.
    pub is_default: Option<bool>,
    /// How much data it holds, in bytes. Not what's billed for storage.
    #[ts(type = "number | null")]
    pub logical_size_bytes: Option<i64>,
}

/// One compute: the server a branch runs on.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseCompute {
    /// The provider's id for it.
    pub id: String,
    /// The branch it serves.
    pub branch_id: String,
    /// The provider's kind, such as `read_write` or `read_only`.
    pub kind: String,
    /// Its state. `idle` means asleep, not broken.
    pub state: String,
    /// The state it's changing to, while it changes.
    pub pending_state: Option<String>,
    /// Its region.
    pub region: String,
    /// The fewest compute units it's set to use. A setting, not a measurement.
    pub min_compute_units: Option<f64>,
    /// The most compute units it's set to use. A setting, not a measurement.
    pub max_compute_units: Option<f64>,
    /// How long it waits idle before sleeping, in seconds. `0` is the
    /// provider's default; `-1` means it never sleeps.
    #[ts(type = "number | null")]
    pub suspend_timeout_seconds: Option<i64>,
}

/// A database's branches and computes. They come from two separate calls, so
/// one can be a moment newer than the other.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseObservation {
    /// When both calls had finished (RFC 3339).
    pub observed_at: String,
    /// The branches that exist now. Deleted ones are left out.
    pub branches: Vec<DatabaseBranch>,
    /// The computes.
    pub computes: Vec<DatabaseCompute>,
    /// How many times it woke up, when the provider reports it.
    #[ts(type = "number | null")]
    pub wake_count: Option<i64>,
}

/// One thing the provider did to the database, such as starting or suspending
/// a compute. No error text or connection details.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseOperation {
    /// The provider's id. The same one can come back on the next page.
    pub id: String,
    /// The branch, which may no longer exist.
    pub branch_id: Option<String>,
    /// The compute, which may no longer exist.
    pub compute_id: Option<String>,
    /// What was done, in the provider's words, such as `start_compute`.
    pub action: String,
    /// How it went. `failed` can still be retried by the provider.
    pub status: String,
    /// When the provider started it.
    pub created_at: String,
    /// When it last changed, which isn't necessarily when it finished.
    pub updated_at: String,
}

/// One page of a database's operations. Providers only keep so much history,
/// so pages never add up to a complete record.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DatabaseOperationHistory {
    /// When the provider answered (RFC 3339).
    pub observed_at: String,
    /// Up to 100 operations.
    pub items: Vec<DatabaseOperation>,
    /// Pass this to get the next page. `null` means the provider gave no next page.
    pub next_cursor: Option<String>,
    /// How many computes finished starting on this page alone.
    pub completed_starts_in_page: usize,
    /// Always `false`: this is never the whole history.
    pub complete_history: bool,
}
