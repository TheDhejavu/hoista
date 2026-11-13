//! Hoista's records in SQLite: accounts, projects, resources, deploys,
//! agents and the rest. Each record is kept as JSON in its table's `data`
//! column, and never holds a token or a secret value.

use super::SqliteStore;
use crate::application::data::transfer::payload::TransferRestoreStatus;
use crate::{
    application::ports::catalog::{
        CatalogRepository, DeploymentAdmission, EngineError, EnvironmentAdmissionKey,
        EnvironmentKeySnapshot, Page,
    },
    domain::model::*,
};
use serde::{Serialize, de::DeserializeOwned};

pub(super) fn encode<T: Serialize>(value: &T) -> Result<String, EngineError> {
    serde_json::to_string(value).map_err(|_| EngineError::Store)
}

pub(super) fn decode<T: DeserializeOwned>(data: &str) -> Result<T, EngineError> {
    serde_json::from_str(data).map_err(|_| EngineError::Store)
}

fn label<T: Serialize>(value: &T) -> Result<String, EngineError> {
    serde_json::to_value(value)
        .map_err(|_| EngineError::Store)?
        .as_str()
        .map(str::to_owned)
        .ok_or(EngineError::Store)
}

pub(super) fn failure(_: sqlx::Error) -> EngineError {
    EngineError::Store
}

/// Rows per page. Page queries ask for one more, to tell whether there's a
/// next page.
const PAGE_SIZE: usize = 100;
const OPERATIONS_PAGE_SIZE: usize = 50;

// The most rows read at once where the answer has to be whole. Most queries
// ask for one more, so going over is caught rather than cut short.
const MAX_IGNORED_RESOURCES: usize = 10_000;
const MAX_PROJECT_RESOURCES: usize = 10_000;
const MAX_LINKS: usize = 10_000;
const MAX_PROJECT_LINKS: usize = 1_000;
const MAX_RESOURCE_LINKS: usize = 1_000;
const MAX_USAGE_ROWS: usize = 10_000;
/// The most bindings one refresh of a resource can report.
const MAX_OBSERVED_BINDINGS: usize = 500;
const MAX_QUEUED_DEPLOYS: i64 = 100;
const MAX_PENDING_AGENTS: i64 = 100;
const MAX_AGENTS: i64 = 1_000;

/// Drops the extra row a page query asked for, and says whether it was there.
fn trim_page<T>(rows: &mut Vec<T>) -> bool {
    let more = rows.len() > PAGE_SIZE;
    rows.truncate(PAGE_SIZE);
    more
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) async fn append(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: &str,
    project_id: Option<&str>,
    kind: &str,
    summary: &str,
) -> Result<(), EngineError> {
    append_about(tx, actor, project_id, None, kind, summary).await
}

/// Appends an event that names the record it is about, inside the caller's
/// transaction so the event commits with the change it describes.
pub(super) async fn append_about(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: &str,
    project_id: Option<&str>,
    subject_id: Option<&str>,
    kind: &str,
    summary: &str,
) -> Result<(), EngineError> {
    let at = now();
    sqlx::query!(
        "INSERT INTO events (at, actor, project_id, subject_id, kind, summary) VALUES (?, ?, ?, ?, ?, ?)",
        at,
        actor,
        project_id,
        subject_id,
        kind,
        summary
    )
    .execute(&mut **tx)
    .await
    .map_err(failure)?;
    Ok(())
}

#[async_trait::async_trait]
impl CatalogRepository for SqliteStore {
    async fn prune_history(&self, before: &str) -> Result<u64, EngineError> {
        // The database itself refuses to remove recent activity.
        let floor = (chrono::Utc::now()
            - chrono::Duration::days(i64::from(crate::retention::MIN_HISTORY_DAYS)))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let before = if before < floor.as_str() {
            before
        } else {
            floor.as_str()
        };
        let mut transaction = self.pool.begin().await.map_err(failure)?;
        let mut removed = 0;
        for statement in [
            "DELETE FROM events WHERE at < ?",
            "DELETE FROM operations WHERE started_at < ? AND state NOT IN ('running','unclear')",
            "DELETE FROM webhook_deliveries WHERE next_at < ? AND state != 'pending'",
        ] {
            removed += sqlx::query(statement)
                .bind(before)
                .execute(&mut *transaction)
                .await
                .map_err(failure)?
                .rows_affected();
        }
        transaction.commit().await.map_err(failure)?;
        Ok(removed)
    }

    async fn latest_event_id(&self) -> Result<i64, EngineError> {
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM events")
            .fetch_one(&self.pool)
            .await
            .map(Option::unwrap_or_default)
            .map_err(failure)
    }

    async fn webhooks(&self) -> Result<Vec<(crate::domain::model::Webhook, i64)>, EngineError> {
        use sqlx::Row as _;
        let rows = sqlx::query("SELECT cursor, data FROM webhooks ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        rows.iter()
            .map(|row| {
                Ok((
                    decode(&row.try_get::<String, _>("data").map_err(failure)?)?,
                    row.try_get::<i64, _>("cursor").map_err(failure)?,
                ))
            })
            .collect()
    }

    async fn put_webhook(
        &self,
        value: &crate::domain::model::Webhook,
        cursor: i64,
    ) -> Result<(), EngineError> {
        sqlx::query("INSERT INTO webhooks(id,cursor,data) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data")
            .bind(&value.id)
            .bind(cursor)
            .bind(encode(value)?)
            .execute(&self.pool)
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn delete_webhook(&self, id: &str) -> Result<bool, EngineError> {
        let mut transaction = self.pool.begin().await.map_err(failure)?;
        sqlx::query("DELETE FROM webhook_deliveries WHERE webhook_id=?")
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(failure)?;
        let removed = sqlx::query("DELETE FROM webhooks WHERE id=?")
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(failure)?
            .rows_affected();
        transaction.commit().await.map_err(failure)?;
        Ok(removed > 0)
    }

    async fn queue_deliveries(
        &self,
        webhook_id: &str,
        deliveries: &[crate::domain::model::WebhookDelivery],
        cursor: i64,
    ) -> Result<(), EngineError> {
        let mut transaction = self.pool.begin().await.map_err(failure)?;
        for delivery in deliveries {
            let event: i64 = delivery.event_id.parse().map_err(|_| EngineError::Store)?;
            sqlx::query("INSERT INTO webhook_deliveries(id,webhook_id,event_id,state,next_at,data) VALUES(?,?,?,?,?,?) ON CONFLICT(webhook_id,event_id) DO NOTHING")
                .bind(&delivery.id)
                .bind(webhook_id)
                .bind(event)
                .bind("pending")
                .bind(&delivery.next_attempt_at)
                .bind(encode(delivery)?)
                .execute(&mut *transaction)
                .await
                .map_err(failure)?;
        }
        sqlx::query("UPDATE webhooks SET cursor=? WHERE id=? AND cursor<?")
            .bind(cursor)
            .bind(webhook_id)
            .bind(cursor)
            .execute(&mut *transaction)
            .await
            .map_err(failure)?;
        transaction.commit().await.map_err(failure)?;
        Ok(())
    }

    async fn due_deliveries(
        &self,
        now: &str,
        limit: u32,
    ) -> Result<Vec<crate::domain::model::WebhookDelivery>, EngineError> {
        use sqlx::Row as _;
        let rows = sqlx::query("SELECT data FROM webhook_deliveries WHERE state='pending' AND next_at<=? ORDER BY next_at, event_id LIMIT ?")
            .bind(now)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        rows.iter()
            .map(|row| decode(&row.try_get::<String, _>("data").map_err(failure)?))
            .collect()
    }

    async fn put_delivery(
        &self,
        value: &crate::domain::model::WebhookDelivery,
    ) -> Result<(), EngineError> {
        let state = encode(&value.state)?;
        sqlx::query("UPDATE webhook_deliveries SET state=?, next_at=?, data=? WHERE id=?")
            .bind(state.trim_matches('"'))
            .bind(&value.next_attempt_at)
            .bind(encode(value)?)
            .bind(&value.id)
            .execute(&self.pool)
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn deliveries(
        &self,
        webhook_id: &str,
        limit: u32,
    ) -> Result<Vec<crate::domain::model::WebhookDelivery>, EngineError> {
        use sqlx::Row as _;
        let rows = sqlx::query(
            "SELECT data FROM webhook_deliveries WHERE webhook_id=? ORDER BY event_id DESC LIMIT ?",
        )
        .bind(webhook_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        rows.iter()
            .map(|row| decode(&row.try_get::<String, _>("data").map_err(failure)?))
            .collect()
    }

    async fn put_operation(
        &self,
        value: &crate::domain::model::Operation,
    ) -> Result<(), EngineError> {
        let state = encode(&value.state)?;
        sqlx::query("INSERT INTO operations(id,key,state,started_at,data) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,data=excluded.data")
            .bind(&value.id)
            .bind(&value.key)
            .bind(state.trim_matches('"'))
            .bind(&value.started_at)
            .bind(encode(value)?)
            .execute(&self.pool)
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn operation(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::model::Operation>, EngineError> {
        use sqlx::Row as _;
        let row = sqlx::query("SELECT data FROM operations WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| decode(&row.try_get::<String, _>("data").map_err(failure)?))
            .transpose()
    }

    async fn open_operation(
        &self,
        key: &str,
    ) -> Result<Option<crate::domain::model::Operation>, EngineError> {
        use sqlx::Row as _;
        let row = sqlx::query("SELECT data FROM operations WHERE key=? AND state IN ('running','unclear') ORDER BY started_at DESC, id DESC LIMIT 1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| decode(&row.try_get::<String, _>("data").map_err(failure)?))
            .transpose()
    }

    async fn operations(
        &self,
        cursor: Option<&str>,
    ) -> Result<Page<crate::domain::model::Operation>, EngineError> {
        use sqlx::Row as _;
        // The cursor is the last row's `started_at` and id.
        let (at, id) = match cursor {
            Some(cursor) => cursor
                .split_once('|')
                .map(|(at, id)| (at.to_owned(), id.to_owned()))
                .ok_or(EngineError::Invalid("That page cursor isn't valid."))?,
            None => ("\u{10ffff}".to_owned(), String::new()),
        };
        let rows = sqlx::query("SELECT data FROM operations WHERE started_at < ? OR (started_at = ? AND id < ?) ORDER BY started_at DESC, id DESC LIMIT 51")
            .bind(&at)
            .bind(&at)
            .bind(&id)
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        let mut items = rows
            .iter()
            .map(|row| decode(&row.try_get::<String, _>("data").map_err(failure)?))
            .collect::<Result<Vec<crate::domain::model::Operation>, _>>()?;
        let next_cursor = if items.len() > OPERATIONS_PAGE_SIZE {
            items.truncate(OPERATIONS_PAGE_SIZE);
            items
                .last()
                .map(|last| format!("{}|{}", last.started_at, last.id))
        } else {
            None
        };
        Ok(Page { items, next_cursor })
    }

    async fn map_layout(
        &self,
        project_id: &str,
    ) -> Result<crate::domain::map_layout::MapLayout, EngineError> {
        let row = sqlx::query!(
            "SELECT data FROM map_layouts WHERE project_id = ?",
            project_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(failure)?;
        row.map(|r| decode(&r.data))
            .transpose()
            .map(|v| v.unwrap_or_default())
    }

    async fn put_map_layout(
        &self,
        actor: &str,
        project_id: &str,
        mut value: crate::domain::map_layout::MapLayout,
    ) -> Result<crate::domain::map_layout::MapLayout, EngineError> {
        if !value.valid() || value.revision == 9_007_199_254_740_991 {
            return Err(EngineError::Invalid(
                "Use at most 1,000 unique map nodes with bounded finite coordinates and a valid revision.",
            ));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        if sqlx::query!("SELECT id FROM projects WHERE id = ?", project_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .is_none()
        {
            return Err(EngineError::NotFound);
        }
        let current = sqlx::query!(
            "SELECT revision FROM map_layouts WHERE project_id = ?",
            project_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .map_or(0, |r| r.revision);
        if current != value.revision {
            return Err(EngineError::Conflict(
                "The map layout changed. Refresh and review before saving again.",
            ));
        }
        value.revision += 1;
        let data = encode(&value)?;
        sqlx::query!(
            "INSERT INTO map_layouts(project_id,revision,data) VALUES(?,?,?) ON CONFLICT(project_id) DO UPDATE SET revision=excluded.revision,data=excluded.data",
            project_id,
            value.revision,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            actor,
            Some(project_id),
            "map.layout_changed",
            "Changed map positions without changing cloud relationships.",
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(value)
    }

    async fn import_candidates(
        &self,
        ignored: bool,
        cursor: Option<&str>,
    ) -> Result<Page<Resource>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT p.id,p.data FROM resources p WHERE p.project_id IS NULL AND p.id > ? AND EXISTS(SELECT 1 FROM ignored_resources i WHERE i.resource_id=p.id)=? ORDER BY p.id LIMIT 101",
            cursor,
            ignored
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|r| r.id.clone())
        } else {
            None
        };
        Ok(Page {
            items: rows
                .into_iter()
                .map(|r| decode(&r.data))
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    async fn ignored_resources(&self) -> Result<Vec<String>, EngineError> {
        let rows = sqlx::query!(
            "SELECT resource_id FROM ignored_resources ORDER BY resource_id LIMIT 10001"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        if rows.len() > MAX_IGNORED_RESOURCES {
            return Err(EngineError::Conflict(
                "Ignored-resource catalog exceeds 10,000 entries.",
            ));
        }
        Ok(rows.into_iter().map(|r| r.resource_id).collect())
    }

    async fn set_resource_ignored(
        &self,
        actor: &str,
        id: &str,
        ignored: bool,
    ) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let resource = sqlx::query!("SELECT project_id FROM resources WHERE id=?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        if resource.project_id.is_some() {
            return Err(EngineError::Conflict(
                "Only resources not in a project can be hidden or shown again.",
            ));
        }
        let changed = if ignored {
            let at = now();
            sqlx::query!(
                "INSERT INTO ignored_resources(resource_id,ignored_at) VALUES(?,?) ON CONFLICT(resource_id) DO NOTHING",
                id,
                at
            )
            .execute(&mut *tx)
            .await
            .map_err(failure)?
            .rows_affected()
        } else {
            sqlx::query!("DELETE FROM ignored_resources WHERE resource_id=?", id)
                .execute(&mut *tx)
                .await
                .map_err(failure)?
                .rows_affected()
        };
        if changed > 0 {
            append(
                &mut tx,
                actor,
                None,
                if ignored {
                    "resource.ignored"
                } else {
                    "resource.unignored"
                },
                if ignored {
                    "Set aside a discovered resource locally; cloud state is unchanged."
                } else {
                    "Showed a hidden resource again; nothing changed at the provider."
                },
            )
            .await?;
        }
        tx.commit().await.map_err(failure)
    }

    async fn prepare_transfer_restore(
        &self,
        record: &crate::application::data::transfer::payload::TransferRestoreRecord,
    ) -> Result<(), EngineError> {
        self.prepare_transfer_restore_inner(record).await
    }

    async fn install_transfer(
        &self,
        payload: &crate::application::data::transfer::payload::TransferPayload,
        record: &crate::application::data::transfer::payload::TransferRestoreRecord,
    ) -> Result<(), EngineError> {
        self.install_transfer_inner(payload, record).await
    }

    async fn transfer_restores(&self) -> Result<Vec<TransferRestoreStatus>, EngineError> {
        self.transfer_restores_inner().await
    }

    async fn abandon_transfer_restore(&self, id: &str) -> Result<(), EngineError> {
        self.abandon_transfer_restore_inner(id).await
    }

    async fn transfer_snapshot(
        &self,
    ) -> Result<crate::application::data::transfer::payload::TransferPayload, EngineError> {
        self.transfer_snapshot_inner().await
    }

    async fn account(&self, id: &str) -> Result<Option<Account>, EngineError> {
        let row = sqlx::query!("SELECT data FROM accounts WHERE id = ?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| decode(&row.data)).transpose()
    }

    async fn accounts(&self, cursor: Option<&str>) -> Result<Page<Account>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT id, data FROM accounts WHERE id > ? ORDER BY id LIMIT 101",
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.id.clone())
        } else {
            None
        };
        Ok(Page {
            items: rows
                .into_iter()
                .map(|row| decode(&row.data))
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    async fn put_account(&self, value: &Account) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        let provider = value.provider.slug();
        sqlx::query!(
            "INSERT INTO accounts (id, provider, data) VALUES (?, ?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            provider,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            "engine",
            None,
            Some(&value.id),
            "account.updated",
            "Updated account metadata.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn project(&self, id: &str) -> Result<Option<Project>, EngineError> {
        let row = sqlx::query!("SELECT data FROM projects WHERE id = ?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| decode(&row.data)).transpose()
    }

    async fn projects(&self, cursor: Option<&str>) -> Result<Page<Project>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT id, data FROM projects WHERE id > ? ORDER BY id LIMIT 101",
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.id.clone())
        } else {
            None
        };
        Ok(Page {
            items: rows
                .into_iter()
                .map(|row| decode(&row.data))
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    async fn put_project(&self, value: &Project) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "INSERT INTO projects (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "engine",
            Some(value.id.as_str()),
            "app.updated",
            "Updated project metadata.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn create_project(&self, actor: &str, value: &Project) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        create_project_in_transaction(&mut tx, actor, value).await?;
        tx.commit().await.map_err(failure)
    }

    async fn environments(&self) -> Result<Vec<Environment>, EngineError> {
        let rows = sqlx::query!("SELECT data FROM environments")
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        let mut items = rows
            .into_iter()
            .map(|row| decode::<Environment>(&row.data))
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_by(|a, b| {
            (a.id != DEFAULT_ENVIRONMENT)
                .cmp(&(b.id != DEFAULT_ENVIRONMENT))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(items)
    }

    async fn create_environment(
        &self,
        actor: &str,
        value: &Environment,
    ) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let taken = sqlx::query!(
            "SELECT id FROM environments WHERE id = ? OR json_extract(data, '$.name') = ?",
            value.id,
            value.name
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?;
        if taken.is_some() {
            return Err(EngineError::Conflict(
                "An environment already has that name.",
            ));
        }
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO environments (id, data) VALUES (?, ?)",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            actor,
            None,
            Some(&value.id),
            "environment.created",
            &format!("Created the {} environment.", value.name),
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn rename_environment(
        &self,
        actor: &str,
        id: &str,
        name: &str,
    ) -> Result<Environment, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!("SELECT data FROM environments WHERE id = ?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut value: Environment = decode(&row.data)?;
        let taken = sqlx::query!(
            "SELECT id FROM environments WHERE id != ? AND json_extract(data, '$.name') = ?",
            id,
            name
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?;
        if taken.is_some() {
            return Err(EngineError::Conflict(
                "An environment already has that name.",
            ));
        }
        let before = std::mem::replace(&mut value.name, name.to_owned());
        let data = encode(&value)?;
        sqlx::query!("UPDATE environments SET data = ? WHERE id = ?", data, id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        append_about(
            &mut tx,
            actor,
            None,
            Some(id),
            "environment.renamed",
            &format!("Renamed the {before} environment to {name}."),
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(value)
    }

    async fn remove_environment(&self, actor: &str, id: &str) -> Result<(), EngineError> {
        if id == DEFAULT_ENVIRONMENT {
            return Err(EngineError::Conflict(
                "The default environment can't be removed.",
            ));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!("SELECT data FROM environments WHERE id = ?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let value: Environment = decode(&row.data)?;
        let used = sqlx::query!(
            "SELECT id FROM projects WHERE json_extract(data, '$.environment_id') = ? LIMIT 1",
            id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?;
        if used.is_some() {
            return Err(EngineError::Conflict(
                "Move or remove the projects in this environment first.",
            ));
        }
        sqlx::query!("DELETE FROM environments WHERE id = ?", id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        append_about(
            &mut tx,
            actor,
            None,
            Some(id),
            "environment.removed",
            &format!("Removed the {} environment.", value.name),
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn move_project(
        &self,
        actor: &str,
        project_id: &str,
        environment_id: &str,
    ) -> Result<Project, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let environment =
            sqlx::query!("SELECT data FROM environments WHERE id = ?", environment_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(failure)?
                .ok_or(EngineError::NotFound)?;
        let environment: Environment = decode(&environment.data)?;
        let row = sqlx::query!("SELECT data FROM projects WHERE id = ?", project_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut value: Project = decode(&row.data)?;
        if value.environment_id == environment_id {
            return Ok(value);
        }
        let clash = sqlx::query!(
            "SELECT id FROM projects WHERE id != ? AND json_extract(data, '$.name') = ? AND coalesce(json_extract(data, '$.environment_id'), 'env_default') = ?",
            project_id,
            value.name,
            environment_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?;
        if clash.is_some() {
            return Err(EngineError::Conflict(
                "That environment already has a project of this name. Rename one first.",
            ));
        }
        value.environment_id = environment_id.to_owned();
        let data = encode(&value)?;
        sqlx::query!(
            "UPDATE projects SET data = ? WHERE id = ?",
            data,
            project_id
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            actor,
            Some(project_id),
            "project.environment_changed",
            &format!("Moved the project to the {} environment.", environment.name),
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(value)
    }

    async fn import_project(
        &self,
        actor: &str,
        value: &Project,
        ids: &[String],
    ) -> Result<Vec<Resource>, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        create_project_in_transaction(&mut tx, actor, value).await?;
        let resources = group_in_transaction(&mut tx, actor, &value.id, ids, value.mode).await?;
        tx.commit().await.map_err(failure)?;
        Ok(resources)
    }

    async fn link_source(
        &self,
        actor: &str,
        project_id: &str,
        source: &Source,
    ) -> Result<Project, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!("SELECT data FROM projects WHERE id = ?", project_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut value: Project = decode(&row.data)?;
        // A watched project's code can be linked again; a managed project keeps the
        // source its deploys were planned from.
        if value.source != Source::Unlinked && value.mode == Mode::Managed {
            return Err(EngineError::Conflict(
                "This project already has a linked source.",
            ));
        }
        // Only a watched project can be disconnected from its code.
        if *source == Source::Unlinked && value.mode == Mode::Managed {
            return Err(EngineError::Invalid(
                "Choose a folder or pinned GitHub source.",
            ));
        }
        let unlinking = *source == Source::Unlinked;
        value.source = source.clone();
        let data = encode(&value)?;
        sqlx::query!(
            "INSERT INTO projects (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            actor,
            Some(project_id),
            if unlinking {
                "app.source_unlinked"
            } else {
                "app.source_linked"
            },
            if unlinking {
                "Disconnected the project's source; nothing changed at the provider."
            } else {
                "Linked a deployment source without changing provider resources."
            },
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(value)
    }

    async fn resource(&self, id: &str) -> Result<Option<Resource>, EngineError> {
        let row = sqlx::query!("SELECT data FROM resources WHERE id = ?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| decode(&row.data)).transpose()
    }

    async fn resources(&self, cursor: Option<&str>) -> Result<Page<Resource>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT id, data FROM resources WHERE id > ? ORDER BY id LIMIT 101",
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.id.clone())
        } else {
            None
        };
        Ok(Page {
            items: rows
                .into_iter()
                .map(|row| decode(&row.data))
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    async fn put_resource(&self, value: &Resource) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        let account = sqlx::query!(
            "SELECT provider FROM accounts WHERE id = ?",
            value.account_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        if account.provider != value.provider.slug() {
            return Err(EngineError::Invalid(
                "Resource provider must match its connected account.",
            ));
        }
        sqlx::query!(
            "INSERT INTO resources (id, project_id, account_id, provider_ref, data) VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET project_id = excluded.project_id, account_id = excluded.account_id, provider_ref = excluded.provider_ref, data = excluded.data",
            value.id,
            value.project_id,
            value.account_id,
            value.provider_ref,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            "engine",
            value.project_id.as_deref(),
            Some(&value.id),
            "resource.updated",
            "Updated resource metadata.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn set_project_mode(&self, project_id: &str, mode: Mode) -> Result<Project, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!("SELECT data FROM projects WHERE id = ?", project_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut project: Project = decode(&row.data)?;
        project.mode = mode;
        let data = encode(&project)?;
        sqlx::query!(
            "UPDATE projects SET data = ? WHERE id = ?",
            data,
            project_id
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        let rows = sqlx::query!(
            "SELECT id, data FROM resources WHERE project_id = ? ORDER BY id LIMIT 10001",
            project_id
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(failure)?;
        if rows.len() > MAX_PROJECT_RESOURCES {
            return Err(EngineError::Conflict(
                "This project exceeds 10,000 resources.",
            ));
        }
        for row in rows {
            let mut resource: Resource = decode(&row.data)?;
            resource.mode = mode;
            let data = encode(&resource)?;
            sqlx::query!("UPDATE resources SET data = ? WHERE id = ?", data, row.id)
                .execute(&mut *tx)
                .await
                .map_err(failure)?;
        }
        append(
            &mut tx,
            "person",
            Some(project_id),
            "app.mode",
            "Updated project and resource management permissions.",
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(project)
    }

    async fn group_resources(
        &self,
        actor: &str,
        project_id: &str,
        ids: &[String],
    ) -> Result<Vec<Resource>, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let resources =
            group_in_transaction(&mut tx, actor, project_id, ids, Mode::Watching).await?;
        tx.commit().await.map_err(failure)?;
        Ok(resources)
    }

    async fn custom_domain(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::domains::CustomDomain>, EngineError> {
        sqlx::query!("SELECT data FROM custom_domains WHERE id = ?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?
            .map(|row| decode(&row.data))
            .transpose()
    }

    async fn custom_domains(
        &self,
        project_id: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Page<crate::domain::domains::CustomDomain>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT id,data FROM custom_domains WHERE (? IS NULL OR project_id = ?) AND id > ? ORDER BY id LIMIT 101",
            project_id,
            project_id,
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.id.clone())
        } else {
            None
        };
        Ok(Page {
            items: rows
                .into_iter()
                .map(|row| decode(&row.data))
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    async fn put_custom_domain(
        &self,
        value: &crate::domain::domains::CustomDomain,
    ) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let resource = sqlx::query!(
            "SELECT project_id FROM resources WHERE id = ?",
            value.resource_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        if resource.project_id.as_deref() != Some(&value.project_id) {
            return Err(EngineError::Conflict(
                "Domain resource must belong to the selected project.",
            ));
        }
        let existing = sqlx::query!(
            "SELECT id,resource_id,hostname FROM custom_domains WHERE hostname = ? OR id = ?",
            value.hostname,
            value.id
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(failure)?;
        if existing.iter().any(|row| {
            row.id != value.id
                || row.resource_id != value.resource_id
                || row.hostname != value.hostname
        }) {
            return Err(EngineError::Conflict(
                "This hostname is already assigned to another resource.",
            ));
        }
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO custom_domains(id,project_id,resource_id,hostname,data) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            value.project_id,
            value.resource_id,
            value.hostname,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            "engine",
            Some(&value.project_id),
            Some(&value.id),
            "domain.updated",
            "Updated custom-domain DNS and certificate readiness.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn remove_custom_domain(&self, id: &str) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!(
            "DELETE FROM custom_domains WHERE id = ? RETURNING project_id",
            id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?;
        if let Some(row) = row {
            append_about(
                &mut tx,
                "person",
                Some(&row.project_id),
                Some(id),
                "domain.removed",
                "Detached a custom hostname; its DNS zone remains in the provider account.",
            )
            .await?;
        }
        tx.commit().await.map_err(failure)
    }

    async fn agent(&self, id: &str) -> Result<Option<Agent>, EngineError> {
        let row = sqlx::query!("SELECT data FROM agents WHERE id = ?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| decode(&row.data)).transpose()
    }

    async fn agents(&self, cursor: Option<&str>) -> Result<Page<Agent>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT id, data FROM agents WHERE id > ? ORDER BY id LIMIT 101",
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.id.clone())
        } else {
            None
        };
        Ok(Page {
            items: rows
                .into_iter()
                .map(|row| decode(&row.data))
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    async fn put_agent(&self, value: &Agent) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "INSERT INTO agents (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            "engine",
            None,
            Some(&value.id),
            "agent.updated",
            "Updated agent metadata.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn authorize_agent(
        &self,
        id: &str,
        scope: &str,
        at: &str,
        expiry_days: u16,
    ) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let settings = sqlx::query!("SELECT data FROM settings WHERE id = 1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?;
        let settings: Settings = settings
            .map(|row| decode(&row.data))
            .transpose()?
            .unwrap_or_default();
        if !settings.agents_enabled {
            return Err(EngineError::AgentsDisabled);
        }

        let row = sqlx::query!("SELECT data FROM agents WHERE id = ?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut value: Agent = decode(&row.data)?;
        if !value.permits(scope, at, expiry_days) {
            return Err(EngineError::Conflict(
                "This agent is revoked, expired or lacks permission for this operation.",
            ));
        }
        value.last_seen = at.into();
        let data = encode(&value)?;
        sqlx::query!(
            "INSERT INTO agents (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        tx.commit().await.map_err(failure)
    }

    async fn reconnect_agent(&self, actor: &str, id: &str, at: &str) -> Result<Agent, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!("SELECT data FROM agents WHERE id = ?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut value: Agent = decode(&row.data)?;
        if value.transport != AgentTransport::Stdio {
            return Err(EngineError::Invalid(
                "Only local coding tools can reconnect this way.",
            ));
        }
        value.revoked = false;
        value.last_seen = at.into();
        let data = encode(&value)?;
        sqlx::query!(
            "INSERT INTO agents (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            actor,
            None,
            Some(&value.id),
            "agent.reconnected",
            "Reconnected a coding tool with rotated credentials and unchanged permissions.",
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(value)
    }

    async fn edit_agent(
        &self,
        actor: &str,
        id: &str,
        scopes: Option<&[String]>,
        revoke: bool,
    ) -> Result<Agent, EngineError> {
        if scopes.is_some_and(|value| !valid_agent_scopes(value)) {
            return Err(EngineError::Invalid(
                "Choose distinct apps:read, apps:write or apps:create permissions.",
            ));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!("SELECT data FROM agents WHERE id = ?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut value: Agent = decode(&row.data)?;
        if scopes.is_some() && value.revoked {
            return Err(EngineError::Conflict(
                "Reconnect a revoked agent before changing its permissions.",
            ));
        }
        if let Some(scopes) = scopes {
            value.scopes = scopes.to_vec();
            value.scopes.sort();
        }
        value.revoked |= revoke;
        let data = encode(&value)?;
        sqlx::query!(
            "INSERT INTO agents (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            actor,
            None,
            Some(&value.id),
            if revoke {
                "agent.revoked"
            } else {
                "agent.permissions_changed"
            },
            "Updated agent access without changing credentials or activity expiry.",
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(value)
    }

    async fn set_agent_grant(
        &self,
        actor: &str,
        id: &str,
        grant: crate::domain::model::AgentGrantInput,
    ) -> Result<Agent, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!("SELECT data FROM agents WHERE id = ?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut value: Agent = decode(&row.data)?;
        if value.revoked {
            return Err(EngineError::Conflict(
                "Reconnect a revoked agent before changing what it may touch.",
            ));
        }
        value.grant = crate::domain::model::AgentGrant {
            projects: grant.projects,
            environment_ids: grant.environment_ids,
            deny: grant.deny,
            expires_at: grant.expires_at,
            version: value.grant.version.saturating_add(1),
            without_asking: grant.without_asking,
            environment_rules: grant.environment_rules,
            exceptions: grant.exceptions,
        };
        let data = encode(&value)?;
        sqlx::query!(
            "INSERT INTO agents (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            actor,
            None,
            Some(&value.id),
            "agent.grant_changed",
            &format!(
                "Changed what the agent may touch (version {}).",
                value.grant.version
            ),
        )
        .await?;
        tx.commit().await.map_err(failure)?;
        Ok(value)
    }

    async fn plan(&self, id: &str) -> Result<Option<Plan>, EngineError> {
        let row = sqlx::query!("SELECT data FROM plans WHERE id = ?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| decode(&row.data)).transpose()
    }

    async fn plans(&self, cursor: Option<&str>) -> Result<Page<Plan>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT id, data FROM plans WHERE id > ? ORDER BY id LIMIT 101",
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.id.clone())
        } else {
            None
        };
        Ok(Page {
            items: rows
                .into_iter()
                .map(|row| decode(&row.data))
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    async fn put_plan(&self, value: &Plan) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "INSERT INTO plans (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "engine",
            None,
            "plan.updated",
            "Updated plan metadata.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn update_plan(&self, value: &Plan, expected_revision: i64) -> Result<(), EngineError> {
        if value.id.starts_with("queued:")
            || expected_revision < 0
            || value.revision > 9_007_199_254_740_991
            || expected_revision.checked_add(1) != Some(value.revision)
        {
            return Err(EngineError::Invalid("Use a valid editable plan revision."));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        let id = &value.id;
        let row = sqlx::query!("SELECT data FROM plans WHERE id = ?", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let current: Plan = decode(&row.data)?;
        if current.revision != expected_revision {
            return Err(EngineError::Conflict(
                "The plan changed since you read it. Run hoist plan show, review the new version and reapply your edit.",
            ));
        }
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO plans (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "engine",
            None,
            "plan.updated",
            "Updated plan metadata.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn remove_account(&self, id: &str) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let count = sqlx::query!(
            "SELECT count(*) AS count FROM resources WHERE account_id = ?",
            id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(failure)?;
        if count.count > 0 {
            return Err(EngineError::Conflict(
                "Remove or disconnect this account's resources first.",
            ));
        }
        sqlx::query!("DELETE FROM accounts WHERE id = ?", id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        append_about(
            &mut tx,
            "engine",
            None,
            Some(id),
            "account.removed",
            "Removed a provider account.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn remove_resource(&self, id: &str) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!("DELETE FROM resources WHERE id = ?", id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        append_about(
            &mut tx,
            "engine",
            None,
            Some(id),
            "resource.removed",
            "Removed a provider resource.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn remove_project(&self, id: &str) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let count = sqlx::query!(
            "SELECT count(*) AS count FROM resources WHERE project_id = ?",
            id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(failure)?;
        if count.count > 0 {
            return Err(EngineError::Conflict(
                "Delete the project's provider resources first.",
            ));
        }
        sqlx::query!("DELETE FROM deploys WHERE project_id = ?", id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        sqlx::query!("DELETE FROM projects WHERE id = ?", id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        append(
            &mut tx,
            "engine",
            Some(id),
            "app.removed",
            "Removed a project.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn links(&self, project_id: &str) -> Result<Vec<Link>, EngineError> {
        let rows = sqlx::query!(
            "SELECT links.data FROM links JOIN resources ON resources.id = links.from_resource WHERE resources.project_id = ? ORDER BY links.id LIMIT 1000",
            project_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        if rows.len() >= MAX_PROJECT_LINKS {
            return Err(EngineError::Conflict(
                "The project map exceeds the safe read limit; no partial graph was returned.",
            ));
        }
        rows.into_iter().map(|row| decode(&row.data)).collect()
    }

    async fn all_links(&self) -> Result<Vec<Link>, EngineError> {
        let rows = sqlx::query!("SELECT data FROM links ORDER BY id LIMIT 10001")
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        if rows.len() > MAX_LINKS {
            return Err(EngineError::Conflict(
                "The relationship graph exceeds 10,000 edges; no partial impact was returned.",
            ));
        }
        rows.into_iter().map(|row| decode(&row.data)).collect()
    }

    async fn resource_links(&self, resource_id: &str) -> Result<Vec<Link>, EngineError> {
        let rows = sqlx::query!(
            "SELECT data FROM links WHERE from_resource = ? OR to_resource = ? ORDER BY id LIMIT 1001",
            resource_id,
            resource_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        if rows.len() > MAX_RESOURCE_LINKS {
            return Err(EngineError::Conflict(
                "The resource has more than 1,000 recorded links; no partial detail was returned.",
            ));
        }
        rows.into_iter().map(|row| decode(&row.data)).collect()
    }

    async fn latest_resource_usage(&self, resource_id: &str) -> Result<Option<Usage>, EngineError> {
        sqlx::query!(
            "SELECT data FROM usage WHERE resource_id = ? ORDER BY day DESC LIMIT 1",
            resource_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(failure)?
        .map(|row| decode(&row.data))
        .transpose()
    }

    async fn latest_resource_deploy(
        &self,
        project_id: &str,
        resource_id: &str,
    ) -> Result<Option<Deploy>, EngineError> {
        let row = sqlx::query!(
            "SELECT data, version, status, cancel_requested FROM deploys WHERE project_id = ? AND EXISTS (SELECT 1 FROM json_each(deploys.data, '$.releases') WHERE json_extract(value, '$.resource_id') = ?) ORDER BY version DESC LIMIT 1",
            project_id,
            resource_id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(failure)?;
        row.map(|row| {
            let mut deploy: Deploy = decode(&row.data)?;
            deploy.version = row.version;
            deploy.cancel_requested = row.cancel_requested != 0;
            deploy.status = decode(&encode(&row.status)?)?;
            Ok(deploy)
        })
        .transpose()
    }

    async fn reconcile_observed_links(
        &self,
        actor: &str,
        from: &str,
        links: &[Link],
        preserve_names: &[String],
    ) -> Result<(), EngineError> {
        if links.len() > MAX_OBSERVED_BINDINGS || preserve_names.len() > MAX_OBSERVED_BINDINGS {
            return Err(EngineError::Invalid("Too many observed bindings."));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        let source = sqlx::query!("SELECT project_id FROM resources WHERE id = ?", from)
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let project_id = source.project_id.ok_or(EngineError::Invalid(
            "Group the source resource before refreshing relationships.",
        ))?;
        let rows = sqlx::query!(
            "SELECT data FROM links WHERE from_resource = ? ORDER BY id LIMIT 1001",
            from
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(failure)?;
        if rows.len() > MAX_RESOURCE_LINKS {
            return Err(EngineError::Conflict(
                "Too many resource relationships to reconcile safely.",
            ));
        }
        let mut changed = false;
        let mut total = rows.len();
        for row in rows {
            let mut old: Link = decode(&row.data)?;
            if !old.id.starts_with(OBSERVED_LINK_PREFIX) || !old.derived {
                continue;
            }
            let present = links.iter().any(|link| link.id == old.id);
            if !present && preserve_names.contains(&old.via) {
                continue;
            }
            if old.missing == present {
                old.missing = !present;
                let data = encode(&old)?;
                sqlx::query!("UPDATE links SET data = ? WHERE id = ?", data, old.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(failure)?;
                changed = true;
            }
        }
        for link in links {
            if link.from_resource != from
                || link.to_resource == from
                || !link.derived
                || link.missing
                || !link.id.starts_with(OBSERVED_LINK_PREFIX)
                || !valid_env_name(&link.via)
            {
                return Err(EngineError::Invalid("Invalid observed relationship."));
            }
            let target = sqlx::query!(
                "SELECT project_id FROM resources WHERE id = ?",
                link.to_resource
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
            if target.project_id.is_none() {
                return Err(EngineError::Invalid(
                    "Group the target resource before recording its relationship.",
                ));
            }
            let data = encode(link)?;
            let inserted = sqlx::query!(
                "INSERT INTO links(id,from_resource,to_resource,data) VALUES(?,?,?,?) ON CONFLICT(id) DO NOTHING",
                link.id,
                link.from_resource,
                link.to_resource,
                data
            )
            .execute(&mut *tx)
            .await
            .map_err(failure)?
            .rows_affected();
            total += inserted as usize;
            if total >= MAX_RESOURCE_LINKS {
                return Err(EngineError::Conflict(
                    "Observed relationships exceed the resource map limit; existing state was retained.",
                ));
            }
            changed |= inserted > 0;
        }
        if changed {
            append(
                &mut tx,
                actor,
                Some(&project_id),
                "map.bindings_observed",
                "Refreshed supported provider bindings; disappeared relationships remain marked missing.",
            )
            .await?;
        }
        tx.commit().await.map_err(failure)
    }

    async fn put_link(&self, value: &Link) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        let left = sqlx::query!(
            "SELECT project_id FROM resources WHERE id = ?",
            value.from_resource
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        let right = sqlx::query!(
            "SELECT project_id FROM resources WHERE id = ?",
            value.to_resource
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        if left.project_id.is_none()
            || right.project_id.is_none()
            || value.from_resource == value.to_resource
        {
            return Err(EngineError::Invalid(
                "Links must join two different resources assigned to projects.",
            ));
        }
        sqlx::query!(
            "INSERT INTO links (id, from_resource, to_resource, data) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET from_resource = excluded.from_resource, to_resource = excluded.to_resource, data = excluded.data",
            value.id,
            value.from_resource,
            value.to_resource,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "engine",
            left.project_id.as_deref(),
            "app.map",
            "Updated project relationships.",
        )
        .await?;
        if right.project_id != left.project_id {
            append(
                &mut tx,
                "engine",
                right.project_id.as_deref(),
                "app.map",
                "Updated shared-resource relationships.",
            )
            .await?;
        }
        tx.commit().await.map_err(failure)
    }

    async fn deploy(&self, id: &str) -> Result<Option<Deploy>, EngineError> {
        let row = sqlx::query!(
            "SELECT version, status, cancel_requested, data FROM deploys WHERE id = ?",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(failure)?;
        row.map(|row| {
            let mut value: Deploy = decode(&row.data)?;
            value.version = row.version;
            value.cancel_requested = row.cancel_requested != 0;
            value.status = decode(&encode(&row.status)?)?;
            Ok(value)
        })
        .transpose()
    }

    async fn deploys(
        &self,
        project_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<Deploy>, EngineError> {
        let cursor = cursor
            .map(str::parse::<i64>)
            .transpose()
            .map_err(|_| EngineError::Invalid("Invalid deployment cursor."))?
            .unwrap_or(i64::MAX);
        let mut rows = sqlx::query!(
            "SELECT version, status, cancel_requested, data FROM deploys WHERE project_id = ? AND version < ? ORDER BY version DESC LIMIT 101",
            project_id,
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.version.to_string())
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|row| {
                let mut value: Deploy = decode(&row.data)?;
                value.version = row.version;
                value.cancel_requested = row.cancel_requested != 0;
                value.status = decode(&encode(&row.status)?)?;
                Ok(value)
            })
            .collect::<Result<_, EngineError>>()?;
        Ok(Page { items, next_cursor })
    }

    async fn recent_deploys(&self) -> Result<Vec<Deploy>, EngineError> {
        // rowid follows insertion, which is queue order across projects.
        let rows = sqlx::query!("SELECT version, status, cancel_requested, data FROM deploys ORDER BY rowid DESC LIMIT 50")
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        rows.into_iter()
            .map(|row| {
                let mut value: Deploy = decode(&row.data)?;
                value.version = row.version;
                value.cancel_requested = row.cancel_requested != 0;
                value.status = decode(&encode(&row.status)?)?;
                Ok(value)
            })
            .collect()
    }

    async fn environment_key_snapshot(&self) -> Result<EnvironmentKeySnapshot, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let snapshot = key_snapshot(&mut tx).await?;
        tx.commit().await.map_err(failure)?;
        Ok(snapshot)
    }

    async fn retire_environment_key_setup(&self, review_token: &str) -> Result<(), EngineError> {
        if review_token.len() != 64
            || !review_token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(EngineError::Invalid(
                "Use the review token from the environment-key recovery report.",
            ));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        let snapshot = key_snapshot(&mut tx).await?;
        if snapshot.active.is_some() {
            return Err(EngineError::Conflict(
                "An active admission key exists. Retry-setup cannot replace it; restore or unlock the original key.",
            ));
        }
        if snapshot.pending.is_empty() {
            return Err(EngineError::Conflict(
                "No incomplete environment-key setup is waiting to be retired.",
            ));
        }
        if snapshot.review_token()? != review_token {
            return Err(EngineError::Conflict(
                "Environment-key setup changed. Inspect it again before retrying setup.",
            ));
        }
        sqlx::query!(
            "UPDATE environment_admission_keys SET retired = 1 WHERE active = 0 AND retired = 0"
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "person",
            None,
            "key.admission.setup_retired",
            "Retired reviewed incomplete environment-key setup references. Credentials and deployment requests were preserved.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn environment_admission_key(
        &self,
    ) -> Result<Option<EnvironmentAdmissionKey>, EngineError> {
        let row = sqlx::query!("SELECT reference, commitment AS \"commitment!: String\" FROM environment_admission_keys WHERE active = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        Ok(row.map(|row| EnvironmentAdmissionKey {
            reference: row.reference,
            commitment: row.commitment,
        }))
    }

    async fn reserve_environment_admission_key(&self, reference: &str) -> Result<(), EngineError> {
        let valid = reference
            .strip_prefix("environment-admission:")
            .is_some_and(|id| id.len() == 36 && uuid::Uuid::parse_str(id).is_ok());
        if !valid {
            return Err(EngineError::Invalid("Invalid admission key reference."));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        let count = sqlx::query!("SELECT count(*) AS count FROM environment_admission_keys WHERE active = 0 AND retired = 0")
            .fetch_one(&mut *tx)
            .await
            .map_err(failure)?;
        if count.count >= 16 {
            return Err(EngineError::Conflict(
                "Admission key setup has too many incomplete attempts. Run hoist doctor environment-key and review --retry-setup; existing keys were not replaced.",
            ));
        }
        sqlx::query!(
            "INSERT INTO environment_admission_keys (reference) VALUES (?)",
            reference
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        tx.commit().await.map_err(failure)
    }

    async fn activate_environment_admission_key(
        &self,
        key: &EnvironmentAdmissionKey,
    ) -> Result<EnvironmentAdmissionKey, EngineError> {
        if key.commitment.len() != 64
            || !key
                .commitment
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(EngineError::Invalid("Invalid admission key commitment."));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        let candidate = sqlx::query!(
            "SELECT retired FROM environment_admission_keys WHERE reference = ?",
            key.reference
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        if candidate.retired != 0 {
            return Err(EngineError::Conflict(
                "This key setup was retired. Inspect recovery status before starting a new setup attempt.",
            ));
        }
        if let Some(row) =
            sqlx::query!("SELECT reference, commitment AS \"commitment!: String\" FROM environment_admission_keys WHERE active = 1")
                .fetch_optional(&mut *tx)
                .await
                .map_err(failure)?
        {
            tx.commit().await.map_err(failure)?;
            return Ok(EnvironmentAdmissionKey {
                reference: row.reference,
                commitment: row.commitment,
            });
        }
        let changed = sqlx::query!(
            "UPDATE environment_admission_keys SET commitment = ?, active = 1 WHERE reference = ? AND active = 0 AND retired = 0",
            key.commitment,
            key.reference
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        if changed.rows_affected() != 1 {
            return Err(EngineError::NotFound);
        }
        tx.commit().await.map_err(failure)?;
        Ok(key.clone())
    }

    async fn deployment_request(
        &self,
        actor: &str,
        request_key: &str,
    ) -> Result<Option<DeploymentRequest>, EngineError> {
        let request_key = DeploymentAdmission::request_key(actor, request_key)?;
        let request_key = request_key.as_str();
        let mut tx = self.begin_write().await.map_err(failure)?;
        let Some(receipt) = sqlx::query!(
            "SELECT deploy_id, project_id FROM deployment_admissions WHERE actor = ? AND request_key = ?",
            actor,
            request_key
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        else {
            tx.commit().await.map_err(failure)?;
            return Ok(None);
        };
        let row = sqlx::query!(
            "SELECT version, status, cancel_requested, data FROM deploys WHERE id = ?",
            receipt.deploy_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?;
        let deployment = row
            .map(|row| {
                let mut value: Deploy = decode(&row.data)?;
                if value.project_id != receipt.project_id || value.started_by != actor {
                    return Err(EngineError::Store);
                }
                value.version = row.version;
                value.status = decode(&encode(&row.status)?)?;
                value.cancel_requested = row.cancel_requested != 0;
                Ok(value)
            })
            .transpose()?;
        tx.commit().await.map_err(failure)?;
        Ok(Some(DeploymentRequest {
            request_key: request_key.into(),
            actor: actor.into(),
            project_id: receipt.project_id,
            deploy_id: receipt.deploy_id,
            deployment,
        }))
    }

    async fn admitted_deploy(
        &self,
        admission: &DeploymentAdmission,
    ) -> Result<Option<Deploy>, EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let result = admitted(&mut tx, admission).await?;
        tx.commit().await.map_err(failure)?;
        Ok(result)
    }

    async fn enqueue_with_admission(
        &self,
        value: &Deploy,
        admission: Option<&DeploymentAdmission>,
    ) -> Result<Deploy, EngineError> {
        if admission.is_some_and(|request| request.actor() != value.started_by) {
            return Err(EngineError::Invalid(
                "The deployment request actor does not match.",
            ));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        if let Some(admission) = admission
            && let Some(existing) = admitted(&mut tx, admission).await?
        {
            if existing.project_id != value.project_id {
                return Err(EngineError::Conflict(
                    "This request key belongs to another project.",
                ));
            }
            tx.commit().await.map_err(failure)?;
            return Ok(existing);
        }
        let mut value = value.clone();
        value.release_version = None;
        let data = encode(&value)?;
        let queued = sqlx::query!("SELECT count(*) AS count FROM deploys WHERE status = 'queued'")
            .fetch_one(&mut *tx)
            .await
            .map_err(failure)?;
        if queued.count >= MAX_QUEUED_DEPLOYS {
            return Err(EngineError::Conflict(
                "The deployment queue is full. Wait for a deployment to finish.",
            ));
        }
        let row = sqlx::query!(
            "INSERT INTO deploys (id, project_id, version, status, data) SELECT ?, ?, coalesce(max(version), 0) + 1, 'queued', ? FROM deploys WHERE project_id = ? RETURNING version, cancel_requested",
            value.id,
            value.project_id,
            data,
            value.project_id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            &value.started_by,
            Some(&value.project_id),
            Some(&value.id),
            "deploy.queued",
            "Queued a deployment.",
        )
        .await?;
        if let Some(admission) = admission {
            let actor = admission.actor();
            let key = admission.key();
            let fingerprint = admission.fingerprint();
            sqlx::query!(
                "INSERT INTO deployment_admissions (actor, request_key, fingerprint, deploy_id, project_id) VALUES (?, ?, ?, ?, ?)",
                actor,
                key,
                fingerprint,
                value.id,
                value.project_id
            )
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        }
        tx.commit().await.map_err(failure)?;
        value.version = row.version;
        value.cancel_requested = row.cancel_requested != 0;
        value.status = DeployStatus::Queued;
        Ok(value)
    }

    async fn put_deploy(&self, value: &Deploy) -> Result<(), EngineError> {
        let status = label(&value.status)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!(
            "SELECT project_id, status FROM deploys WHERE id = ?",
            value.id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        if row.project_id != value.project_id {
            return Err(EngineError::Invalid(
                "A deployment cannot change its project.",
            ));
        }
        if row.status != "queued" && row.status != "running" {
            return Err(EngineError::Conflict("This deployment already finished."));
        }
        if value.status == DeployStatus::Queued && row.status == "running" {
            return Err(EngineError::Conflict(
                "A running deployment cannot return to the queue.",
            ));
        }
        let mut stored = value.clone();
        stored.release_version = None;
        if matches!(stored.status, DeployStatus::Live | DeployStatus::RolledBack) {
            let maximum = sqlx::query!(
                "SELECT coalesce(max(CAST(json_extract(data, '$.release_version') AS INTEGER)), 0) AS \"maximum!: i64\" FROM deploys WHERE project_id = ?",
                row.project_id
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(failure)?;
            stored.release_version =
                Some(maximum.maximum.checked_add(1).ok_or(EngineError::Store)?);
        }
        let data = encode(&stored)?;
        sqlx::query!(
            "UPDATE deploys SET status = ?, data = ? WHERE id = ?",
            status,
            data,
            value.id
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        let kind = if matches!(
            value.status,
            DeployStatus::Live | DeployStatus::Failed | DeployStatus::RolledBack
        ) {
            "deploy.finished"
        } else {
            "deploy.step"
        };
        append_about(
            &mut tx,
            &value.started_by,
            Some(&value.project_id),
            Some(&value.id),
            kind,
            "Updated deployment progress.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn has_pending_deploy(&self, project_id: &str) -> Result<bool, EngineError> {
        let row = sqlx::query!(
            "SELECT EXISTS(SELECT 1 FROM deploys WHERE project_id = ? AND status IN ('queued','running')) AS pending",
            project_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(failure)?;
        Ok(row.pending != 0)
    }

    async fn cancel_deploy(&self, actor: &str, id: &str) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!(
            "SELECT project_id, status, cancel_requested FROM deploys WHERE id = ?",
            id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        if row.status != "queued" && row.status != "running" {
            return Err(EngineError::Conflict("This deployment already finished."));
        }
        if row.cancel_requested == 0 {
            sqlx::query!("UPDATE deploys SET cancel_requested = 1 WHERE id = ?", id)
                .execute(&mut *tx)
                .await
                .map_err(failure)?;
            append_about(
                &mut tx,
                actor,
                Some(&row.project_id),
                Some(id),
                "deploy.cancel_requested",
                "Cancellation requested; any in-flight provider operation will finish before stopping.",
            )
            .await?;
        }
        tx.commit().await.map_err(failure)
    }

    async fn queued(&self) -> Result<Vec<Deploy>, EngineError> {
        let rows = sqlx::query!("SELECT version, cancel_requested, data FROM deploys WHERE status = 'queued' ORDER BY rowid LIMIT 32")
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        rows.into_iter()
            .map(|row| {
                let mut value: Deploy = decode(&row.data)?;
                value.version = row.version;
                value.cancel_requested = row.cancel_requested != 0;
                value.status = DeployStatus::Queued;
                Ok(value)
            })
            .collect()
    }

    async fn recover_deploys(&self) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let result = sqlx::query!("UPDATE deploys SET status = 'failed' WHERE status = 'running'")
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        if result.rows_affected() > 0 {
            append(
                &mut tx,
                "engine",
                None,
                "deploy.finished",
                "Interrupted deployments need inspection before retrying; remote releases may already exist.",
            )
            .await?;
        }
        tx.commit().await.map_err(failure)
    }

    async fn approval(&self, id: &str) -> Result<Option<Approval>, EngineError> {
        let row = sqlx::query!("SELECT data, decision FROM approvals WHERE id = ?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?;
        row.map(|row| {
            let mut value: Approval = decode(&row.data)?;
            value.decision = decode(&encode(&row.decision)?)?;
            if value.expires_at <= now()
                && matches!(
                    value.decision,
                    ApprovalDecision::Pending | ApprovalDecision::Allowed
                )
            {
                value.decision = ApprovalDecision::Expired;
            }
            Ok(value)
        })
        .transpose()
    }

    async fn approvals(&self, cursor: Option<&str>) -> Result<Page<Approval>, EngineError> {
        let cursor = cursor.unwrap_or("");
        let mut rows = sqlx::query!(
            "SELECT id, data, decision FROM approvals WHERE id > ? ORDER BY id LIMIT 101",
            cursor
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last().map(|row| row.id.clone())
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|row| {
                let mut value: Approval = decode(&row.data)?;
                value.decision = decode(&encode(&row.decision)?)?;
                if value.expires_at <= now()
                    && matches!(
                        value.decision,
                        ApprovalDecision::Pending | ApprovalDecision::Allowed
                    )
                {
                    value.decision = ApprovalDecision::Expired;
                }
                Ok(value)
            })
            .collect::<Result<_, EngineError>>()?;
        Ok(Page { items, next_cursor })
    }

    async fn put_approval(&self, value: &Approval) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "INSERT INTO approvals (id, agent_id, decision, expires_at, data) VALUES (?, ?, 'pending', ?, ?)",
            value.id,
            value.agent_id,
            value.expires_at,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            &value.agent_id,
            None,
            Some(&value.id),
            "approval.created",
            "An agent requested approval.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn decide_approval(&self, id: &str, allow: bool, now: &str) -> Result<(), EngineError> {
        let decision = if allow { "allowed" } else { "declined" };
        let mut tx = self.begin_write().await.map_err(failure)?;
        let result = sqlx::query!(
            "UPDATE approvals SET decision = ? WHERE id = ? AND decision = 'pending' AND expires_at > ?",
            decision,
            id,
            now
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        if result.rows_affected() != 1 {
            return Err(EngineError::Conflict(
                "Approval has expired or already has a decision.",
            ));
        }
        append_about(
            &mut tx,
            "person",
            None,
            Some(id),
            "approval.resolved",
            "Resolved an agent approval.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn consume_approval(
        &self,
        id: &str,
        agent_id: &str,
        action: GatedAction,
        target: &str,
        now: &str,
    ) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let row = sqlx::query!(
            "SELECT data FROM approvals WHERE id = ? AND agent_id = ? AND decision = 'allowed' AND expires_at > ?",
            id,
            agent_id,
            now
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::ApprovalRequired)?;
        let approval: Approval = decode(&row.data)?;
        if approval.action != action || approval.target != target {
            return Err(EngineError::ApprovalRequired);
        }
        let result = sqlx::query!(
            "UPDATE approvals SET decision = 'consumed' WHERE id = ? AND decision = 'allowed'",
            id
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        if result.rows_affected() != 1 {
            return Err(EngineError::ApprovalRequired);
        }
        append_about(
            &mut tx,
            agent_id,
            None,
            Some(id),
            "approval.consumed",
            "Consumed an approval for its exact action.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn secrets(&self, project_id: &str) -> Result<Vec<SecretMetadata>, EngineError> {
        let mut cursor = None;
        let mut values = Vec::new();
        loop {
            let page = self.secrets_page(project_id, cursor.as_deref()).await?;
            values.extend(page.items);
            if values.len() > 10_000 {
                return Err(EngineError::Conflict(
                    "This project exceeds the 10,000-secret metadata limit.",
                ));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(values);
            }
        }
    }

    async fn secrets_page(
        &self,
        project_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<SecretMetadata>, EngineError> {
        let (resource, name) = page_key(cursor, "secrets", project_id, "")?;
        let mut rows = sqlx::query!(
            "SELECT secret_metadata.resource_id,secret_metadata.name,secret_metadata.data FROM secret_metadata JOIN resources ON resources.id = secret_metadata.resource_id WHERE resources.project_id = ? AND (secret_metadata.resource_id > ? OR (secret_metadata.resource_id = ? AND secret_metadata.name > ?)) ORDER BY secret_metadata.resource_id,secret_metadata.name LIMIT 101",
            project_id,
            resource,
            resource,
            name
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last()
                .map(|row| encode_page_key("secrets", project_id, "", &row.resource_id, &row.name))
                .transpose()?
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|row| decode(&row.data))
            .collect::<Result<_, _>>()?;
        Ok(Page { items, next_cursor })
    }

    async fn put_secret(&self, value: &SecretMetadata) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "INSERT INTO secret_metadata (resource_id, name, data) VALUES (?, ?, ?) ON CONFLICT(resource_id, name) DO UPDATE SET data = excluded.data",
            value.resource_id,
            value.name,
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "engine",
            Some(&value.project_id),
            "secret.updated",
            "Updated a provider secret; no value was retained.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn delete_secret_metadata(
        &self,
        actor: &str,
        project_id: &str,
        resource_id: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "DELETE FROM secret_metadata WHERE resource_id = ? AND name = ?",
            resource_id,
            name
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            actor,
            Some(project_id),
            "secret.deleted",
            &format!("Removed hosted secret {name} from resource {resource_id}; source files were not changed."),
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn usage(&self, project_id: &str, since: &str) -> Result<Vec<Usage>, EngineError> {
        let rows = sqlx::query!(
            "SELECT usage.data FROM usage JOIN resources ON resources.id = usage.resource_id WHERE resources.project_id = ? AND day >= ? ORDER BY day, usage.resource_id LIMIT 10001",
            project_id,
            since
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        if rows.len() > MAX_USAGE_ROWS {
            return Err(EngineError::Conflict(
                "Narrow the usage date range; more than 10,000 observations match.",
            ));
        }
        rows.into_iter().map(|row| decode(&row.data)).collect()
    }

    async fn usage_page(
        &self,
        project_id: &str,
        since: &str,
        cursor: Option<&str>,
    ) -> Result<Page<Usage>, EngineError> {
        let (day, resource) = page_key(cursor, "usage", project_id, since)?;
        let mut rows = sqlx::query!(
            "SELECT usage.day,usage.resource_id,usage.data FROM usage JOIN resources ON resources.id = usage.resource_id WHERE resources.project_id = ? AND usage.day >= ? AND (usage.day > ? OR (usage.day = ? AND usage.resource_id > ?)) ORDER BY usage.day,usage.resource_id LIMIT 101",
            project_id,
            since,
            day,
            day,
            resource
        )
        .fetch_all(&self.pool)
        .await
        .map_err(failure)?;
        let more = trim_page(&mut rows);
        let next_cursor = if more {
            rows.last()
                .map(|row| encode_page_key("usage", project_id, since, &row.day, &row.resource_id))
                .transpose()?
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|row| decode(&row.data))
            .collect::<Result<_, _>>()?;
        Ok(Page { items, next_cursor })
    }

    async fn put_usage(&self, value: &Usage) -> Result<(), EngineError> {
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO usage (resource_id, day, data) VALUES (?, ?, ?) ON CONFLICT(resource_id, day) DO UPDATE SET data = excluded.data",
            value.resource_id,
            value.day,
            data
        )
        .execute(&self.pool)
        .await
        .map_err(failure)?;
        Ok(())
    }

    async fn settings(&self) -> Result<Settings, EngineError> {
        sqlx::query!("SELECT data FROM settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?
            .map(|row| decode(&row.data))
            .transpose()
            .map(|value| value.unwrap_or_default())
    }

    async fn put_settings(&self, value: &Settings) -> Result<(), EngineError> {
        let data = encode(value)?;
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "INSERT INTO settings (id, data) VALUES (1, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append(
            &mut tx,
            "person",
            None,
            "settings.updated",
            "Updated local settings.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn restore(&self, backup: &crate::domain::backup::Backup) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        let count = sqlx::query!(
            "SELECT (SELECT count(*) FROM projects)+(SELECT count(*) FROM accounts) AS count"
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(failure)?;
        if count.count != 0 {
            return Err(EngineError::Conflict(
                "Restore into an empty Hoista state directory.",
            ));
        }
        insert_backup(&mut tx, backup).await?;
        append(
            &mut tx,
            "person",
            None,
            "backup.restored",
            "Restored metadata in watching mode. Reconnect accounts before management.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn oauth_client(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::oauth::OAuthClient>, EngineError> {
        sqlx::query!("SELECT data FROM oauth_clients WHERE id=?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?
            .map(|row| decode(&row.data))
            .transpose()
    }

    async fn put_oauth_client(
        &self,
        value: &crate::domain::oauth::OAuthClient,
    ) -> Result<(), EngineError> {
        let data = encode(value)?;
        let insert = || {
            sqlx::query!(
                "INSERT INTO oauth_clients(id,data) SELECT ?,? WHERE (SELECT count(*) FROM oauth_clients) < 1000",
                value.id,
                data
            )
            .execute(&self.pool)
        };
        if insert().await.map_err(failure)?.rows_affected() > 0 {
            return Ok(());
        }
        // Full: anyone who can reach the address can register, so drop the
        // registrations that no sign-in in progress and no grant uses.
        sqlx::query!("DELETE FROM oauth_clients WHERE id NOT IN (SELECT json_extract(data,'$.client_id') FROM oauth_pending) AND id NOT IN (SELECT json_extract(data,'$.client_id') FROM oauth_grants)")
            .execute(&self.pool)
            .await
            .map_err(failure)?;
        if insert().await.map_err(failure)?.rows_affected() == 0 {
            return Err(EngineError::Conflict(
                "Too many agents are signing in at once. Try again once some finish.",
            ));
        }
        Ok(())
    }

    async fn begin_oauth(
        &self,
        agent: &Agent,
        approval: &Approval,
        pending: &crate::domain::oauth::OAuthPending,
    ) -> Result<(), EngineError> {
        if agent.transport != AgentTransport::Link
            || agent.id != approval.agent_id
            || agent.id != pending.agent_id
            || approval.id != pending.approval_id
            || approval.target != pending.id
            || approval.action != GatedAction::ConnectAgent
        {
            return Err(EngineError::Invalid("OAuth approval records do not match."));
        }
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query!(
            "SELECT id FROM oauth_clients WHERE id = ?",
            pending.client_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
        let timestamp = now();
        // Only identities that never received a grant can be pruned without vault access.
        let expired = sqlx::query!(
            "SELECT agents.id FROM agents JOIN oauth_pending ON json_extract(oauth_pending.data,'$.agent_id') = agents.id LEFT JOIN oauth_grants ON oauth_grants.agent_id = agents.id WHERE oauth_grants.agent_id IS NULL AND json_extract(oauth_pending.data,'$.expires_at') <= ? LIMIT 100",
            timestamp
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(failure)?;
        for row in expired {
            sqlx::query!("DELETE FROM approvals WHERE agent_id = ?", row.id)
                .execute(&mut *tx)
                .await
                .map_err(failure)?;
            sqlx::query!("DELETE FROM agents WHERE id = ?", row.id)
                .execute(&mut *tx)
                .await
                .map_err(failure)?;
        }
        sqlx::query!(
            "DELETE FROM oauth_pending WHERE json_extract(data,'$.expires_at') <= ?",
            timestamp
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        let limits = sqlx::query!("SELECT (SELECT count(*) FROM oauth_pending) AS pending, (SELECT count(*) FROM agents WHERE json_extract(data,'$.revoked') = 0) AS agents")
            .fetch_one(&mut *tx)
            .await
            .map_err(failure)?;
        if limits.pending >= MAX_PENDING_AGENTS || limits.agents >= MAX_AGENTS {
            return Err(EngineError::Conflict(
                "Too many pending or registered agents. Disconnect unused agents before connecting another.",
            ));
        }
        let agent_data = encode(agent)?;
        let approval_data = encode(approval)?;
        let pending_data = encode(pending)?;
        sqlx::query!(
            "INSERT INTO agents(id,data) VALUES(?,?)",
            agent.id,
            agent_data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        sqlx::query!(
            "INSERT INTO approvals(id,agent_id,decision,expires_at,data) VALUES(?,?,'pending',?,?)",
            approval.id,
            agent.id,
            approval.expires_at,
            approval_data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        sqlx::query!(
            "INSERT INTO oauth_pending(id,data) VALUES(?,?)",
            pending.id,
            pending_data
        )
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        append_about(
            &mut tx,
            &agent.id,
            None,
            Some(&approval.id),
            "approval.created",
            "An unverified remote client requested permission to connect.",
        )
        .await?;
        tx.commit().await.map_err(failure)
    }

    async fn oauth_pending(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::oauth::OAuthPending>, EngineError> {
        sqlx::query!("SELECT data FROM oauth_pending WHERE id=?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?
            .map(|row| decode(&row.data))
            .transpose()
    }

    async fn enrollments(
        &self,
    ) -> Result<Vec<crate::application::engine::StoredEnrollment>, EngineError> {
        use sqlx::Row as _;
        let rows = sqlx::query("SELECT data FROM enrollment_requests ORDER BY expires_at, id")
            .fetch_all(&self.pool)
            .await
            .map_err(failure)?;
        rows.iter()
            .map(|row| decode(&row.try_get::<String, _>("data").map_err(failure)?))
            .collect()
    }

    async fn put_enrollment(
        &self,
        value: &crate::application::engine::StoredEnrollment,
    ) -> Result<(), EngineError> {
        let data = encode(value)?;
        sqlx::query("INSERT INTO enrollment_requests(id,expires_at,data) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data")
            .bind(&value.request.id)
            .bind(&value.request.expires_at)
            .bind(data)
            .execute(&self.pool)
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn forget_enrollments(&self, before: &str) -> Result<(), EngineError> {
        sqlx::query("DELETE FROM enrollment_requests WHERE expires_at < ?")
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn put_oauth_pending(
        &self,
        value: &crate::domain::oauth::OAuthPending,
    ) -> Result<(), EngineError> {
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO oauth_pending(id,data) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            value.id,
            data
        )
        .execute(&self.pool)
        .await
        .map_err(failure)?;
        Ok(())
    }

    async fn oauth_grant(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::oauth::OAuthGrant>, EngineError> {
        sqlx::query!("SELECT data FROM oauth_grants WHERE agent_id=?", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(failure)?
            .map(|row| decode(&row.data))
            .transpose()
    }

    async fn put_oauth_grant(
        &self,
        value: &crate::domain::oauth::OAuthGrant,
    ) -> Result<(), EngineError> {
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO oauth_grants(agent_id,data) VALUES(?,?) ON CONFLICT(agent_id) DO UPDATE SET data=excluded.data",
            value.agent_id,
            data
        )
        .execute(&self.pool)
        .await
        .map_err(failure)?;
        Ok(())
    }

    async fn event_with_detail(
        &self,
        actor: &str,
        project_id: Option<&str>,
        kind: &str,
        summary: &str,
        detail: &str,
    ) -> Result<(), EngineError> {
        let at = now();
        let mut tx = self.begin_write().await.map_err(failure)?;
        sqlx::query("INSERT INTO events (at, actor, project_id, subject_id, kind, summary, detail) VALUES (?, ?, ?, NULL, ?, ?, ?)")
            .bind(&at)
            .bind(actor)
            .bind(project_id)
            .bind(kind)
            .bind(summary)
            .bind(detail)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        tx.commit().await.map_err(failure)
    }

    async fn event_detail(&self, id: i64) -> Result<Option<String>, EngineError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT detail FROM events WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(failure)?;
        Ok(row.and_then(|(detail,)| detail))
    }

    async fn event(
        &self,
        actor: &str,
        project_id: Option<&str>,
        kind: &str,
        summary: &str,
    ) -> Result<(), EngineError> {
        let mut tx = self.begin_write().await.map_err(failure)?;
        append(&mut tx, actor, project_id, kind, summary).await?;
        tx.commit().await.map_err(failure)
    }
}

async fn group_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: &str,
    project_id: &str,
    ids: &[String],
    new_mode: Mode,
) -> Result<Vec<Resource>, EngineError> {
    if ids.is_empty() || ids.len() > 100 {
        return Err(EngineError::Invalid(
            "Choose between 1 and 100 resources to import.",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    sqlx::query!("SELECT id FROM projects WHERE id = ?", project_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(failure)?
        .ok_or(EngineError::NotFound)?;
    let mut resources = Vec::new();
    for id in ids {
        if sqlx::query!(
            "SELECT resource_id FROM ignored_resources WHERE resource_id=?",
            id
        )
        .fetch_optional(&mut **tx)
        .await
        .map_err(failure)?
        .is_some()
        {
            return Err(EngineError::Conflict(
                "Unignore the resource before grouping it into a project.",
            ));
        }

        if !seen.insert(id) {
            return Err(EngineError::Invalid(
                "Each imported resource must appear once.",
            ));
        }
        let row = sqlx::query!("SELECT data FROM resources WHERE id = ?", id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(failure)?
            .ok_or(EngineError::NotFound)?;
        let mut resource: Resource = decode(&row.data)?;
        if resource
            .project_id
            .as_deref()
            .is_some_and(|owner| owner != project_id)
        {
            return Err(EngineError::Conflict(
                "A resource already belongs to another project.",
            ));
        }
        // Retrying import must not silently downgrade an already-managed resource.
        if resource.project_id.is_none() {
            resource.mode = new_mode;
        }
        resource.project_id = Some(project_id.into());
        let data = encode(&resource)?;
        sqlx::query!(
            "UPDATE resources SET project_id = ?, data = ? WHERE id = ?",
            project_id,
            data,
            id
        )
        .execute(&mut **tx)
        .await
        .map_err(failure)?;
        resources.push(resource);
    }
    append(
        tx,
        actor,
        Some(project_id),
        "import.grouped",
        "Added resources found on the provider. Nothing changed there.",
    )
    .await?;
    Ok(resources)
}

async fn create_project_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: &str,
    value: &Project,
) -> Result<(), EngineError> {
    let project_id = &value.id;
    if sqlx::query!("SELECT id FROM projects WHERE id = ?", project_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(failure)?
        .is_some()
    {
        return Err(EngineError::Conflict("The project already exists."));
    }
    let data = encode(value)?;
    sqlx::query!(
        "INSERT INTO projects (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        value.id,
        data
    )
    .execute(&mut **tx)
    .await
    .map_err(failure)?;
    append(
        tx,
        actor,
        Some(&value.id),
        "app.created",
        "Created the project in Hoista.",
    )
    .await?;
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PageKey {
    kind: String,
    project: String,
    filter: String,
    first: String,
    second: String,
}

fn page_key(
    cursor: Option<&str>,
    kind: &str,
    project: &str,
    filter: &str,
) -> Result<(String, String), EngineError> {
    use base64::Engine as _;
    let Some(cursor) = cursor else {
        return Ok((String::new(), String::new()));
    };
    let invalid = || EngineError::Invalid("Use the next cursor from this same project and filter.");
    if cursor.len() > 2048 {
        return Err(invalid());
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| invalid())?;
    let value: PageKey = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if value.kind != kind
        || value.project != project
        || value.filter != filter
        || value.first.is_empty()
        || value.first.len() > 512
        || value.second.len() > 512
    {
        return Err(invalid());
    }
    Ok((value.first, value.second))
}

fn encode_page_key(
    kind: &str,
    project: &str,
    filter: &str,
    first: &str,
    second: &str,
) -> Result<String, EngineError> {
    use base64::Engine as _;
    let bytes = serde_json::to_vec(&PageKey {
        kind: kind.into(),
        project: project.into(),
        filter: filter.into(),
        first: first.into(),
        second: second.into(),
    })
    .map_err(|_| EngineError::Store)?;
    let cursor = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    // Never issue a cursor which this same build cannot read.
    page_key(Some(&cursor), kind, project, filter)?;
    Ok(cursor)
}

async fn admitted(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    admission: &DeploymentAdmission,
) -> Result<Option<Deploy>, EngineError> {
    let actor = admission.actor();
    let key = admission.key();
    let Some(receipt) = sqlx::query!(
        "SELECT fingerprint, deploy_id, project_id FROM deployment_admissions WHERE actor = ? AND request_key = ?",
        actor,
        key
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(failure)?
    else {
        return Ok(None);
    };
    if receipt.fingerprint != admission.fingerprint() {
        return Err(EngineError::Conflict(
            "This request key was already used for different deployment input. Inspect its original result; use a new key only for a new operation.",
        ));
    }
    let row = sqlx::query!(
        "SELECT version, status, cancel_requested, data FROM deploys WHERE id = ?",
        receipt.deploy_id
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(failure)?
    .ok_or(EngineError::Conflict(
        "This request was accepted but its deployment record was removed. It will not be submitted again; inspect the provider before starting a new operation.",
    ))?;
    let mut value: Deploy = decode(&row.data)?;
    if value.project_id != receipt.project_id || value.started_by != actor {
        return Err(EngineError::Store);
    }
    value.version = row.version;
    value.status = decode(&encode(&row.status)?)?;
    value.cancel_requested = row.cancel_requested != 0;
    Ok(Some(value))
}

async fn key_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<EnvironmentKeySnapshot, EngineError> {
    let rows = sqlx::query!("SELECT reference, commitment, active FROM environment_admission_keys WHERE retired = 0 ORDER BY reference LIMIT 18")
        .fetch_all(&mut **tx)
        .await
        .map_err(failure)?;
    let count =
        sqlx::query!("SELECT count(*) AS count FROM environment_admission_keys WHERE retired = 1")
            .fetch_one(&mut **tx)
            .await
            .map_err(failure)?;
    let mut snapshot = EnvironmentKeySnapshot {
        active: None,
        pending: Vec::new(),
        retired: u32::try_from(count.count).unwrap_or(u32::MAX),
    };
    for row in rows {
        if row.active == 1 {
            if snapshot.active.is_some() {
                return Err(EngineError::Store);
            }
            snapshot.active = Some(EnvironmentAdmissionKey {
                reference: row.reference,
                commitment: row.commitment.ok_or(EngineError::Store)?,
            });
        } else {
            snapshot.pending.push(row.reference);
        }
    }
    if snapshot.pending.len() > 16 {
        return Err(EngineError::Store);
    }
    Ok(snapshot)
}

pub(super) async fn insert_backup(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    backup: &crate::domain::backup::Backup,
) -> Result<(), EngineError> {
    for value in &backup.accounts {
        let data = encode(value)?;
        let provider = label(&value.provider)?;
        sqlx::query!(
            "INSERT INTO accounts(id,provider,data) VALUES(?,?,?)",
            value.id,
            provider,
            data
        )
        .execute(&mut **tx)
        .await
        .map_err(failure)?;
    }
    for value in &backup.environments {
        if value.id == DEFAULT_ENVIRONMENT {
            continue;
        }
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO environments(id,data) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            value.id,
            data
        )
        .execute(&mut **tx)
        .await
        .map_err(failure)?;
    }
    for value in &backup.projects {
        let data = encode(value)?;
        sqlx::query!("INSERT INTO projects(id,data) VALUES(?,?)", value.id, data)
            .execute(&mut **tx)
            .await
            .map_err(failure)?;
    }
    for value in &backup.resources {
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO resources(id,project_id,account_id,provider_ref,data) VALUES(?,?,?,?,?)",
            value.id,
            value.project_id,
            value.account_id,
            value.provider_ref,
            data
        )
        .execute(&mut **tx)
        .await
        .map_err(failure)?;
    }
    for value in &backup.links {
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO links(id,from_resource,to_resource,data) VALUES(?,?,?,?)",
            value.id,
            value.from_resource,
            value.to_resource,
            data
        )
        .execute(&mut **tx)
        .await
        .map_err(failure)?;
    }
    for value in &backup.custom_domains {
        let data = encode(value)?;
        sqlx::query!(
            "INSERT INTO custom_domains(id,project_id,resource_id,hostname,data) VALUES(?,?,?,?,?)",
            value.id,
            value.project_id,
            value.resource_id,
            value.hostname,
            data
        )
        .execute(&mut **tx)
        .await
        .map_err(failure)?;
    }
    let data = encode(&backup.settings)?;
    sqlx::query!(
        "INSERT INTO settings(id,data) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        data
    )
    .execute(&mut **tx)
    .await
    .map_err(failure)?;
    Ok(())
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use crate::{paths::StatePaths, store::SqliteStore};

    async fn old_event(store: &SqliteStore, days: i64) {
        let at = (chrono::Utc::now() - chrono::Duration::days(days))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query("INSERT INTO events(at,actor,project_id,kind,summary) VALUES(?,?,?,?,?)")
            .bind(at)
            .bind("person")
            .bind(Option::<String>::None)
            .bind("test.old")
            .bind(format!("{days} days ago"))
            .execute(&store.pool)
            .await
            .expect("event");
    }

    #[tokio::test]
    async fn recent_activity_can_never_be_deleted_and_old_history_is_pruned() {
        let temp = tempfile::tempdir().expect("temp");
        let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
        let store = SqliteStore::open(&paths).await.expect("store");
        old_event(&store, 400).await;
        old_event(&store, 40).await;
        old_event(&store, 2).await;
        store
            .event("person", None, "test.now", "Just now.")
            .await
            .expect("event");

        // Nothing can delete the last 30 days, not even a direct statement.
        assert!(
            sqlx::query("DELETE FROM events WHERE kind='test.now'")
                .execute(&store.pool)
                .await
                .is_err()
        );
        // Pruning to a year keeps the 40-day-old one; asking for "now"
        // still stops at 30 days.
        let year = (chrono::Utc::now() - chrono::Duration::days(365))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        assert_eq!(store.prune_history(&year).await.expect("pruned"), 1);
        let everything = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        assert_eq!(store.prune_history(&everything).await.expect("pruned"), 1);
        let left: Vec<String> = sqlx::query_scalar("SELECT summary FROM events ORDER BY id")
            .fetch_all(&store.pool)
            .await
            .expect("left");
        assert_eq!(left, vec!["2 days ago".to_owned(), "Just now.".to_owned()]);

        // An unclear operation is kept however old it is: it still needs an answer.
        let old = (chrono::Utc::now() - chrono::Duration::days(400))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        for (id, state) in [
            ("op-done", crate::domain::model::OperationState::Done),
            ("op-unclear", crate::domain::model::OperationState::Unclear),
        ] {
            store
                .put_operation(&crate::domain::model::Operation {
                    id: id.into(),
                    provider: crate::Provider::Railway,
                    account: "ws".into(),
                    kind: "create".into(),
                    target: "service web".into(),
                    state,
                    repeats: true,
                    started_at: old.clone(),
                    finished_at: Some(old.clone()),
                    message: None,
                    key: id.into(),
                })
                .await
                .expect("operation");
        }
        store.prune_history(&everything).await.expect("pruned");
        assert!(store.operation("op-done").await.expect("read").is_none());
        assert!(store.operation("op-unclear").await.expect("read").is_some());
        store.close().await;
    }
}
