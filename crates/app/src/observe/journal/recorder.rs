//! The operation journal. Every adapter is wrapped in `Journaled`, so each
//! change sent to a provider is recorded first and marked done, failed or unclear.
//!
//! An unclear change that isn't safe to repeat (create, upload, redeploy)
//! blocks an identical one until the person confirms whether it happened.

use crate::Provider;
use crate::domain::{
    Secret,
    model::{Operation, OperationState, *},
};
use crate::ports::{
    catalog::{CatalogRepository, EngineError},
    providers::*,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

/// Changes being sent right now by this engine, by digest. A journal entry
/// still `running` that isn't here was cut off by a stop: it's unclear.
pub type InFlight = Arc<Mutex<HashSet<String>>>;

/// One change about to be sent.
struct Change<'a> {
    kind: &'static str,
    account: &'a str,
    target: String,
    repeats: bool,
    /// What else makes two changes the same, digested; never kept as text.
    inputs: String,
}

/// An adapter whose changes go through the journal.
pub struct Journaled {
    inner: Arc<dyn ProviderAdapter>,
    journal: Arc<dyn CatalogRepository>,
    in_flight: InFlight,
}

impl Journaled {
    /// Wraps `inner`.
    pub fn new(
        inner: Arc<dyn ProviderAdapter>,
        journal: Arc<dyn CatalogRepository>,
        in_flight: InFlight,
    ) -> Self {
        Self {
            inner,
            journal,
            in_flight,
        }
    }

    fn key(&self, change: &Change<'_>) -> String {
        let mut digest = Sha256::new();
        for resource in [
            self.inner.provider().slug(),
            change.kind,
            change.account,
            change.target.as_str(),
            change.inputs.as_str(),
        ] {
            digest.update(resource.len().to_be_bytes());
            digest.update(resource.as_bytes());
        }
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    async fn begin(&self, change: &Change<'_>) -> Result<Operation, EngineError> {
        let key = self.key(change);
        if change.repeats {
            if self
                .in_flight
                .lock()
                .map_err(|_| EngineError::Store)?
                .contains(&key)
            {
                return Err(EngineError::Conflict(
                    "That change is already being made. Wait for it to finish.",
                ));
            }
            if let Some(earlier) = self.journal.open_operation(&key).await? {
                return Err(unclear(&earlier));
            }
        }
        let operation = Operation {
            id: uuid::Uuid::new_v4().to_string(),
            provider: self.inner.provider(),
            account: change.account.to_owned(),
            kind: change.kind.to_owned(),
            target: change.target.clone(),
            state: OperationState::Running,
            repeats: change.repeats,
            started_at: now(),
            finished_at: None,
            message: None,
            key: key.clone(),
        };
        // Written down before anything is sent; if this fails, nothing is sent.
        self.journal.put_operation(&operation).await?;
        if let Ok(mut flying) = self.in_flight.lock() {
            flying.insert(key);
        }
        Ok(operation)
    }

    /// Before refusing an identical create whose earlier try ended unclear,
    /// looks at the provider for what it would have made, by name. Not there:
    /// the earlier try didn't happen, so this one goes ahead. There: it
    /// happened, and its reference is returned so it's used, not made again.
    /// Hoista can't tell (the provider can't be read): it leaves the earlier try
    /// for the person, and the create is refused as before.
    async fn settle_by_looking(
        &self,
        change: &Change<'_>,
        token: &Secret,
        name: Option<&str>,
    ) -> Result<Option<String>, EngineError> {
        let Some(name) = name else {
            return Ok(None);
        };
        let key = self.key(change);
        if self
            .in_flight
            .lock()
            .map_err(|_| EngineError::Store)?
            .contains(&key)
        {
            return Ok(None);
        }
        let Some(mut earlier) = self.journal.open_operation(&key).await? else {
            return Ok(None);
        };
        let Ok(found) = self.inner.discover(token, change.account).await else {
            return Ok(None);
        };
        let there = found.into_iter().find(|resource| resource.name == name);
        earlier.state = if there.is_some() {
            OperationState::Happened
        } else {
            OperationState::DidNotHappen
        };
        earlier.finished_at = Some(now());
        earlier.message = Some(
            if there.is_some() {
                "Hoista looked at the provider: it's there."
            } else {
                "Hoista looked at the provider: it isn't there."
            }
            .into(),
        );
        self.journal.put_operation(&earlier).await?;
        Ok(there.map(|resource| resource.reference))
    }

    async fn finish<T>(&self, mut operation: Operation, result: &Result<T, EngineError>) {
        if let Ok(mut flying) = self.in_flight.lock() {
            flying.remove(&operation.key);
        }
        operation.finished_at = Some(now());
        operation.state = match result {
            Ok(_) => OperationState::Done,
            Err(error) if outcome_unknown(error, operation.repeats) => OperationState::Unclear,
            Err(_) => OperationState::Failed,
        };
        if let Err(error) = result {
            operation.message = Some(error.to_string());
        }
        if self.journal.put_operation(&operation).await.is_err() {
            tracing::warn!(
                code = "operation.unrecorded",
                id = %operation.id,
                "Couldn't record how a provider change ended"
            );
        }
    }
}

/// Whether a failed change may still have happened at the provider.
fn outcome_unknown(error: &EngineError, repeats: bool) -> bool {
    match error {
        // A command that ran to the end and said why it failed is a clear
        // answer, like a provider refusing a request: it didn't happen.
        EngineError::Provider { code, .. } if code == "provider.command_failed" => false,
        // Such a change is often several calls; a refusal after the first can leave part of it.
        EngineError::Provider { .. } if repeats => true,
        EngineError::Provider { code, .. } => matches!(
            code.as_str(),
            "provider.unreachable"
                | "provider.timeout"
                | "provider.invalid_response"
                | "provider.response_limit"
                | "provider.cli_failed"
        ),
        _ => false,
    }
}

/// The refusal while an identical change is unclear.
fn unclear(earlier: &Operation) -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "operation.unclear".into(),
        message: format!(
            "An identical change started {} ({} at {}) may have gone through: it stopped without a clear answer. Nothing was sent this time. Read {} to see whether it happened (list_provider_operations has the read calls), then mark operation {} with resolve_operation and what you read as evidence; the person can also mark it in the project. Then try again if it didn't happen.",
            earlier.started_at,
            earlier.target,
            earlier.provider.display_name(),
            earlier.provider.display_name(),
            earlier.id,
        ),
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The action's name, like `Redeploy` or `AddVolume`, with none of its values.
fn action_name(action: &ProviderAction<'_>) -> String {
    format!("{action:?}")
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect()
}

/// Deploying, redeploying, restarting or rolling back, or adding a volume or a TCP proxy,
/// makes a second one if repeated; settings and removals don't.
const fn action_repeats(action: &ProviderAction<'_>) -> bool {
    matches!(
        action,
        ProviderAction::Redeploy
            | ProviderAction::Restart
            | ProviderAction::Rollback { .. }
            | ProviderAction::DeployLatest
            | ProviderAction::DeployCommit { .. }
            | ProviderAction::AddVolume { .. }
            | ProviderAction::AddTcpProxy { .. }
    )
}

fn create_name(what: &ProviderCreate<'_>) -> String {
    match what {
        ProviderCreate::Service { name, .. } => format!("service {name}"),
        ProviderCreate::Template { code, .. } => format!("{code} template"),
        ProviderCreate::Bucket { name } => format!("bucket {name}"),
    }
}

fn new_name(what: &ProviderNew<'_>) -> String {
    match what {
        ProviderNew::Repo { name, .. }
        | ProviderNew::Folder { name, .. }
        | ProviderNew::Image { name, .. } => (*name).to_owned(),
        ProviderNew::Database { name, engine, .. } => format!("{engine} database {name}"),
        ProviderNew::Bucket { name } => format!("bucket {name}"),
        ProviderNew::EmailDomain { domain } => format!("sending domain {domain}"),
    }
}

/// The name the provider lists what an add makes under, where it's the name
/// given. A provider that names a database after something else (Railway: its
/// template) isn't looked up by name.
fn looked_up_by<'a>(names_databases: bool, what: &ProviderNew<'a>) -> Option<&'a str> {
    match *what {
        ProviderNew::Database { .. } if !names_databases => None,
        ProviderNew::Repo { name, .. }
        | ProviderNew::Folder { name, .. }
        | ProviderNew::Image { name, .. }
        | ProviderNew::Database { name, .. }
        | ProviderNew::Bucket { name } => Some(name),
        ProviderNew::EmailDomain { domain } => Some(domain),
    }
}

/// What makes two adds the same.
fn new_inputs(what: &ProviderNew<'_>) -> String {
    match what {
        ProviderNew::Repo {
            name,
            repo,
            branch,
            runs_as,
            schedule,
            build,
        } => format!(
            "repo|{name}|{repo}|{}|{}|{}|{}|{}|{}|{}",
            branch.unwrap_or_default(),
            runs_as.unwrap_or_default(),
            schedule.unwrap_or_default(),
            build.runtime.unwrap_or_default(),
            build.build.unwrap_or_default(),
            build.start.unwrap_or_default(),
            build.publish.unwrap_or_default()
        ),
        ProviderNew::Folder { name, folder } => format!("folder|{name}|{}", folder.display()),
        ProviderNew::Database {
            name,
            engine,
            region,
        } => format!("database|{name}|{engine}|{}", region.unwrap_or_default()),
        ProviderNew::Bucket { name } => format!("bucket|{name}"),
        ProviderNew::EmailDomain { domain } => format!("email|{domain}"),
        ProviderNew::Image {
            name,
            image,
            gpu,
            cpu,
            port,
            args,
        } => format!(
            "image|{name}|{image}|{}|{}|{}|{}",
            gpu.unwrap_or_default(),
            cpu.map(|cpu| format!("{}x{}", cpu.flavor, cpu.vcpus))
                .unwrap_or_default(),
            port.map(|port| port.to_string()).unwrap_or_default(),
            args.unwrap_or_default()
        ),
    }
}

/// What makes two creates the same, with template values left out: they can be passwords.
fn create_inputs(what: &ProviderCreate<'_>) -> String {
    match what {
        ProviderCreate::Service {
            name,
            repo,
            branch,
            image,
        } => format!(
            "{name}|{}|{}|{}",
            repo.unwrap_or_default(),
            branch.unwrap_or_default(),
            image.unwrap_or_default()
        ),
        ProviderCreate::Template { code, variables } => format!(
            "{code}|{}",
            variables.keys().cloned().collect::<Vec<_>>().join(",")
        ),
        ProviderCreate::Bucket { name } => (*name).to_owned(),
    }
}

fn variable_names(values: &[(String, Secret)]) -> String {
    let names: Vec<&str> = values.iter().map(|(name, _)| name.as_str()).collect();
    match names.len() {
        0 => "no variables".into(),
        1..=3 => names.join(", "),
        count => format!("{count} variables"),
    }
}

/// A provider call's arguments with anything that may be a secret blanked.
fn without_secrets(args: &serde_json::Value) -> serde_json::Value {
    match args {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(name, value)| {
                    let lower = name.to_ascii_lowercase();
                    let secret = ["value", "secret", "password", "token", "key", "variables"]
                        .iter()
                        .any(|word| lower.contains(word));
                    (
                        name.clone(),
                        if secret {
                            serde_json::Value::Null
                        } else {
                            without_secrets(value)
                        },
                    )
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(without_secrets).collect())
        }
        other => other.clone(),
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Journaled {
    fn provider(&self) -> Provider {
        self.inner.provider()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn reports_bindings(&self, reference: &str) -> bool {
        self.inner.reports_bindings(reference)
    }

    fn keeps_known_role(&self, reference: &str) -> bool {
        self.inner.keeps_known_role(reference)
    }

    fn provider_project(&self, reference: &str) -> Option<String> {
        self.inner.provider_project(reference)
    }

    fn removes_services_by_action(&self) -> bool {
        self.inner.removes_services_by_action()
    }

    fn names_added_databases(&self) -> bool {
        self.inner.names_added_databases()
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        self.inner.account_choices(token).await
    }

    async fn resource_bindings(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<Vec<ResourceBinding>, EngineError> {
        self.inner
            .resource_bindings(token, external_id, reference)
            .await
    }

    async fn verify(&self, token: &Secret, external_id: &str) -> Result<Vec<String>, EngineError> {
        self.inner.verify(token, external_id).await
    }

    async fn discover(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        self.inner.discover(token, external_id).await
    }

    async fn discover_report(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<DiscoveryReport, EngineError> {
        self.inner.discover_report(token, external_id).await
    }

    async fn deploy(
        &self,
        token: &Secret,
        external_id: &str,
        resource: &ResourcePlan,
        artifact: &Artifact,
    ) -> Result<Release, EngineError> {
        self.inner
            .deploy(token, external_id, resource, artifact)
            .await
    }

    async fn rollback(
        &self,
        token: &Secret,
        external_id: &str,
        release: &Release,
    ) -> Result<Release, EngineError> {
        self.inner.rollback(token, external_id, release).await
    }

    async fn set_secret(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "set_secret",
            account: external_id,
            target: format!("{name} on {reference}"),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .set_secret(token, external_id, reference, name, value)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn delete_secret(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "delete_secret",
            account: external_id,
            target: format!("{name} on {reference}"),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .delete_secret(token, external_id, reference, name)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn health(
        &self,
        token: &Secret,
        external_id: &str,
        release: &Release,
    ) -> Result<(), EngineError> {
        self.inner.health(token, external_id, release).await
    }

    async fn connection_string(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<Secret, EngineError> {
        self.inner
            .connection_string(token, external_id, reference)
            .await
    }

    async fn migration_connection_string(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<Secret, EngineError> {
        self.inner
            .migration_connection_string(token, external_id, reference)
            .await
    }

    async fn database_observation(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<crate::domain::database_observation::DatabaseObservation, EngineError> {
        self.inner
            .database_observation(token, external_id, reference)
            .await
    }

    async fn database_operations(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        cursor: Option<&str>,
    ) -> Result<crate::domain::database_observation::DatabaseOperationHistory, EngineError> {
        self.inner
            .database_operations(token, external_id, reference, cursor)
            .await
    }

    async fn database_inventory(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<Vec<crate::domain::database_export::DatabaseIdentity>, EngineError> {
        self.inner
            .database_inventory(token, external_id, reference)
            .await
    }

    async fn database_export_connection(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        database: &crate::domain::database_export::DatabaseIdentity,
    ) -> Result<Secret, EngineError> {
        self.inner
            .database_export_connection(token, external_id, reference, database)
            .await
    }

    async fn configure_domain(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        hostname: &str,
        zone_name: &str,
        create_zone: bool,
    ) -> Result<crate::domain::domains::DomainObservation, EngineError> {
        let change = Change {
            kind: "add_domain",
            account,
            target: format!("{hostname} on {reference}"),
            repeats: false,
            inputs: format!("{zone_name}|{create_zone}"),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .configure_domain(token, account, reference, hostname, zone_name, create_zone)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn observe_domain(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        hostname: &str,
        zone_name: &str,
    ) -> Result<crate::domain::domains::DomainObservation, EngineError> {
        self.inner
            .observe_domain(token, account, reference, hostname, zone_name)
            .await
    }

    async fn detach_domain(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        domain_id: Option<&str>,
        hostname: &str,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "remove_domain",
            account,
            target: format!("{hostname} on {reference}"),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .detach_domain(token, account, reference, domain_id, hostname)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn delete(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "delete",
            account: external_id,
            target: reference.to_owned(),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self.inner.delete(token, external_id, reference).await;
        self.finish(operation, &result).await;
        result
    }

    async fn pause(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "pause",
            account: external_id,
            target: reference.to_owned(),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self.inner.pause(token, external_id, reference).await;
        self.finish(operation, &result).await;
        result
    }

    async fn resume(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<ResourceStatus, EngineError> {
        let change = Change {
            kind: "resume",
            account: external_id,
            target: reference.to_owned(),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self.inner.resume(token, external_id, reference).await;
        self.finish(operation, &result).await;
        result
    }

    fn oauth(&self) -> Option<ProviderOAuth> {
        self.inner.oauth()
    }

    async fn oauth_token(&self, grant: OAuthGrant<'_>) -> Result<OAuthTokens, EngineError> {
        self.inner.oauth_token(grant).await
    }

    async fn observe_resource(
        &self,
        token: &Secret,
        external_id: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        self.inner
            .observe_resource(token, external_id, resource)
            .await
    }

    async fn provider_action(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        action: &ProviderAction<'_>,
    ) -> Result<String, EngineError> {
        let change = Change {
            kind: "action",
            account: external_id,
            target: format!("{} on {reference}", action_name(action)),
            repeats: action_repeats(action),
            inputs: format!("{action:?}"),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_action(token, external_id, reference, action)
            .await;
        self.finish(operation, &result).await;
        result
    }

    fn provider_operations(&self) -> Vec<ProviderOperationInfo> {
        self.inner.provider_operations()
    }

    fn provider_quick_actions(&self) -> Vec<crate::ports::providers::ProviderQuickAction> {
        self.inner.provider_quick_actions()
    }

    fn prepare_tools(&self) {
        self.inner.prepare_tools();
    }

    async fn provider_operation(
        &self,
        token: &Secret,
        external_id: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        let reads = self
            .inner
            .provider_operations()
            .iter()
            .any(|known| known.name == name && known.risk == ProviderRisk::Read);
        if reads {
            return self
                .inner
                .provider_operation(token, external_id, reference, folder, name, args)
                .await;
        }
        let change = Change {
            kind: "operation",
            account: external_id,
            target: format!("{name} on {}", reference.unwrap_or("the account")),
            // An API operation is one request: a refusal means it didn't
            // happen. A command (`railway up`) is several steps, and one that
            // fails partway may have done some of them.
            repeats: name.contains(' '),
            inputs: without_secrets(&args).to_string(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_operation(token, external_id, reference, folder, name, args)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn provider_bill(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Option<ProviderBill>, EngineError> {
        self.inner.provider_bill(token, external_id).await
    }

    async fn provider_plan_usage(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<crate::ports::providers::ProviderPlanUsage>, EngineError> {
        self.inner.provider_plan_usage(token, external_id).await
    }

    async fn provider_traffic(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderTraffic, EngineError> {
        self.inner
            .provider_traffic(token, external_id, reference, from, to)
            .await
    }

    async fn repositories(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<crate::ports::providers::ProviderRepository>, EngineError> {
        self.inner.repositories(token, external_id).await
    }

    async fn branches(
        &self,
        token: &Secret,
        external_id: &str,
        repository: &str,
    ) -> Result<Vec<String>, EngineError> {
        self.inner.branches(token, external_id, repository).await
    }

    async fn branch_head(
        &self,
        token: &Secret,
        external_id: &str,
        repository: &str,
        branch: &str,
    ) -> Result<String, EngineError> {
        self.inner
            .branch_head(token, external_id, repository, branch)
            .await
    }

    async fn provider_metrics(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        step_seconds: u32,
    ) -> Result<crate::ports::providers::ProviderMetrics, EngineError> {
        self.inner
            .provider_metrics(token, external_id, reference, from, to, step_seconds)
            .await
    }

    async fn provider_create(
        &self,
        token: &Secret,
        external_id: &str,
        beside: &str,
        what: &ProviderCreate<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let change = Change {
            kind: "create",
            account: external_id,
            target: format!("{} next to {beside}", create_name(what)),
            repeats: true,
            inputs: create_inputs(what),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_create(token, external_id, beside, what)
            .await;
        self.finish(operation, &result).await;
        result
    }

    fn missing_grant(&self, granted: &[String], what: &ProviderNew<'_>) -> Option<&'static str> {
        self.inner.missing_grant(granted, what)
    }

    async fn provider_add(
        &self,
        token: &Secret,
        external_id: &str,
        project: &str,
        beside: Option<&str>,
        what: &ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let change = Change {
            kind: "create",
            account: external_id,
            target: format!("{} for {project}", new_name(what)),
            repeats: true,
            inputs: new_inputs(what),
        };
        if let Some(reference) = self
            .settle_by_looking(
                &change,
                token,
                looked_up_by(self.inner.names_added_databases(), what),
            )
            .await?
        {
            return Ok((
                vec![reference],
                format!(
                    "{} was already at {} from an earlier try",
                    new_name(what),
                    self.inner.provider().display_name()
                ),
            ));
        }
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_add(token, external_id, project, beside, what)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn provider_empty_projects(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<crate::ports::providers::ProviderEmptyProject>, EngineError> {
        self.inner.provider_empty_projects(token, external_id).await
    }

    async fn provider_new_from_folder(
        &self,
        token: &Secret,
        external_id: &str,
        name: &str,
        folder: &std::path::Path,
    ) -> Result<(Vec<String>, String), EngineError> {
        let change = Change {
            kind: "new_project",
            account: external_id,
            target: format!("project {name}"),
            repeats: true,
            inputs: folder.display().to_string(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_new_from_folder(token, external_id, name, folder)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn provider_new_project(
        &self,
        token: &Secret,
        external_id: &str,
        name: &str,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let change = Change {
            kind: "new_project",
            account: external_id,
            target: format!("project {name}"),
            repeats: true,
            inputs: format!("{repo}|{}", branch.unwrap_or_default()),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_new_project(token, external_id, name, repo, branch)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn provider_new_hoista_server(
        &self,
        token: &Secret,
        external_id: &str,
        source: &ServerSource,
        variables: &std::collections::BTreeMap<String, Secret>,
    ) -> Result<(String, String), EngineError> {
        let change = Change {
            kind: "new_server",
            account: external_id,
            target: "Hoista on a server".to_owned(),
            repeats: true,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_new_hoista_server(token, external_id, source, variables)
            .await;
        self.finish(operation, &result).await;
        result
    }

    fn server_key_steps(&self, reference: &str) -> Option<(String, Option<String>)> {
        self.inner.server_key_steps(reference)
    }

    async fn provider_hide_server_key(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<bool, EngineError> {
        let change = Change {
            kind: "seal_variable",
            account: external_id,
            target: format!("HOISTA_SEALED_KEY on {reference}"),
            repeats: false,
            inputs: reference.to_owned(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_hide_server_key(token, external_id, reference)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn provider_upload(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        folder: &std::path::Path,
        message: Option<&str>,
    ) -> Result<String, EngineError> {
        let change = Change {
            kind: "upload",
            account: external_id,
            target: format!("folder to {reference}"),
            repeats: true,
            inputs: format!("{}|{}", folder.display(), message.unwrap_or_default()),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .provider_upload(token, external_id, reference, folder, message)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn set_provider_variable(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "set_variable",
            account: external_id,
            target: format!("{name} on {reference}"),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .set_provider_variable(token, external_id, reference, name, value)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn set_provider_variables(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        values: &[(String, Secret)],
        shared: bool,
        redeploy: bool,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "set_variables",
            account: external_id,
            target: format!("{} on {reference}", variable_names(values)),
            repeats: false,
            inputs: format!("{shared}|{redeploy}"),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .set_provider_variables(token, external_id, reference, values, shared, redeploy)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn delete_shared_variable(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "delete_shared_variable",
            account: external_id,
            target: format!("{name} on {reference}"),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .delete_shared_variable(token, external_id, reference, name)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn delete_provider_variable(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        let change = Change {
            kind: "delete_variable",
            account: external_id,
            target: format!("{name} on {reference}"),
            repeats: false,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .delete_provider_variable(token, external_id, reference, name)
            .await;
        self.finish(operation, &result).await;
        result
    }

    async fn issue_key(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        name: &str,
    ) -> Result<Secret, EngineError> {
        let change = Change {
            kind: "issue_key",
            account: external_id,
            target: format!("{name} for {reference}"),
            repeats: true,
            inputs: String::new(),
        };
        let operation = self.begin(&change).await?;
        let result = self
            .inner
            .issue_key(token, external_id, reference, name)
            .await;
        self.finish(operation, &result).await;
        result
    }

    fn key_variable(&self) -> Option<&'static str> {
        self.inner.key_variable()
    }

    async fn provider_deployment_logs(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        deployment: &str,
    ) -> Result<Vec<crate::domain::logs::LogEntry>, EngineError> {
        self.inner
            .provider_deployment_logs(token, external_id, reference, deployment)
            .await
    }

    async fn provider_details(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<ProviderDetails, EngineError> {
        self.inner
            .provider_details(token, external_id, reference)
            .await
    }

    async fn log_history(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        self.inner
            .log_history(token, external_id, reference, query)
            .await
    }

    async fn open_logs(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        query: &str,
    ) -> Result<Box<dyn LogSession>, EngineError> {
        self.inner
            .open_logs(token, external_id, reference, query)
            .await
    }

    async fn logs(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
        since: &str,
        query: &str,
    ) -> Result<Vec<String>, EngineError> {
        self.inner
            .logs(token, external_id, reference, since, query)
            .await
    }

    async fn usage(
        &self,
        token: &Secret,
        external_id: &str,
        resource: &Resource,
        day: &str,
    ) -> Result<Usage, EngineError> {
        self.inner.usage(token, external_id, resource, day).await
    }
}
