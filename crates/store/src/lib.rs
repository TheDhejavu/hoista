//! What Hoista keeps on this computer and the work it does here: the SQLite
//! database, provider tokens (keychain, 1Password, sealed file, the credential
//! keeper), backups, transfers, webhooks, and building projects in a sandbox.
//! Domain rules live in `hoista-domain`, use cases in `hoista-app`.

// The adapters name the use cases and core types the way the rest of Hoista
// does: `crate::application::…` and `crate::domain::…`.
use hoista_app as application;
use hoista_domain as domain;
use hoista_domain::Provider;
use hoista_system::process;

pub use hoista_providers as providers;
pub use hoista_providers::network_history;
pub use hoista_system::{paths, tool_paths};

// The database.
pub mod backups;
pub mod database_export;
pub mod retention;
pub mod store;

// Provider tokens and where they're kept.
pub mod credentials;
pub mod keeper;
pub mod onepassword;
pub mod sealed_store;
pub mod token_store;

// Moving Hoista's records between computers.
pub mod transfer_crypto;
pub mod transfer_file;
pub mod transfer_seal;

// Building and inspecting a project's source.
pub mod build;
pub mod inspection;
pub mod sandbox;
pub mod source;

// Everything else this computer does for Hoista.
pub mod agent_setup;
pub mod diagnostic_report;
pub mod owner_check;
pub mod webhooks;

mod jsonc;
