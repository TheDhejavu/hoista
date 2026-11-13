//! Database migrations.

use sqlx::{SqlitePool, migrate::Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Applies pending migrations.
///
/// # Errors
/// Fails if SQLx rejects a migration, including a changed checksum.
pub(super) async fn run(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}
