//! Reading everything a transfer carries in one consistent snapshot, and
//! writing a transfer into an empty database on the other side.
//!
//! Each query asks for one row more than a transfer can hold, so the payload's
//! own check refuses a database that's too big rather than quietly cutting it.

use super::{
    SqliteStore,
    catalog::{append, decode, encode, failure, insert_backup},
};
use crate::{
    application::{
        data::transfer::payload::{
            MAX_ACTIVITY, TRANSFER_VERSION, TransferEvent, TransferPayload, TransferProfile,
            TransferRestoreRecord, TransferRestoreStatus,
        },
        ports::catalog::EngineError,
    },
    domain::backup::{BACKUP_VERSION, Backup, MAX_ACCOUNTS},
};
use std::collections::BTreeMap;

impl SqliteStore {
    pub(super) async fn transfer_snapshot_inner(&self) -> Result<TransferPayload, EngineError> {
        let mut tx = self.pool.begin().await.map_err(failure)?;
        let accounts = sqlx::query!("SELECT data FROM accounts ORDER BY id LIMIT 1001")
            .fetch_all(&mut *tx)
            .await
            .map_err(failure)?
            .into_iter()
            .map(|r| decode(&r.data))
            .collect::<Result<Vec<_>, _>>()?;
        let projects = sqlx::query!("SELECT data FROM projects ORDER BY id LIMIT 10001")
            .fetch_all(&mut *tx)
            .await
            .map_err(failure)?
            .into_iter()
            .map(|r| decode(&r.data))
            .collect::<Result<Vec<_>, _>>()?;
        let resources = sqlx::query!("SELECT data FROM resources ORDER BY id LIMIT 10001")
            .fetch_all(&mut *tx)
            .await
            .map_err(failure)?
            .into_iter()
            .map(|r| decode(&r.data))
            .collect::<Result<Vec<_>, _>>()?;
        let links = sqlx::query!("SELECT data FROM links ORDER BY id LIMIT 50001")
            .fetch_all(&mut *tx)
            .await
            .map_err(failure)?
            .into_iter()
            .map(|r| decode(&r.data))
            .collect::<Result<Vec<_>, _>>()?;
        let custom_domains =
            sqlx::query!("SELECT data FROM custom_domains ORDER BY id LIMIT 10001")
                .fetch_all(&mut *tx)
                .await
                .map_err(failure)?
                .into_iter()
                .map(|r| decode(&r.data))
                .collect::<Result<Vec<_>, _>>()?;
        let environments = sqlx::query!(
            "SELECT data FROM environments WHERE id != 'env_default' ORDER BY id LIMIT 21"
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(failure)?
        .into_iter()
        .map(|r| decode(&r.data))
        .collect::<Result<Vec<_>, _>>()?;
        let settings = sqlx::query!("SELECT data FROM settings WHERE id=1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .map(|r| decode(&r.data))
            .transpose()?
            .unwrap_or_default();
        let profile = sqlx::query!("SELECT name, initials FROM profile WHERE id = 1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .map(|r| TransferProfile {
                name: r.name,
                initials: r.initials,
            });
        // The newest activity, then put back in the order it happened.
        let mut activity: Vec<_> = sqlx::query!(
            "SELECT id,at,actor,project_id,kind,summary FROM events ORDER BY id DESC LIMIT 10001"
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(failure)?
        .into_iter()
        .map(|r| TransferEvent {
            source_id: r.id,
            at: r.at,
            actor: r.actor,
            project_id: r.project_id,
            kind: r.kind,
            summary: r.summary,
        })
        .collect();
        let activity_truncated = activity.len() > MAX_ACTIVITY;
        activity.truncate(MAX_ACTIVITY);
        activity.reverse();
        let ignored_resources = sqlx::query!(
            "SELECT resource_id FROM ignored_resources ORDER BY resource_id LIMIT 10001"
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(failure)?
        .into_iter()
        .map(|r| r.resource_id)
        .collect();
        let rows =
            sqlx::query!("SELECT project_id,data FROM map_layouts ORDER BY project_id LIMIT 10001")
                .fetch_all(&mut *tx)
                .await
                .map_err(failure)?;
        let map_layouts = rows
            .into_iter()
            .map(|r| Ok((r.project_id, decode(&r.data)?)))
            .collect::<Result<BTreeMap<_, _>, EngineError>>()?;
        let payload = TransferPayload {
            map_layouts,
            version: TRANSFER_VERSION,
            created_at: chrono::Utc::now().to_rfc3339(),
            catalog: Backup {
                version: BACKUP_VERSION,
                accounts,
                projects,
                resources,
                links,
                custom_domains,
                settings,
                environments,
            },
            profile,
            ignored_resources,
            activity,
            activity_truncated,
            credentials: vec![],
        };
        payload.validate()?;
        tx.commit().await.map_err(failure)?;
        Ok(payload)
    }

    pub(super) async fn prepare_transfer_restore_inner(
        &self,
        record: &TransferRestoreRecord,
    ) -> Result<(), EngineError> {
        if uuid::Uuid::parse_str(&record.id).is_err()
            || record.credential_references.len() > MAX_ACCOUNTS
            || record.credential_references.iter().any(|r| {
                r.strip_prefix("account/")
                    .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
            })
        {
            return Err(EngineError::Invalid("Invalid transfer recovery record."));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        require_empty(&mut tx).await?;
        if sqlx::query!("SELECT id FROM transfer_restores WHERE status='staging'")
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .is_some()
        {
            return Err(EngineError::Conflict(
                "A restore is pending. Run hoist backups recovery and abandon that attempt before trying another file.",
            ));
        }
        let data = encode(record)?;
        sqlx::query!(
            "INSERT INTO transfer_restores(id,status,data) VALUES(?,'staging',?)",
            record.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "person",
            None,
            "transfer.staging",
            "Prepared credential staging for encrypted restore.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    pub(super) async fn install_transfer_inner(
        &self,
        payload: &TransferPayload,
        record: &TransferRestoreRecord,
    ) -> Result<(), EngineError> {
        payload.validate()?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        require_empty(&mut tx).await?;
        let expected = encode(record)?;
        let row = sqlx::query!(
            "SELECT status,data FROM transfer_restores WHERE id=?",
            record.id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        if row.status != "staging" || row.data != expected {
            return Err(EngineError::Conflict(
                "Restore recovery state changed. Inspect it before retrying.",
            ));
        }
        insert_backup(&mut tx, &payload.catalog).await?;
        for (project_id, layout) in &payload.map_layouts {
            let data = encode(layout)?;
            sqlx::query!(
                "INSERT INTO map_layouts(project_id,revision,data) VALUES(?,?,?)",
                project_id,
                layout.revision,
                data
            )
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        }
        for id in &payload.ignored_resources {
            sqlx::query!(
                "INSERT INTO ignored_resources(resource_id,ignored_at) VALUES(?,?)",
                id,
                record.created_at
            )
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        }
        if let Some(profile) = &payload.profile {
            let at = chrono::Utc::now().to_rfc3339();
            sqlx::query!(
                "INSERT INTO profile(id,name,initials,created_at) VALUES(1,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,initials=excluded.initials",
                profile.name,
                profile.initials,
                at
            )
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        }
        for event in &payload.activity {
            let kind = if event.kind.starts_with("imported.") {
                event.kind.clone()
            } else {
                format!("imported.{}", event.kind)
            };
            sqlx::query!(
                "INSERT INTO events(at,actor,project_id,kind,summary) VALUES(?,?,?,?,?)",
                event.at,
                event.actor,
                event.project_id,
                kind,
                event.summary
            )
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        }
        sqlx::query!(
            "UPDATE transfer_restores SET status='installed' WHERE id=? AND status='staging'",
            record.id
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "person",
            None,
            "transfer.installed",
            "Restored encrypted metadata and staged account credentials in watching mode. Imported activity does not grant agent access.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    pub(super) async fn transfer_restores_inner(
        &self,
    ) -> Result<Vec<TransferRestoreStatus>, EngineError> {
        sqlx::query!("SELECT status,data FROM transfer_restores ORDER BY rowid DESC LIMIT 100")
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?
            .into_iter()
            .map(|r| {
                Ok(TransferRestoreStatus {
                    record: decode(&r.data)?,
                    status: r.status,
                })
            })
            .collect()
    }

    pub(super) async fn abandon_transfer_restore_inner(&self, id: &str) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let changed = sqlx::query!(
            "UPDATE transfer_restores SET status='abandoned' WHERE id=? AND status='staging'",
            id
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        if changed.rows_affected() != 1 {
            return Err(EngineError::Conflict(
                "That restore is not pending. Inspect recovery state.",
            ));
        }
        append(
            &mut tx,
            "person",
            None,
            "transfer.abandoned",
            "Abandoned pending restore; recorded credential references are retained and no credentials were deleted.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }
}

/// Refuses unless the database is empty: a transfer only ever goes into a
/// fresh Hoista.
async fn require_empty(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<(), EngineError> {
    let row = sqlx::query!("SELECT (SELECT count(*) FROM projects)+(SELECT count(*) FROM accounts)+(SELECT count(*) FROM resources)+(SELECT count(*) FROM links)+(SELECT count(*) FROM usage)+(SELECT count(*) FROM plans)+(SELECT count(*) FROM deploys)+(SELECT count(*) FROM agents)+(SELECT count(*) FROM approvals)+(SELECT count(*) FROM secret_metadata)+(SELECT count(*) FROM custom_domains)+(SELECT count(*) FROM deployment_admissions)+(SELECT count(*) FROM environment_admission_keys)+(SELECT count(*) FROM oauth_clients)+(SELECT count(*) FROM oauth_pending)+(SELECT count(*) FROM oauth_grants)+(SELECT count(*) FROM private_link) AS count")
        .fetch_one(&mut **tx)
        .await
        .map_err(failure)?;
    if row.count != 0 {
        return Err(EngineError::Conflict(
            "Restore requires an empty destination without projects, accounts, grants or deployment history. Use a fresh state directory.",
        ));
    }
    Ok(())
}
