//! Changing Hoista's own settings. Settings take their own lock, not the
//! one held through long provider writes, so they never wait on a deploy.

use crate::engine::{Actor, Engine};
use crate::{domain::model::*, ports::catalog::EngineError};

/// Hoista's own settings.
#[derive(Clone, Copy)]
pub struct SettingsService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Hoista's own settings.
    #[must_use]
    pub fn settings(&self) -> SettingsService<'_> {
        SettingsService { engine: self }
    }
}

impl<'a> SettingsService<'a> {
    /// Checks and saves the settings given. Turning on anything that opens
    /// Hoista up asks the person to confirm it's them first.
    ///
    /// # Errors
    /// Fails for an agent, a value outside its choices, a declined
    /// confirmation, or when settings can't be saved.
    pub async fn update_settings(
        self,
        actor: &Actor,
        input: SettingsPatch,
    ) -> Result<Settings, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the local person can change engine settings.",
            ));
        }
        let _mutation = self.engine.settings_changes.lock().await;
        let mut settings = self.engine.repository.settings().await?;
        if let Some(value) = input.agents_enabled {
            settings.agents_enabled = value;
        }
        if let Some(value) = input.idle_days {
            settings.idle_days = value;
        }
        if let Some(value) = input.agent_expiry_days {
            settings.agent_expiry_days = value;
        }
        if let Some(value) = input.check_updates {
            settings.check_updates = value;
        }
        if let Some(value) = input.network_page {
            if value && !settings.network_page {
                // Opens the consent page to other devices on the network.
                self.engine
                    .confirm_owner("open Hoista's page to your network")
                    .await?;
            }
            settings.network_page = value;
        }
        if let Some(value) = input.mcp_http {
            if value && !settings.mcp_http {
                // Lets web agents sign in again.
                self.engine
                    .confirm_owner("turn on MCP over HTTP for web agents")
                    .await?;
            }
            settings.mcp_http = value;
        }
        if let Some(value) = input.default_access {
            if !AGENT_ACCESS_LEVELS.contains(&value.as_str()) {
                return Err(EngineError::Invalid(
                    "Start new agents on view, ask or full access.",
                ));
            }
            settings.default_access = value;
        }
        if let Some(value) = input.unisolated_builds {
            if value && !settings.unisolated_builds {
                // Project build code would then run with this account's access.
                self.engine
                    .confirm_owner("build projects without isolating them")
                    .await?;
            }
            settings.unisolated_builds = value;
        }
        if let Some(value) = input.backup_every_hours {
            if !BACKUP_INTERVALS_HOURS.contains(&value) {
                return Err(EngineError::Invalid(
                    "Back up every 6 hours, every day, every week, or never.",
                ));
            }
            settings.backup_every_hours = value;
        }
        if let Some(value) = input.backup_keep {
            if !BACKUPS_KEPT.contains(&value) {
                return Err(EngineError::Invalid("Keep from 1 to 100 backups."));
            }
            settings.backup_keep = value;
        }
        if let Some(value) = input.history_days {
            if !HISTORY_DAYS.contains(&value) {
                return Err(EngineError::Invalid(
                    "Keep history for 30 days, 90 days, a year or three years.",
                ));
            }
            settings.history_days = value;
        }
        if let Some(value) = input.crash_reports {
            settings.crash_reports = value;
        }
        if !SETTING_DAYS.contains(&settings.idle_days)
            || !SETTING_DAYS.contains(&settings.agent_expiry_days)
        {
            return Err(EngineError::Invalid("Use 1–365 days."));
        }
        self.engine.repository.put_settings(&settings).await?;
        Ok(settings)
    }

    /// How project build code is kept apart from the rest of this computer: its
    /// name, or why it can't be.
    ///
    /// # Errors
    /// The reason, when builds can't be isolated here.
    pub fn build_isolation(self) -> Result<&'static str, &'static str> {
        self.engine.builder.isolation()
    }
}
