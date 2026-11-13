//! The hosts Hoista has sent requests to lately, for diagnostics.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How the latest request to a host went. It's about that request, not
/// whether the host is healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum NetworkOutcome {
    /// Still waiting for an answer.
    InFlight,
    /// An answer came back. What it said may still have been an error.
    Response,
    /// No answer: the connection failed.
    TransportFailure,
    /// Given up on before an answer came.
    Cancelled,
}

/// What's known about one host: its name and the latest request, never the
/// path, query, headers, body or error text.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct HostObservation {
    /// The host name, such as a provider's API or a deployed app's address.
    pub host: String,
    /// When the latest request started (RFC 3339, UTC).
    pub last_attempt_at: String,
    /// Requests since Hoista started or this entry was last dropped.
    pub attempts: u32,
    /// How the latest request went.
    pub outcome: NetworkOutcome,
    /// The latest HTTP status, when there was one.
    pub http_status: Option<u16>,
}

/// The requests this process has sent. Not a record of all network traffic.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct NetworkReport {
    /// The provider API hosts Hoista may call, whether or not it has.
    pub allowed_api_hosts: Vec<String>,
    /// Up to 64 hosts contacted in the last 24 hours.
    pub recent_hosts: Vec<HostObservation>,
    /// What the report leaves out, and how long it keeps entries.
    pub limitations: Vec<String>,
}
