//! The provider adapter interface, and the types its operations take and return.

use crate::ports::catalog::EngineError;
use crate::{
    Provider,
    domain::{Secret, model::*},
};

/// A project at the provider with nothing in it yet (no service, database or
/// bucket), which discovery has nothing to show for.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ProviderEmptyProject {
    /// The provider's id for it.
    pub id: String,
    /// Its name at the provider.
    pub name: String,
    /// Its page in the provider's dashboard, when the provider has one.
    pub url: Option<String>,
}

/// What a provider reports about one watched resource: its own deployment
/// history, the names of its variables and its domains. Read live, never
/// stored; variable values are never read into this.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ProviderDetails {
    /// Newest first, at most 20.
    pub deployments: Vec<ProviderDeployment>,
    /// Variable names only, sorted. Null when the provider does not report them.
    pub variable_names: Option<Vec<String>>,
    /// Names of the provider project's shared variables in this environment, never
    /// their values; null when the provider doesn't say.
    pub shared_variable_names: Option<Vec<String>>,
    /// Addresses the provider serves this resource on.
    pub domains: Vec<ProviderDomain>,
    /// Variables that point at another service on the same provider (Railway's
    /// `${{Postgres.DATABASE_URL}}`): how this resource connects to others.
    pub references: Vec<ProviderReference>,
    /// Whether the provider puts it to sleep when nobody uses it (Railway's
    /// Serverless setting). Null when the provider doesn't say.
    pub sleeps: Option<bool>,
    /// How the provider builds and runs it, as far as the provider says.
    pub config: Option<ProviderConfig>,
    /// Disks attached to it.
    pub volumes: Vec<ProviderVolume>,
    /// Public TCP endpoints (for databases and non-HTTP services).
    pub tcp_proxies: Vec<ProviderTcpProxy>,
    /// The name other services in the same environment reach it by, for
    /// example Railway's `postgres.railway.internal`. Null when the provider
    /// doesn't say or it has none.
    pub private_domain: Option<String>,
    /// Where its variables point outside this provider: the host name (and the
    /// user name, when the address has one) of each address a variable holds,
    /// such as another provider's database. Never the value itself, a password
    /// or anything after the host.
    pub addresses: Vec<VariableAddress>,
}

/// One address a variable points at, as far as it shows where, never what
/// it holds: `DATABASE_URL` → `aws-0-eu-west-3.pooler.supabase.com`, user
/// `postgres.abcd`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct VariableAddress {
    /// The variable's name, such as `SUPABASE_URL`.
    pub variable: String,
    /// The host name in the address.
    pub host: String,
    /// The user name in the address, when it has one.
    pub user: Option<String>,
}

impl VariableAddress {
    /// The addresses in one variable's value: `scheme://user:password@host/...`
    /// gives its host and user; anything that isn't such an address gives none.
    #[must_use]
    pub fn from_value(variable: &str, value: &str) -> Option<Self> {
        let value = value.trim();
        if value.len() > 2048 || !value.contains("://") || value.contains("${{") {
            return None;
        }
        let url = url::Url::parse(value).ok()?;
        let host = url.host_str()?.to_ascii_lowercase();
        if !host.contains('.') {
            return None;
        }
        let user = (!url.username().is_empty()).then(|| url.username().to_owned());
        Some(Self {
            variable: variable.to_owned(),
            host,
            user,
        })
    }
}

/// How a provider builds and runs a resource right now. Null fields are unset.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderConfig {
    /// The repository it deploys from, `owner/name`.
    pub repo: Option<String>,
    /// The image it runs, when it runs an image.
    pub image: Option<String>,
    /// The command that starts it.
    pub start_command: Option<String>,
    /// The command that builds it.
    pub build_command: Option<String>,
    /// The folder inside the repository it builds from.
    pub root_directory: Option<String>,
    /// The cron schedule it runs on, when it is a job.
    pub cron_schedule: Option<String>,
    /// The path checked before traffic is sent to it.
    pub healthcheck_path: Option<String>,
    /// How many copies run.
    pub replicas: Option<u32>,
    /// The region it runs in.
    pub region: Option<String>,
    /// When it restarts after exiting, as the provider names it.
    pub restart_policy: Option<String>,
}

/// A disk attached to a resource.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderVolume {
    /// The provider's id, for removing it.
    pub id: String,
    /// Its name at the provider.
    pub name: String,
    /// Where it is mounted, such as `/data`.
    pub mount_path: Option<String>,
    /// The provider's id for this volume on this resource, which backups use.
    pub instance_id: Option<String>,
    /// Space used now, in MB, when the provider reports it.
    pub used_mb: Option<u64>,
    /// Space it has, in MB, when the provider reports it.
    pub size_mb: Option<u64>,
    /// Where it is, such as `europe-west4`.
    pub region: Option<String>,
}

/// A public TCP endpoint in front of a resource.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderTcpProxy {
    /// The provider's id, for removing it.
    pub id: String,
    /// Connect to `domain:proxy_port`.
    pub domain: String,
    /// The public port.
    pub proxy_port: u32,
    /// The port it listens on.
    pub application_port: u32,
}

/// One variable that refers to another service on the same provider.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ProviderReference {
    /// The variable's name, such as `DATABASE_URL`.
    pub variable: String,
    /// The service it refers to, by its name on the provider.
    pub service: String,
}

/// What a person or agent can ask a provider to do with a watched resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAction<'a> {
    /// Build and start the current version again.
    Redeploy,
    /// Restart the running deployment without rebuilding.
    Restart,
    /// Serve an earlier deployment again.
    Rollback {
        /// The provider's id of that deployment.
        deployment_id: &'a str,
    },
    /// Deploy the newest commit of the linked repository.
    DeployLatest,
    /// Let the provider put it to sleep when nobody uses it, or keep it awake.
    Sleep {
        /// True to let it sleep.
        on: bool,
    },
    /// Stop the running deployment (it stays stopped until the next deploy).
    Stop,
    /// Deploy one commit of the linked repository.
    DeployCommit {
        /// The full commit hash.
        sha: &'a str,
    },
    /// Change how the provider builds and runs it.
    Configure(&'a ServiceConfig),
    /// Deploy from a GitHub repository from now on.
    ConnectRepo {
        /// `owner/name`.
        repo: &'a str,
        /// The branch to deploy; the repository's default when absent.
        branch: Option<&'a str>,
    },
    /// Give it an address: a free provider address, or your own domain.
    AddDomain {
        /// Your own domain, such as `api.example.com`; absent for a free one.
        domain: Option<&'a str>,
        /// The port it listens on, when it isn't the provider's default.
        port: Option<u16>,
    },
    /// Take an address away.
    RemoveDomain {
        /// The address, as listed in the resource's provider details.
        domain: &'a str,
    },
    /// Change the port an address sends traffic to.
    UpdateDomain {
        /// The address, as listed in the resource's provider details.
        domain: &'a str,
        /// The port it listens on.
        port: u16,
    },
    /// Give the provider's own address (`*.up.railway.app`) a new name.
    RenameDomain {
        /// The address, as listed in the resource's provider details.
        domain: &'a str,
        /// The new address, on the same host suffix.
        to: &'a str,
    },
    /// Ask the provider to try issuing a custom domain's certificate again.
    RetryCertificate {
        /// The custom domain.
        domain: &'a str,
    },
    /// Stop deploying from its repository or image.
    DisconnectSource,
    /// Attach a disk mounted at a path, such as `/data`.
    AddVolume {
        /// Where to mount it.
        mount_path: &'a str,
    },
    /// Delete a disk and everything on it.
    RemoveVolume {
        /// The volume's id from provider details.
        volume_id: &'a str,
    },
    /// Open a public TCP endpoint to a port it listens on.
    AddTcpProxy {
        /// The port it listens on.
        port: u16,
    },
    /// Close a public TCP endpoint.
    RemoveTcpProxy {
        /// The proxy's id from provider details.
        proxy_id: &'a str,
    },
    /// Rename its private network address (`name.railway.internal`).
    RenamePrivateHost {
        /// The short name, without `.railway.internal`.
        name: &'a str,
    },
    /// Delete the resource at the provider.
    RemoveService,
}

/// Something to create next to a project's existing resources at their provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCreate<'a> {
    /// A new service, empty or from a repository or an image.
    Service {
        /// Its name at the provider.
        name: &'a str,
        /// `owner/name` to deploy from.
        repo: Option<&'a str>,
        /// The branch, with a repository.
        branch: Option<&'a str>,
        /// A container image to run instead, such as `nginx:latest`.
        image: Option<&'a str>,
    },
    /// A ready-made template, such as a database: `postgres`, `mysql`,
    /// `redis` or `mongo`, or any template code the provider lists.
    Template {
        /// The template's code.
        code: &'a str,
        /// Values for the template's variables, by name.
        variables: &'a std::collections::BTreeMap<String, String>,
    },
    /// An object storage bucket.
    Bucket {
        /// Its name.
        name: &'a str,
    },
}

/// Something new to make at a provider and add to a Hoista project. Which kinds a
/// provider takes follows its capabilities: `new_project` for a repository,
/// `new_from_folder` for a folder, `new_database`, `new_bucket`,
/// `new_email_domain` and `new_from_image`.
#[derive(Debug, Clone, Copy)]
pub enum ProviderNew<'a> {
    /// Code the provider builds from a GitHub repository (`owner/name`).
    Repo {
        /// Its name at the provider.
        name: &'a str,
        /// `owner/name`.
        repo: &'a str,
        /// The branch; the repository's default when `None`.
        branch: Option<&'a str>,
        /// What it runs as, where the provider offers more than one kind: `web`
        /// (the default), `static`, `worker` or `cron`.
        runs_as: Option<&'a str>,
        /// For `cron`: when it runs, as a five-field UTC cron expression.
        schedule: Option<&'a str>,
        /// How to build and start it, for a provider that asks rather than
        /// detecting it: the language (`node`, `python`, `go`, `ruby`,
        /// `rust`, `elixir` or `docker`), build and start commands, and for
        /// `static`, the folder the build writes the site to.
        build: RepoBuild<'a>,
    },
    /// Code deployed from a folder on this computer with the provider's own tool.
    Folder {
        /// Its name at the provider.
        name: &'a str,
        /// The folder, already checked to exist.
        folder: &'a std::path::Path,
    },
    /// A database.
    Database {
        /// Its name at the provider.
        name: &'a str,
        /// `postgres`, `mysql`, `redis` or `mongo`, as the provider offers them.
        engine: &'a str,
        /// The region to make it in, in the provider's own words; the provider's
        /// default when `None`.
        region: Option<&'a str>,
    },
    /// An object storage bucket.
    Bucket {
        /// Its name.
        name: &'a str,
    },
    /// A domain to send email from, such as `mail.example.com`.
    EmailDomain {
        /// The domain.
        domain: &'a str,
    },
    /// A container image to run.
    Image {
        /// Its name at the provider.
        name: &'a str,
        /// Such as `nginx:latest` or `vllm/vllm-openai:latest`.
        image: &'a str,
        /// A GPU type, in the provider's own words, where the provider runs GPUs.
        gpu: Option<&'a str>,
        /// A CPU-only machine instead of a GPU, where the provider offers both.
        cpu: Option<CpuMachine<'a>>,
        /// The port it listens on, to make public.
        port: Option<u16>,
        /// The command line after the image's entrypoint, when it needs one.
        args: Option<&'a str>,
    },
}

/// A CPU-only machine for an image, in the provider's own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuMachine<'a> {
    /// The CPU type, such as Runpod's `cpu3c`.
    pub flavor: &'a str,
    /// How many vCPUs.
    pub vcpus: u32,
}

/// How a provider builds and starts code from a repository, where it asks.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoBuild<'a> {
    /// `node`, `python`, `go`, `ruby`, `rust`, `elixir` or `docker`.
    pub runtime: Option<&'a str>,
    /// The build command, such as `npm ci && npm run build`.
    pub build: Option<&'a str>,
    /// The start command, such as `npm start`.
    pub start: Option<&'a str>,
    /// For a static site: the folder the build writes, such as `dist`.
    pub publish: Option<&'a str>,
}

impl ProviderNew<'_> {
    /// The capability a provider claims to take this kind.
    #[must_use]
    pub const fn capability(&self) -> &'static str {
        match self {
            Self::Repo { .. } => "new_project",
            Self::Folder { .. } => "new_from_folder",
            Self::Database { .. } => "new_database",
            Self::Bucket { .. } => "new_bucket",
            Self::EmailDomain { .. } => "new_email_domain",
            Self::Image { .. } => "new_from_image",
        }
    }

    /// Whether the provider claims it.
    #[must_use]
    pub const fn claimed_by(&self, can: &crate::domain::model::Capabilities) -> bool {
        match self {
            Self::Repo { .. } => can.new_project,
            Self::Folder { .. } => can.new_from_folder,
            Self::Database { .. } => can.new_database,
            Self::Bucket { .. } => can.new_bucket,
            Self::EmailDomain { .. } => can.new_email_domain,
            Self::Image { .. } => can.new_from_image,
        }
    }

    /// Whether what it makes runs code, so its first deploy is under way.
    #[must_use]
    pub const fn runs_code(&self) -> bool {
        matches!(
            self,
            Self::Repo { .. } | Self::Folder { .. } | Self::Image { .. }
        )
    }
}

/// How a provider builds and runs a resource. Only the fields given are changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// The command that starts it, such as `npm start`.
    pub start_command: Option<String>,
    /// The command that builds it, such as `npm run build`.
    pub build_command: Option<String>,
    /// The folder inside the repository it builds from, such as `/backend`.
    pub root_directory: Option<String>,
    /// A cron schedule to run it on instead of keeping it up, such as `0 * * * *`.
    pub cron_schedule: Option<String>,
    /// The path the provider checks before sending traffic, such as `/health`.
    pub healthcheck_path: Option<String>,
    /// How many copies run, 1 to 20.
    pub replicas: Option<u8>,
    /// The region it runs in, such as `us-west2`.
    pub region: Option<String>,
    /// When it restarts after exiting: `on_failure`, `always` or `never`.
    pub restart_policy: Option<RestartPolicy>,
}

/// When a provider restarts a resource that exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Only after a crash.
    OnFailure,
    /// Whenever it exits.
    Always,
    /// Never.
    Never,
}

/// One deployment as the provider recorded it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ProviderDeployment {
    /// Provider deployment id.
    pub id: String,
    /// The provider's own status word, such as `SUCCESS` or `CRASHED`.
    pub status: String,
    /// Hoista's reading of that status.
    pub state: ProviderDeployState,
    /// RFC 3339 creation time.
    pub created_at: String,
    /// Public URL of this deployment, when the provider gives one.
    pub url: Option<String>,
    /// Commit hash, when deployed from a repository.
    pub commit: Option<String>,
    /// Commit message or deploy note.
    pub message: Option<String>,
    /// Branch, when deployed from a repository.
    pub branch: Option<String>,
    /// Repository, such as `owner/name`, when deployed from one.
    pub repo: Option<String>,
    /// RFC 3339 time its build finished, live or failed; null while it is
    /// still building, and when the provider can't say (a later status, such as
    /// being replaced, moves the provider's own timestamp).
    pub finished_at: Option<String>,
    /// Whether the provider says it can go back to this deployment; null when the
    /// provider doesn't say.
    pub can_roll_back: Option<bool>,
    /// Who made the change it deploys, as the provider records it (the commit's
    /// author); null when the provider doesn't say.
    pub author: Option<String>,
    /// The deployment's own page in the provider's dashboard, when the provider
    /// gives one or its address can be built from the ids.
    pub provider_page: Option<String>,
}

/// How much a provider operation can change.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRisk {
    /// Only reads.
    Read,
    /// Changes something that can be changed back.
    Change,
    /// Removes, restores over, or resets something: needs the person's approval.
    Destroy,
}

/// One argument of a provider operation.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
pub struct ProviderOperationArg {
    /// Argument name, as the provider spells it.
    pub name: String,
    /// Whether the caller must give it (after Hoista fills in what it knows).
    pub required: bool,
    /// What Hoista fills in itself when it is left out: `project`,
    /// `environment`, `service` or `workspace` of the resource it runs on.
    pub filled_in: Option<String>,
    /// JSON schema of the value.
    #[ts(type = "unknown")]
    pub schema: serde_json::Value,
}

/// A button the project shows for a provider's resource: one operation from the
/// provider's list, with its arguments set. From the provider's manifest.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
pub struct ProviderQuickAction {
    /// The button's words, such as `Apply migrations`.
    pub label: String,
    /// The kind of resource it's for; none for every kind.
    pub on: Option<crate::domain::model::ResourceKind>,
    /// The operation it runs, from the provider's operation list.
    pub operation: String,
    /// What it does, in a sentence.
    pub description: String,
    /// Its arguments.
    #[ts(type = "Record<string, unknown>")]
    pub args: serde_json::Value,
    /// Arguments for a first run that changes nothing (a dry run), where
    /// the operation has one; shown before the real run.
    #[ts(type = "Record<string, unknown> | null")]
    pub try_first: Option<serde_json::Value>,
    /// Only for resources whose provider reference starts with one of these
    /// (`pod:` on Runpod); every resource of its kind when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

/// Something a provider can do, from its full operation list.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
pub struct ProviderOperationInfo {
    /// Operation name, as the provider spells it (Railway: the GraphQL field).
    pub name: String,
    /// Which part of the provider it belongs to, such as `Deploys` or `Storage, backups and databases`.
    pub area: String,
    /// Read, change, or destroy (needs approval).
    pub risk: ProviderRisk,
    /// What it is in the provider's own terms: the HTTP method (`GET`, `DELETE`)
    /// of a REST call, `Query` or `Mutation` for GraphQL, `Command` for its CLI.
    pub request: String,
    /// What it does.
    pub description: String,
    /// Its arguments.
    pub args: Vec<ProviderOperationArg>,
}

/// What a provider has billed an account so far in its current billing period,
/// as the provider reports it.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderBill {
    /// Billed so far this period.
    pub billed: crate::domain::model::Money,
    /// RFC 3339 start of the billing period, when the provider says.
    pub period_start: Option<String>,
    /// RFC 3339 end of the billing period, when the provider says.
    pub period_end: Option<String>,
    /// The provider's soft usage limit (a warning), if set.
    pub soft_limit: Option<crate::domain::model::Money>,
    /// The provider's hard usage limit (services stop), if set.
    pub hard_limit: Option<crate::domain::model::Money>,
    /// Whether the account is over its limit.
    pub over_limit: bool,
    /// The provider's id for the paying customer, needed to change its usage
    /// limits (Railway's `customer.id`). An id, not a secret.
    pub customer_id: Option<String>,
}

/// One measure of plan usage, as the provider counts it: `1,240 of 3,000
/// emails this month`.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
pub struct ProviderPlanUsage {
    /// What is counted, in the provider's words, such as `Emails this month`.
    pub label: String,
    /// How much was used.
    pub used: f64,
    /// The plan's limit, when it has one.
    pub limit: Option<f64>,
    /// The unit, such as `emails`, `hours` or `GB`.
    pub unit: String,
}

/// What a resource's address served in a time range, from the provider's
/// request logs.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderTraffic {
    /// Start of the range, RFC 3339.
    pub from: String,
    /// End of the range, RFC 3339.
    pub to: String,
    /// Requests served. Null when there were more than Hoista reads in one
    /// pass, so a partial count is never shown as the total.
    pub requests: Option<u32>,
    /// Responses with a 4xx status, counted the same way.
    pub client_errors: Option<u32>,
    /// Responses with a 5xx status, counted the same way.
    pub server_errors: Option<u32>,
    /// Median response time in milliseconds, of the requests read.
    pub p50_ms: Option<u32>,
    /// 95th percentile response time in milliseconds.
    pub p95_ms: Option<u32>,
    /// 99th percentile response time in milliseconds.
    pub p99_ms: Option<u32>,
    /// The most frequent failing requests, most frequent first.
    pub top_errors: Vec<ProviderErrorCount>,
    /// The newest requests, newest first, at most 200.
    pub recent: Vec<ProviderRequest>,
}

/// A resource's numbers over time, as its provider measures them: one chart per
/// thing the provider reports, each with one or more lines.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderMetrics {
    /// Start of the range, RFC 3339.
    pub from: String,
    /// End of the range, RFC 3339.
    pub to: String,
    /// The charts, in the order to show them.
    pub charts: Vec<ProviderChart>,
    /// What the provider couldn't answer, in a sentence, such as a chart its
    /// plan doesn't include. Null when everything was read.
    pub note: Option<String>,
}

/// One chart of a resource's metrics.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
pub struct ProviderChart {
    /// A stable id for the chart, such as `requests`.
    pub id: String,
    /// Its title, such as "Requests".
    pub label: String,
    /// What the values measure.
    pub unit: MetricUnit,
    /// How the chart's headline number is worked out from its points.
    pub summary: MetricSummary,
    /// The lines, at most a handful.
    pub series: Vec<ProviderSeries>,
}

/// What a chart's values measure.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnit {
    /// How many: requests, emails, invocations.
    Count,
    /// Milliseconds.
    Milliseconds,
    /// A percentage, 0 to 100.
    Percent,
    /// Bytes.
    Bytes,
    /// CPU cores in use (1 is one full core).
    Cores,
    /// Seconds, such as compute time used.
    Seconds,
}

/// How a chart's headline number comes from its points.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum MetricSummary {
    /// Added up over the range, for counts.
    Total,
    /// The average over the range, for usage and latency.
    Average,
    /// The newest point, for levels such as disk used.
    Latest,
}

/// One line on a chart.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
pub struct ProviderSeries {
    /// What the line is, such as "5xx" or "p95".
    pub name: String,
    /// Its points, oldest first.
    pub points: Vec<ProviderPoint>,
}

/// One value at one time.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
pub struct ProviderPoint {
    /// When, RFC 3339.
    pub at: String,
    /// The value, in the chart's unit.
    pub value: f64,
}

/// One repository a code provider's account can reach.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderRepository {
    /// `owner/name`.
    pub full_name: String,
    /// The branch it builds from by default, such as `main`.
    pub default_branch: String,
    /// Only people given access can see it.
    pub private: bool,
    /// When something was last pushed to it, RFC 3339.
    pub pushed_at: Option<String>,
    /// Its page.
    pub url: String,
    /// Its one-line description, when it has one.
    pub description: Option<String>,
}

/// How often one request failed with one status.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderErrorCount {
    /// The HTTP status.
    pub status: u16,
    /// The HTTP method.
    pub method: String,
    /// The path, without its query string.
    pub path: String,
    /// How many times.
    pub count: u32,
}

/// One request a resource's address served.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct ProviderRequest {
    /// RFC 3339 time.
    pub at: String,
    /// The HTTP method.
    pub method: String,
    /// The path, without its query string (it may carry secrets).
    pub path: String,
    /// The HTTP status.
    pub status: u16,
    /// Total time in milliseconds.
    pub duration_ms: Option<u32>,
    /// The provider's request id, for support.
    pub request_id: String,
}

/// A provider deployment status in Hoista's terms.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDeployState {
    /// Serving, or served and since replaced.
    Succeeded,
    /// Failed or crashed.
    Failed,
    /// Queued, building or deploying.
    InProgress,
    /// Removed or skipped.
    Stopped,
}

/// An address a provider serves a resource on.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ProviderDomain {
    /// Hostname.
    pub name: String,
    /// True for the person's own domain, false for the provider's generated one.
    pub custom: bool,
    /// Whether it serves yet, when the provider reports it.
    pub state: Option<ProviderDomainState>,
    /// DNS records the provider still needs, while it waits for them.
    pub records: Vec<ProviderDnsRecord>,
    /// The port traffic goes to, when the provider says.
    pub port: Option<u16>,
    /// The certificate in a few words, such as `Valid` or `Issuing`, for
    /// the person's own domains; null for the provider's own addresses.
    pub certificate: Option<String>,
    /// Every DNS record the domain uses, with what the DNS host has now.
    pub dns: Vec<ProviderDnsCheck>,
}

/// One DNS record a domain uses, checked against what the DNS host serves.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ProviderDnsCheck {
    /// Record type, such as `CNAME`.
    pub kind: String,
    /// Full name the record goes on.
    pub name: String,
    /// The value it must have.
    pub value: String,
    /// What the DNS host serves now, when anything.
    pub current: Option<String>,
    /// Whether it matches.
    pub ok: bool,
}

/// A custom domain's readiness as the provider reports it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDomainState {
    /// DNS is right and the certificate is issued.
    Working,
    /// Waiting for DNS records or a certificate.
    Pending,
    /// The provider reports a failure.
    Failed,
}

/// A DNS record the provider asks for.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ProviderDnsRecord {
    /// Record type, such as `CNAME`.
    pub kind: String,
    /// Full name the record goes on.
    pub name: String,
    /// The value it must have.
    pub value: String,
}

/// How a provider lets people connect with one click: its OAuth app for Hoista,
/// as a native (public) client using PKCE and a loopback redirect on this
/// computer. No secret and no server are involved.
#[derive(Debug, Clone)]
pub struct ProviderOAuth {
    /// The provider's authorization page.
    pub authorize_url: &'static str,
    /// Hoista's public client id at the provider.
    pub client_id: &'static str,
    /// Space-separated scopes to ask for.
    pub scopes: &'static str,
    /// Loopback ports registered as `http://{redirect_host}:{port}/callback`, tried in order.
    pub ports: &'static [u16],
    /// The loopback name in the registered redirect: `127.0.0.1`, or `localhost`
    /// for providers that only allow plain http there.
    pub redirect_host: &'static str,
    /// Extra authorization parameters the provider needs (for example `prompt=consent`).
    pub extra: &'static [(&'static str, &'static str)],
    /// The provider's mark as trusted SVG, for Hoista's own sign-in pages.
    pub logo: &'static str,
}

/// What to exchange at a provider's token endpoint.
#[derive(Debug, Clone, Copy)]
pub enum OAuthGrant<'a> {
    /// A code from the authorization redirect.
    Code {
        /// The code.
        code: &'a str,
        /// The PKCE verifier the challenge was made from.
        verifier: &'a str,
        /// The exact redirect URI used.
        redirect_uri: &'a str,
    },
    /// A refresh token from an earlier exchange.
    Refresh(&'a str),
}

/// Tokens from a provider's OAuth token endpoint.
#[derive(Debug)]
pub struct OAuthTokens {
    /// Short-lived access token.
    pub access: Secret,
    /// Refresh token, when the provider issued one.
    pub refresh: Option<Secret>,
    /// Seconds until the access token expires.
    pub expires_in: i64,
    /// The scopes the person granted, as the provider lists them, where its
    /// answer says (empty when it doesn't).
    pub granted: Vec<String>,
}

/// A watched resource's current state as its provider reports it.
#[derive(Debug, Clone)]
pub struct Observed {
    /// Current state.
    pub status: ResourceStatus,
    /// Public URL, when the provider gives one.
    pub url: Option<String>,
    /// The resource's role when the provider makes it clear (a Railway database
    /// service), so earlier guesses get corrected; None keeps the recorded role.
    pub kind: Option<ResourceKind>,
}

/// A resource found by discovery.
#[derive(Debug, Clone)]
pub struct RemoteResource {
    /// Opaque reference understood only by its adapter.
    pub reference: String,
    /// Display name.
    pub name: String,
    /// Resource role.
    pub kind: ResourceKind,
    /// Region if available.
    pub region: Option<String>,
    /// State as the provider reports it.
    pub status: ResourceStatus,
    /// Public URL, as the provider reports it.
    pub url: Option<String>,
    /// The provider's grouping by name (a Railway project), if any.
    pub group: Option<String>,
}

/// What discovery found, and which kinds of resource it couldn't read.
pub struct DiscoveryReport {
    /// Every resource of the kinds the token can read.
    pub resources: Vec<RemoteResource>,
    /// Kinds of resource left out because the token can't read them.
    pub warnings: Vec<DiscoveryWarning>,
}

/// One static file to upload; it has already passed the leak check.
pub struct Asset {
    /// Absolute URL path beginning with a slash.
    pub path: String,
    /// MIME type.
    pub content_type: String,
    /// File bytes in memory.
    pub bytes: Vec<u8>,
}

/// A build's output, ready to release. Builds never see provider credentials.
pub struct Artifact {
    /// Which source was copied before the build.
    pub provenance: Option<crate::domain::provenance::SourceProvenance>,
    /// Checked static files.
    pub assets: Vec<Asset>,
    /// Bundled Worker module, where required.
    pub worker: Option<Vec<u8>>,
    /// Worker entry module filename.
    pub worker_name: String,
    /// Additional bundled Worker modules, separate from public assets.
    pub worker_modules: Vec<Asset>,
    /// Fly image or local build directory, where required.
    pub image: Option<String>,
    /// Secret bindings held only in memory until release.
    pub secrets: Vec<(String, Secret)>,
}

/// A live log tail, owned by its caller. Always call `close`, even after an error: dropping
/// it closes the socket but can't clean up at the provider.
#[async_trait::async_trait]
pub trait LogSession: Send {
    /// Waits for the next log entry. Cancelling this future doesn't close the session.
    ///
    /// # Errors
    /// Reports malformed data, a lost connection or a provider limit; never skips them silently.
    async fn next(&mut self) -> Result<Option<crate::domain::logs::LogEntry>, EngineError>;

    /// Closes the connection and deletes the tail at the provider. Safe to retry if cleanup fails.
    ///
    /// # Errors
    /// Reports a failed cleanup at the provider; the provider expires the tail later anyway.
    async fn close(&mut self) -> Result<(), EngineError>;
}

/// One binding's name and the resource it points at, never its value.
pub struct ResourceBinding {
    /// Binding name.
    pub name: String,
    /// The resource it points at in the same account; null when unknown or unsupported.
    pub target_reference: Option<String>,
}

/// One provider's adapter. An operation the provider doesn't do returns [`unsupported`]'s error.
#[async_trait::async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// The provider this adapter is for.
    fn provider(&self) -> Provider;

    /// What this provider supports, for the UI to offer.
    fn capabilities(&self) -> Capabilities;

    /// The provider reports the bindings of the resource at `reference` (see
    /// [`Self::resource_bindings`]), so Hoista checks them.
    fn reports_bindings(&self, _reference: &str) -> bool {
        false
    }

    /// The provider lists the resource at `reference` without saying whether it
    /// serves requests or only runs on a schedule, so Hoista keeps the web or
    /// job role it already knows.
    fn keeps_known_role(&self, _reference: &str) -> bool {
        false
    }

    /// The provider project a resource belongs to, from its reference, where the
    /// provider groups resources into projects. Used to suggest imports together.
    fn provider_project(&self, _reference: &str) -> Option<String> {
        None
    }

    /// Removing a service is the provider's own action for it (which works per
    /// environment) rather than [`Self::delete`].
    fn removes_services_by_action(&self) -> bool {
        false
    }

    /// A database the provider adds is listed under the name it was given. False
    /// for a provider that names it after something else, such as its template.
    fn names_added_databases(&self) -> bool {
        true
    }

    /// Lists the accounts a token can see, before the token is saved. Changes nothing.
    ///
    /// # Errors
    /// Reports a failed lookup or a token that can't list accounts.
    async fn account_choices(&self, _token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        Err(unsupported())
    }

    /// Reads every binding name on a resource. An error must not be taken to mean the
    /// bindings are gone.
    ///
    /// # Errors
    /// Rejects unsupported providers, partial answers and unavailable credentials.
    async fn resource_bindings(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<Vec<ResourceBinding>, EngineError> {
        Err(unsupported())
    }

    /// Checks what the token can access. Changes nothing.
    ///
    /// # Errors
    /// Returns invalid credentials or missing permissions.
    async fn verify(&self, token: &Secret, external_id: &str) -> Result<Vec<String>, EngineError>;

    /// Lists the account's resources, following the provider's paging. Changes nothing.
    ///
    /// # Errors
    /// Returns provider failures, or an error when there are too many results.
    async fn discover(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<RemoteResource>, EngineError>;

    /// Like [`Self::discover`], and also reports kinds of resource the token can't read.
    ///
    /// # Errors
    /// Returns connection, authentication, invalid-response or too-many-pages failures.
    async fn discover_report(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<DiscoveryReport, EngineError> {
        Ok(DiscoveryReport {
            resources: self.discover(token, external_id).await?,
            warnings: vec![],
        })
    }

    /// Releases a build to a managed resource.
    ///
    /// # Errors
    /// Returns provider failure or a capability error.
    async fn deploy(
        &self,
        _token: &Secret,
        _external_id: &str,
        _resource: &ResourcePlan,
        _artifact: &Artifact,
    ) -> Result<Release, EngineError> {
        Err(unsupported())
    }

    /// Goes back to an earlier release at the provider.
    ///
    /// # Errors
    /// Returns provider failure or a capability error.
    async fn rollback(
        &self,
        _token: &Secret,
        _external_id: &str,
        _release: &Release,
    ) -> Result<Release, EngineError> {
        Err(unsupported())
    }

    /// Writes a secret straight to the provider; the value must not be kept.
    ///
    /// # Errors
    /// Returns provider failure or a capability error.
    async fn set_secret(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _name: &str,
        _value: &Secret,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Removes one secret at the provider. Local files are not changed.
    ///
    /// # Errors
    /// Returns provider failure or an unsupported capability error.
    async fn delete_secret(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _name: &str,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Checks that a released public address is serving.
    ///
    /// # Errors
    /// Returns failed health checks or unsupported verification.
    async fn health(
        &self,
        _token: &Secret,
        _external_id: &str,
        _release: &Release,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Gets a database connection string; it is not kept locally.
    ///
    /// # Errors
    /// Returns provider failure or unsupported database connections.
    async fn connection_string(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<Secret, EngineError> {
        Err(unsupported())
    }

    /// A direct (not pooled) database connection, for migrations.
    ///
    /// # Errors
    /// Reports unsupported direct connections or provider errors.
    async fn migration_connection_string(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<Secret, EngineError> {
        Err(unsupported())
    }

    /// Reads a database's branches and compute settings from the provider, without connecting
    /// to the database.
    ///
    /// # Errors
    /// Rejects unsupported, partial or unreadable answers; never truncates silently.
    async fn database_observation(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<crate::domain::database_observation::DatabaseObservation, EngineError> {
        Err(unsupported())
    }

    /// Reads one page of the provider's operations on a database, without connecting to it.
    ///
    /// # Errors
    /// Rejects unsupported providers, or unavailable or invalid pages.
    async fn database_operations(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _cursor: Option<&str>,
    ) -> Result<crate::domain::database_observation::DatabaseOperationHistory, EngineError> {
        Err(unsupported())
    }

    /// Lists every database that can be exported, without connection credentials.
    ///
    /// # Errors
    /// Reports an incomplete list or an unsupported provider.
    async fn database_inventory(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<Vec<crate::domain::database_export::DatabaseIdentity>, EngineError> {
        Err(unsupported())
    }

    /// Gets a direct connection for one database from [`Self::database_inventory`].
    ///
    /// # Errors
    /// Reports an unsupported connection or a provider failure.
    async fn database_export_connection(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _database: &crate::domain::database_export::DatabaseIdentity,
    ) -> Result<Secret, EngineError> {
        Err(unsupported())
    }

    /// Sets up or refreshes a custom hostname, attached only to the given resource.
    ///
    /// # Errors
    /// Reports ownership conflicts, missing scopes and unsupported providers.
    async fn configure_domain(
        &self,
        _token: &Secret,
        _account: &str,
        _reference: &str,
        _hostname: &str,
        _zone_name: &str,
        _create_zone: bool,
    ) -> Result<crate::domain::domains::DomainObservation, EngineError> {
        Err(unsupported())
    }

    /// Refreshes an existing hostname. The default calls [`Self::configure_domain`]; adapters
    /// override it so a route removed outside Hoista isn't created again.
    async fn observe_domain(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        hostname: &str,
        zone_name: &str,
    ) -> Result<crate::domain::domains::DomainObservation, EngineError> {
        self.configure_domain(token, account, reference, hostname, zone_name, false)
            .await
    }

    /// Detaches the domain from this resource only; never deletes its DNS zone.
    ///
    /// # Errors
    /// Reports ownership conflicts, missing permissions or provider failure.
    async fn detach_domain(
        &self,
        _token: &Secret,
        _account: &str,
        _reference: &str,
        _domain_id: Option<&str>,
        _hostname: &str,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Deletes one resource at the provider, once approved.
    ///
    /// # Errors
    /// Returns provider failure or a capability error.
    async fn delete(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Pauses a managed resource, where the provider supports it.
    ///
    /// # Errors
    /// Returns provider failure or a capability error.
    async fn pause(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Starts a paused resource again with the startup settings saved when it was paused.
    ///
    /// # Errors
    /// Rejects unsupported providers, a missing pause record, or changes made at the provider since.
    async fn resume(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<ResourceStatus, EngineError> {
        Err(unsupported())
    }

    /// The provider's one-click connection, when Hoista has an OAuth app there.
    fn oauth(&self) -> Option<ProviderOAuth> {
        None
    }

    /// Exchanges an authorization code (with its PKCE verifier) or a refresh
    /// token at the provider's token endpoint.
    ///
    /// # Errors
    /// Returns unsupported, a refused grant or provider failure.
    async fn oauth_token(&self, _grant: OAuthGrant<'_>) -> Result<OAuthTokens, EngineError> {
        Err(unsupported())
    }

    /// Reads a resource's current state and public URL for resources Hoista
    /// watches but did not deploy, where the provider reports them.
    ///
    /// # Errors
    /// Returns unsupported, a missing resource or provider failure.
    async fn observe_resource(
        &self,
        _token: &Secret,
        _external_id: &str,
        _resource: &Resource,
    ) -> Result<Observed, EngineError> {
        Err(unsupported())
    }

    /// Runs one [`ProviderAction`] on a watched resource at its provider. Returns a
    /// sentence for the activity log.
    ///
    /// # Errors
    /// Returns unsupported, missing permission or provider failure.
    async fn provider_action(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _action: &ProviderAction<'_>,
    ) -> Result<String, EngineError> {
        Err(unsupported())
    }

    /// Everything this provider can do through Hoista's generic operation path.
    fn provider_operations(&self) -> Vec<ProviderOperationInfo> {
        vec![]
    }

    /// The project's buttons for this provider's resources.
    fn provider_quick_actions(&self) -> Vec<ProviderQuickAction> {
        vec![]
    }

    /// Starts setting up the provider's command-line tool in the background, so
    /// an agent's first command doesn't wait for a download. Returns at once.
    fn prepare_tools(&self) {}
    /// Runs one operation from [`Self::provider_operations`]. `reference` is the
    /// resource it runs on, when it runs on one; context the provider needs
    /// (project, environment, service, workspace) is filled in from it.
    /// `folder` is the project's source folder, for provider command-line tools
    /// that work on files there. The answer has secret-looking values removed.
    ///
    /// # Errors
    /// Returns unsupported, unknown operations, invalid arguments or provider failure.
    async fn provider_operation(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: Option<&str>,
        _folder: Option<&std::path::Path>,
        _name: &str,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        Err(unsupported())
    }

    /// The account's projects at the provider that hold nothing yet. Providers
    /// without projects have none.
    ///
    /// # Errors
    /// Returns provider failure.
    async fn provider_empty_projects(
        &self,
        _token: &Secret,
        _external_id: &str,
    ) -> Result<Vec<ProviderEmptyProject>, EngineError> {
        Ok(Vec::new())
    }

    /// What the provider has billed this account so far this period. None when
    /// the provider gives no bill for it.
    ///
    /// # Errors
    /// Returns unsupported or provider failure.
    async fn provider_bill(
        &self,
        _token: &Secret,
        _external_id: &str,
    ) -> Result<Option<ProviderBill>, EngineError> {
        Err(unsupported())
    }

    /// How much of its plan this account has used this period, as the provider
    /// counts it (emails sent, compute hours), for providers that report usage
    /// but not money.
    ///
    /// # Errors
    /// Returns unsupported or provider failure.
    async fn provider_plan_usage(
        &self,
        _token: &Secret,
        _external_id: &str,
    ) -> Result<Vec<ProviderPlanUsage>, EngineError> {
        Err(unsupported())
    }

    /// What a resource's address served between two times, from the provider's
    /// request logs.
    ///
    /// # Errors
    /// Returns unsupported, invalid ranges or provider failure.
    async fn provider_traffic(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _from: chrono::DateTime<chrono::Utc>,
        _to: chrono::DateTime<chrono::Utc>,
    ) -> Result<ProviderTraffic, EngineError> {
        Err(unsupported())
    }

    /// The repositories the account can reach, newest pushed first, for a
    /// provider that keeps code (GitHub).
    ///
    /// # Errors
    /// Returns unsupported or provider failure.
    async fn repositories(
        &self,
        _token: &Secret,
        _external_id: &str,
    ) -> Result<Vec<ProviderRepository>, EngineError> {
        Err(unsupported())
    }

    /// A repository's branch names, as the code provider lists them.
    ///
    /// # Errors
    /// Returns unsupported, an unknown repository or provider failure.
    async fn branches(
        &self,
        _token: &Secret,
        _external_id: &str,
        _repository: &str,
    ) -> Result<Vec<String>, EngineError> {
        Err(unsupported())
    }

    /// The newest commit on a branch of a repository the account can reach.
    ///
    /// # Errors
    /// Returns unsupported, an unknown repository or provider failure.
    async fn branch_head(
        &self,
        _token: &Secret,
        _external_id: &str,
        _repository: &str,
        _branch: &str,
    ) -> Result<String, EngineError> {
        Err(unsupported())
    }

    /// A resource's metrics between two times, as its provider measures them,
    /// one point every `step_seconds` where the provider allows it.
    ///
    /// # Errors
    /// Returns unsupported, invalid ranges or provider failure.
    async fn provider_metrics(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _from: chrono::DateTime<chrono::Utc>,
        _to: chrono::DateTime<chrono::Utc>,
        _step_seconds: u32,
    ) -> Result<ProviderMetrics, EngineError> {
        Err(unsupported())
    }

    /// Creates a service, template or bucket in the same project as an
    /// existing resource. Returns the references of resources it created
    /// that discovery will find, and a sentence for the activity log.
    ///
    /// # Errors
    /// Returns unsupported, invalid input or provider failure.
    async fn provider_create(
        &self,
        _token: &Secret,
        _external_id: &str,
        _beside: &str,
        _what: &ProviderCreate<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        Err(unsupported())
    }

    /// Creates a new project at the provider: a provider project of its own with one service
    /// deploying `repo` (owner/name, optional branch) and a public address.
    /// Returns the new services' references and a sentence for the activity log.
    ///
    /// # Errors
    /// Returns unsupported, invalid input or provider failure.
    async fn provider_new_project(
        &self,
        _token: &Secret,
        _external_id: &str,
        _name: &str,
        _repo: &str,
        _branch: Option<&str>,
    ) -> Result<(Vec<String>, String), EngineError> {
        Err(unsupported())
    }

    /// Creates a new project at the provider from a folder on this computer,
    /// named `name`, as the provider's own CLI does. Returns the new resources'
    /// references and a sentence for the activity log.
    ///
    /// # Errors
    /// Returns unsupported, a folder the provider's tool can't use, or provider failure.
    async fn provider_new_from_folder(
        &self,
        _token: &Secret,
        _external_id: &str,
        _name: &str,
        _folder: &std::path::Path,
    ) -> Result<(Vec<String>, String), EngineError> {
        Err(unsupported())
    }

    /// What's missing from `granted` (the scopes a sign-in recorded on the
    /// account) for `what`, said for the person, when the provider can tell.
    /// `None` when nothing is missing or the scopes aren't known.
    fn missing_grant(&self, granted: &[String], what: &ProviderNew<'_>) -> Option<&'static str> {
        let _ = (granted, what);
        None
    }

    /// Makes something new at the provider for a Hoista project, next to
    /// `beside` when the project already has a resource on this account (so
    /// a provider that groups resources, such as in a provider project, keeps them
    /// together). `project` is the Hoista project's name, for a provider that
    /// needs a group to put the first one in. Returns the references of what
    /// it made that discovery will find, and a sentence for the activity log.
    ///
    /// The default covers a repository and a folder with
    /// [`Self::provider_new_project`] and [`Self::provider_new_from_folder`].
    ///
    /// # Errors
    /// Returns unsupported, invalid input or provider failure.
    async fn provider_add(
        &self,
        token: &Secret,
        external_id: &str,
        project: &str,
        beside: Option<&str>,
        what: &ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let _ = (project, beside);
        match *what {
            ProviderNew::Repo {
                name,
                repo,
                branch,
                runs_as: None | Some("web"),
                schedule: None,
                ..
            } => {
                self.provider_new_project(token, external_id, name, repo, branch)
                    .await
            }
            ProviderNew::Folder { name, folder } => {
                self.provider_new_from_folder(token, external_id, name, folder)
                    .await
            }
            _ => Err(unsupported()),
        }
    }

    /// Creates Hoista itself on this provider from `repo`, with `variables` set
    /// before its first start. Returns the resource reference and its public
    /// host name.
    ///
    /// # Errors
    /// Returns unsupported, invalid input or provider failure.
    async fn provider_new_hoista_server(
        &self,
        _token: &Secret,
        _external_id: &str,
        _source: &ServerSource,
        _variables: &std::collections::BTreeMap<String, Secret>,
    ) -> Result<(String, String), EngineError> {
        Err(unsupported())
    }

    /// Makes the sealing key of a Hoista server made with
    /// [`Self::provider_new_hoista_server`] unreadable at the provider: hidden from its
    /// dashboard, API and CLI, while the server still gets it. Returns whether
    /// the provider now hides it; `false` means the person should do it there.
    ///
    /// # Errors
    /// Returns provider failure.
    async fn provider_hide_server_key(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<bool, EngineError> {
        Ok(false)
    }

    /// When the provider doesn't hide the sealing key on its own, the steps for
    /// the person to do it there, in the provider's own words, and the page to do
    /// it on. `None` gives a general sentence naming the provider.
    fn server_key_steps(&self, _reference: &str) -> Option<(String, Option<String>)> {
        None
    }

    /// Uploads a folder and deploys it on a watched resource, as the provider's
    /// own CLI does. Returns a sentence for the activity log.
    ///
    /// # Errors
    /// Returns unsupported, an unreadable or oversized folder, or provider failure.
    async fn provider_upload(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _folder: &std::path::Path,
        _message: Option<&str>,
    ) -> Result<String, EngineError> {
        Err(unsupported())
    }

    /// Sets one variable on a watched resource at its provider. The value goes
    /// straight to the provider and is not kept. The provider may redeploy to apply it.
    ///
    /// # Errors
    /// Returns unsupported, missing permission or provider failure.
    async fn set_provider_variable(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _name: &str,
        _value: &Secret,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Sets many variables at once on a watched resource, or on its provider project's
    /// shared variables (`shared`). The values go straight to the provider and
    /// are not kept. `redeploy` false asks the provider not to redeploy yet.
    ///
    /// # Errors
    /// Returns unsupported, missing permission or provider failure.
    async fn set_provider_variables(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _values: &[(String, Secret)],
        _shared: bool,
        _redeploy: bool,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Deletes one of the provider project's shared variables.
    ///
    /// # Errors
    /// Returns unsupported, missing permission or provider failure.
    async fn delete_shared_variable(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _name: &str,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Makes a key another service uses to reach this one, such as a Resend
    /// key that can only send from one domain. The caller puts it straight
    /// into the other service's variables; it is never kept or shown.
    ///
    /// # Errors
    /// Returns unsupported, missing permission or provider failure.
    async fn issue_key(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _name: &str,
    ) -> Result<Secret, EngineError> {
        Err(unsupported())
    }

    /// The variable name services usually read this provider's key from.
    fn key_variable(&self) -> Option<&'static str> {
        None
    }

    /// Deletes one variable of a watched resource at its provider.
    ///
    /// # Errors
    /// Returns unsupported, missing permission or provider failure.
    async fn delete_provider_variable(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _name: &str,
    ) -> Result<(), EngineError> {
        Err(unsupported())
    }

    /// Reads one provider deployment's build and runtime log lines, oldest first,
    /// redacted, at most 400.
    ///
    /// # Errors
    /// Returns unsupported, a missing deployment or provider failure.
    async fn provider_deployment_logs(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _deployment: &str,
    ) -> Result<Vec<crate::domain::logs::LogEntry>, EngineError> {
        Err(unsupported())
    }

    /// Reads the provider's own deployment history, variable names and domains for
    /// a resource Hoista watches.
    ///
    /// # Errors
    /// Returns unsupported, a missing resource or provider failure.
    async fn provider_details(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
    ) -> Result<ProviderDetails, EngineError> {
        Err(unsupported())
    }

    /// Reads one page of the logs the provider keeps.
    ///
    /// # Errors
    /// Returns unsupported, missing permission, invalid query or provider failure.
    async fn log_history(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        Err(unsupported())
    }

    /// Opens a live log tail at the provider. Nothing is read from history.
    ///
    /// # Errors
    /// Reports unsupported resources, missing tail permissions or transport failure.
    async fn open_logs(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _query: &str,
    ) -> Result<Box<dyn LogSession>, EngineError> {
        Err(unsupported())
    }

    /// Reads recent log lines, up to a limit, without writing them to disk.
    ///
    /// # Errors
    /// Returns provider failure or a capability error.
    async fn logs(
        &self,
        _token: &Secret,
        _external_id: &str,
        _reference: &str,
        _since: &str,
        _query: &str,
    ) -> Result<Vec<String>, EngineError> {
        Err(unsupported())
    }

    /// Reads one day's usage for a resource, as the provider counts it.
    ///
    /// # Errors
    /// Returns provider failure or a capability error.
    async fn usage(
        &self,
        _token: &Secret,
        _external_id: &str,
        _resource: &Resource,
        _day: &str,
    ) -> Result<Usage, EngineError> {
        Err(unsupported())
    }
}

/// True for the error a provider gives for an operation it doesn't do.
#[must_use]
pub fn is_unsupported(error: &EngineError) -> bool {
    matches!(error, EngineError::Provider { code, .. } if code == "provider.not_supported")
}

/// Sets one variable at the provider, whichever way the provider takes it: as a
/// variable on the service, or as a secret (a Cloudflare Worker, a Fly app).
/// The value goes straight to the provider and isn't kept.
///
/// # Errors
/// Reports the provider's failure, or unsupported when it takes neither.
pub async fn put_variable(
    adapter: &dyn ProviderAdapter,
    token: &Secret,
    external_id: &str,
    reference: &str,
    name: &str,
    value: &Secret,
) -> Result<(), EngineError> {
    match adapter
        .set_provider_variable(token, external_id, reference, name, value)
        .await
    {
        Err(error) if is_unsupported(&error) => {
            adapter
                .set_secret(token, external_id, reference, name, value)
                .await
        }
        other => other,
    }
}

/// Removes one variable at the provider, as a service variable or a secret.
///
/// # Errors
/// Reports the provider's failure, or unsupported when it takes neither.
pub async fn remove_variable(
    adapter: &dyn ProviderAdapter,
    token: &Secret,
    external_id: &str,
    reference: &str,
    name: &str,
) -> Result<(), EngineError> {
    match adapter
        .delete_provider_variable(token, external_id, reference, name)
        .await
    {
        Err(error) if is_unsupported(&error) => {
            adapter
                .delete_secret(token, external_id, reference, name)
                .await
        }
        other => other,
    }
}

/// The error for an operation this provider's adapter doesn't do.
pub fn unsupported() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.not_supported".into(),
        message: "This operation is not supported by this provider adapter. Open the provider dashboard for other actions.".into(),
    }
}

/// Where a new Hoista server's code comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSource {
    /// A published, signed image pinned by digest: what most people use.
    Image(String),
    /// Built by the provider from the person's own copy of the repository.
    Repo {
        /// `owner/name` on GitHub.
        repo: String,
        /// The branch to build; the repository's default when absent.
        branch: Option<String>,
    },
}
