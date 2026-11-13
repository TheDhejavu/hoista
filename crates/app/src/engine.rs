//! The engine every front end talks to: the desktop app, the CLI and MCP
//! all go through it, and it goes through the ports to everything else.

use crate::{
    Provider,
    accounts::{enrollment::Enrollments, rescan::Rescans, sign_in::SignIns},
    deploys::planning,
    domain::{Secret, model::*},
    observe::{
        journal::recorder::{InFlight, Journaled},
        logs::LogTasks,
    },
    ports::{
        backups::BackupFolder,
        build::{BuildService, SourceInspector},
        catalog::{CatalogRepository, EngineError},
        database_export::DatabaseExporter,
        owner_check::{NoOwnerCheck, OwnerCheck},
        providers::{
            DiscoveryReport, ProviderAction, ProviderAdapter, ProviderBill, ProviderPlanUsage,
            unsupported,
        },
        sign_in_page::SignInPage,
        transfer::TransferSeal,
        vault::TokenVault,
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::Duration,
};

use crate::projects::deletion::require_executable;
use tokio::sync::{Mutex, Semaphore};

pub use crate::accounts::browse::{ProjectListQuery, ProjectSort, ProjectTotals};
pub use crate::accounts::enrollment::{
    EnrollmentPoll, EnrollmentRequest, EnrollmentStatus, StoredEnrollment,
};
pub use crate::accounts::import_suggestions::{ImportEvidence, ImportEvidenceKind, ImportProposal};
pub use crate::accounts::oauth::{AuthorizationRequest, OAuthRequest, pairing_code};
pub use crate::accounts::sign_in::SignInStart;
pub use crate::deploys::provider_operations::OperationTarget;
pub use crate::observe::diagnostics::{AccountDiagnostic, AccountDiagnosticStatus};
pub use crate::observe::logs::{LogCapture, LogFrame, LogSubscription};
pub use crate::projects::active::{ActiveProject, FoundBy, PROJECT_FILE};
pub use crate::projects::attention::{AttentionFeed, AttentionNotice, AttentionQuery};
pub use crate::projects::bindings::BindingRefresh;
pub use crate::projects::map::{
    MapEdge, MapEdgeKind, MapEdgeOrigin, MapGraph, MapNode, ProjectMap,
};
pub use crate::projects::overview::{
    DailyTraffic, ProjectOverview, ProjectStats, SourceGit, temporary_folder,
};
pub use crate::projects::resource::{
    RecordedResourceRelease, ResourceDetail, ResourceRelationship,
};
pub use crate::webhooks::NewWebhook;

/// Accounts read from their providers at once.
const MAX_ACCOUNT_READS: usize = 8;
const ACCOUNT_READ_TIMEOUT: Duration = Duration::from_secs(60);
const APPROVAL_LIFETIME_MINUTES: i64 = 15;
const MAX_APPROVAL_TARGET_BYTES: usize = 256;
const MAX_APPROVAL_REASON_BYTES: usize = 512;

/// Hoista put on a server: where it is and the one link that claims it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerSetup {
    /// `https://host`.
    pub url: String,
    /// The claim page with the one-time code after `#`. Open it once.
    pub claim_link: String,
    /// The provider's reference for the service.
    pub reference: String,
    /// Whether the provider now hides the sealing key from its dashboard, API
    /// and CLI. When it doesn't, the person seals it there ([`Self::seal_help`]).
    pub key_hidden: bool,
    /// What to do at the provider when the key isn't hidden yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal_help: Option<String>,
    /// The provider's page where that's done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seal_page: Option<String>,
}

/// Who is asking. Decided by how the request arrived, never by anything in it.
#[derive(Clone, Debug)]
pub enum Actor {
    /// The person, through the desktop app or the CLI.
    Person,
    /// A connected agent. Its access is checked on every request.
    Agent {
        /// The agent's id, given by Hoista.
        id: String,
    },
}

impl Actor {
    /// The name it's recorded under in the activity log.
    pub fn id(&self) -> &str {
        match self {
            Self::Person => "person",
            Self::Agent { id } => id,
        }
    }
}

/// The engine: every use case, over the catalog, the vault, the source
/// inspector and the provider adapters.
pub struct Engine {
    /// Held through every change at a provider, so changes never overlap.
    pub(crate) mutation: Mutex<()>,
    /// Held while an account is read for the first time, so it's read once.
    pub(crate) first_reads: Mutex<()>,
    pub(crate) diagnostics: Mutex<()>,
    /// New variable values waiting for their deploy, by deploy id. Memory only.
    pub(crate) environment_inputs: Mutex<HashMap<String, Vec<(String, Secret)>>>,
    pub(crate) settings_changes: Mutex<()>,
    pub(crate) log_tasks: LogTasks,
    pub(crate) resource_reads: Semaphore,
    pub(crate) builder: Arc<dyn BuildService>,
    pub(crate) database_exporter: Option<Arc<dyn DatabaseExporter>>,
    /// Where backups are kept; unset where Hoista keeps none.
    pub(crate) backup_folder: Option<Arc<dyn BackupFolder>>,
    pub(crate) transfer_seal: Option<Arc<dyn TransferSeal>>,
    pub(crate) sign_in_page: Option<Arc<dyn SignInPage>>,
    /// Hoista's saved records.
    pub repository: Arc<dyn CatalogRepository>,
    /// Where tokens and keys are kept.
    pub vault: Arc<dyn TokenVault>,
    pub(crate) adapters: HashMap<Provider, Arc<dyn ProviderAdapter>>,
    pub(crate) inspector: Arc<dyn SourceInspector>,
    pub(crate) sign_ins: SignIns,
    pub(crate) enrollments: Enrollments,
    pub(crate) owner_check: Arc<dyn OwnerCheck>,
    /// The public HTTPS address when Hoista runs on a server (cloud mode).
    pub(crate) public_url: Option<String>,
    /// On a computer, the public address its MCP over HTTP is reached at
    /// (through a tunnel or proxy the person runs),
    /// once known: remote agents sign in with OAuth there, as on a server.
    pub(crate) link_url: RwLock<Option<String>>,
    /// Whether MCP over HTTP is listening now.
    pub(crate) mcp_http_open: AtomicBool,
    /// When a request last came in through the tunnel or proxy (RFC 3339).
    pub(crate) link_seen: RwLock<Option<String>>,
    pub(crate) vault_info: VaultInfo,
    /// The image a new server runs; the release's own unless a test sets one.
    pub(crate) server_image: Option<String>,
    /// Provider changes being sent right now, shared with every journaled adapter.
    pub(crate) in_flight: InFlight,
    /// Accounts to read again after a change there.
    pub(crate) rescans: Rescans,
}

impl Engine {
    /// An engine over these parts, made where the process starts.
    pub fn new(
        repository: Arc<dyn CatalogRepository>,
        vault: Arc<dyn TokenVault>,
        inspector: Arc<dyn SourceInspector>,
        adapters: Vec<Arc<dyn ProviderAdapter>>,
        builder: Arc<dyn BuildService>,
    ) -> Self {
        let in_flight = InFlight::default();
        let journal = repository.clone();
        Self {
            mutation: Mutex::new(()),
            first_reads: Mutex::new(()),
            diagnostics: Mutex::new(()),
            environment_inputs: Mutex::new(HashMap::new()),
            settings_changes: Mutex::new(()),
            repository,
            vault,
            inspector,
            builder,
            database_exporter: None,
            backup_folder: None,
            transfer_seal: None,
            sign_in_page: None,
            log_tasks: LogTasks::default(),
            resource_reads: Semaphore::new(MAX_ACCOUNT_READS),
            sign_ins: SignIns::default(),
            enrollments: Enrollments::default(),
            owner_check: Arc::new(NoOwnerCheck),
            public_url: None,
            link_url: RwLock::new(None),
            mcp_http_open: AtomicBool::new(false),
            link_seen: RwLock::new(None),
            vault_info: VaultInfo {
                kept_in: VaultPlace::Keychain,
                token_store: "built-in".into(),
                key_source: None,
                references: true,
            },
            server_image: crate::RELEASE_IMAGE.map(str::to_owned),
            // Every change to a provider goes through the journal.
            adapters: adapters
                .into_iter()
                .map(|adapter| {
                    let wrapped: Arc<dyn ProviderAdapter> =
                        Arc::new(Journaled::new(adapter, journal.clone(), in_flight.clone()));
                    (wrapped.provider(), wrapped)
                })
                .collect(),
            in_flight,
            rescans: Rescans::default(),
        }
    }

    /// Keeps backups in `folder`.
    #[must_use]
    pub fn with_backup_folder(mut self, folder: Arc<dyn BackupFolder>) -> Self {
        self.backup_folder = Some(folder);
        self
    }

    /// Seals and saves transfers with `seal`.
    #[must_use]
    pub fn with_transfer_seal(mut self, seal: Arc<dyn TransferSeal>) -> Self {
        self.transfer_seal = Some(seal);
        self
    }

    /// Shows `page` when a provider sign-in returns to the browser. Without
    /// one the browser gets plain text.
    #[must_use]
    pub fn with_sign_in_page(mut self, page: Arc<dyn SignInPage>) -> Self {
        self.sign_in_page = Some(page);
        self
    }

    /// How transfers are sealed and saved.
    ///
    /// # Errors
    /// Reports a process that wasn't given one.
    pub fn transfer_seal(&self) -> Result<Arc<dyn TransferSeal>, EngineError> {
        self.transfer_seal
            .clone()
            .ok_or_else(|| EngineError::Unsupported("Transfers aren't available here.".into()))
    }

    /// Exports databases with `exporter`.
    pub fn with_database_exporter(mut self, exporter: Arc<dyn DatabaseExporter>) -> Self {
        self.database_exporter = Some(exporter);
        self
    }

    /// Sets how Hoista confirms it's the person (Touch ID or password).
    /// Without one, nothing that needs the person's OK can be allowed.
    pub fn with_owner_check(mut self, check: Arc<dyn OwnerCheck>) -> Self {
        self.owner_check = check;
        self
    }

    /// Uses `image` (pinned by digest) for new servers instead of the
    /// release's own; `None` leaves only building from a repository.
    #[must_use]
    pub fn with_server_image(mut self, image: Option<String>) -> Self {
        self.server_image = image;
        self
    }

    /// The image a new server runs, pinned by digest, when this build has one.
    #[must_use]
    pub fn server_image(&self) -> Option<&str> {
        self.server_image.as_deref()
    }

    /// Runs Hoista on a server reachable at `url` (`https://…`, no trailing
    /// slash): remote agents sign in with OAuth at this address.
    #[must_use]
    pub fn with_public_url(mut self, url: String) -> Self {
        self.public_url = Some(url.trim_end_matches('/').to_owned());
        self
    }

    /// Says where tokens are kept, for `GET /vault`.
    #[must_use]
    pub fn with_vault_info(mut self, info: VaultInfo) -> Self {
        self.vault_info = info;
        self
    }

    /// Where tokens are kept.
    #[must_use]
    pub fn vault_info(&self) -> &VaultInfo {
        &self.vault_info
    }

    /// Checks the vault can be used, once at start.
    ///
    /// # Errors
    /// Reports a token store that can't be reached.
    pub async fn check_vault(&self) -> Result<(), EngineError> {
        self.vault.check().await
    }

    /// The public address remote agents use: the server's own, or on a
    /// computer the one its MCP over HTTP is reached at, once known.
    #[must_use]
    pub fn public_url(&self) -> Option<String> {
        self.public_url
            .clone()
            .or_else(|| self.link_url.read().ok().and_then(|link| link.clone()))
    }

    /// Whether this Hoista runs on a server.
    #[must_use]
    pub const fn is_server(&self) -> bool {
        self.public_url.is_some()
    }

    /// On a computer, opens remote-agent sign-in at `url`, the public address
    /// its MCP over HTTP is reached at (`https://host`, no path). Agents that
    /// signed in at another address have to sign in again.
    ///
    /// # Errors
    /// Rejects a server (it has its own address) and an address that isn't
    /// `https://host`, or plain HTTP to this computer.
    pub fn open_link(&self, url: &str) -> Result<(), EngineError> {
        if self.is_server() {
            return Err(EngineError::Conflict(
                "Hoista on a server already takes agents at its own address.",
            ));
        }
        let parsed = url::Url::parse(url)
            .map_err(|_| EngineError::Invalid("Give the address as https://your-host."))?;
        let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback))
            || parsed.host_str().is_none()
            || !matches!(parsed.path(), "" | "/")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(EngineError::Invalid(
                "Give the address as https://your-host, with no path.",
            ));
        }
        let origin = parsed.origin().ascii_serialization();
        let mut link = self.link_url.write().map_err(|_| EngineError::Store)?;
        *link = Some(origin);
        Ok(())
    }

    /// Forgets the tunnel's or proxy's address on a computer; agents that signed in
    /// through it stop working until it's at the same address again.
    pub fn close_link(&self) {
        if let Ok(mut link) = self.link_url.write() {
            *link = None;
        }
        if let Ok(mut seen) = self.link_seen.write() {
            *seen = None;
        }
    }

    /// Records whether MCP over HTTP is listening.
    pub fn set_mcp_http_open(&self, open: bool) {
        self.mcp_http_open
            .store(open, std::sync::atomic::Ordering::Relaxed);
    }

    /// Notes a request that came in through the tunnel or proxy just now.
    pub fn note_link_seen(&self) {
        if let Ok(mut seen) = self.link_seen.write() {
            *seen = Some(now());
        }
    }

    /// When a request last came in through the tunnel or proxy.
    #[must_use]
    pub fn link_seen(&self) -> Option<String> {
        self.link_seen.read().ok().and_then(|seen| seen.clone())
    }

    /// Whether MCP over HTTP is listening now.
    #[must_use]
    pub fn mcp_http_open(&self) -> bool {
        self.mcp_http_open
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Asks the person to confirm it's them, showing `reason`.
    ///
    /// # Errors
    /// Reports a refusal or a computer that can't ask.
    pub async fn confirm_owner(&self, reason: &str) -> Result<(), EngineError> {
        self.owner_check.confirm(reason).await
    }

    /// The adapter for `provider`.
    ///
    /// # Errors
    /// Fails if the provider is not implemented in this build.
    pub fn adapter(&self, provider: Provider) -> Result<&Arc<dyn ProviderAdapter>, EngineError> {
        self.adapters.get(&provider).ok_or_else(unsupported)
    }

    /// Every provider this engine has an adapter for, with what it can do, in
    /// the order Hoista suggests them: what plans are made and checked against.
    pub fn providers(&self) -> planning::Providers {
        planning::Providers::new(
            Provider::ALL
                .into_iter()
                .filter_map(|provider| {
                    let adapter = self.adapters.get(&provider)?;
                    Some((provider, adapter.capabilities()))
                })
                .collect(),
        )
    }

    /// Whether the actor may do something needing `scope`. The person always
    /// may; an agent needs agents switched on, and access that's current and
    /// covers it.
    ///
    /// # Errors
    /// Fails for an agent that's unknown, revoked, expired or not allowed.
    pub async fn authorize(&self, actor: &Actor, scope: &str) -> Result<(), EngineError> {
        if let Actor::Agent { id } = actor {
            let settings = self.repository.settings().await?;
            if !settings.agents_enabled {
                return Err(EngineError::AgentsDisabled);
            }
            self.repository
                .authorize_agent(id, scope, &now(), settings.agent_expiry_days)
                .await?;
        }
        Ok(())
    }

    /// Asks the person to approve one action on one exact target, for an
    /// agent. The request lasts fifteen minutes.
    ///
    /// # Errors
    /// Fails for the person (who needs no approval), an agent without access,
    /// an empty or overlong target or reason, or a delete that can't go ahead.
    pub async fn request_approval(
        &self,
        actor: &Actor,
        action: GatedAction,
        target: String,
        reason: String,
    ) -> Result<Approval, EngineError> {
        self.authorize(actor, "apps:write").await?;
        let Actor::Agent { id } = actor else {
            return Err(EngineError::Invalid(
                "Local person actions do not require an agent approval.",
            ));
        };
        if target.is_empty()
            || target.len() > MAX_APPROVAL_TARGET_BYTES
            || reason.is_empty()
            || reason.len() > MAX_APPROVAL_REASON_BYTES
        {
            return Err(EngineError::Invalid(
                "Provide a short exact target and approval reason.",
            ));
        }
        let _mutation = self.mutation.lock().await;
        let deletion = if matches!(
            action,
            GatedAction::DeleteProject | GatedAction::DeleteResource
        ) {
            let preview = self.deletion_impact(action, &target).await?;
            require_executable(&preview)?;
            Some(preview)
        } else {
            None
        };
        let time = chrono::Utc::now();
        let value = Approval {
            deletion,
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: id.clone(),
            action,
            target,
            reason,
            created_at: time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            expires_at: (time + chrono::Duration::minutes(APPROVAL_LIFETIME_MINUTES))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            decision: ApprovalDecision::Pending,
        };
        self.repository.put_approval(&value).await?;
        Ok(value)
    }

    /// Consumes an agent's approval only for the exact action being performed.
    ///
    /// # Errors
    /// Rejects missing, reused, declined, expired or mismatched approvals.
    pub async fn gate(
        &self,
        actor: &Actor,
        action: GatedAction,
        target: &str,
        approval: Option<&str>,
    ) -> Result<(), EngineError> {
        if let Actor::Agent { id } = actor {
            self.repository
                .consume_approval(
                    approval.ok_or(EngineError::ApprovalRequired)?,
                    id,
                    action,
                    target,
                    &now(),
                )
                .await?;
        }
        Ok(())
    }

    /// A plan from the source, not saved yet.
    pub(crate) async fn plan_unsaved(
        &self,
        source: Source,
        name: &str,
        pinned: Option<Provider>,
        existing: Option<Provider>,
    ) -> Result<Plan, EngineError> {
        let detected = self.inspector.detect(&source).await?;
        planning::plan(
            source,
            name,
            detected,
            &self.accounts().all_accounts().await?,
            &self.providers(),
            pinned,
            existing,
        )
    }

    pub(crate) async fn save_plan(&self, actor: &Actor, plan: Plan) -> Result<Plan, EngineError> {
        self.repository.put_plan(&plan).await?;
        self.repository
            .event(
                actor.id(),
                None,
                "plan.created",
                "Inspected source and created a deployment plan.",
            )
            .await?;
        Ok(plan)
    }

    /// Reads an account's provider without holding Hoista's lock, which a
    /// provider can keep busy for up to a minute, then records what it found
    /// under the lock. Imports, deploys and edits wait only for the recording.
    pub(crate) async fn scan_unlocked(
        &self,
        account_id: &str,
    ) -> Result<DiscoveryResult, EngineError> {
        let fetched = self.fetch_report(account_id).await;
        let _mutation = self.mutation.lock().await;
        self.record_scan(&Actor::Person, account_id, true, fetched)
            .await
    }

    /// Reads each connected account Hoista hasn't looked at yet, such as
    /// one connected a moment ago, so what's on it is known before anyone is
    /// told there's nothing. Each is read once: a read that fails is
    /// recorded too, and the background monitor tries again later.
    ///
    /// # Errors
    /// Returns unavailable persistent state; a provider's failure is recorded
    /// on its account instead.
    pub(crate) async fn look_at_new_accounts(&self) -> Result<(), EngineError> {
        for account in self.accounts().all_accounts().await? {
            let new = account.status == AccountStatus::Connected
                && account.last_discovery.is_none()
                && self
                    .adapter(account.provider)
                    .is_ok_and(|adapter| adapter.capabilities().discover);
            if !new {
                continue;
            }
            // One read per account, even when several lists ask at once.
            let _reading = self.first_reads.lock().await;
            let still_new = self
                .repository
                .account(&account.id)
                .await?
                .is_some_and(|now| now.last_discovery.is_none());
            if still_new {
                let _ = self.scan_unlocked(&account.id).await;
            }
        }
        Ok(())
    }

    pub(crate) async fn discover_locked(
        &self,
        actor: &Actor,
        account_id: &str,
    ) -> Result<DiscoveryResult, EngineError> {
        self.scan(actor, account_id, false).await
    }

    pub(crate) async fn scan(
        &self,
        actor: &Actor,
        account_id: &str,
        quiet: bool,
    ) -> Result<DiscoveryResult, EngineError> {
        let fetched = self.fetch_report(account_id).await;
        self.record_scan(actor, account_id, quiet, fetched).await
    }

    /// What the provider lists on an account, within a minute. Reads only.
    pub(crate) async fn fetch_report(
        &self,
        account_id: &str,
    ) -> Result<DiscoveryReport, EngineError> {
        let read = async {
            let account = self
                .repository
                .account(account_id)
                .await?
                .ok_or(EngineError::NotFound)?;
            let token = self.accounts().account_token(account_id).await?;
            self.adapter(account.provider)?
                .discover_report(&token, &account.external_id)
                .await
        };
        tokio::time::timeout(ACCOUNT_READ_TIMEOUT, read)
            .await
            .unwrap_or_else(|_| {
                Err(EngineError::Provider {
                    evidence: None,
                    code: "discovery.timeout".into(),
                    message: "Discovery exceeded its time limit. Retry the account scan.".into(),
                })
            })
    }

    /// Records a read of an account's provider: its resources, and how the
    /// read went on the account. The caller holds the lock.
    pub(crate) async fn record_scan(
        &self,
        actor: &Actor,
        account_id: &str,
        quiet: bool,
        fetched: Result<DiscoveryReport, EngineError>,
    ) -> Result<DiscoveryResult, EngineError> {
        let mut account = self
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let result = match fetched {
            Ok(remote) => {
                self.discover_resources(actor, &account, remote, quiet)
                    .await
            }
            Err(error) => Err(error),
        };
        account.last_discovery = Some(match &result {
            Ok(report) => DiscoverySummary {
                observed_at: now(),
                complete: report.complete,
                observed_resources: Some(
                    u32::try_from(report.items.len()).map_err(|_| EngineError::Store)?,
                ),
                warnings: report.warnings.clone(),
            },
            Err(error) => DiscoverySummary {
                observed_at: now(),
                complete: false,
                observed_resources: None,
                warnings: vec![DiscoveryWarning {
                    resource_type: "account".into(),
                    code: error.code().into(),
                    message: "The inventory scan did not finish. Existing resources were retained; retry discovery.".into(),
                }],
            },
        });
        self.repository.put_account(&account).await?;
        result
    }

    pub(crate) async fn discover_resources(
        &self,
        actor: &Actor,
        account: &Account,
        remote: DiscoveryReport,
        quiet: bool,
    ) -> Result<DiscoveryResult, EngineError> {
        let account_id = account.id.as_str();
        let adapter = self.adapter(account.provider)?;
        let existing = self.projects().all_resources().await?;
        let mut imported = Vec::new();
        for found in remote.resources {
            let previous = existing.iter().find(|resource| {
                resource.account_id == account_id && resource.provider_ref == found.reference
            });
            let keeps_role = adapter.keeps_known_role(&found.reference);
            let resource = Resource {
                id: previous
                    .map(|resource| resource.id.clone())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                project_id: previous.and_then(|resource| resource.project_id.clone()),
                // Some providers list a resource the same way whether it serves
                // requests or only runs on a schedule; keep the role already known.
                kind: if keeps_role
                    && found.kind == ResourceKind::Web
                    && previous.is_some_and(|resource| resource.kind == ResourceKind::Job)
                {
                    ResourceKind::Job
                } else {
                    found.kind
                },
                provider: account.provider,
                account_id: account_id.into(),
                provider_ref: found.reference,
                name: found.name,
                region: found.region,
                // A resource whose provider reports no status keeps the one
                // Hoista last saw, instead of going back to unknown.
                status: match (found.status, previous) {
                    (ResourceStatus::Unknown, Some(previous)) => previous.status,
                    (status, _) => status,
                },
                mode: previous
                    .map(|resource| resource.mode)
                    .unwrap_or(Mode::Watching),
                // Nor does an address the provider didn't report go away.
                url: found
                    .url
                    .or_else(|| previous.and_then(|resource| resource.url.clone())),
                group: found.group,
            };
            self.repository.put_resource(&resource).await?;
            imported.push(resource);
        }
        if !quiet {
            self.repository
                .event(
                    actor.id(),
                    None,
                    "import.discovered",
                    "Looked at the provider. Nothing changed there.",
                )
                .await?;
        }
        if !quiet && !remote.warnings.is_empty() {
            self.repository
                .event(
                    actor.id(),
                    None,
                    "import.incomplete",
                    "Discovery could not inspect every supported resource family. Review the scan warnings; existing resources were retained.",
                )
                .await?;
        }
        Ok(DiscoveryResult {
            items: imported,
            complete: remote.warnings.is_empty(),
            warnings: remote.warnings,
        })
    }

    pub(crate) async fn watched_resource(
        &self,
        resource_id: &str,
    ) -> Result<(Resource, Account), EngineError> {
        let resource = self
            .repository
            .resource(resource_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.mode != Mode::Watching {
            return Err(EngineError::Conflict(
                "Set this project's secrets from its Environment tab; Hoista manages them.",
            ));
        }
        let account = self
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        Ok((resource, account))
    }

    /// The approval an agent needs for a destructive provider action, and what it
    /// names: removing a service, a volume, a TCP endpoint or an address.
    #[must_use]
    pub fn provider_action_gate(
        resource_id: &str,
        action: &ProviderAction<'_>,
    ) -> Option<(GatedAction, String)> {
        // `{resource}:{what}:{which}`, like generic provider operations, so the
        // approval screen can say exactly what will happen.
        let (what, which) = match action {
            ProviderAction::RemoveService => ("remove_service", String::new()),
            ProviderAction::RemoveVolume { volume_id } => {
                ("remove_volume", (*volume_id).to_owned())
            }
            ProviderAction::RemoveTcpProxy { proxy_id } => {
                ("remove_tcp_proxy", (*proxy_id).to_owned())
            }
            ProviderAction::RemoveDomain { domain } => ("remove_domain", (*domain).to_owned()),
            _ => return None,
        };
        Some((
            GatedAction::ProviderChange,
            format!("{resource_id}:{what}:{which}"),
        ))
    }

    /// One plain sentence for a destructive provider action, for the approval.
    #[must_use]
    pub fn provider_action_sentence(action: &ProviderAction<'_>) -> String {
        match action {
            ProviderAction::RemoveService => {
                "Delete this service and its deployments at the provider.".into()
            }
            ProviderAction::RemoveVolume { .. } => {
                "Delete this disk and everything stored on it.".into()
            }
            ProviderAction::RemoveTcpProxy { .. } => "Close this public TCP endpoint.".into(),
            ProviderAction::RemoveDomain { domain } => {
                format!("Remove {domain}. Visitors to it stop reaching the project.")
            }
            _ => String::new(),
        }
    }

    /// What discovery finds of the references the provider just made, not yet
    /// in a project.
    pub(crate) async fn made_resources(
        &self,
        actor: &Actor,
        account: &Account,
        references: &[String],
    ) -> Result<Vec<String>, EngineError> {
        let resource_ids: Vec<String> = {
            let _mutation = self.mutation.lock().await;
            self.discover_locked(actor, &account.id).await?;
            self.projects()
                .all_resources()
                .await?
                .into_iter()
                .filter(|resource| {
                    resource.account_id == account.id
                        && references.contains(&resource.provider_ref)
                        && resource.project_id.is_none()
                })
                .map(|resource| resource.id)
                .collect()
        };
        if resource_ids.is_empty() {
            return Err(EngineError::Conflict(
                "The provider created the project, but Hoista can't see it yet. It will show under Found on your providers.",
            ));
        }
        Ok(resource_ids)
    }

    /// The provider starts the first deploy as it makes the resource.
    pub(crate) async fn mark_deploying(&self, resource_ids: &[String]) -> Result<(), EngineError> {
        for id in resource_ids {
            if let Some(mut resource) = self.repository.resource(id).await? {
                resource.status = ResourceStatus::Deploying;
                self.repository.put_resource(&resource).await?;
            }
        }
        Ok(())
    }

    /// The connected account a new project is made on, once the caller may
    /// and the name is free.
    pub(crate) async fn new_project_account(
        &self,
        actor: &Actor,
        account_id: &str,
        name: &str,
    ) -> Result<Account, EngineError> {
        self.authorize(actor, "apps:write").await?;
        self.authorize(actor, "apps:create").await?;
        if !valid_slug(name) {
            return Err(EngineError::Invalid(
                "Use a lowercase project name with letters, digits and hyphens.",
            ));
        }
        let mut cursor = None;
        loop {
            let page = self.repository.projects(cursor.as_deref()).await?;
            if page.items.iter().any(|project| project.name == name) {
                return Err(EngineError::Conflict(
                    "A project with this name already exists in Hoista. Pick another name.",
                ));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        let account = self
            .repository
            .account(account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if account.status != AccountStatus::Connected {
            return Err(EngineError::Conflict(
                "This account isn't connected. Reconnect it in Hoista first.",
            ));
        }
        Ok(account)
    }

    /// Finds what the provider just made and imports it as a watched project.
    pub(crate) async fn adopt_new_project(
        &self,
        actor: &Actor,
        account: &Account,
        name: String,
        source: Option<Source>,
        references: &[String],
        done: &str,
    ) -> Result<(ImportedProject, String), EngineError> {
        let resource_ids = self.made_resources(actor, account, references).await?;
        let imported = self
            .projects()
            .import_project(actor, name, source, Mode::Watching, &resource_ids)
            .await?;
        self.mark_deploying(&resource_ids).await?;
        let summary = format!("{done}.");
        self.repository
            .event(
                actor.id(),
                Some(&imported.project.id),
                "provider.action",
                &summary,
            )
            .await?;
        Ok((imported, summary))
    }

    /// The environment id to use: the one given, which must exist, or `default`.
    pub(crate) async fn known_environment(&self, id: Option<&str>) -> Result<String, EngineError> {
        let Some(id) = id.filter(|id| *id != DEFAULT_ENVIRONMENT) else {
            return Ok(DEFAULT_ENVIRONMENT.to_owned());
        };
        if self
            .repository
            .environments()
            .await?
            .iter()
            .any(|environment| environment.id == id)
        {
            Ok(id.to_owned())
        } else {
            Err(EngineError::Invalid("Choose an environment that exists."))
        }
    }
}

/// Now, in RFC 3339 UTC to the millisecond.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// GitHub refused to show a repository: almost always because the connected
/// token doesn't reach it, or reaches it (so it's listed) without being allowed
/// to read its code. Says which, and how to fix it, instead of a bare refusal.
pub(crate) fn unreachable_repository(
    error: EngineError,
    repository: &str,
    listed: bool,
) -> EngineError {
    let refused = match &error {
        EngineError::Provider { code, .. } => {
            code == "provider.token_scope" || code == "provider.not_found"
        }
        EngineError::Invalid(_) | EngineError::Unsupported(_) => true,
        _ => false,
    };
    if !refused {
        return error;
    }
    let fix = if listed {
        format!(
            "GitHub lists {repository} for this account but doesn't let Hoista read its code, which linking needs to find the newest commit. If you connected a token, edit it on GitHub (Settings, Developer settings, Personal access tokens) and give it Contents: read on {repository}. If you signed in from Hoista, open github.com/settings/installations, configure Hoista and accept any new permissions it asks for."
        )
    } else {
        format!(
            "Hoista's GitHub access doesn't reach {repository}. If you signed in from Hoista, open github.com/settings/installations, configure Hoista and add {repository} (or choose All repositories). If you connected a token, edit it on GitHub (Settings, Developer settings, Personal access tokens) and add {repository} with Contents: read."
        )
    };
    EngineError::Provider {
        evidence: None,
        code: "github.repository_not_shared".into(),
        message: format!(
            "{fix} Then link again. Nothing was changed. Until then, a project whose folder is a clone of {repository} shows it from the folder's git remote (source_git)."
        ),
    }
}

/// The project an import created, and the resources assigned to it.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ImportedProject {
    /// New local project metadata.
    pub project: Project,
    /// The assigned resources, each with the mode chosen for it.
    pub resources: Vec<Resource>,
}

/// One account's bill from its provider.
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
pub struct AccountBill {
    /// Local account id.
    pub account_id: String,
    /// The provider.
    pub provider: Provider,
    /// The account's label in Hoista.
    pub label: String,
    /// What the provider has billed so far this period.
    pub bill: ProviderBill,
}

/// One connected account's billing, as its provider shares it.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema, ts_rs::TS)]
pub struct AccountBilling {
    /// Local account id.
    pub account_id: String,
    /// The provider.
    pub provider: Provider,
    /// The account's label in Hoista.
    pub label: String,
    /// What the provider has billed so far this period, when it says.
    pub bill: Option<ProviderBill>,
    /// Plan usage, for providers that count usage but don't share money.
    pub plan_usage: Vec<ProviderPlanUsage>,
    /// Why there is no bill, in a sentence, when there isn't one.
    pub note: Option<String>,
}

/// A provider variable name: letters, digits and underscores, not starting with a digit.
pub(crate) fn variable_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}
