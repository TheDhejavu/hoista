//! How Desktop, the `hoist` command line and agents reach Hoista: the local
//! HTTP API, the MCP server (over stdio and HTTP), server mode with passkeys
//! and OAuth, and the client the CLI uses. Also builds the engine from its
//! real adapters (`wiring`) for the daemon and the CLI.
//!
//! Hoista talks to the providers' APIs directly with the person's own tokens;
//! there is no Hoista service in between.

// The rest of Hoista, re-exported so the server code and the two programs
// name it one way: `application` (hoista-app), `domain` (hoista-domain) and
// `infrastructure` (hoista-store).
/// What Hoista does, from the `hoista-app` crate.
pub use hoista_app as application;
pub use hoista_app::{RELEASE_IMAGE, VERSION, is_pinned_image};
/// Hoista's core types and rules, from the `hoista-domain` crate.
pub use hoista_domain as domain;
pub use hoista_domain::{Provider, UnknownProvider};
/// What Hoista keeps and does on this computer, from the `hoista-store` crate.
pub use hoista_store as infrastructure;

// Building the engine, and the client the CLI uses to reach the daemon.
pub mod client;
mod client_logs;
pub mod contracts;
pub mod doctor;
pub mod wiring;

// The owner's local API: the socket or named pipe and the routes behind it.
pub mod approval_display;
pub mod backups;
pub mod catalog;
pub mod diagnostics;
pub mod env_file;
pub mod http;
pub mod lifecycle;
pub mod recovery;
pub mod server;
pub mod transfer;
mod transport;

// Agents: MCP over stdio, and over HTTP with OAuth.
pub mod agent_http;
pub mod link;
pub mod mcp;
pub mod remote;
pub mod stdio;

// Hoista on a server, and the browser pages it and `link` serve.
pub mod cloud;
pub mod pages;
pub mod views;

/// The largest request body an agent or the owner's API reads.
pub(crate) const MAX_REQUEST_BYTES: usize = 128 * 1024;

/// JSON-RPC error codes: the standard ones, then Hoista's own from the range
/// JSON-RPC leaves to servers.
pub(crate) mod rpc_code {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    /// A call before the session's `initialize`.
    pub const NOT_INITIALIZED: i32 = -32000;
    /// Hoista couldn't answer: paused, locked, restarting or too slow.
    pub const UNAVAILABLE: i32 = -32001;
    /// The engine refused an approval decision.
    pub const REFUSED: i32 = -32003;
}
