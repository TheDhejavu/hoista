//! Hoista's contact with the operating system: running programs, finding them
//! on this computer, where Hoista keeps its files, and reading files safely.
//!
//! Nothing here knows about providers, projects or deploys.

pub mod channel;
pub mod files;
pub mod paths;
pub mod process;
pub mod tool_paths;
