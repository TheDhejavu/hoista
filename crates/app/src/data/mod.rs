//! Hoista's own data: backups, exporting a project's databases, and moving everything
//! to another computer or a remote in an encrypted transfer.

pub(crate) mod backup;
pub(crate) mod database_export;
pub mod transfer;

mod service;
pub use service::DataService;
