//! Database exports: finding every database on a resource (so none is missed before a
//! delete) and exporting them.

use crate::data::DataService;
use crate::engine::{Actor, Engine};
use crate::{
    domain::{
        database_export::{DatabaseArchive, DatabaseTarget},
        model::*,
    },
    ports::{catalog::EngineError, database_export::DatabaseExportSource},
};
use std::collections::BTreeSet;

/// The most databases one delete can export first.
const MAX_DATABASES: usize = 10_000;

impl Engine {
    pub(crate) async fn database_targets(
        &self,
        resources: &[Resource],
    ) -> Result<Vec<DatabaseTarget>, EngineError> {
        let mut targets = Vec::new();
        for resource in resources
            .iter()
            .filter(|resource| resource.kind == ResourceKind::Database)
        {
            let account = self
                .repository
                .account(&resource.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let token = self.accounts().account_token(&account.id).await?;
            let databases = self
                .adapter(resource.provider)?
                .database_inventory(&token, &account.external_id, &resource.provider_ref)
                .await?;
            if databases.len() > MAX_DATABASES {
                return Err(EngineError::Conflict(
                    "The provider did not return a complete bounded database inventory.",
                ));
            }
            let mut seen = BTreeSet::new();
            for database in databases {
                if !seen.insert((database.branch_id.clone(), database.database_name.clone())) {
                    return Err(EngineError::Conflict(
                        "The provider returned duplicate database identities.",
                    ));
                }
                targets.push(DatabaseTarget {
                    resource_id: resource.id.clone(),
                    database,
                });
                if targets.len() > MAX_DATABASES {
                    return Err(EngineError::Conflict(
                        "The deletion contains more than 10,000 databases.",
                    ));
                }
            }
        }
        targets.sort_by(|a, b| (&a.resource_id, &a.database).cmp(&(&b.resource_id, &b.database)));
        Ok(targets)
    }

    pub(crate) async fn export_targets(
        &self,
        actor: &Actor,
        targets: &[DatabaseTarget],
    ) -> Result<Vec<DatabaseArchive>, EngineError> {
        if targets.is_empty() {
            return Ok(vec![]);
        }
        let exporter = self
            .database_exporter
            .as_ref()
            .filter(|exporter| exporter.available())
            .ok_or(EngineError::Conflict(
                "Database export tools are unavailable. Install pg_dump and pg_restore before deleting database data.",
            ))?;
        let mut archives = Vec::new();
        for target in targets {
            let resource = self
                .repository
                .resource(&target.resource_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let account = self
                .repository
                .account(&resource.account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let token = self.accounts().account_token(&account.id).await?;
            let connection = self
                .adapter(resource.provider)?
                .database_export_connection(
                    &token,
                    &account.external_id,
                    &resource.provider_ref,
                    &target.database,
                )
                .await?;
            let archive = exporter
                .export(
                    &resource.id,
                    &DatabaseExportSource {
                        branch_id: target.database.branch_id.clone(),
                        database_name: target.database.database_name.clone(),
                        connection,
                    },
                )
                .await?;
            if archive.resource_id != resource.id
                || archive.branch_id != target.database.branch_id
                || archive.database_name != target.database.database_name
            {
                return Err(EngineError::Conflict(
                    "Database export returned a mismatched receipt.",
                ));
            }
            self.repository
                .event(
                    actor.id(),
                    resource.project_id.as_deref(),
                    "database.exported",
                    "Saved and verified a private PostgreSQL data archive.",
                )
                .await?;
            archives.push(archive);
        }
        Ok(archives)
    }
}

impl<'a> DataService<'a> {
    /// Lists the person's completed database exports, without reading the archives.
    ///
    /// # Errors
    /// Rejects agents, invalid cursors and unavailable or inconsistent archive storage.
    pub async fn database_archives(
        self,
        actor: &Actor,
        cursor: Option<&str>,
    ) -> Result<crate::ports::catalog::Page<DatabaseArchive>, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::ApprovalRequired);
        }
        self.engine
            .database_exporter
            .as_ref()
            .ok_or(EngineError::Conflict(
                "Database archive storage is unavailable.",
            ))?
            .archives(cursor)
            .await
    }

    /// Exports every active database on one resource to a local archive; deletes nothing.
    /// Only the person can start this.
    ///
    /// # Errors
    /// Rejects unsupported resources, agents, incomplete inventories and failed exports.
    pub async fn export_database(
        self,
        actor: &Actor,
        id: &str,
    ) -> Result<Vec<DatabaseArchive>, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::ApprovalRequired);
        }
        let _mutation = self.engine.mutation.lock().await;
        let resource = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.kind != ResourceKind::Database {
            return Err(EngineError::Invalid(
                "Select a database resource to export.",
            ));
        }
        let targets = self
            .engine
            .database_targets(std::slice::from_ref(&resource))
            .await?;
        let archives = self.engine.export_targets(actor, &targets).await?;
        if targets != self.engine.database_targets(&[resource]).await? {
            return Err(EngineError::Conflict(
                "The provider database inventory changed during export; saved archives are retained, but coverage must be checked again.",
            ));
        }
        Ok(archives)
    }
}
