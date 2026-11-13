//! What the webview can ask of the Rust side. Each command is a thin
//! wrapper; the work is in the modules they call.

pub mod app;
pub mod engine;
pub mod remotes;
pub mod windows;
