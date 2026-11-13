//! Backups of Hoista's own records (no credentials): export, checking and restore.

use crate::data::DataService;
use crate::domain::domains::{DomainReadiness, belongs_to_zone};
use crate::{
    domain::{
        backup::{
            BACKUP_VERSION, Backup, MAX_ACCOUNTS, MAX_CUSTOM_DOMAINS, MAX_LINKS, MAX_PROJECTS,
            MAX_RESOURCES,
        },
        model::*,
    },
    ports::{
        backups::{BackupFile, BackupFolder},
        catalog::EngineError,
    },
};
use std::{sync::Arc, time::Duration};

impl<'a> DataService<'a> {
    /// Exports Hoista's records, without credentials or agent grants.
    ///
    /// # Errors
    /// Reports storage failures or a catalog over its size limits.
    pub async fn export(self) -> Result<Backup, EngineError> {
        let mut projects = Vec::new();
        let mut links = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.engine.repository.projects(cursor.as_deref()).await?;
            for project in &page.items {
                links.extend(self.engine.repository.links(&project.id).await?);
            }
            projects.extend(page.items);
            if projects.len() > MAX_PROJECTS {
                return Err(EngineError::Conflict("The backup exceeds 10,000 projects."));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(Backup {
            version: BACKUP_VERSION,
            custom_domains: self.engine.environments().all_domains().await?,
            accounts: self.engine.accounts().all_accounts().await?,
            projects,
            resources: self.engine.projects().all_resources().await?,
            links,
            settings: self.engine.repository.settings().await?,
            environments: self
                .engine
                .repository
                .environments()
                .await?
                .into_iter()
                .filter(|environment| environment.id != DEFAULT_ENVIRONMENT)
                .collect(),
        })
    }

    /// Restores a backup after checking every relationship. Every project comes back watched.
    ///
    /// # Errors
    /// Rejects unsupported formats, oversized catalogs, invalid relationships or a catalog
    /// that isn't empty.
    pub async fn restore(self, backup: Backup) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        let backup = validate_backup(backup)?;
        self.engine.repository.restore(&backup).await
    }
}

/// Checks a backup's relationships and sets everything to watching. Used by both backup
/// restore and transfer restore.
///
/// # Errors
/// Rejects unsupported versions, size limits, invalid settings or inconsistent relationships.
pub(crate) fn validate_backup(mut backup: Backup) -> Result<Backup, EngineError> {
    if backup.version != BACKUP_VERSION
        || backup.projects.len() > MAX_PROJECTS
        || backup.resources.len() > MAX_RESOURCES
        || backup.accounts.len() > MAX_ACCOUNTS
        || backup.links.len() > MAX_LINKS
        || backup.custom_domains.len() > MAX_CUSTOM_DOMAINS
    {
        return Err(EngineError::Invalid(
            "Unsupported or oversized Hoista backup.",
        ));
    }
    let settings = &backup.settings;
    if !SETTING_DAYS.contains(&settings.idle_days)
        || !SETTING_DAYS.contains(&settings.agent_expiry_days)
    {
        return Err(EngineError::Invalid(
            "The backup contains invalid settings.",
        ));
    }
    let accounts = backup
        .accounts
        .iter()
        .map(|a| (a.id.clone(), a.provider))
        .collect::<std::collections::HashMap<_, _>>();
    let projects = backup
        .projects
        .iter()
        .map(|a| a.id.clone())
        .collect::<std::collections::HashSet<_>>();
    if accounts.len() != backup.accounts.len() || projects.len() != backup.projects.len() {
        return Err(EngineError::Invalid("Backup ids must be unique."));
    }
    for account in &mut backup.accounts {
        account.status = AccountStatus::Disconnected;
        account.last_discovery = None;
        account.scopes.clear();
    }
    for project in &mut backup.projects {
        if project.environment_id != DEFAULT_ENVIRONMENT
            && !backup
                .environments
                .iter()
                .any(|environment| environment.id == project.environment_id)
        {
            return Err(EngineError::Invalid(
                "A backed-up project is in an environment the backup doesn't have.",
            ));
        }
        if !valid_slug(&project.name) {
            return Err(EngineError::Invalid(
                "The backup contains an invalid project name.",
            ));
        }
        project.mode = Mode::Watching;
    }
    for resource in &mut backup.resources {
        if accounts.get(&resource.account_id) != Some(&resource.provider)
            || resource
                .project_id
                .as_ref()
                .is_some_and(|id| !projects.contains(id))
        {
            return Err(EngineError::Invalid(
                "A backed-up resource refers to a missing project or account.",
            ));
        }
        resource.mode = Mode::Watching;
        resource.status = ResourceStatus::Unknown;
    }
    let resources = backup
        .resources
        .iter()
        .map(|p| (&p.id, &p.project_id))
        .collect::<std::collections::HashMap<_, _>>();
    if resources.len() != backup.resources.len() {
        return Err(EngineError::Invalid("Resource ids must be unique."));
    }
    for link in &backup.links {
        let from = resources
            .get(&link.from_resource)
            .ok_or(EngineError::Invalid(
                "A map edge refers to a missing resource.",
            ))?;
        let to = resources
            .get(&link.to_resource)
            .ok_or(EngineError::Invalid(
                "A map edge refers to a missing resource.",
            ))?;
        if from.is_none() || to.is_none() || link.from_resource == link.to_resource {
            return Err(EngineError::Invalid(
                "Map edges must connect distinct resources assigned to projects.",
            ));
        }
    }
    for domain in &mut backup.custom_domains {
        if !belongs_to_zone(&domain.hostname, &domain.observed.zone_name)
            || resources
                .get(&domain.resource_id)
                .and_then(|owner| owner.as_deref())
                != Some(domain.project_id.as_str())
        {
            return Err(EngineError::Invalid(
                "A backed-up hostname has an invalid resource or zone.",
            ));
        }
        domain.observed.dns = DomainReadiness::Unknown;
        domain.observed.https = DomainReadiness::Unknown;
    }
    Ok(backup)
}

/// How often Hoista looks whether a backup is due.
const BACKUP_CHECK: Duration = Duration::from_secs(10 * 60);
/// The first look waits until start-up work has settled.
const FIRST_BACKUP_CHECK: Duration = Duration::from_secs(60);

impl<'a> DataService<'a> {
    fn backup_folder(self) -> Result<Arc<dyn BackupFolder>, EngineError> {
        self.engine
            .backup_folder
            .clone()
            .ok_or_else(|| EngineError::Unsupported("Backups aren't kept here.".into()))
    }

    /// Saved backups, newest first.
    ///
    /// # Errors
    /// Reports a backup folder that can't be read.
    pub async fn backups(self) -> Result<Vec<BackupFile>, EngineError> {
        let folder = self.backup_folder()?;
        tokio::task::spawn_blocking(move || folder.list())
            .await
            .map_err(|_| EngineError::Store)?
    }

    /// Saves a backup now and keeps only the newest ones, as many as the person chose.
    ///
    /// # Errors
    /// Reports unreadable state or a backup folder that can't be written.
    pub async fn save_backup(self) -> Result<BackupFile, EngineError> {
        let backup = self.export().await?;
        let bytes = serde_json::to_vec_pretty(&backup).map_err(|_| EngineError::Store)?;
        let folder = self.backup_folder()?;
        let saved = {
            let folder = folder.clone();
            tokio::task::spawn_blocking(move || folder.save(&bytes))
                .await
                .map_err(|_| EngineError::Store)??
        };
        let keep = usize::from(self.engine.repository.settings().await?.backup_keep.max(1));
        for old in self.backups().await?.into_iter().skip(keep) {
            folder.remove(&old.name);
        }
        self.engine
            .repository
            .event(
                "hoista",
                None,
                "backup.saved",
                &format!(
                    "Saved a backup: {} projects, {} accounts.",
                    backup.projects.len(),
                    backup.accounts.len()
                ),
            )
            .await?;
        Ok(saved)
    }

    /// Restores a saved backup into this Hoista, which must be empty. Projects
    /// come back watching; accounts reconnect from the keychain as before.
    ///
    /// # Errors
    /// Rejects unknown names, unreadable files and a Hoista that isn't empty.
    pub async fn restore_backup(self, name: &str) -> Result<(), EngineError> {
        let folder = self.backup_folder()?;
        let name = name.to_owned();
        let bytes = tokio::task::spawn_blocking(move || folder.read(&name))
            .await
            .map_err(|_| EngineError::Store)??;
        let backup = serde_json::from_slice(&bytes)
            .map_err(|_| EngineError::Invalid("That backup can't be read."))?;
        self.restore(backup).await
    }

    /// Saves a backup when the newest is older than the person's interval.
    ///
    /// # Errors
    /// Reports unreadable settings or a backup that couldn't be saved.
    pub async fn backup_if_due(self) -> Result<(), EngineError> {
        let settings = self.engine.repository.settings().await?;
        if settings.backup_every_hours == 0 {
            return Ok(());
        }
        let newest = self.backups().await?.into_iter().next();
        let due = newest.is_none_or(|newest| {
            chrono::DateTime::parse_from_rfc3339(&newest.at).is_ok_and(|at| {
                chrono::Utc::now().signed_duration_since(at)
                    >= chrono::Duration::hours(i64::from(settings.backup_every_hours))
            })
        });
        if due {
            self.save_backup().await?;
        }
        Ok(())
    }
}

impl crate::engine::Engine {
    /// Saves a backup whenever one is due, until `stop`.
    pub async fn run_backups(
        self: std::sync::Arc<Self>,
        stop: tokio_util::sync::CancellationToken,
    ) {
        // Shortly after starting, then every few minutes.
        let mut wait = FIRST_BACKUP_CHECK;
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
            }
            wait = BACKUP_CHECK;
            if let Err(error) = self.data().backup_if_due().await {
                tracing::warn!(code = "backups.failed", %error);
            }
        }
    }
}
