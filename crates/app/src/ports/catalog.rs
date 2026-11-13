//! The catalog storage interface and the engine's error type.

use crate::domain::model::*;

/// What went wrong in the engine, each with a code that doesn't change. Never
/// carries a token or a provider's raw response.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// What was asked for isn't there.
    #[error("The requested resource was not found. Refresh and try again.")]
    NotFound,
    /// What was sent breaks one of Hoista's rules.
    #[error("{0}")]
    Invalid(&'static str),
    /// The provider can't do what was asked. The message names the provider and says
    /// what to do instead.
    #[error("{0}")]
    Unsupported(String),
    /// It can't be done in the state things are in now.
    #[error("{0}")]
    Conflict(&'static str),
    /// The database couldn't be read or written.
    #[error("Could not update local state. Run hoist doctor and retry.")]
    Store,
    /// A provider call failed. The message is redacted and says what to do.
    #[error("{message}")]
    Provider {
        /// Response details safe to show, when an HTTP response came back.
        evidence: Option<ProviderFailureEvidence>,
        /// What went wrong, as a code.
        code: String,
        /// What went wrong and what to do, with any token taken out.
        message: String,
    },
    /// Required environment values were not given and aren't already set on every runtime
    /// resource.
    #[error(
        "Required environment inputs are missing. Supply the listed names locally and deploy again; existing authentication secrets are never automatically rotated."
    )]
    MissingSecrets(Vec<String>),
    /// The action needs the owner's approval first.
    #[error("The owner must review this action in Hoista's app before continuing.")]
    ApprovalRequired,
    /// The owner has paused agent access.
    #[error("Agent access is paused. The owner can review access and use hoist agents resume.")]
    AgentsDisabled,
    /// The project is watched only, so Hoista won't change its resources.
    #[error(
        "This project is watching only. Switch it to managed before changing provider resources."
    )]
    Watching,
}

impl EngineError {
    /// The response details of a provider failure, without raw headers, bodies or credentials.
    pub fn provider_response(&self) -> Option<&ProviderFailureEvidence> {
        match self {
            Self::Provider { evidence, .. } => evidence.as_ref(),
            _ => None,
        }
    }

    /// Attaches response details to a provider failure; its code stays the same.
    pub fn with_provider_response(mut self, response: ProviderFailureEvidence) -> Self {
        if let Self::Provider { evidence, .. } = &mut self {
            *evidence = Some(response);
        }
        self
    }

    /// The missing variable names of [`EngineError::MissingSecrets`], never values; empty for
    /// any other error.
    pub fn missing_secrets(&self) -> &[String] {
        match self {
            Self::MissingSecrets(names) => names,
            _ => &[],
        }
    }

    /// The error's code, for programs to match on.
    pub fn code(&self) -> &str {
        match self {
            Self::NotFound => "resource.not_found",
            Self::Invalid(_) | Self::Unsupported(_) => "request.invalid",
            Self::Conflict(_) => "state.conflict",
            Self::Store => "state.unavailable",
            Self::Provider { code, .. } => code,
            Self::MissingSecrets(_) => "deploy.secret_missing",
            Self::ApprovalRequired => "approval.required",
            Self::Watching => "app.watching",
            Self::AgentsDisabled => "agent.disabled",
        }
    }
}

/// The active admission key's keychain reference and commitment. Neither holds key
/// material or a hash of an environment value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentAdmissionKey {
    /// Unique keychain reference, recorded before the key is written.
    pub reference: String,
    /// SHA-256 of the random key, to notice if it was replaced.
    pub commitment: String,
}

/// Key setup records, read together and without key material, for the owner's review.
#[derive(Debug, Clone)]
pub struct EnvironmentKeySnapshot {
    /// The active key's reference and commitment, once set up.
    pub active: Option<EnvironmentAdmissionKey>,
    /// Up to sixteen pending setup references (not active, not retired), sorted.
    pub pending: Vec<String>,
    /// Number of retired setup attempts, which stay recorded; stops at `u32::MAX`.
    pub retired: u32,
}

impl EnvironmentKeySnapshot {
    /// A hash of this state, so a repair applies only to the state the owner reviewed.
    /// It grants no access.
    ///
    /// # Errors
    /// Reports an encoding failure.
    pub fn review_token(&self) -> Result<String, EngineError> {
        use sha2::{Digest, Sha256};
        let active = self
            .active
            .as_ref()
            .map(|key| (&key.reference, &key.commitment));
        let encoded = serde_json::to_vec(&(
            "environment-key-setup-v1",
            active,
            &self.pending,
            self.retired,
        ))
        .map_err(|_| EngineError::Store)?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}

const MAX_ACTOR_BYTES: usize = 128;
/// A UUID written out with hyphens.
const UUID_CHARS: usize = 36;

fn invalid_identity() -> EngineError {
    EngineError::Invalid("Use a valid deployment request identity.")
}

/// A deploy request's saved identity, used to recognise retries. The fingerprint must cover
/// the whole normalized request and the signed-in actor. Never hash secret values without
/// a key: plain hashes of secrets can be guessed.
#[derive(Debug, Clone)]
pub struct DeploymentAdmission {
    actor: String,
    key: String,
    fingerprint: String,
}

impl DeploymentAdmission {
    /// Checks the actor, the request key (a UUID) and the fingerprint (64 lowercase hex
    /// digits).
    ///
    /// # Errors
    /// Rejects malformed or oversized values without echoing them.
    pub fn new(actor: &str, key: &str, fingerprint: &str) -> Result<Self, EngineError> {
        let key = Self::request_key(actor, key)?;
        if fingerprint.len() != 64
            || !fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid_identity());
        }
        Ok(Self {
            actor: actor.into(),
            key,
            fingerprint: fingerprint.into(),
        })
    }

    /// Checks the actor and the request key, and returns the key in
    /// lowercase, as it's saved: what looking up an earlier request needs.
    ///
    /// # Errors
    /// Rejects malformed or oversized values without echoing them.
    pub fn request_key(actor: &str, key: &str) -> Result<String, EngineError> {
        if actor.is_empty()
            || actor.len() > MAX_ACTOR_BYTES
            || actor.chars().any(char::is_control)
            || key.len() != UUID_CHARS
            || uuid::Uuid::parse_str(key).is_err()
        {
            return Err(invalid_identity());
        }
        Ok(key.to_ascii_lowercase())
    }

    /// The signed-in actor; request keys are unique per actor.
    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// The request key, a lowercase UUID.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Fingerprint of the whole request; not a secret, nor the request itself.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

/// One page of records. Pass `next_cursor` back as it is for the next page.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct Page<T> {
    /// Up to 100 records, in the same order every time.
    pub items: Vec<T>,
    /// Cursor for the next page, or null on the last page.
    pub next_cursor: Option<String>,
}

/// Saved state, shared by the desktop app, the CLI and MCP.
#[async_trait::async_trait]
pub trait CatalogRepository: Send + Sync {
    /// Removes activity, finished operations and finished webhook deliveries
    /// from before `before` (RFC 3339 UTC). Activity from the last 30 days
    /// is never removed. Returns how many rows went.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn prune_history(&self, before: &str) -> Result<u64, EngineError>;

    /// The newest event's id, or zero.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn latest_event_id(&self) -> Result<i64, EngineError>;

    /// Every webhook, with the last event id each has looked at.
    ///
    /// # Errors
    /// Returns unavailable or invalid persistent state.
    async fn webhooks(&self) -> Result<Vec<(crate::domain::model::Webhook, i64)>, EngineError>;

    /// Adds or replaces a webhook, starting it at `cursor`.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn put_webhook(
        &self,
        value: &crate::domain::model::Webhook,
        cursor: i64,
    ) -> Result<(), EngineError>;

    /// Removes a webhook and its delivery history.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn delete_webhook(&self, id: &str) -> Result<bool, EngineError>;

    /// Queues deliveries (an identical one already queued is kept) and moves
    /// the webhook's cursor, together.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn queue_deliveries(
        &self,
        webhook_id: &str,
        deliveries: &[crate::domain::model::WebhookDelivery],
        cursor: i64,
    ) -> Result<(), EngineError>;

    /// Pending deliveries due by `now`, oldest first.
    ///
    /// # Errors
    /// Returns unavailable or invalid persistent state.
    async fn due_deliveries(
        &self,
        now: &str,
        limit: u32,
    ) -> Result<Vec<crate::domain::model::WebhookDelivery>, EngineError>;

    /// Records how a try went.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn put_delivery(
        &self,
        value: &crate::domain::model::WebhookDelivery,
    ) -> Result<(), EngineError>;

    /// A webhook's newest deliveries.
    ///
    /// # Errors
    /// Returns unavailable or invalid persistent state.
    async fn deliveries(
        &self,
        webhook_id: &str,
        limit: u32,
    ) -> Result<Vec<crate::domain::model::WebhookDelivery>, EngineError>;

    /// Writes down a change to a provider, or its new state.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn put_operation(
        &self,
        value: &crate::domain::model::Operation,
    ) -> Result<(), EngineError>;

    /// One change by id.
    ///
    /// # Errors
    /// Returns unavailable or invalid persistent state.
    async fn operation(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::model::Operation>, EngineError>;

    /// The newest change with this digest that is still running or unclear.
    ///
    /// # Errors
    /// Returns unavailable or invalid persistent state.
    async fn open_operation(
        &self,
        key: &str,
    ) -> Result<Option<crate::domain::model::Operation>, EngineError>;

    /// Changes, newest first, 50 a page.
    ///
    /// # Errors
    /// Returns unavailable or invalid persistent state, or a bad cursor.
    async fn operations(
        &self,
        cursor: Option<&str>,
    ) -> Result<Page<crate::domain::model::Operation>, EngineError>;

    /// Reads a project's map layout; with none saved, revision zero and automatic placement.
    ///
    /// # Errors
    /// Returns unavailable or invalid persistent state.
    async fn map_layout(
        &self,
        project_id: &str,
    ) -> Result<crate::domain::map_layout::MapLayout, EngineError>;

    /// Replaces the layout if its revision still matches, and records the activity, in one
    /// transaction.
    ///
    /// # Errors
    /// Rejects stale revisions, missing projects and invalid coordinates.
    async fn put_map_layout(
        &self,
        actor: &str,
        project_id: &str,
        value: crate::domain::map_layout::MapLayout,
    ) -> Result<crate::domain::map_layout::MapLayout, EngineError>;

    /// Lists unassigned resources in either the active or ignored import list.
    /// # Errors
    /// Reports invalid pagination or unavailable state.
    async fn import_candidates(
        &self,
        ignored: bool,
        cursor: Option<&str>,
    ) -> Result<Page<Resource>, EngineError>;

    /// Ids of ignored resources, from local state only.
    /// # Errors
    /// Rejects more than 10,000 ignored resources, or unavailable state.
    async fn ignored_resources(&self) -> Result<Vec<String>, EngineError>;

    /// Ignores or un-ignores an unassigned resource, with its activity event. Nothing changes
    /// at the provider.
    /// # Errors
    /// Rejects missing or assigned resources, or unavailable state.
    async fn set_resource_ignored(
        &self,
        actor: &str,
        id: &str,
        ignored: bool,
    ) -> Result<(), EngineError>;

    /// Records new keychain references before any token is written. Local state must be empty.
    /// # Errors
    /// Rejects existing state, another pending restore or invalid references.
    async fn prepare_transfer_restore(
        &self,
        record: &crate::data::transfer::payload::TransferRestoreRecord,
    ) -> Result<(), EngineError>;

    /// Installs a checked transfer and marks its restore record installed, in one transaction.
    /// # Errors
    /// Rejects a changed destination or restore record, or failed writes; tokens already
    /// written stay recorded.
    async fn install_transfer(
        &self,
        payload: &crate::data::transfer::payload::TransferPayload,
        record: &crate::data::transfer::payload::TransferRestoreRecord,
    ) -> Result<(), EngineError>;

    /// The newest 100 restore attempts, without credentials.
    /// # Errors
    /// Reports unavailable or malformed state.
    async fn transfer_restores(
        &self,
    ) -> Result<Vec<crate::data::transfer::payload::TransferRestoreStatus>, EngineError>;

    /// Marks one pending restore abandoned. Deletes no credentials and installs nothing.
    /// # Errors
    /// Rejects unknown or already completed attempts.
    async fn abandon_transfer_restore(&self, id: &str) -> Result<(), EngineError>;

    /// Reads everything a transfer carries in one consistent read, without credentials or
    /// agent grants.
    ///
    /// # Errors
    /// Rejects oversized or inconsistent state, or database failures.
    async fn transfer_snapshot(
        &self,
    ) -> Result<crate::data::transfer::payload::TransferPayload, EngineError>;

    /// Reads one account.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn account(&self, id: &str) -> Result<Option<Account>, EngineError>;

    /// Lists accounts in stable id order.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn accounts(&self, cursor: Option<&str>) -> Result<Page<Account>, EngineError>;

    /// Persists a validated account and its event.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_account(&self, value: &Account) -> Result<(), EngineError>;

    /// Reads one project.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn project(&self, id: &str) -> Result<Option<Project>, EngineError>;

    /// Lists projects in stable id order.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn projects(&self, cursor: Option<&str>) -> Result<Page<Project>, EngineError>;

    /// Persists a validated project and its event.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_project(&self, value: &Project) -> Result<(), EngineError>;

    /// Creates a project with its event in one transaction.
    ///
    /// # Errors
    /// Rejects an existing id or unavailable persistent state.
    async fn create_project(&self, actor: &str, value: &Project) -> Result<(), EngineError>;

    /// Every environment, `default` first, then by name.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    async fn environments(&self) -> Result<Vec<crate::domain::model::Environment>, EngineError>;

    /// Saves a new environment; names are unique.
    ///
    /// # Errors
    /// Returns a conflict when the name is taken.
    async fn create_environment(
        &self,
        actor: &str,
        value: &crate::domain::model::Environment,
    ) -> Result<(), EngineError>;

    /// Renames an environment; names are unique.
    ///
    /// # Errors
    /// Returns not found or a conflict when the name is taken.
    async fn rename_environment(
        &self,
        actor: &str,
        id: &str,
        name: &str,
    ) -> Result<crate::domain::model::Environment, EngineError>;

    /// Removes an empty environment other than `default`.
    ///
    /// # Errors
    /// Returns not found, or a conflict for `default` or one with projects in it.
    async fn remove_environment(&self, actor: &str, id: &str) -> Result<(), EngineError>;

    /// Moves a project into another environment. Nothing changes at the provider.
    ///
    /// # Errors
    /// Returns not found, or a conflict when that environment has a project of the same name.
    async fn move_project(
        &self,
        actor: &str,
        project_id: &str,
        environment_id: &str,
    ) -> Result<Project, EngineError>;

    /// Creates an imported project and assigns the chosen resources to it, in one transaction.
    ///
    /// # Errors
    /// Rejects duplicate, missing or already-assigned resources, or unavailable state.
    async fn import_project(
        &self,
        actor: &str,
        value: &Project,
        ids: &[String],
    ) -> Result<Vec<Resource>, EngineError>;

    /// Links a deploy source to an imported project that has none, with its activity event.
    ///
    /// # Errors
    /// Rejects missing projects, already-linked sources or unavailable state.
    async fn link_source(
        &self,
        actor: &str,
        project_id: &str,
        source: &Source,
    ) -> Result<Project, EngineError>;

    /// Reads one resource.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn resource(&self, id: &str) -> Result<Option<Resource>, EngineError>;

    /// Lists resources in stable id order.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn resources(&self, cursor: Option<&str>) -> Result<Page<Resource>, EngineError>;

    /// Persists a validated resource and its event.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_resource(&self, value: &Resource) -> Result<(), EngineError>;

    /// Switches a project and all its resources between watching and managed, in one
    /// transaction.
    ///
    /// # Errors
    /// Rejects a missing project; on a storage failure nothing changes.
    async fn set_project_mode(&self, project_id: &str, mode: Mode) -> Result<Project, EngineError>;

    /// Assigns discovered resources to a project in one transaction, keeping existing
    /// assignments and modes.
    ///
    /// # Errors
    /// Rejects duplicates, missing resources, another project's resources or storage failures.
    async fn group_resources(
        &self,
        actor: &str,
        project_id: &str,
        ids: &[String],
    ) -> Result<Vec<Resource>, EngineError>;

    /// Reads a saved custom hostname.
    ///
    /// # Errors
    /// Reports unavailable state.
    async fn custom_domain(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::domains::CustomDomain>, EngineError>;

    /// Lists a project's custom hostnames, or every project's for the monitor.
    ///
    /// # Errors
    /// Reports unavailable state.
    async fn custom_domains(
        &self,
        project_id: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<Page<crate::domain::domains::CustomDomain>, EngineError>;

    /// Saves what was seen about a hostname, with its activity, in one transaction.
    ///
    /// # Errors
    /// Rejects conflicting ownership or state that can't be saved.
    async fn put_custom_domain(
        &self,
        value: &crate::domain::domains::CustomDomain,
    ) -> Result<(), EngineError>;

    /// Removes a domain's local records once the provider has detached it.
    ///
    /// # Errors
    /// Reports state that can't be saved.
    async fn remove_custom_domain(&self, id: &str) -> Result<(), EngineError>;

    /// Reads one agent.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn agent(&self, id: &str) -> Result<Option<Agent>, EngineError>;

    /// Lists agents in stable id order.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn agents(&self, cursor: Option<&str>) -> Result<Page<Agent>, EngineError>;

    /// Persists a validated agent and its event.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_agent(&self, value: &Agent) -> Result<(), EngineError>;

    /// Checks the agent's current scopes and records its activity, in one write transaction.
    ///
    /// # Errors
    /// Rejects revoked, expired, missing or under-scoped agents.
    async fn authorize_agent(
        &self,
        id: &str,
        scope: &str,
        at: &str,
        expiry_days: u16,
    ) -> Result<(), EngineError>;

    /// Changes an agent's scopes or revokes it, keeping its activity and credentials.
    ///
    /// # Errors
    /// Rejects missing agents, invalid scopes, or scope changes to a revoked agent.
    async fn edit_agent(
        &self,
        actor: &str,
        id: &str,
        scopes: Option<&[String]>,
        revoke: bool,
    ) -> Result<Agent, EngineError>;

    /// Replaces an agent's grant with the next version, with its event, in one transaction.
    ///
    /// # Errors
    /// Rejects missing or revoked agents.
    async fn set_agent_grant(
        &self,
        actor: &str,
        id: &str,
        grant: crate::domain::model::AgentGrantInput,
    ) -> Result<Agent, EngineError>;

    /// Reconnects one local agent, keeping its scopes, with its event.
    ///
    /// # Errors
    /// Rejects a missing or non-stdio agent, or unavailable state.
    async fn reconnect_agent(&self, actor: &str, id: &str, at: &str) -> Result<Agent, EngineError>;

    /// Reads one plan.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn plan(&self, id: &str) -> Result<Option<Plan>, EngineError>;

    /// Lists plans in stable id order.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn plans(&self, cursor: Option<&str>) -> Result<Page<Plan>, EngineError>;

    /// Persists a validated plan and its event.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_plan(&self, value: &Plan) -> Result<(), EngineError>;

    /// Saves a plan edit as the next revision, only if the stored revision still matches.
    ///
    /// # Errors
    /// Rejects stale edits, invalid revision advances, immutable snapshots or missing plans.
    async fn update_plan(&self, value: &Plan, expected_revision: i64) -> Result<(), EngineError>;

    /// Removes account metadata only when no resources reference it.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn remove_account(&self, id: &str) -> Result<(), EngineError>;

    /// Removes a resource's local records once the provider has deleted it.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn remove_resource(&self, id: &str) -> Result<(), EngineError>;

    /// Removes project metadata after its resources are gone.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn remove_project(&self, id: &str) -> Result<(), EngineError>;

    /// Reads a project's links.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn links(&self, project_id: &str) -> Result<Vec<Link>, EngineError>;

    /// Every link, for checking what a change affects in other projects.
    async fn all_links(&self) -> Result<Vec<Link>, EngineError>;

    /// Reads both directions of one resource's relationships; fails above 1,000 links.
    async fn resource_links(&self, resource_id: &str) -> Result<Vec<Link>, EngineError>;

    /// Latest saved daily usage, including for resources not assigned to a project.
    async fn latest_resource_usage(&self, resource_id: &str) -> Result<Option<Usage>, EngineError>;

    /// The newest deployment that actually released this resource, across all history.
    async fn latest_resource_deploy(
        &self,
        project_id: &str,
        resource_id: &str,
    ) -> Result<Option<Deploy>, EngineError>;

    /// Saves a link between assigned resources, keeping the projects they belong to.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_link(&self, value: &Link) -> Result<(), EngineError>;

    /// Updates links seen at Cloudflare in one transaction; links no longer seen are kept,
    /// marked missing.
    ///
    /// # Errors
    /// Rejects invalid scope, unassigned targets or unavailable persistent state.
    async fn reconcile_observed_links(
        &self,
        actor: &str,
        from: &str,
        links: &[Link],
        preserve_names: &[String],
    ) -> Result<(), EngineError>;

    /// Reads a deployment.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn deploy(&self, id: &str) -> Result<Option<Deploy>, EngineError>;

    /// Lists a project's deployments, newest first, paged by version.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn deploys(
        &self,
        project_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<Deploy>, EngineError>;

    /// The 50 most recently queued deployment attempts across all projects,
    /// newest first. A bounded snapshot, not a paginated history.
    ///
    /// # Errors
    /// Reports unavailable or invalid persistent state.
    async fn recent_deploys(&self) -> Result<Vec<Deploy>, EngineError>;

    /// Gives a deployment its version and queues it, in one transaction.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn enqueue(&self, value: &Deploy) -> Result<Deploy, EngineError> {
        self.enqueue_with_admission(value, None).await
    }

    /// Reads the pending setup records and the active key reference in one transaction.
    ///
    /// # Errors
    /// Reports unavailable or inconsistent local state.
    async fn environment_key_snapshot(&self) -> Result<EnvironmentKeySnapshot, EngineError>;

    /// Retires exactly the reviewed pending references, only while there is no active key.
    /// Never deletes credentials, receipts or references, and never activates a key.
    ///
    /// # Errors
    /// Rejects a stale review token, an active key, no pending references, or unavailable
    /// state.
    async fn retire_environment_key_setup(&self, review_token: &str) -> Result<(), EngineError>;

    /// Reads the active admission key's reference, never the key.
    ///
    /// # Errors
    /// Reports unreadable local state.
    async fn environment_admission_key(
        &self,
    ) -> Result<Option<EnvironmentAdmissionKey>, EngineError>;

    /// Records a new key reference before the key is written to the keychain. At most
    /// sixteen attempts are kept; an interrupted attempt never overwrites the active key.
    ///
    /// # Errors
    /// Rejects an invalid or reused reference, sixteen pending attempts, or unreadable state.
    async fn reserve_environment_admission_key(&self, reference: &str) -> Result<(), EngineError>;

    /// Activates a reserved, verified key, or returns the key another attempt activated
    /// first.
    ///
    /// # Errors
    /// Rejects invalid commitments, unknown references and unavailable state.
    async fn activate_environment_admission_key(
        &self,
        key: &EnvironmentAdmissionKey,
    ) -> Result<EnvironmentAdmissionKey, EngineError>;

    /// Reads an accepted deploy request's receipt, without secret inputs, keychain reads or
    /// re-running it. Callers must check they may see `actor`'s requests first.
    ///
    /// # Errors
    /// Rejects a malformed actor or key, or unavailable or inconsistent state.
    async fn deployment_request(
        &self,
        actor: &str,
        request_key: &str,
    ) -> Result<Option<DeploymentRequest>, EngineError>;

    /// Looks up an earlier accepted request with the same key. A different fingerprint, or a
    /// deployment since deleted, is a conflict. Callers must check the actor first.
    ///
    /// # Errors
    /// Reports a conflict, a missing deployment, or unavailable state.
    async fn admitted_deploy(
        &self,
        admission: &DeploymentAdmission,
    ) -> Result<Option<Deploy>, EngineError>;

    /// Writes the receipt, queue item, version and event in one transaction, or, for an exact
    /// retry, returns the existing deployment and writes nothing.
    ///
    /// # Errors
    /// Reports a fingerprint mismatch, a full queue or unavailable state.
    async fn enqueue_with_admission(
        &self,
        value: &Deploy,
        admission: Option<&DeploymentAdmission>,
    ) -> Result<Deploy, EngineError>;

    /// Saves step progress and records a deployment event.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_deploy(&self, value: &Deploy) -> Result<(), EngineError>;

    /// Marks an unfinished deployment cancelled; a provider call already under way is not
    /// interrupted.
    ///
    /// # Errors
    /// Rejects unknown or finished deployments and state that can't be saved.
    async fn cancel_deploy(&self, actor: &str, id: &str) -> Result<(), EngineError>;

    /// Whether the project has any queued or running deployment, across all history pages.
    ///
    /// # Errors
    /// Reports state that can't be saved.
    async fn has_pending_deploy(&self, project_id: &str) -> Result<bool, EngineError>;

    /// Reads the next batch of queued deployments.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn queued(&self) -> Result<Vec<Deploy>, EngineError>;

    /// Marks interrupted running deployments failed after a restart.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn recover_deploys(&self) -> Result<(), EngineError>;

    /// Reads an approval and its saved decision.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn approval(&self, id: &str) -> Result<Option<Approval>, EngineError>;

    /// Lists approvals.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn approvals(&self, cursor: Option<&str>) -> Result<Page<Approval>, EngineError>;

    /// Creates an approval once.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_approval(&self, value: &Approval) -> Result<(), EngineError>;

    /// Resolves only an unexpired pending approval.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn decide_approval(&self, id: &str, allow: bool, now: &str) -> Result<(), EngineError>;

    /// Uses up an allowed, unexpired approval that matches the agent, action and target
    /// exactly, in one transaction.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn consume_approval(
        &self,
        id: &str,
        agent_id: &str,
        action: GatedAction,
        target: &str,
        now: &str,
    ) -> Result<(), EngineError>;

    /// Reads secret names and fingerprints without values.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn secrets(&self, project_id: &str) -> Result<Vec<SecretMetadata>, EngineError>;

    /// Lists secret names, ordered by resource then name; the cursor works only for this
    /// project.
    ///
    /// # Errors
    /// Rejects invalid cursors or unavailable state.
    async fn secrets_page(
        &self,
        project_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<SecretMetadata>, EngineError>;

    /// Records a secret's metadata after the provider accepted it.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_secret(&self, value: &SecretMetadata) -> Result<(), EngineError>;

    /// Removes a secret's local metadata and records the provider's deletion, in one transaction.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn delete_secret_metadata(
        &self,
        actor: &str,
        project_id: &str,
        resource_id: &str,
        name: &str,
    ) -> Result<(), EngineError>;

    /// Reads a project's usage rows since a UTC date.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn usage(&self, project_id: &str, since: &str) -> Result<Vec<Usage>, EngineError>;

    /// Lists daily usage ordered by day then resource; the cursor works only for this
    /// project and date.
    ///
    /// # Errors
    /// Rejects invalid cursors or unavailable state.
    async fn usage_page(
        &self,
        project_id: &str,
        since: &str,
        cursor: Option<&str>,
    ) -> Result<Page<Usage>, EngineError>;

    /// Appends an event with a structured detail (an agent's call: what it
    /// sent, secrets removed, and how it ended). Stores without detail
    /// support record the summary alone.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn event_with_detail(
        &self,
        actor: &str,
        project_id: Option<&str>,
        kind: &str,
        summary: &str,
        _detail: &str,
    ) -> Result<(), EngineError> {
        self.event(actor, project_id, kind, summary).await
    }

    /// The detail recorded with an event, if any.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn event_detail(&self, _id: i64) -> Result<Option<String>, EngineError> {
        Ok(None)
    }

    /// Inserts or replaces the day's usage row, so a day is never counted twice.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_usage(&self, value: &Usage) -> Result<(), EngineError>;

    /// Reads settings or their first-run defaults.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn settings(&self) -> Result<Settings, EngineError>;

    /// Persists validated settings.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn put_settings(&self, value: &Settings) -> Result<(), EngineError>;

    /// Restores a checked backup into an empty catalog, in one transaction.
    ///
    /// # Errors
    /// Rejects a catalog that isn't empty, or any failed insert; then nothing is restored.
    async fn restore(&self, backup: &crate::domain::backup::Backup) -> Result<(), EngineError>;

    /// Reads an OAuth client registration.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn oauth_client(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::oauth::OAuthClient>, EngineError>;

    /// Persists an OAuth client registration.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn put_oauth_client(
        &self,
        value: &crate::domain::oauth::OAuthClient,
    ) -> Result<(), EngineError>;

    /// Creates a pending remote agent and its approval in one transaction, within the limit
    /// on open requests.
    ///
    /// # Errors
    /// Rejects inconsistent records, too many open requests or storage failure.
    async fn begin_oauth(
        &self,
        agent: &Agent,
        approval: &Approval,
        pending: &crate::domain::oauth::OAuthPending,
    ) -> Result<(), EngineError>;

    /// Reads a pending OAuth authorization, without credential values.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn oauth_pending(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::oauth::OAuthPending>, EngineError>;

    /// Saves a pending OAuth authorization.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn put_oauth_pending(
        &self,
        value: &crate::domain::oauth::OAuthPending,
    ) -> Result<(), EngineError>;

    /// Reads an OAuth grant, without token values.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn oauth_grant(
        &self,
        id: &str,
    ) -> Result<Option<crate::domain::oauth::OAuthGrant>, EngineError>;

    /// Saves an OAuth grant.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn put_oauth_grant(
        &self,
        value: &crate::domain::oauth::OAuthGrant,
    ) -> Result<(), EngineError>;

    /// Connection requests from coding tools on this computer, oldest first.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn enrollments(&self) -> Result<Vec<crate::engine::StoredEnrollment>, EngineError>;

    /// Saves one connection request.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn put_enrollment(
        &self,
        value: &crate::engine::StoredEnrollment,
    ) -> Result<(), EngineError>;

    /// Forgets connection requests that ended before `before` (RFC 3339).
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    async fn forget_enrollments(&self, before: &str) -> Result<(), EngineError>;

    /// Appends a redacted activity entry.
    ///
    /// # Errors
    /// Reports missing, conflicting or unavailable persistent state.
    async fn event(
        &self,
        actor: &str,
        project_id: Option<&str>,
        kind: &str,
        summary: &str,
    ) -> Result<(), EngineError>;
}
