//! The data service: backups, database exports and encrypted transfers.

use crate::engine::Engine;

/// Backups, database exports and encrypted transfers.
#[derive(Clone, Copy)]
pub struct DataService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Backups, database exports and encrypted transfers.
    #[must_use]
    pub fn data(&self) -> DataService<'_> {
        DataService { engine: self }
    }
}
