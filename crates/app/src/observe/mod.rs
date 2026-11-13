//! Seeing what happened: logs, account diagnostics, the diagnostic report and the
//! operation journal.

pub mod diagnostic_report;
pub(crate) mod diagnostics;
pub(crate) mod journal;
pub(crate) mod logs;

mod service;
pub use service::ObserveService;
