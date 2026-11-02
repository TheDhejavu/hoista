//! The Hoista engine.
//!
//! Everything here runs on the user's computer. The engine talks to provider
//! APIs directly with tokens held in the operating system keychain; there is no
//! Hoista server in between. The `hoist` CLI, the MCP server and Hoista Desktop
//! are thin front ends over this crate.

mod provider;

pub use provider::{Provider, UnknownProvider};

/// Version of the engine, as published in its manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
