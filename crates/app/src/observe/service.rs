//! The observe service: logs, metrics, diagnostics and the operation journal.

use crate::engine::*;
use crate::ports::catalog::EngineError;
use std::ops::RangeInclusive;

/// The longest range of metrics asked for at once.
const MAX_METRICS_DAYS: i64 = 31;
/// Seconds between metric points: from a minute to a day.
const METRICS_STEP_SECONDS: RangeInclusive<u32> = 60..=86_400;

/// Logs, metrics, diagnostics and the operation journal.
#[derive(Clone, Copy)]
pub struct ObserveService<'a> {
    pub(crate) engine: &'a Engine,
}

impl Engine {
    /// Logs, metrics, diagnostics and the operation journal.
    #[must_use]
    pub fn observe(&self) -> ObserveService<'_> {
        ObserveService { engine: self }
    }
}

impl<'a> ObserveService<'a> {
    /// The build and runtime log lines of one of a resource's deployments at
    /// its provider.
    ///
    /// # Errors
    /// Fails without access, for a missing resource or account, or when the
    /// provider has no such logs.
    pub async fn provider_deployment_logs(
        self,
        actor: &Actor,
        resource_id: &str,
        deployment: &str,
    ) -> Result<Vec<crate::domain::logs::LogEntry>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .adapter(resource.provider)?
            .provider_deployment_logs(
                &token,
                &account.external_id,
                &resource.provider_ref,
                deployment,
            )
            .await
    }

    /// A resource's metrics as its provider measures them, for a range of at
    /// most 31 days.
    ///
    /// # Errors
    /// Reports missing permission, an invalid range, a provider without
    /// metrics or provider failure.
    pub async fn provider_metrics(
        self,
        actor: &Actor,
        resource_id: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        step_seconds: u32,
    ) -> Result<crate::ports::providers::ProviderMetrics, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        if to <= from || to - from > chrono::Duration::days(MAX_METRICS_DAYS) {
            return Err(EngineError::Invalid("Choose a range of at most 31 days."));
        }
        if !METRICS_STEP_SECONDS.contains(&step_seconds) {
            return Err(EngineError::Invalid(
                "Choose a step between a minute and a day.",
            ));
        }
        let resource = self
            .engine
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        self.engine
            .adapter(resource.provider)?
            .provider_metrics(
                &token,
                &account.external_id,
                &resource.provider_ref,
                from,
                to,
                step_seconds,
            )
            .await
    }
}
