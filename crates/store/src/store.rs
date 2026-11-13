//! The SQLite database Hoista keeps everything but tokens in. Only one
//! `hoistd` opens it at a time, and the activity log is only ever added to.

mod catalog;
mod migrations;
mod transfer;

use super::paths::{StatePaths, private_file, reject_symlink};
use crate::{
    application::{Event, StateError, StateStore},
    domain::Profile,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{fs::File, sync::Arc, time::Duration};

/// How long a query waits for another connection's write to finish.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: u32 = 4;

/// Why the database couldn't be opened. The messages never include what's in it.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The state folder is in use by another `hoistd`, or isn't safe to use.
    #[error("Cannot own local state: {0}")]
    Filesystem(#[from] std::io::Error),
    /// SQLite couldn't open the file.
    #[error("Cannot open the local database.")]
    Database(#[from] sqlx::Error),
    /// A migration failed. The ones before it stay applied.
    #[error("Cannot migrate the local database.")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

/// The open database. Clones share it, and the lock on the state folder is
/// held until the last clone is dropped.
#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
    _lock: Arc<File>,
}

impl SqliteStore {
    /// Locks the state folder, opens the database and applies any new
    /// migrations.
    ///
    /// # Errors
    /// Fails when another `hoistd` holds the folder, a database file is a
    /// link, or SQLite or a migration fails.
    pub async fn open(paths: &StatePaths) -> Result<Self, StoreError> {
        let lock = Arc::new(paths.lock()?);
        for path in [
            paths.database(),
            paths.root().join("hoista.db-wal"),
            paths.root().join("hoista.db-shm"),
        ] {
            reject_symlink(&path)?;
        }
        drop(private_file(&paths.database())?);
        let options = SqliteConnectOptions::new()
            .filename(paths.database())
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await?;
        migrations::run(&pool).await?;
        Ok(Self { pool, _lock: lock })
    }

    /// A transaction that takes the write lock at once, so what it reads
    /// before writing can't change underneath it. A plain `BEGIN` reads from
    /// a snapshot that SQLite refuses to write from once another write has
    /// committed (`SQLITE_BUSY_SNAPSHOT`).
    async fn begin_write(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
        self.pool.begin_with("BEGIN IMMEDIATE").await
    }

    /// Closes every connection. Call before exiting, so the lock is released last.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

fn unavailable(_: sqlx::Error) -> StateError {
    StateError::Unavailable
}

impl StateStore for SqliteStore {
    async fn profile(&self) -> Result<Option<Profile>, StateError> {
        let row = sqlx::query!("SELECT name, initials FROM profile WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(unavailable)?;
        row.map(|row| Profile::new(row.name, row.initials).map_err(|_| StateError::Invalid))
            .transpose()
    }

    async fn save_profile(&self, profile: &Profile) -> Result<(), StateError> {
        let mut tx = self.begin_write().await.map_err(unavailable)?;
        let name = profile.name();
        let initials = profile.initials();
        let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query!(
            "INSERT INTO profile (id, name, initials, created_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name, initials = excluded.initials",
            name,
            initials,
            at
        )
        .execute(&mut *tx)
        .await
        .map_err(unavailable)?;
        sqlx::query!(
            "INSERT INTO events (at, actor, kind, summary) VALUES (?, 'person', 'profile.updated', 'Updated local profile.')",
            at
        )
        .execute(&mut *tx)
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)
    }

    async fn events_before(
        &self,
        before: Option<i64>,
        actor: Option<&str>,
        project_id: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<Event>, StateError> {
        let before = before.unwrap_or(i64::MAX);
        let rows = sqlx::query!(
            "SELECT id, at, actor, project_id, subject_id, kind, summary FROM events WHERE id < ? AND (? IS NULL OR actor = ?) AND (? IS NULL OR project_id = ?) AND (? IS NULL OR kind = ?) ORDER BY id DESC LIMIT 50",
            before,
            actor,
            actor,
            project_id,
            project_id,
            kind,
            kind
        )
        .fetch_all(&self.pool)
        .await
        .map_err(unavailable)?;
        Ok(rows
            .into_iter()
            .map(|row| Event {
                id: row.id,
                project_id: row.project_id,
                subject_id: row.subject_id,
                at: row.at,
                actor: row.actor,
                kind: row.kind,
                summary: row.summary,
            })
            .collect())
    }

    async fn latest_event_id(&self) -> Result<i64, StateError> {
        let row = sqlx::query!("SELECT coalesce(max(id), 0) AS \"id!: i64\" FROM events")
            .fetch_one(&self.pool)
            .await
            .map_err(unavailable)?;
        Ok(row.id)
    }

    async fn events_after(&self, cursor: i64) -> Result<Vec<Event>, StateError> {
        let rows = sqlx::query!(
            "SELECT id, at, actor, project_id, subject_id, kind, summary FROM events WHERE id > ? ORDER BY id LIMIT 100",
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(unavailable)?;
        Ok(rows
            .into_iter()
            .map(|row| Event {
                id: row.id,
                project_id: row.project_id,
                subject_id: row.subject_id,
                at: row.at,
                actor: row.actor,
                kind: row.kind,
                summary: row.summary,
            })
            .collect())
    }
}
