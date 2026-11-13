//! The demo's providers. Each wraps the real adapter only for what it knows
//! without asking anyone (its capabilities, operation catalog and buttons);
//! everything that would reach the provider is answered from the demo world
//! instead, so nothing ever leaves this computer.

use crate::{answers, logs, world};
use chrono::{DateTime, Duration, Utc};
use hoista_server::application::ports::{
    catalog::EngineError,
    providers::{
        Artifact, LogSession, Observed, ProviderAction, ProviderAdapter, ProviderBill,
        ProviderChart, ProviderConfig, ProviderDeployState, ProviderDeployment, ProviderDetails,
        ProviderDomain, ProviderDomainState, ProviderEmptyProject, ProviderErrorCount,
        ProviderMetrics, ProviderNew, ProviderOperationInfo, ProviderPlanUsage, ProviderPoint,
        ProviderQuickAction, ProviderRepository, ProviderRequest, ProviderSeries, ProviderTraffic,
        ProviderVolume, RemoteResource, ResourceBinding, unsupported,
    },
};
use hoista_server::domain::{
    Provider, Secret,
    domains::{
        DomainDnsPurpose, DomainDnsRecord, DomainDnsRecordType, DomainObservation, DomainReadiness,
    },
    logs::{LogEntry, LogHistory, LogHistoryQuery},
    model::{
        AccountChoice, Capabilities, Money, Release, Resource, ResourceKind, ResourcePlan,
        ResourceStatus, Usage,
    },
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// How long a release at a demo provider takes.
const RELEASE_TIME: std::time::Duration = std::time::Duration::from_secs(3);
/// Provider deployments listed for a watched service.
const PROVIDER_DEPLOYMENTS: usize = 6;
/// The commits a watched service's deployments were built from, newest first.
const COMMITS: [&str; PROVIDER_DEPLOYMENTS] = [
    "713ba4f", "f0e603a", "d2e0668", "8c41b9e", "2a7d3f0", "5e9c1b4",
];
/// One request in this many gets a 4xx answer, and one in this many a 5xx.
const CLIENT_ERROR_EVERY: u32 = 400;
const SERVER_ERROR_EVERY: u32 = 2_000;
/// A chart's finest step.
const MIN_STEP_SECONDS: u32 = 60;
/// Sample requests listed on a traffic read.
const RECENT_REQUESTS: usize = 40;

/// What has changed at the demo providers since the demo started: services
/// paused or woken, and how many times each resource was released.
#[derive(Default)]
pub struct Changes {
    status: Mutex<HashMap<String, ResourceStatus>>,
    releases: Mutex<HashMap<String, u32>>,
}

impl Changes {
    fn status(&self, resource: &world::Resource) -> ResourceStatus {
        self.status
            .lock()
            .ok()
            .and_then(|changed| changed.get(resource.reference).copied())
            .unwrap_or(resource.status)
    }

    fn set_status(&self, reference: &str, status: ResourceStatus) {
        if let Ok(mut changed) = self.status.lock() {
            changed.insert(reference.to_owned(), status);
        }
    }

    fn next_release(&self, reference: &str) -> u32 {
        let Ok(mut releases) = self.releases.lock() else {
            return 1;
        };
        let count = releases.entry(reference.to_owned()).or_default();
        *count += 1;
        *count
    }
}

pub struct DemoProvider {
    provider: Provider,
    real: Arc<dyn ProviderAdapter>,
    changes: Arc<Changes>,
}

impl DemoProvider {
    pub fn new(real: Arc<dyn ProviderAdapter>, changes: Arc<Changes>) -> Self {
        Self {
            provider: real.provider(),
            real,
            changes,
        }
    }

    /// The demo account `external_id` names. The seed saves each account's id
    /// as its token; an account whose token stopped working refuses that one
    /// until it's reconnected with any other.
    fn account(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<&'static world::Account, EngineError> {
        let account = world::account_at(self.provider, external_id).ok_or_else(unknown)?;
        if account.expired && token.expose() == account.id {
            return Err(signed_out());
        }
        Ok(account)
    }

    /// Refuses what the real adapter doesn't do, as it would.
    fn can(&self, does: impl Fn(&Capabilities) -> bool) -> Result<(), EngineError> {
        if does(&self.real.capabilities()) {
            Ok(())
        } else {
            Err(unsupported())
        }
    }

    /// The account, when it's one that keeps code (GitHub).
    fn code_account(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<&'static world::Account, EngineError> {
        if self.provider != Provider::Github {
            return Err(unsupported());
        }
        self.account(token, external_id)
    }

    fn resource(&self, reference: &str) -> Result<&'static world::Resource, EngineError> {
        world::resource_at(self.provider, reference).ok_or(EngineError::NotFound)
    }

    fn remote(&self, resource: &world::Resource) -> RemoteResource {
        RemoteResource {
            reference: resource.reference.to_owned(),
            name: resource.name.to_owned(),
            kind: resource.kind,
            region: resource.region.map(str::to_owned),
            status: self.changes.status(resource),
            url: resource.url.map(str::to_owned),
            group: resource.group.map(str::to_owned),
        }
    }

    /// Whether a resource answers requests now: asleep, paused and idle ones don't.
    fn serving(&self, resource: &world::Resource) -> bool {
        matches!(
            self.changes.status(resource),
            ResourceStatus::Live | ResourceStatus::Failing
        )
    }
}

fn unknown() -> EngineError {
    EngineError::Invalid("The demo has no such account. Use one of Sam's example accounts.")
}

fn signed_out() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.auth".into(),
        message: "The provider says the token was deleted or expired. Reconnect the account."
            .into(),
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for DemoProvider {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn capabilities(&self) -> Capabilities {
        self.real.capabilities()
    }

    fn reports_bindings(&self, reference: &str) -> bool {
        self.real.reports_bindings(reference)
    }

    fn keeps_known_role(&self, reference: &str) -> bool {
        self.real.keeps_known_role(reference)
    }

    fn provider_project(&self, reference: &str) -> Option<String> {
        self.real.provider_project(reference)
    }

    fn removes_services_by_action(&self) -> bool {
        self.real.removes_services_by_action()
    }

    fn names_added_databases(&self) -> bool {
        self.real.names_added_databases()
    }

    fn missing_grant(&self, granted: &[String], what: &ProviderNew<'_>) -> Option<&'static str> {
        self.real.missing_grant(granted, what)
    }

    fn provider_operations(&self) -> Vec<ProviderOperationInfo> {
        self.real.provider_operations()
    }

    fn provider_quick_actions(&self) -> Vec<ProviderQuickAction> {
        self.real.provider_quick_actions()
    }

    fn key_variable(&self) -> Option<&'static str> {
        self.real.key_variable()
    }

    /// Any token reaches the provider's example accounts; one saved for an
    /// account reaches only that account.
    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        let reachable: Vec<AccountChoice> = world::ACCOUNTS
            .iter()
            .filter(|account| account.provider == self.provider)
            .filter(|account| world::account(token.expose()).is_none_or(|own| own.id == account.id))
            .map(|account| AccountChoice {
                external_id: account.external_id.to_owned(),
                name: account.label.to_owned(),
            })
            .collect();
        if reachable.is_empty() {
            return Err(unknown());
        }
        Ok(reachable)
    }

    async fn verify(&self, token: &Secret, external_id: &str) -> Result<Vec<String>, EngineError> {
        let account = self.account(token, external_id)?;
        Ok(account
            .scopes
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect())
    }

    async fn discover(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let account = self.account(token, external_id)?;
        Ok(world::resources_of(account.id)
            .map(|resource| self.remote(resource))
            .collect())
    }

    async fn resource_bindings(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<Vec<ResourceBinding>, EngineError> {
        self.account(token, external_id)?;
        let from = self.resource(reference)?;
        Ok(world::LINKS
            .iter()
            .filter(|link| link.from == from.id)
            .filter_map(|link| {
                let to = world::RESOURCES
                    .iter()
                    .find(|resource| resource.id == link.to)?;
                Some(ResourceBinding {
                    name: link.via.to_owned(),
                    target_reference: (to.account == from.account).then(|| to.reference.to_owned()),
                })
            })
            .collect())
    }

    async fn deploy(
        &self,
        token: &Secret,
        external_id: &str,
        planned: &ResourcePlan,
        _artifact: &Artifact,
    ) -> Result<Release, EngineError> {
        self.can(|can| can.deploy)?;
        self.account(token, external_id)?;
        tokio::time::sleep(RELEASE_TIME).await;
        let reference = planned
            .provider_ref
            .clone()
            .unwrap_or_else(|| planned.name.clone());
        let url = world::resource_at(self.provider, &reference).and_then(|resource| resource.url);
        let version = self.changes.next_release(&reference);
        self.changes.set_status(&reference, ResourceStatus::Live);
        Ok(Release {
            schedules: None,
            version: format!("{reference}@{version}"),
            reference,
            url: url.map(str::to_owned),
        })
    }

    async fn rollback(
        &self,
        token: &Secret,
        external_id: &str,
        release: &Release,
    ) -> Result<Release, EngineError> {
        self.can(|can| can.rollback)?;
        self.account(token, external_id)?;
        tokio::time::sleep(RELEASE_TIME).await;
        self.changes
            .set_status(&release.reference, ResourceStatus::Live);
        Ok(release.clone())
    }

    async fn health(
        &self,
        _token: &Secret,
        _external_id: &str,
        _release: &Release,
    ) -> Result<(), EngineError> {
        Ok(())
    }

    async fn set_secret(
        &self,
        token: &Secret,
        external_id: &str,
        _reference: &str,
        _name: &str,
        _value: &Secret,
    ) -> Result<(), EngineError> {
        self.can(|can| can.secrets)?;
        self.account(token, external_id).map(|_| ())
    }

    async fn delete_secret(
        &self,
        token: &Secret,
        external_id: &str,
        _reference: &str,
        _name: &str,
    ) -> Result<(), EngineError> {
        self.can(|can| can.secrets)?;
        self.account(token, external_id).map(|_| ())
    }

    async fn connection_string(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<Secret, EngineError> {
        self.account(token, external_id)?;
        // A database a deploy has just made isn't in the demo world yet.
        let host = reference.replace(['/', ':'], "-");
        Ok(Secret::new(format!(
            "postgres://app:demo-password@{host}.db.example.com:5432/app"
        )))
    }

    async fn configure_domain(
        &self,
        token: &Secret,
        account: &str,
        _reference: &str,
        hostname: &str,
        zone_name: &str,
        _create_zone: bool,
    ) -> Result<DomainObservation, EngineError> {
        self.can(|can| can.domains)?;
        self.account(token, account)?;
        Ok(DomainObservation {
            zone_id: format!("zone-{zone_name}"),
            zone_name: zone_name.to_owned(),
            nameservers: vec!["ada.ns.example.net".into(), "bob.ns.example.net".into()],
            registrar: None,
            dns: DomainReadiness::Active,
            https: DomainReadiness::Pending,
            provider_domain_id: Some(format!("domain-{hostname}")),
            certificate_id: None,
            dns_records: vec![DomainDnsRecord {
                name: hostname.to_owned(),
                record_type: DomainDnsRecordType::CNAME,
                value: "edge.example.net".into(),
                purpose: DomainDnsPurpose::Routing,
            }],
            warnings: Vec::new(),
        })
    }

    async fn detach_domain(
        &self,
        token: &Secret,
        account: &str,
        _reference: &str,
        _domain_id: Option<&str>,
        _hostname: &str,
    ) -> Result<(), EngineError> {
        self.can(|can| can.domains)?;
        self.account(token, account).map(|_| ())
    }

    async fn delete(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        self.can(|can| can.delete)?;
        self.account(token, external_id)?;
        self.changes.set_status(reference, ResourceStatus::Stopped);
        Ok(())
    }

    async fn pause(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        self.can(|can| can.pause)?;
        self.account(token, external_id)?;
        self.changes.set_status(reference, ResourceStatus::Paused);
        Ok(())
    }

    async fn resume(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<ResourceStatus, EngineError> {
        self.can(|can| can.pause)?;
        self.account(token, external_id)?;
        self.changes.set_status(reference, ResourceStatus::Live);
        Ok(ResourceStatus::Live)
    }

    async fn observe_resource(
        &self,
        token: &Secret,
        external_id: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        self.account(token, external_id)?;
        let found = self.resource(&resource.provider_ref)?;
        Ok(Observed {
            status: self.changes.status(found),
            url: found.url.map(str::to_owned),
            kind: None,
        })
    }

    async fn provider_action(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        action: &ProviderAction<'_>,
    ) -> Result<String, EngineError> {
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        let done = match action {
            ProviderAction::Stop => {
                self.changes.set_status(reference, ResourceStatus::Stopped);
                "Stopped"
            }
            ProviderAction::Sleep { on: true } => {
                self.changes.set_status(reference, ResourceStatus::Sleeping);
                "Set to sleep when idle"
            }
            ProviderAction::Redeploy | ProviderAction::DeployLatest => {
                self.changes.set_status(reference, ResourceStatus::Live);
                "Started a new deployment of"
            }
            ProviderAction::Restart => "Restarted",
            _ => "Updated",
        };
        Ok(format!("{done} {}.", resource.name))
    }

    async fn provider_operation(
        &self,
        token: &Secret,
        external_id: &str,
        _reference: Option<&str>,
        _folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        self.account(token, external_id)?;
        if !self
            .real
            .provider_operations()
            .iter()
            .any(|operation| operation.name == name)
        {
            return Err(EngineError::Invalid("Unknown provider operation."));
        }
        let now = Utc::now();
        // A command-line command answers as the engine reports a command run.
        if name.contains(' ') {
            return Ok(answers::command(self.provider, name, &args, now));
        }
        Ok(answers::answer(self.provider, name, &args, now).unwrap_or(Value::Null))
    }

    async fn provider_empty_projects(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<ProviderEmptyProject>, EngineError> {
        let account = self.account(token, external_id)?;
        Ok(if account.id == "acc-railway" {
            vec![ProviderEmptyProject {
                id: "prj-scratch".into(),
                name: "scratch".into(),
                url: None,
            }]
        } else {
            Vec::new()
        })
    }

    async fn provider_bill(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Option<ProviderBill>, EngineError> {
        self.can(|can| can.bills)?;
        let account = self.account(token, external_id)?;
        let cents = match self.provider {
            Provider::Railway if account.id == "acc-railway" => 1_284,
            Provider::Railway => 4_610,
            Provider::Cloudflare => 500,
            Provider::Fly => 1_937,
            _ => return Err(unsupported()),
        };
        let now = Utc::now();
        let start = now - Duration::days(12);
        Ok(Some(ProviderBill {
            billed: Money {
                cents,
                currency: "USD".into(),
            },
            period_start: Some(start.to_rfc3339()),
            period_end: Some((start + Duration::days(30)).to_rfc3339()),
            soft_limit: (self.provider == Provider::Railway).then(|| Money {
                cents: 2_000,
                currency: "USD".into(),
            }),
            hard_limit: (self.provider == Provider::Railway).then(|| Money {
                cents: 5_000,
                currency: "USD".into(),
            }),
            over_limit: false,
            customer_id: (self.provider == Provider::Railway).then(|| "cus_demo_sam".into()),
        }))
    }

    async fn provider_plan_usage(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<ProviderPlanUsage>, EngineError> {
        self.account(token, external_id)?;
        match self.provider {
            Provider::Neon => Ok(vec![
                ProviderPlanUsage {
                    label: "Compute this month".into(),
                    used: 42.6,
                    limit: Some(191.9),
                    unit: "hours".into(),
                },
                ProviderPlanUsage {
                    label: "Storage".into(),
                    used: 0.33,
                    limit: Some(0.5),
                    unit: "GB".into(),
                },
            ]),
            Provider::Supabase => Ok(vec![ProviderPlanUsage {
                label: "Database size".into(),
                used: 0.06,
                limit: Some(0.5),
                unit: "GB".into(),
            }]),
            _ => Err(unsupported()),
        }
    }

    async fn provider_traffic(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<ProviderTraffic, EngineError> {
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        let minutes = (to - from).num_minutes().max(1);
        let requests = if self.serving(resource) {
            u32::try_from(i64::from(resource.daily_requests) * minutes / (24 * 60)).unwrap_or(0)
        } else {
            0
        };
        let recent = logs::between(resource, from, to, "")
            .into_iter()
            .rev()
            .take(RECENT_REQUESTS)
            .filter_map(|line| request(&line))
            .collect();
        Ok(ProviderTraffic {
            from: from.to_rfc3339(),
            to: to.to_rfc3339(),
            requests: Some(requests),
            client_errors: Some(requests / CLIENT_ERROR_EVERY),
            server_errors: Some(requests / SERVER_ERROR_EVERY),
            p50_ms: Some(41),
            p95_ms: Some(184),
            p99_ms: Some(612),
            top_errors: vec![ProviderErrorCount {
                status: 413,
                method: "POST".into(),
                path: "/api/upload".into(),
                count: requests / CLIENT_ERROR_EVERY,
            }],
            recent,
        })
    }

    async fn repositories(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<ProviderRepository>, EngineError> {
        self.code_account(token, external_id)?;
        let now = Utc::now();
        Ok(world::PROJECTS
            .iter()
            .filter_map(|project| match project.code {
                world::Code::Github { repository, .. } => Some(repository),
                _ => None,
            })
            .enumerate()
            .map(|(age, repository)| ProviderRepository {
                full_name: repository.to_owned(),
                default_branch: "main".into(),
                private: true,
                pushed_at: Some(
                    (now - Duration::hours(3 + 17 * i64::try_from(age).unwrap_or(0))).to_rfc3339(),
                ),
                url: format!("https://github.com/{repository}"),
                description: None,
            })
            .collect())
    }

    async fn branches(
        &self,
        token: &Secret,
        external_id: &str,
        _repository: &str,
    ) -> Result<Vec<String>, EngineError> {
        self.code_account(token, external_id)?;
        Ok(vec![
            "main".into(),
            "dev".into(),
            "feature/referrals".into(),
        ])
    }

    async fn branch_head(
        &self,
        token: &Secret,
        external_id: &str,
        repository: &str,
        _branch: &str,
    ) -> Result<String, EngineError> {
        self.code_account(token, external_id)?;
        world::PROJECTS
            .iter()
            .find_map(|project| match project.code {
                world::Code::Github {
                    repository: known,
                    commit,
                } if known == repository => Some(commit.to_owned()),
                _ => None,
            })
            .ok_or(EngineError::NotFound)
    }

    async fn provider_metrics(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        step_seconds: u32,
    ) -> Result<ProviderMetrics, EngineError> {
        self.can(|can| can.metrics)?;
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        let serving = self.serving(resource);
        let points = |value: &dyn Fn(usize) -> f64| -> Vec<ProviderPoint> {
            let step = Duration::seconds(i64::from(step_seconds.max(MIN_STEP_SECONDS)));
            let mut at = from;
            let mut points = Vec::new();
            while at < to && points.len() < answers::MAX_POINTS as usize {
                points.push(ProviderPoint {
                    at: at.to_rfc3339(),
                    value: if serving { value(points.len()) } else { 0.0 },
                });
                at += step;
            }
            points
        };
        let per_step = f64::from(resource.daily_requests) * f64::from(step_seconds) / 86_400.0;
        let chart =
            |id: &str, label: &str, unit, summary, value: &dyn Fn(usize) -> f64| ProviderChart {
                id: id.to_owned(),
                label: label.to_owned(),
                unit,
                summary,
                series: vec![ProviderSeries {
                    name: label.to_owned(),
                    points: points(value),
                }],
            };
        use hoista_server::application::ports::providers::{MetricSummary, MetricUnit};
        let charts = match resource.kind {
            ResourceKind::Database => vec![
                chart(
                    "connections",
                    "Connections",
                    MetricUnit::Count,
                    MetricSummary::Average,
                    &|i| answers::wave(i, 3.0, 6.0).round(),
                ),
                chart(
                    "storage",
                    "Storage",
                    MetricUnit::Bytes,
                    MetricSummary::Latest,
                    &|_| 212.0e6,
                ),
            ],
            ResourceKind::Bucket => vec![chart(
                "stored",
                "Stored",
                MetricUnit::Bytes,
                MetricSummary::Latest,
                &|i| 2.1e9 + 1.0e6 * i as f64,
            )],
            _ => vec![
                chart(
                    "requests",
                    "Requests",
                    MetricUnit::Count,
                    MetricSummary::Total,
                    &|i| (per_step * answers::wave(i, 0.8, 0.5)).round(),
                ),
                chart(
                    "errors",
                    "Errors",
                    MetricUnit::Count,
                    MetricSummary::Total,
                    &|i| {
                        if i % 17 == 3 { 1.0 } else { 0.0 }
                    },
                ),
                chart(
                    "p95",
                    "Slowest 5%",
                    MetricUnit::Milliseconds,
                    MetricSummary::Average,
                    &|i| answers::wave(i, 140.0, 60.0),
                ),
                chart(
                    "cpu",
                    "CPU time",
                    MetricUnit::Milliseconds,
                    MetricSummary::Average,
                    &|i| answers::wave(i, 4.0, 3.0),
                ),
            ],
        };
        Ok(ProviderMetrics {
            from: from.to_rfc3339(),
            to: to.to_rfc3339(),
            charts,
            note: None,
        })
    }

    async fn set_provider_variable(
        &self,
        token: &Secret,
        external_id: &str,
        _reference: &str,
        _name: &str,
        _value: &Secret,
    ) -> Result<(), EngineError> {
        self.can(|can| can.provider_variables)?;
        self.account(token, external_id).map(|_| ())
    }

    async fn set_provider_variables(
        &self,
        token: &Secret,
        external_id: &str,
        _reference: &str,
        _values: &[(String, Secret)],
        _shared: bool,
        _redeploy: bool,
    ) -> Result<(), EngineError> {
        self.can(|can| can.provider_variables)?;
        self.account(token, external_id).map(|_| ())
    }

    async fn delete_provider_variable(
        &self,
        token: &Secret,
        external_id: &str,
        _reference: &str,
        _name: &str,
    ) -> Result<(), EngineError> {
        self.can(|can| can.provider_variables)?;
        self.account(token, external_id).map(|_| ())
    }

    async fn delete_shared_variable(
        &self,
        token: &Secret,
        external_id: &str,
        _reference: &str,
        _name: &str,
    ) -> Result<(), EngineError> {
        self.can(|can| can.provider_variables)?;
        self.account(token, external_id).map(|_| ())
    }

    async fn provider_deployment_logs(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        _deployment: &str,
    ) -> Result<Vec<LogEntry>, EngineError> {
        self.can(|can| can.provider_details)?;
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        let now = Utc::now();
        Ok(logs::between(
            resource,
            now - Duration::minutes(10),
            now,
            "",
        ))
    }

    async fn provider_details(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<ProviderDetails, EngineError> {
        self.can(|can| can.provider_details)?;
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        Ok(details(self.provider, resource, Utc::now()))
    }

    async fn log_history(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        query: &LogHistoryQuery,
    ) -> Result<LogHistory, EngineError> {
        self.can(|can| can.logs)?;
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        let (from, to) = query.milliseconds().map_err(EngineError::Invalid)?;
        let (Some(from), Some(to)) = (
            DateTime::from_timestamp_millis(from),
            DateTime::from_timestamp_millis(to),
        ) else {
            return Err(EngineError::Invalid("Use RFC 3339 log history times."));
        };
        let to = to.min(Utc::now());
        let items = if self.serving(resource) || resource.kind == ResourceKind::Database {
            logs::between(resource, from, to, &query.query)
        } else {
            Vec::new()
        };
        Ok(LogHistory {
            items,
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: true,
        })
    }

    async fn open_logs(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        query: &str,
    ) -> Result<Box<dyn LogSession>, EngineError> {
        self.can(|can| can.logs)?;
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        Ok(Box::new(logs::Tail::new(resource, query)))
    }

    async fn logs(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        _since: &str,
        query: &str,
    ) -> Result<Vec<String>, EngineError> {
        self.can(|can| can.logs)?;
        self.account(token, external_id)?;
        let resource = self.resource(reference)?;
        let now = Utc::now();
        Ok(
            logs::between(resource, now - Duration::hours(1), now, query)
                .into_iter()
                .map(|line| line.message)
                .collect(),
        )
    }

    async fn usage(
        &self,
        token: &Secret,
        external_id: &str,
        resource: &Resource,
        day: &str,
    ) -> Result<Usage, EngineError> {
        self.can(|can| can.usage)?;
        self.account(token, external_id)?;
        let found = self.resource(&resource.provider_ref)?;
        Ok(crate::seed::usage(resource.id.as_str(), found, day))
    }
}

/// A request as a traffic read lists it, read back from its log line.
fn request(line: &LogEntry) -> Option<ProviderRequest> {
    let mut words = line.message.split_whitespace();
    let method = words.next()?.to_owned();
    let path = words.next()?.to_owned();
    let status = words.next()?.parse().ok()?;
    let duration_ms = words
        .next()
        .and_then(|took| took.strip_suffix("ms"))
        .and_then(|took| took.parse().ok());
    let at = line.timestamp.clone()?;
    Some(ProviderRequest {
        request_id: format!("req-{}", at.replace([':', '-', '.', '+'], "")),
        at,
        method,
        path,
        status,
        duration_ms,
    })
}

/// What a provider reports about one of its resources: its deployments,
/// variables, addresses and how it runs.
fn details(provider: Provider, resource: &world::Resource, now: DateTime<Utc>) -> ProviderDetails {
    let repository = resource
        .project
        .and_then(world::project)
        .and_then(|project| match project.code {
            world::Code::Github { repository, .. } => Some(repository),
            world::Code::Folder(_) => None,
        });
    let runs_code = resource.kind.runs_code();
    let deployments = if runs_code {
        (0..PROVIDER_DEPLOYMENTS)
            .map(|age| {
                let created = now - Duration::hours(6 + 31 * i64::try_from(age).unwrap_or(0));
                let (status, state) = match age {
                    0 => ("SUCCESS", ProviderDeployState::Succeeded),
                    3 => ("CRASHED", ProviderDeployState::Failed),
                    _ => ("REMOVED", ProviderDeployState::Stopped),
                };
                ProviderDeployment {
                    id: format!("{}-deploy-{age}", resource.id),
                    status: status.into(),
                    state,
                    created_at: created.to_rfc3339(),
                    url: resource.url.map(str::to_owned),
                    commit: Some(COMMITS[age].into()),
                    message: Some(
                        [
                            "Cache profile pages",
                            "Add rate limits",
                            "Upgrade Node to 22",
                            "Paginate the feed",
                            "Log slow queries",
                            "First deploy",
                        ][age]
                            .into(),
                    ),
                    branch: Some("main".into()),
                    repo: repository.map(str::to_owned),
                    finished_at: Some((created + Duration::seconds(84)).to_rfc3339()),
                    can_roll_back: Some(state == ProviderDeployState::Stopped),
                    author: Some("Sam Rivera".into()),
                    provider_page: None,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let database = resource.kind == ResourceKind::Database;
    ProviderDetails {
        deployments,
        variable_names: Some(if database {
            vec![
                "DATABASE_PUBLIC_URL".into(),
                "DATABASE_URL".into(),
                "PGDATA".into(),
                "PGHOST".into(),
                "POSTGRES_DB".into(),
            ]
        } else {
            vec![
                "DATABASE_URL".into(),
                "NODE_ENV".into(),
                "PORT".into(),
                "SESSION_SECRET".into(),
            ]
        }),
        shared_variable_names: Some(vec!["SENTRY_DSN".into()]),
        domains: resource
            .url
            .map(|url| {
                vec![ProviderDomain {
                    name: url.trim_start_matches("https://").to_owned(),
                    custom: !url.contains(".up.railway.app") && !url.contains(".supabase.co"),
                    state: Some(ProviderDomainState::Working),
                    records: Vec::new(),
                    port: Some(8080),
                    certificate: Some("Valid".into()),
                    dns: Vec::new(),
                }]
            })
            .unwrap_or_default(),
        references: Vec::new(),
        sleeps: Some(false),
        config: Some(ProviderConfig {
            repo: repository.filter(|_| runs_code).map(str::to_owned),
            image: (database && provider == Provider::Railway)
                .then(|| "ghcr.io/railwayapp-templates/postgres-ssl:17".into()),
            start_command: runs_code.then(|| "node server.js".into()),
            build_command: runs_code.then(|| "npm run build".into()),
            root_directory: None,
            cron_schedule: (resource.kind == ResourceKind::Job).then(|| "*/15 * * * *".into()),
            healthcheck_path: runs_code.then(|| "/health".into()),
            replicas: Some(1),
            region: resource.region.map(str::to_owned),
            restart_policy: Some("ON_FAILURE".into()),
        }),
        volumes: if database && provider == Provider::Railway {
            vec![ProviderVolume {
                id: format!("{}-volume", resource.id),
                name: "pgdata".into(),
                mount_path: Some("/var/lib/postgresql/data".into()),
                instance_id: Some(format!("{}-volume-instance", resource.id)),
                used_mb: Some(1_400),
                size_mb: Some(5_000),
                region: resource.region.map(str::to_owned),
            }]
        } else {
            Vec::new()
        },
        tcp_proxies: Vec::new(),
        private_domain: match provider {
            Provider::Railway => Some(format!("{}.railway.internal", resource.name.to_lowercase())),
            Provider::Fly => Some(format!("{}.internal", resource.name)),
            _ => None,
        },
        addresses: Vec::new(),
    }
}

/// Every demo provider, wrapping the real adapters' catalogs.
pub fn all(real: Vec<Arc<dyn ProviderAdapter>>) -> Vec<Arc<dyn ProviderAdapter>> {
    let changes = Arc::new(Changes::default());
    real.into_iter()
        .map(|adapter| {
            Arc::new(DemoProvider::new(adapter, changes.clone())) as Arc<dyn ProviderAdapter>
        })
        .collect()
}
