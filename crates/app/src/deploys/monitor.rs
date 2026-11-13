//! The background monitor: reads what providers measured and marks idle resources.

use crate::deploys::DeployService;
use crate::engine::Engine;
use crate::{domain::model::*, ports::catalog::EngineError};
use chrono::Datelike;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

use std::collections::HashSet;

/// How often the monitor goes round, and how soon a resource with no
/// reading yet is looked at again.
const CYCLE: Duration = Duration::from_secs(60);
const ACCOUNT_CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
const DOMAIN_CHECK_EVERY: Duration = Duration::from_secs(5 * 60);
const BINDINGS_CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
const RESOURCE_CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
/// A sleeping, paused or idle resource changes less often.
const RESTING_CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// The fewest days of usage kept fresh, however short the idle setting.
const MIN_USAGE_DAYS: u16 = 7;
impl Engine {
    /// Polls active resources every fifteen minutes and sleeping resources hourly.
    ///
    /// # Errors
    /// Returns local state errors it can't recover from. Provider failures are recorded; no usage
    /// is made up.
    pub async fn run_monitor(
        self: Arc<Self>,
        shutdown: CancellationToken,
    ) -> Result<(), EngineError> {
        let mut checked: HashMap<String, Instant> = HashMap::new();
        // Accounts already connected get their provider's tool set up once, now.
        if let Ok(accounts) = self.accounts().all_accounts().await {
            let mut seen = HashSet::new();
            for account in accounts {
                if account.status == AccountStatus::Connected
                    && seen.insert(account.provider)
                    && let Ok(adapter) = self.adapter(account.provider)
                {
                    adapter.prepare_tools();
                }
            }
        }
        let every = CYCLE;
        let mut last_cycle: Option<Instant> = None;
        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            if last_cycle.is_none_or(|at| at.elapsed() >= every) {
                self.deploys()
                    .monitor_cycle(&mut checked, &shutdown)
                    .await?;
                last_cycle = Some(Instant::now());
            }
            // Accounts an agent or the person just changed are read again
            // between cycles.
            self.run_due_rescans().await?;
            let until_cycle = every.saturating_sub(last_cycle.map_or(every, |at| at.elapsed()));
            let wait = self
                .rescans
                .next_in()
                .map_or(until_cycle, |next| next.min(until_cycle));
            tokio::select! {
                () = shutdown.cancelled() => return Ok(()),
                () = tokio::time::sleep(wait) => {}
                () = self.rescans.changed() => {}
            }
        }
    }
}

impl<'a> DeployService<'a> {
    async fn monitor_cycle(
        self,
        checked: &mut HashMap<String, Instant>,
        shutdown: &CancellationToken,
    ) -> Result<(), EngineError> {
        let idle_days = self.engine.repository.settings().await?.idle_days;
        let resources = self.engine.projects().all_resources().await?;
        let accounts = self.engine.accounts().all_accounts().await?;
        let domains = self.engine.environments().all_domains().await?;
        checked.retain(|id, _| {
            resources
                .iter()
                .any(|resource| &resource.id == id || id == &format!("bindings:{}", resource.id))
                || accounts
                    .iter()
                    .any(|account| id == &format!("account:{}", account.id))
                || domains
                    .iter()
                    .any(|domain| id == &format!("domain:{}", domain.id))
        });
        let mut account_checks = 0;
        for account in accounts {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            let key = format!("account:{}", account.id);
            if account.status == AccountStatus::Disconnected
                || checked
                    .get(&key)
                    .is_some_and(|at| at.elapsed() < ACCOUNT_CHECK_EVERY)
            {
                continue;
            }
            if account_checks == 4 {
                break;
            }
            account_checks += 1;
            checked.insert(key, Instant::now());
            match self.engine.accounts().check_account(&account.id).await {
                // Statuses change at the provider (a provider project paused or resumed):
                // read them again while the credential is good.
                Ok(checked) if checked.status == AccountStatus::Connected => {
                    if let Err(error) = self.engine.accounts().refresh_found(&account.id).await {
                        self.engine
                            .repository
                            .event("engine", None, "account.refresh_unavailable", error.code())
                            .await?;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.engine
                        .repository
                        .event("engine", None, "account.check_unavailable", error.code())
                        .await?;
                }
            }
        }
        for domain in domains {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            let key = format!("domain:{}", domain.id);
            if checked
                .get(&key)
                .is_some_and(|at| at.elapsed() < DOMAIN_CHECK_EVERY)
            {
                continue;
            }
            checked.insert(key, Instant::now());
            if let Err(error) = self.engine.environments().refresh_domain(&domain.id).await {
                if matches!(error, EngineError::Watching | EngineError::NotFound) {
                    continue;
                }
                self.engine
                    .repository
                    .event(
                        "engine",
                        Some(&domain.project_id),
                        "domain.check_failed",
                        error.code(),
                    )
                    .await?;
            }
        }
        let mut binding_checks = 0;
        for resource in &resources {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            let reports = self
                .engine
                .adapter(resource.provider)
                .is_ok_and(|adapter| adapter.reports_bindings(&resource.provider_ref));
            if !reports || resource.project_id.is_none() {
                continue;
            }
            let key = format!("bindings:{}", resource.id);
            if checked
                .get(&key)
                .is_some_and(|at| at.elapsed() < BINDINGS_CHECK_EVERY)
            {
                continue;
            }
            if binding_checks == 4 {
                break;
            }
            binding_checks += 1;
            checked.insert(key, Instant::now());
            if let Err(error) = self
                .engine
                .projects()
                .refresh_bindings(&crate::engine::Actor::Person, &resource.id)
                .await
            {
                self.engine
                    .repository
                    .event(
                        "engine",
                        resource.project_id.as_deref(),
                        "map.bindings_unavailable",
                        error.code(),
                    )
                    .await?;
            }
        }
        for resource in &resources {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            let adapter = self.engine.adapter(resource.provider)?;
            // Every resource's status is read from its provider, whether or
            // not the provider also reports usage: some report no usage but
            // still say whether a resource is live, asleep or deploying.
            let usage = adapter.capabilities().usage;
            // A resource Hoista has no reading for yet (new, or building) is
            // looked at again every minute, so it doesn't sit at Unknown.
            let interval = match resource.status {
                ResourceStatus::Unknown
                | ResourceStatus::Deploying
                | ResourceStatus::NotDeployed => CYCLE,
                ResourceStatus::Sleeping | ResourceStatus::Paused | ResourceStatus::Idle => {
                    RESTING_CHECK_EVERY
                }
                _ => RESOURCE_CHECK_EVERY,
            };
            if checked
                .get(&resource.id)
                .is_some_and(|at| at.elapsed() < interval)
            {
                continue;
            }
            checked.insert(resource.id.clone(), Instant::now());
            let result = async {
                let account = self
                    .engine
                    .repository
                    .account(&resource.account_id)
                    .await?
                    .ok_or(EngineError::NotFound)?;
                let token = self.engine.accounts().account_token(&account.id).await?;
                // Managed resources too: the provider knows best whether a
                // service Hoista deployed is running, asleep or stopped.
                self.engine
                    .deploys()
                    .observe_watched(adapter.as_ref(), &token, &account.external_id, resource)
                    .await?;
                if !usage {
                    return Ok(());
                }
                // Refresh recent usage, and fill in at most four missing days a pass.
                let today = chrono::Utc::now().date_naive();
                let start = today
                    .with_day(1)
                    .ok_or(EngineError::Store)?
                    .min(today - chrono::Duration::days(i64::from(idle_days.max(MIN_USAGE_DAYS))));
                let recorded = if let Some(project_id) = &resource.project_id {
                    self.engine
                        .repository
                        .usage(project_id, &start.to_string())
                        .await?
                } else {
                    vec![]
                };
                let mut dates = vec![today, today.pred_opt().ok_or(EngineError::Store)?];
                if resource.project_id.is_some() {
                    for date in start.iter_days().take_while(|date| *date < today) {
                        if dates.len() >= 6 {
                            break;
                        }
                        if !dates.contains(&date)
                            && !recorded.iter().any(|row| {
                                row.resource_id == resource.id && row.day == date.to_string()
                            })
                        {
                            dates.push(date);
                        }
                    }
                }
                for date in dates {
                    if shutdown.is_cancelled() {
                        return Ok(());
                    }
                    let usage = adapter
                        .usage(&token, &account.external_id, resource, &date.to_string())
                        .await?;
                    self.engine.repository.put_usage(&usage).await?;
                }
                Ok::<(), EngineError>(())
            }
            .await;
            // A provider that reports no usage for this kind of resource is fine;
            // only real failures are noted. A status read that failed is
            // tried again on the next pass, with nothing to note.
            if usage
                && let Err(error) = result
                && error.code() != "provider.not_supported"
            {
                self.engine
                    .repository
                    .event(
                        "engine",
                        resource.project_id.as_deref(),
                        "usage.unavailable",
                        error.code(),
                    )
                    .await?;
            }
        }
        let mut cursor = None;
        loop {
            let page = self.engine.repository.projects(cursor.as_deref()).await?;
            for listed_project in page.items {
                if shutdown.is_cancelled() {
                    return Ok(());
                }
                // Serialize status changes with deploy, disconnect, mode and deletion.
                // Re-read after acquiring the lock rather than using the earlier snapshot.
                let _mutation = self.engine.mutation.lock().await;
                let Some(project) = self.engine.repository.project(&listed_project.id).await?
                else {
                    continue;
                };
                let mut current_resources = self.engine.projects().all_resources().await?;
                let today = chrono::Utc::now().date_naive();
                let idle_days = self.engine.repository.settings().await?.idle_days;
                let since = (today - chrono::Duration::days(i64::from(idle_days))).to_string();
                let activity = self.engine.repository.usage(&project.id, &since).await?;
                for resource in current_resources
                    .iter_mut()
                    .filter(|resource| resource.project_id.as_deref() == Some(&project.id))
                {
                    let status = activity_status(resource, &activity, today, idle_days);
                    if status != resource.status {
                        resource.status = status;
                        // put_resource commits metadata and its resource.updated event together.
                        self.engine.repository.put_resource(resource).await?;
                    }
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(())
    }
}

impl<'a> DeployService<'a> {
    /// Records a resource's provider-reported state (and, when watched, its URL
    /// and kind) when they changed. Providers that do not report state leave
    /// the resource as it is.
    async fn observe_watched(
        self,
        adapter: &dyn crate::ports::providers::ProviderAdapter,
        token: &crate::domain::Secret,
        external_id: &str,
        resource: &Resource,
    ) -> Result<(), EngineError> {
        let observed = match adapter.observe_resource(token, external_id, resource).await {
            Ok(observed) => observed,
            Err(EngineError::Provider { code, .. }) if code == "provider.not_supported" => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let status = observed.status;
        let _mutation = self.engine.mutation.lock().await;
        let Some(mut current) = self.engine.repository.resource(&resource.id).await? else {
            return Ok(());
        };
        let managed = current.mode != Mode::Watching;
        // A managed resource keeps the address and kind its deploy recorded,
        // and a deploy under way owns its status until it finishes.
        let url = if managed {
            current.url.clone().or(observed.url)
        } else {
            observed.url.or_else(|| current.url.clone())
        };
        let kind = if managed {
            current.kind
        } else {
            observed.kind.unwrap_or(current.kind)
        };
        // Only a Hoista deploy still running holds "deploying": one the
        // provider started itself (making the resource, a restart) is over
        // when the provider says so.
        let deploying_here = managed
            && current.status == ResourceStatus::Deploying
            && match current.project_id.as_deref() {
                Some(project) => self
                    .engine
                    .repository
                    .deploys(project, None)
                    .await?
                    .items
                    .iter()
                    .any(|deploy| {
                        matches!(deploy.status, DeployStatus::Queued | DeployStatus::Running)
                    }),
                None => false,
            };
        if deploying_here
            || (managed && status == ResourceStatus::Unknown)
            || (current.status == status && current.url == url && current.kind == kind)
        {
            return Ok(());
        }
        current.status = status;
        current.url = url;
        current.kind = kind;
        self.engine.repository.put_resource(&current).await?;
        // Emitted for a new URL or role too, so open screens refresh.
        self.engine
            .repository
            .event(
                "engine",
                current.project_id.as_deref(),
                "resource.observed",
                &format!(
                    "{} is {}",
                    current.name,
                    match status {
                        ResourceStatus::Live => "live",
                        ResourceStatus::Sleeping => "sleeping",
                        ResourceStatus::Idle => "idle",
                        ResourceStatus::Failing => "failing",
                        ResourceStatus::Paused => "paused",
                        ResourceStatus::Deploying => "deploying",
                        ResourceStatus::NotDeployed => "not deployed yet",
                        ResourceStatus::Stopped => "stopped",
                        ResourceStatus::Unknown => "not checked",
                    }
                ),
            )
            .await?;
        Ok(())
    }
}

fn activity_status(
    resource: &Resource,
    rows: &[Usage],
    today: chrono::NaiveDate,
    days: u16,
) -> ResourceStatus {
    if !matches!(resource.kind, ResourceKind::Web | ResourceKind::Service)
        || !matches!(resource.status, ResourceStatus::Live | ResourceStatus::Idle)
    {
        return resource.status;
    }
    if crate::projects::attention::idle_window(&resource.id, rows, today, days) {
        return ResourceStatus::Idle;
    }
    if resource.status == ResourceStatus::Idle {
        let since = (today - chrono::Duration::days(i64::from(days))).to_string();
        let through = today.to_string();
        if rows.iter().any(|row| {
            row.resource_id == resource.id
                && row.day >= since
                && row.day <= through
                && row.requests.is_some_and(|count| count > 0)
        }) {
            return ResourceStatus::Live;
        }
        // Idle was worked out from usage, not a provider health check. Without recent usage,
        // report neither idle nor healthy.
        return ResourceStatus::Unknown;
    }
    resource.status
}
