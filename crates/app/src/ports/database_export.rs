//! Copying a database's data out to an archive on this computer. Separate
//! from backups of Hoista's own records.

use crate::domain::{Secret, database_export::DatabaseArchive};
use crate::ports::catalog::{EngineError, Page};

/// One database the provider listed, and how to reach it directly (not
/// through a pooler).
pub struct DatabaseExportSource {
    /// The provider's branch id. Two branches can hold databases with the same name.
    pub branch_id: String,
    /// The database's name, as the provider reports it.
    pub database_name: String,
    /// The connection address, password included. Only ever held in memory.
    pub connection: Secret,
}

/// Exports databases to private archives.
#[async_trait::async_trait]
pub trait DatabaseExporter: Send + Sync {
    /// Whether the tools to export and check an archive were found.
    fn available(&self) -> bool;

    /// The archives made so far, by id, a page at a time.
    ///
    /// # Errors
    /// Fails for a bad cursor, a damaged receipt, or a folder that can't be read.
    async fn archives(&self, cursor: Option<&str>) -> Result<Page<DatabaseArchive>, EngineError>;

    /// Exports one database and checks the archive before keeping it. The
    /// password never appears in a command line or a log.
    ///
    /// # Errors
    /// Fails for missing tools, a connection that isn't allowed, an archive
    /// over the limits, or a failed export or check.
    async fn export(
        &self,
        resource_id: &str,
        source: &DatabaseExportSource,
    ) -> Result<DatabaseArchive, EngineError>;
}
