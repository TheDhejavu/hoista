//! The things Hoista keeps track of: projects, resources, accounts, deploys,
//! agents, settings and the changes it makes at providers. None of these
//! types holds a secret value.

use crate::Provider;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Whether Hoista may change a project or resource at its provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The person let Hoista manage it: deploy, change and delete.
    Managed,
    /// Hoista only reads it.
    Watching,
}

/// What a resource is, whatever its provider calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// A website or web app, static or server-rendered.
    Web,
    /// A process that keeps running, such as an API.
    Service,
    /// Something that runs on a schedule.
    Job,
    /// Database.
    Database,
    /// Object storage.
    Bucket,
    /// Sends email.
    Email,
    /// A domain or its DNS.
    Domain,
}

impl ResourceKind {
    /// Whether the resource runs the project's own code. A project needs one: a database,
    /// bucket, email sender or domain is something a project uses, not a project.
    #[must_use]
    pub const fn runs_code(self) -> bool {
        matches!(self, Self::Web | Self::Service | Self::Job)
    }
}

/// How a resource is doing, as its provider last said. Unknown never counts as fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceStatus {
    /// Running and answering.
    Live,
    /// Asleep until something asks for it.
    Sleeping,
    /// Running, but nothing has used it for a while.
    Idle,
    /// Failing.
    Failing,
    /// Paused by the person.
    Paused,
    /// A deploy is building or starting at the provider.
    Deploying,
    /// The provider has no deployment of it yet.
    NotDeployed,
    /// Stopped or removed at the provider; it stays that way until the next deploy.
    Stopped,
    /// Not checked yet, or the provider couldn't be reached on the last check.
    Unknown,
}

/// Which app a deploy was asked for from. Who asked is recorded separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DeployChannel {
    /// Hoista Desktop.
    Desktop,
    /// The `hoist` command line.
    Cli,
    /// An agent, over MCP.
    Mcp,
}

/// Options for a deploy or a rollback. A rollback refuses `expected_plan_revision`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DeployOptions {
    /// A UUID that makes it safe to retry: sending the same key again returns
    /// the first attempt. A deploy with a key must also give `expected_plan_revision`.
    pub request_key: Option<String>,
    /// The plan revision the person looked at; refused if the plan has changed. Without it, the current plan is used.
    #[ts(type = "number | null")]
    pub expected_plan_revision: Option<i64>,
    /// What the change is, in the person's words: 1 to 160 characters.
    pub title: Option<String>,
    /// Which app asked. An agent's deploys are always recorded as MCP.
    pub channel: Option<DeployChannel>,
}

/// A deploy request found by its key.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DeploymentRequest {
    /// The key, as a lowercase UUID. Each caller's keys are their own.
    pub request_key: String,
    /// Who sent it.
    pub actor: String,
    /// The project, kept even if the project has since been deleted.
    pub project_id: String,
    /// The deploy it started, kept even if that deploy's history was deleted.
    pub deploy_id: String,
    /// The deploy as it stands now, or null if its history was deleted. Null
    /// still means the request was accepted.
    pub deployment: Option<Deploy>,
}

/// Where a deploy stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DeployStatus {
    /// Accepted and waiting to start.
    Queued,
    /// Building or releasing.
    Running,
    /// Released and serving.
    Live,
    /// Stopped with an error.
    Failed,
    /// Replaced by a rollback to an earlier deployment.
    RolledBack,
}

/// Where one step of a deploy stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Not started yet.
    Pending,
    /// In progress.
    Running,
    /// Finished without error.
    Succeeded,
    /// Finished with an error.
    Failed,
    /// Not needed for this deployment.
    Skipped,
}

/// Whether a connected account's token works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    /// The token works.
    Connected,
    /// The provider no longer accepts the token.
    Expired,
    /// The token lacks permissions Hoista needs.
    MissingScopes,
    /// No usable token: the person disconnected it, or it came from a backup.
    Disconnected,
}

/// Where a request for the person's approval stands. An approval is used once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Waiting for the person.
    Pending,
    /// The person allowed it; not used yet.
    Allowed,
    /// The person declined it.
    Declined,
    /// Not decided before it expired.
    Expired,
    /// Allowed and used; it can't be used again.
    Consumed,
}

/// Actions an agent must have the person approve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum GatedAction {
    /// Let an app connect as an agent, with the access it asked for.
    ConnectAgent,
    /// Remove a project and its resources.
    DeleteProject,
    /// Remove one provider resource.
    DeleteResource,
    /// Remove a custom domain and its certificate. The project stays.
    DeleteDomain,
    /// Connect a new provider account.
    ConnectAccount,
    /// A destructive operation at a provider (delete, restore over, reset), from
    /// the provider's operation list.
    ProviderChange,
}

/// How an agent reaches the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentTransport {
    /// A coding tool on this computer, over stdio.
    Stdio,
    /// A chat app or cloud agent, connected with OAuth.
    Link,
}

/// An amount of money in whole cents. Never converted between currencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Money {
    /// Amount in the currency’s smallest unit.
    #[ts(type = "number")]
    pub cents: i64,
    /// ISO 4217 currency code, such as `USD`.
    pub currency: String,
}

/// A connected provider account. Its token is kept in the vault, under its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Account {
    /// How the last look for its resources went; null before the first.
    pub last_discovery: Option<DiscoverySummary>,
    /// Its id in Hoista.
    pub id: String,
    /// Its provider.
    pub provider: Provider,
    /// Its name, as shown.
    pub label: String,
    /// The provider's id for the account, team or organization.
    pub external_id: String,
    /// What the token was checked to be allowed to do, when connected.
    pub scopes: Vec<String>,
    /// Whether its token works.
    pub status: AccountStatus,
    /// When it was connected (RFC 3339, UTC).
    pub connected_at: String,
    /// Another account whose saved sign-in this one shares: one provider sign-in
    /// can cover several workspaces, and its refresh token works only once,
    /// so they keep one copy. Null when the account holds its own.
    pub credential: Option<String>,
    /// The token lives in 1Password: Hoista keeps only its `op://` reference.
    pub in_1password: bool,
}

/// A project: an app made of one or more resources, possibly at several providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// Its id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Where its code is: a folder, or a repository at a commit.
    pub source: Source,
    /// Managed or watched.
    pub mode: Mode,
    /// RFC 3339 UTC creation time.
    pub created_at: String,
    /// The Hoista environment it lives in.
    pub environment_id: String,
}

/// The environment every project starts in. It always exists and can't be removed.
pub const DEFAULT_ENVIRONMENT: &str = "env_default";

/// What the id of a link read from a provider's bindings starts with. Links
/// the person drew never do.
pub const OBSERVED_LINK_PREFIX: &str = "observed-";

/// The most resources discovery keeps for one account, and the catalog for
/// the whole machine.
pub const MAX_DISCOVERED_RESOURCES: usize = 10_000;

/// A separate space for projects, such as `default` or `production`. Agents
/// can be limited to some of them. Nothing at the providers changes with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Stable id; `env_default` for the default one.
    pub id: String,
    /// Lowercase name with letters, digits and hyphens, unique among environments.
    pub name: String,
    /// RFC 3339 UTC creation time.
    pub created_at: String,
}

/// A resource at a provider: a site, service, database and so on. It may not be in a project yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// Its id.
    pub id: String,
    /// Its project, or null until it's put in one.
    pub project_id: Option<String>,
    /// What it is.
    pub kind: ResourceKind,
    /// Provider adapter.
    pub provider: Provider,
    /// Connected account id.
    pub account_id: String,
    /// What the provider calls it; Hoista doesn't read anything into it.
    pub provider_ref: String,
    /// Display name.
    pub name: String,
    /// Its region, when the provider has regions.
    pub region: Option<String>,
    /// How it's doing, as last checked.
    pub status: ResourceStatus,
    /// Managed or watched.
    pub mode: Mode,
    /// Its public address, when it has one.
    pub url: Option<String>,
    /// The provider's grouping it belongs to, by name: a Railway project, for
    /// example. Null when the provider has none or it isn't known yet.
    pub group: Option<String>,
}

/// A line on the project map: one resource uses another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// Its id.
    pub id: String,
    /// The resource that uses the other.
    pub from_resource: String,
    /// The resource it uses.
    pub to_resource: String,
    /// How: the variable, route or binding that connects them.
    pub via: String,
    /// Whether Hoista found it at the provider, rather than the person drawing it.
    pub derived: bool,
    /// Whether a link Hoista found earlier has since gone.
    pub missing: bool,
}

/// Progress through one step of a deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DeployStep {
    /// The step's name, such as `build`.
    pub name: String,
    /// Where it stands.
    pub status: StepStatus,
    /// What happened, with secrets taken out.
    pub message: Option<String>,
}

/// What a provider's failed answer said, without its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProviderFailureEvidence {
    /// The HTTP status.
    pub http_status: u16,
    /// The provider's id for the request, for their support, when it gives one.
    pub request_id: Option<String>,
    /// The header that id came from.
    pub request_id_header: Option<String>,
}

/// Why a deploy failed, as far as Hoista knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DeployFailure {
    /// The provider's last failed answer, when the failure was one.
    pub provider_response: Option<ProviderFailureEvidence>,
    /// Variables the project needs that weren't set. Names only.
    pub missing_secrets: Vec<String>,
    /// The error code, such as `build.failed`.
    pub code: String,
    /// What went wrong, in plain words, with secrets taken out.
    pub message: String,
    /// The steps running when it failed; empty when it failed before starting.
    pub steps: Vec<String>,
    /// Whether it had started releasing, so something may have changed at the provider.
    pub release_started: bool,
    /// Whether undoing the partial release failed. `false` doesn't prove it was all undone.
    pub compensation_failed: bool,
}

/// One deploy or rollback, kept in the project's history whether it worked or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Deploy {
    /// Why it failed; null when it didn't, or Hoista stopped before it could tell.
    pub failure: Option<DeployFailure>,
    /// The variables given for this deploy only, by name. Their values are never saved.
    pub environment_keys: Vec<String>,
    /// What the change is, in the person's words. Never taken from a commit message.
    pub title: Option<String>,
    /// Which app it was asked for from.
    pub channel: DeployChannel,
    /// The agent's name when it asked, so renaming it later doesn't rewrite history.
    pub started_by_name: Option<String>,
    /// The project's release number (v1, v2, …); null until it's live, and for failed deploys.
    #[ts(type = "number | null")]
    pub release_version: Option<i64>,
    /// What the source copy held; null when copying didn't finish.
    pub provenance: Option<super::provenance::SourceProvenance>,
    /// A first look at the source before it was copied; null while queued, or
    /// when it stopped before looking.
    pub source_observation: Option<super::provenance::SourceObservation>,
    /// Whether the person stopped it. It stops at the next point where stopping is safe.
    pub cancel_requested: bool,
    /// The plan it ran.
    pub plan_id: String,
    /// What it released at each provider, to roll back to later.
    pub releases: Vec<ResourceRelease>,
    /// Its id.
    pub id: String,
    /// The project.
    pub project_id: String,
    /// Counts up with each deploy of the project, failed ones included.
    #[ts(type = "number")]
    pub version: i64,
    /// The source it deployed.
    pub source: Source,
    /// The person, or the agent's id.
    pub started_by: String,
    /// Where it stands.
    pub status: DeployStatus,
    /// Its steps, in order.
    pub steps: Vec<DeployStep>,
    /// The address it was checked to answer on.
    pub url: Option<String>,
    /// When it started (RFC 3339, UTC).
    pub started_at: String,
    /// When it finished.
    pub finished_at: Option<String>,
    /// How long it took, in milliseconds.
    #[ts(type = "number | null")]
    pub duration_ms: Option<i64>,
    /// For a rollback, the deploy it went back to.
    pub rollback_of: Option<String>,
}

/// An agent the person let connect, and what it may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// Its id.
    pub id: String,
    /// The name it connected with.
    pub name: String,
    /// How it connects.
    pub transport: AgentTransport,
    /// What it may do: `apps:read`, `apps:write`, `apps:create`.
    pub scopes: Vec<String>,
    /// When it last did anything (RFC 3339, UTC).
    pub last_seen: String,
    /// Whether the person cut it off.
    pub revoked: bool,
    /// What it may touch beyond its scopes: which projects and environments,
    /// tools it may never use, and when the grant ends.
    pub grant: AgentGrant,
}

/// Where an agent's scopes apply. Deny wins: a tool in `deny` is refused
/// whatever the scopes say; a project outside `projects` is refused for every tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AgentGrant {
    /// Project ids it may touch; null means every project, including new ones.
    pub projects: Option<Vec<String>>,
    /// Hoista environment ids whose projects it may touch; null means every
    /// environment, including new ones.
    pub environment_ids: Option<Vec<String>>,
    /// Tool names it may never call, whatever its scopes.
    pub deny: Vec<String>,
    /// RFC 3339 UTC end of the grant; null means no end date (inactivity
    /// expiry still applies).
    pub expires_at: Option<String>,
    /// Raised on every change, so a decision can name the policy it used.
    pub version: u32,
    /// Full access: what would need the person's OK (deleting or removing
    /// anything, connecting an account) goes ahead without
    /// asking. Off for "Ask for approval": Hoista asks in the agent first.
    pub without_asking: bool,
    /// Stricter rules by Hoista environment id: what it may do to projects
    /// in that environment, on top of its access.
    pub environment_rules: std::collections::BTreeMap<String, ProductionRule>,
    /// Narrower access for some resources, on top of everything above. An
    /// exception only ever narrows; the strictest one that applies wins.
    pub exceptions: Vec<AgentException>,
}

/// The kinds of resource an exception can name together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceGroup {
    /// Web services, services, jobs and email senders.
    Services,
    /// Databases, key-value stores included.
    Databases,
    /// Buckets and volumes.
    Storage,
    /// Domains.
    Domains,
}

impl ResourceGroup {
    /// The group a kind of resource belongs to.
    #[must_use]
    pub const fn of(kind: ResourceKind) -> Self {
        match kind {
            ResourceKind::Web | ResourceKind::Service | ResourceKind::Job | ResourceKind::Email => {
                Self::Services
            }
            ResourceKind::Database => Self::Databases,
            ResourceKind::Bucket => Self::Storage,
            ResourceKind::Domain => Self::Domains,
        }
    }
}

/// How much an exception narrows an agent's access.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum Restriction {
    /// Ask for approval, even with full access.
    AlwaysAsk,
    /// Only reading.
    ViewOnly,
    /// Hidden from the agent altogether.
    NoAccess,
}

/// One exception: a group of resources (in every project, or one) or a
/// single resource, and how far it narrows access there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AgentException {
    /// A group of resources; set this or `resource_id`.
    pub group: Option<ResourceGroup>,
    /// With a group: the project it applies in, or null for every project.
    /// With a resource: the project it's in, for showing it.
    pub project_id: Option<String>,
    /// One resource; set this or `group`.
    pub resource_id: Option<String>,
    /// How far it narrows access.
    pub access: Restriction,
}

impl AgentException {
    /// Whether it applies to a resource of `kind` in `project`.
    #[must_use]
    pub fn covers(&self, resource: &str, kind: ResourceKind, project: Option<&str>) -> bool {
        match (&self.resource_id, self.group) {
            (Some(id), _) => id == resource,
            (None, Some(group)) => {
                group == ResourceGroup::of(kind)
                    && self
                        .project_id
                        .as_deref()
                        .is_none_or(|only| Some(only) == project)
            }
            (None, None) => false,
        }
    }
}

/// How an agent may act on production when that is stricter than elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProductionRule {
    /// Ask for approval, even with full access.
    AlwaysAsk,
    /// Only reading.
    ViewOnly,
}

impl AgentGrant {
    /// Whether the grant has passed its end date at `at`.
    pub fn expired(&self, at: &str) -> bool {
        let Some(end) = &self.expires_at else {
            return false;
        };
        match (
            chrono::DateTime::parse_from_rfc3339(end),
            chrono::DateTime::parse_from_rfc3339(at),
        ) {
            (Ok(end), Ok(at)) => at >= end,
            // An end date that doesn't parse counts as expired.
            _ => true,
        }
    }

    /// Whether the grant covers this project.
    pub fn allows_project(&self, project_id: &str) -> bool {
        self.projects
            .as_ref()
            .is_none_or(|projects| projects.iter().any(|id| id == project_id))
    }

    /// Whether it is limited to some projects rather than all of them.
    pub fn limited_to_projects(&self) -> bool {
        self.projects.is_some()
    }

    /// Whether the grant covers this Hoista environment.
    pub fn allows_environment_id(&self, id: &str) -> bool {
        self.environment_ids
            .as_ref()
            .is_none_or(|ids| ids.iter().any(|value| value == id))
    }
}

/// An owner's change to an agent's grant; the version is set by the engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AgentGrantInput {
    /// Project ids, or null for every project.
    pub projects: Option<Vec<String>>,
    /// Hoista environment ids, or null for every environment.
    pub environment_ids: Option<Vec<String>>,
    /// Tool names it may never call.
    #[serde(default)]
    pub deny: Vec<String>,
    /// RFC 3339 end, or null for none.
    pub expires_at: Option<String>,
    /// What needs the person's OK goes ahead without asking each time.
    #[serde(default)]
    pub without_asking: bool,
    /// Stricter rules by Hoista environment id.
    #[serde(default)]
    pub environment_rules: std::collections::BTreeMap<String, ProductionRule>,
    /// Narrower access for some resources.
    #[serde(default)]
    pub exceptions: Vec<AgentException>,
}

/// A request for the person's approval, for one action on one thing. It lapses after 15 minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    /// Its id.
    pub id: String,
    /// The agent asking.
    pub agent_id: String,
    /// What it wants to do.
    pub action: GatedAction,
    /// What it wants to do it to: a resource, or a hash of the exact operation.
    pub target: String,
    /// For a delete, everything it would remove.
    pub deletion: Option<super::deletion::DeletionPreview>,
    /// The agent's reason, shown to the person.
    pub reason: String,
    /// When it asked.
    pub created_at: String,
    /// When it lapses, 15 minutes later.
    pub expires_at: String,
    /// Where it stands.
    pub decision: ApprovalDecision,
}

/// A variable Hoista set at a provider. Only a fingerprint of its value is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SecretMetadata {
    /// It was changed in Hoista, and the project's `.env` files haven't caught up yet.
    pub requires_source_sync: bool,
    /// Who set it last; null when it came in with an import.
    pub set_by: Option<String>,
    /// The provider sets this one itself, such as a database address. Not the person's to replace.
    pub managed_by: Option<Provider>,
    /// The project.
    pub project_id: String,
    /// The resource it's set on.
    pub resource_id: String,
    /// Its name.
    pub name: String,
    /// SHA-256 of the value.
    pub fingerprint: String,
    /// When it was last set (RFC 3339, UTC).
    pub set_at: String,
}

/// A variable as the apps see it: never its value, or even its fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct EnvironmentVariable {
    /// It was changed in Hoista; the project's `.env` files haven't caught up, so they mustn't overwrite it.
    pub requires_source_sync: bool,
    /// The resource it's set on.
    pub resource_id: String,
    /// Its name.
    pub name: String,
    /// When it was last set.
    pub set_at: String,
    /// Who set it last, when known.
    pub set_by: Option<String>,
    /// The provider, when the provider sets it itself.
    pub managed_by: Option<Provider>,
    /// Whether it's built into what browsers download, so anyone can read it.
    pub browser_visible: bool,
}

/// Whether the variable `name` is built into what browsers download (it
/// starts with `VITE_`, `NEXT_PUBLIC_` or `PUBLIC_`).
pub fn browser_env(name: &str) -> bool {
    crate::leaks::PUBLIC_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

impl From<SecretMetadata> for EnvironmentVariable {
    fn from(value: SecretMetadata) -> Self {
        Self {
            requires_source_sync: value.requires_source_sync,
            browser_visible: browser_env(&value.name),
            resource_id: value.resource_id,
            name: value.name,
            set_at: value.set_at,
            set_by: value.set_by,
            managed_by: value.managed_by,
        }
    }
}

/// Daily resource usage, as the provider measured it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// The resource.
    pub resource_id: String,
    /// The day (UTC).
    pub day: String,
    /// Requests, when the provider counts them.
    #[ts(type = "number | null")]
    pub requests: Option<i64>,
    /// Errors, when the provider counts them.
    #[ts(type = "number | null")]
    pub errors: Option<i64>,
    /// Seconds of compute, when the provider counts them.
    #[ts(type = "number | null")]
    pub compute_seconds: Option<i64>,
    /// Bytes stored, when the provider says.
    #[ts(type = "number | null")]
    pub storage_bytes: Option<i64>,
    /// Bytes sent out, when the provider says.
    #[ts(type = "number | null")]
    pub egress_bytes: Option<i64>,
}

/// The largest whole number JavaScript holds exactly. Revisions and other
/// counters stay within it, so they survive the trip through the apps.
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Days `idle_days` and `agent_expiry_days` can be set to.
pub const SETTING_DAYS: std::ops::RangeInclusive<u16> = 1..=365;
/// The access a new agent can start on.
pub const AGENT_ACCESS_LEVELS: [&str; 3] = ["view", "ask", "full"];
/// Hours between automatic backups the person can pick; 0 is never.
pub const BACKUP_INTERVALS_HOURS: [u16; 4] = [0, 6, 24, 168];
/// How many automatic backups can be kept.
pub const BACKUPS_KEPT: std::ops::RangeInclusive<u16> = 1..=100;
/// The days of history the person can pick.
pub const HISTORY_DAYS: [u16; 4] = [30, 90, 365, 1095];

/// Hoista's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Whether agents may connect and make requests. Off pauses every agent.
    pub agents_enabled: bool,
    /// After how many days without use a resource counts as idle.
    pub idle_days: u16,
    /// After how many days without use an agent's access ends.
    pub agent_expiry_days: u16,
    /// Whether Hoista may check for new versions.
    pub check_updates: bool,
    /// Whether Hoista may prepare crash reports. The person sees each one before it's sent.
    pub crash_reports: bool,
    /// Also open Hoista's consent page on this computer's private network
    /// address. Off unless the person turns it on; follows the setting and
    /// the current network while Hoista runs.
    pub network_page: bool,
    /// Serve MCP over HTTP on `127.0.0.1:8788`, for web agents such as
    /// ChatGPT through a tunnel or proxy the person runs. On unless the person
    /// turns it off; nothing outside this computer reaches it without one.
    pub mcp_http: bool,
    /// The access Hoista's consent page starts on for a new agent: `view`,
    /// `ask` or `full`. The person still picks it on the page.
    pub default_access: String,
    /// Build projects even where they can't be isolated from the rest of this
    /// computer. Off unless the person turns it on.
    pub unisolated_builds: bool,
    /// Hours between automatic backups; 0 means none.
    pub backup_every_hours: u16,
    /// How many automatic backups are kept.
    pub backup_keep: u16,
    /// Days of activity, finished operations and webhook deliveries kept; at least 30.
    pub history_days: u16,
    /// Hoista put on a server from here, if any.
    pub server: Option<ServerRecord>,
}

/// A Hoista on a server: one this computer put there, or one set up
/// elsewhere (with `hoist` on a server) that this computer connected to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ServerRecord {
    /// The provider it runs on; none for one connected to by its address.
    pub provider: Option<Provider>,
    /// The account it was made with; none for one connected to.
    pub account_id: Option<String>,
    /// `https://host`.
    pub url: String,
    /// The provider's reference for its service; none for one connected to.
    pub reference: Option<String>,
    /// The published image it runs, pinned by digest; none when built from a repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub image: Option<String>,
    /// RFC 3339 UTC.
    pub created_at: String,
}

/// Where a vault's entries are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum VaultPlace {
    /// This computer's keychain (macOS Keychain, Windows Credential Manager,
    /// the Secret Service on Linux).
    Keychain,
    /// The remote's sealed file, opened by its credential keeper.
    SealedFile,
}

/// `GET /vault`: where provider tokens are kept, as the vault in use says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct VaultInfo {
    /// Where entries are kept.
    pub kept_in: VaultPlace,
    /// Where each provider token itself lives (`built-in`: in that place).
    pub token_store: String,
    /// Where the key that seals the file comes from, on a remote.
    #[ts(optional)]
    pub key_source: Option<String>,
    /// A token may be a 1Password reference (`op://…`), read when needed.
    pub references: bool,
}

/// Everything an adapter can do. Clients and agents read these to decide what
/// to show and offer; nothing checks a provider by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// Deploys project code.
    pub deploy: bool,
    /// Reads logs.
    pub logs: bool,
    /// Reads metrics.
    pub metrics: bool,
    /// Reads daily usage.
    pub usage: bool,
    /// Rolls back to an earlier release.
    pub rollback: bool,
    /// Pauses and resumes resources.
    pub pause: bool,
    /// Manages custom domains.
    pub domains: bool,
    /// Sets environment variables and secrets.
    pub secrets: bool,
    /// Lists existing resources in the account.
    pub discover: bool,
    /// Can run Hoista itself as a remote, from the published image.
    pub server: bool,
    /// Reads a watched resource's settings, deployments and addresses as the
    /// provider keeps them (the provider tab).
    pub provider_details: bool,
    /// Lists the provider's deployments of a watched resource, and redeploys or
    /// rolls back through the provider's own actions.
    pub provider_deploys: bool,
    /// Reads and changes a watched resource's variables at the provider.
    pub provider_variables: bool,
    /// Runs the provider's own API operations by name (see `provider_operations`).
    pub provider_operations: bool,
    /// Uploads a folder for the provider to build and deploy.
    pub upload: bool,
    /// Creates a new project at the provider from a repository.
    pub new_project: bool,
    /// Creates a new project at the provider from a folder on this computer,
    /// with the provider's own command-line tool.
    pub new_from_folder: bool,
    /// Creates a database of its own at the provider (a Postgres project, or a
    /// database next to a project's services).
    pub new_database: bool,
    /// Creates an object storage bucket at the provider.
    pub new_bucket: bool,
    /// Adds a domain to send email from.
    pub new_email_domain: bool,
    /// Runs a container image as a new resource (a service, or a GPU pod).
    pub new_from_image: bool,
    /// Reports what the provider has billed so far this period.
    pub bills: bool,
    /// Connects by signing in on the provider's own page, instead of a pasted token.
    pub browser_sign_in: bool,
    /// Deletes a resource at the provider (a provider project, service, Worker, bucket or
    /// domain), when the person confirms it by name.
    pub delete: bool,
    /// Hoista builds a project folder and deploys it here from a plan
    /// (`plan_deploy`).
    pub plan_deploy: bool,
    /// Runs what Hoista deploys as a long-running container built from the
    /// project's Dockerfile, which Hoista writes for a Node project without one.
    pub containers: bool,
    /// Runs a deployed web or job resource on exact UTC cron schedules.
    pub cron: bool,
    /// Where Hoista creates a project's Postgres when its plan needs one.
    pub project_database: bool,
    /// Creates the DNS zone for a domain it adds, when asked to.
    pub dns_zones: bool,
}

impl Capabilities {
    /// An adapter that can do nothing; provider entries list what they add to it.
    pub const NONE: Self = Self {
        deploy: false,
        logs: false,
        metrics: false,
        usage: false,
        rollback: false,
        pause: false,
        domains: false,
        secrets: false,
        discover: false,
        server: false,
        provider_details: false,
        provider_deploys: false,
        provider_variables: false,
        provider_operations: false,
        upload: false,
        new_project: false,
        new_from_folder: false,
        new_database: false,
        new_bucket: false,
        new_email_domain: false,
        new_from_image: false,
        bills: false,
        browser_sign_in: false,
        delete: false,
        plan_deploy: false,
        containers: false,
        cron: false,
        project_database: false,
        dns_zones: false,
    };
}

/// Editable planned resource allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourcePlan {
    /// Cron schedules (UTC) to set. Null leaves the provider's alone; an empty list removes them.
    pub schedules: Option<Vec<String>>,
    /// The resource it deploys over, when there is one. Hoista fills this in.
    pub provider_ref: Option<String>,
    /// Resource name.
    pub name: String,
    /// Resource type.
    pub kind: ResourceKind,
    /// Selected provider.
    pub provider: Provider,
    /// Connected account to use.
    pub account_id: String,
    /// Requested location.
    pub region: Option<String>,
    /// Names of resources released first.
    pub depends_on: Vec<String>,
}

/// Checked-in Prisma migrations applied before releasing application code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PrismaMigration {
    /// Relative schema file inside the source snapshot.
    pub schema: String,
    /// Relative directory containing migration history.
    pub migrations: String,
}

impl PrismaMigration {
    /// Whether both paths are plain relative paths in the project, with the
    /// migrations folder next to the schema.
    pub fn valid(&self) -> bool {
        let valid = |value: &str| {
            value.len() <= 256
                && !value.is_empty()
                && value.split('/').all(|segment| {
                    !segment.is_empty()
                        && segment != "."
                        && segment != ".."
                        && segment
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
                })
        };
        let parent = self
            .schema
            .rsplit_once('/')
            .map(|(parent, _)| format!("{parent}/migrations"))
            .unwrap_or_else(|| "migrations".into());
        valid(&self.schema)
            && valid(&self.migrations)
            && self.schema.ends_with(".prisma")
            && self.migrations == parent
    }
}

/// A detected, editable deployment plan without secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Goes up with every edit. Send back the one you read; an edit made on an older one is refused.
    #[serde(default)]
    #[ts(type = "number")]
    pub revision: i64,
    /// Plan id.
    pub id: String,
    /// Source analyzed.
    pub source: Source,
    /// Detected framework.
    pub framework: String,
    /// Detected package manager.
    pub package_manager: Option<String>,
    /// Executable and arguments; never evaluated by a shell.
    pub build_command: Vec<String>,
    /// Relative directory containing public assets.
    pub output_directory: String,
    /// Required environment names only.
    pub env_names: Vec<String>,
    /// Authentication names generated in memory for new resources when no value was supplied.
    pub generated_secret_names: Vec<String>,
    /// Optional checked-in database migration workflow; never run by code rollback.
    pub prisma_migration: Option<PrismaMigration>,
    /// Planned resources in dependency order.
    pub resources: Vec<ResourcePlan>,
    /// Concrete deployment constraints.
    pub warnings: Vec<String>,
}

/// A project source; repository sources are pinned to a commit before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum Source {
    /// Imported provider resources whose deployment source has not been linked locally.
    Unlinked,
    /// A locally authorized source directory.
    Folder {
        /// Absolute path to the project source.
        path: String,
    },
    /// A GitHub repository at one commit.
    Github {
        /// Owner/repository, without credentials or a URL query.
        repository: String,
        /// Full commit SHA.
        commit: String,
    },
}

impl Source {
    /// Whether code from `self` belongs to a project whose source is
    /// `project`: the same source, or a folder inside the project's folder
    /// (one app of a monorepo). Compared by path, without the file system.
    #[must_use]
    pub fn within(&self, project: &Source) -> bool {
        match (self, project) {
            (Source::Folder { path }, Source::Folder { path: root }) => {
                std::path::Path::new(&same_folder(path))
                    .starts_with(std::path::Path::new(&same_folder(root)))
            }
            _ => self == project,
        }
    }
}

/// A folder path as macOS resolves it: /tmp and /var are links into /private.
fn same_folder(path: &str) -> String {
    let path = path.trim_end_matches('/');
    for linked in ["/private/tmp", "/private/var"] {
        if let Some(rest) = path.strip_prefix(linked)
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return format!("{}{rest}", &linked["/private".len()..]);
        }
    }
    path.to_owned()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            agents_enabled: true,
            idle_days: 21,
            agent_expiry_days: 30,
            check_updates: false,
            crash_reports: false,
            network_page: false,
            mcp_http: true,
            default_access: "ask".into(),
            unisolated_builds: false,
            backup_every_hours: 24,
            backup_keep: 14,
            history_days: 365,
            server: None,
        }
    }
}

/// A GitHub user or organization name: letters, digits and dashes, at most 39.
#[must_use]
pub fn github_login(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 39
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !text.starts_with('-')
}

/// `owner/name`, with the characters GitHub allows in a repository's name.
#[must_use]
pub fn github_repository(text: &str) -> bool {
    text.split_once('/').is_some_and(|(owner, name)| {
        github_login(owner)
            && !name.is_empty()
            && name.len() <= 100
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && name != "."
            && name != ".."
    })
}

/// Validates a provider-safe resource slug without interpolating arbitrary paths.
pub fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}

/// Validates environment names without permitting shell or URL syntax.
pub fn valid_env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !value.as_bytes()[0].is_ascii_digit()
}

/// What a provider released, enough to roll back to it. Nothing secret.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
    PartialEq,
    Eq,
)]
pub struct Release {
    /// The cron schedules to put back with this release; null leaves the current ones alone.
    pub schedules: Option<Vec<String>>,
    /// Provider resource reference.
    pub reference: String,
    /// The provider's version id or image digest, which never changes.
    pub version: String,
    /// Public URL, if applicable.
    pub url: Option<String>,
}

/// Provider release associated with a local project resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ResourceRelease {
    /// Local resource identifier.
    pub resource_id: String,
    /// Planned resource name.
    pub name: String,
    /// What the provider released.
    pub release: Release,
}

/// A resource family that could not be inspected during discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryWarning {
    /// Provider resource family, such as workers or pages.
    pub resource_type: String,
    /// Stable machine-readable failure code.
    pub code: String,
    /// Safe explanation without provider response bodies.
    pub message: String,
}

/// How the last look for an account's resources went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DiscoverySummary {
    /// Completion time of this scan attempt.
    pub observed_at: String,
    /// Whether all supported inventory families completed successfully.
    pub complete: bool,
    /// Number observed in this scan; null if it did not finish.
    pub observed_resources: Option<u32>,
    /// Coverage gaps or a generic failed-scan explanation.
    pub warnings: Vec<DiscoveryWarning>,
}

/// What a look for an account's resources found, and what it couldn't look at.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryResult {
    /// Resources observed and saved locally without changing provider resources.
    pub items: Vec<Resource>,
    /// True only when every supported discovery family was inspected successfully.
    pub complete: bool,
    /// Inaccessible families; absence is not proof that those resources do not exist.
    pub warnings: Vec<DiscoveryWarning>,
}

/// Whether `scopes` is a valid set of agent scopes: each of `apps:read`,
/// `apps:write` and `apps:create` at most once.
pub fn valid_agent_scopes(scopes: &[String]) -> bool {
    !scopes.is_empty()
        && scopes.len() <= 3
        && scopes
            .iter()
            .all(|scope| matches!(scope.as_str(), "apps:read" | "apps:write" | "apps:create"))
        && scopes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == scopes.len()
}

impl Agent {
    /// Checks current scopes and inactivity expiry without modifying the identity.
    pub fn permits(&self, scope: &str, at: &str, expiry_days: u16) -> bool {
        let (Ok(seen), Ok(at)) = (
            chrono::DateTime::parse_from_rfc3339(&self.last_seen),
            chrono::DateTime::parse_from_rfc3339(at),
        ) else {
            return false;
        };
        !self.revoked
            && !self.grant.expired(&at.to_rfc3339())
            && self.scopes.iter().any(|value| value == scope)
            && at.signed_duration_since(seen).num_days() < i64::from(expiry_days)
    }
}

/// Local-person settings changes; omitted fields retain their current values.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    /// Allow or temporarily pause agent connections and requests.
    pub agents_enabled: Option<bool>,
    /// Idle suggestion threshold in days.
    pub idle_days: Option<u16>,
    /// Inactivity expiry in days.
    pub agent_expiry_days: Option<u16>,
    /// Let Hoista check for new versions.
    pub check_updates: Option<bool>,
    /// Allow local crash-report preparation.
    pub crash_reports: Option<bool>,
    /// Open the consent page on the private network address too.
    pub network_page: Option<bool>,
    /// Serve MCP over HTTP on this computer.
    pub mcp_http: Option<bool>,
    /// The access the consent page starts on: `view`, `ask` or `full`.
    pub default_access: Option<String>,
    /// Build projects even where they can't be isolated.
    pub unisolated_builds: Option<bool>,
    /// Hours between automatic backups: 0, 6, 24 or 168.
    pub backup_every_hours: Option<u16>,
    /// How many automatic backups to keep, 1 to 100.
    pub backup_keep: Option<u16>,
    /// Days of history kept: 30, 90, 365 or 1095.
    pub history_days: Option<u16>,
}

/// An account a token can reach, offered for the person to choose. Not connected yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct AccountChoice {
    /// Provider account ID or organization slug consumed by the adapter.
    pub external_id: String,
    /// Its name at the provider, checked before it's shown.
    pub name: String,
}

/// Where a change Hoista asked a provider to make stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    /// Sent; Hoista is waiting for the provider.
    Running,
    /// The provider did it.
    Done,
    /// The provider refused it; nothing changed.
    Failed,
    /// Hoista didn't hear back, or it stopped partway: it may have happened.
    /// Changes that would repeat something wait until the person checks.
    Unclear,
    /// The person checked at the provider: it happened.
    Happened,
    /// The person checked at the provider: it didn't happen.
    DidNotHappen,
}

/// One change Hoista asked a provider to make, written down before it was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    /// Its id, for checking it.
    pub id: String,
    /// The provider it was sent to.
    pub provider: Provider,
    /// The provider account's id.
    pub account: String,
    /// What kind of change, such as `create` or `set_variable`.
    pub kind: String,
    /// What it was for, in plain words, with no secret values.
    pub target: String,
    /// Where it stands.
    pub state: OperationState,
    /// Whether repeating it could make a second one; such changes wait while
    /// an identical one is unclear.
    pub repeats: bool,
    /// RFC 3339 UTC.
    pub started_at: String,
    /// RFC 3339 UTC; when it finished or was checked.
    pub finished_at: Option<String>,
    /// What the provider said when it didn't work, redacted.
    pub message: Option<String>,
    /// Digest of what the change is; identical changes share it.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    #[ts(skip)]
    #[schemars(skip)]
    pub key: String,
}

/// Where a background agent hears about what happens in Hoista.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct Webhook {
    /// Its id.
    pub id: String,
    /// `https://…`, a public address.
    pub url: String,
    /// Event kinds it gets, such as `deploy.succeeded`, or a family ending in
    /// `.*` such as `deploy.*`. Empty means every kind.
    pub events: Vec<String>,
    /// Only this project's events, when set.
    pub project_id: Option<String>,
    /// RFC 3339 UTC.
    pub created_at: String,
}

/// Where one event's delivery to one webhook stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// Waiting for its next try.
    Pending,
    /// The receiver answered 2xx.
    Delivered,
    /// Gave up after six tries; kept as history.
    Failed,
}

/// One event sent, or to be sent, to one webhook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct WebhookDelivery {
    /// Its id.
    pub id: String,
    /// The webhook.
    pub webhook_id: String,
    /// The event, as in `GET /events`.
    pub event_id: String,
    /// The event's kind.
    pub event_kind: String,
    /// Where it stands.
    pub state: DeliveryState,
    /// Tries so far.
    pub attempts: u32,
    /// RFC 3339 UTC: the next try, while pending.
    pub next_attempt_at: String,
    /// The receiver's last HTTP status, if it answered.
    pub last_status: Option<u16>,
    /// Why the last try didn't work, in plain words.
    pub last_error: Option<String>,
    /// The signed body, exactly as sent each time.
    #[ts(skip)]
    #[schemars(skip)]
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
}
