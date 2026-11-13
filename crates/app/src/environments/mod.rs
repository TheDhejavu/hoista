//! Environments, their variables and secrets, domains, and admission keys.

pub mod admission_keys;
pub(crate) mod domains;
pub(crate) mod environment;
pub(crate) mod secrets;

mod service;
pub use service::EnvironmentService;
