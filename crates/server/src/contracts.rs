//! The JSON Schema and TypeScript types for the engine API the desktop app uses (`docs/backend/api`).

use crate::Provider;
use crate::application::engine::{AccountDiagnostic, AccountDiagnosticStatus};
use crate::application::engine::{AttentionFeed, AttentionNotice, AttentionQuery};
use crate::application::engine::{
    DailyTraffic, ProjectListQuery, ProjectOverview, ProjectSort, ProjectStats, SourceGit,
};
use crate::application::engine::{
    ImportEvidence, ImportEvidenceKind, ImportProposal, ImportedProject, LogCapture, LogFrame,
};
use crate::application::engine::{
    MapEdge, MapEdgeKind, MapEdgeOrigin, MapGraph, MapNode, ProjectMap,
};
use crate::application::engine::{RecordedResourceRelease, ResourceDetail, ResourceRelationship};
use crate::application::ports::catalog::Page;
use crate::domain::database_export::{DatabaseArchive, DatabaseIdentity, DatabaseTarget};
use crate::domain::database_observation::{DatabaseBranch, DatabaseCompute, DatabaseObservation};
use crate::domain::deletion::{
    DeletionConfirmation, DeletionDomain, DeletionPreview, DeletionResource,
};
use crate::domain::domains::{
    CustomDomain, DomainDnsPurpose, DomainDnsRecord, DomainDnsRecordType, DomainObservation,
    DomainReadiness,
};
use crate::domain::environment::{EnvironmentChange, EnvironmentDeployRequest};
use crate::domain::logs::{LogEntry, LogLevel};
use crate::domain::logs::{LogHistory, LogHistoryQuery};
use crate::domain::provenance::{SourceObservation, SourceProvenance};
use crate::domain::{backup::Backup, model::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Published encrypted file; durability is separate from successful publication.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct TransferExportResult {
    /// Absolute output path.
    pub path: String,
    /// The encrypted file's size in bytes. Small enough for JavaScript to hold exactly.
    #[ts(type = "number")]
    pub bytes: u64,
    /// SHA-256 of the encrypted file.
    pub sha256: String,
    /// Whether the containing directory was synchronized after publication.
    pub durable: bool,
    /// Recovery guidance if publication succeeded but durability is uncertain.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_step: Option<String>,
}

/// Authenticated transfer contents summarized without disclosing credential values.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct TransferInspection {
    /// Payload format version.
    pub version: u32,
    /// Snapshot time.
    pub created_at: String,
    /// Whether a profile is included.
    pub has_profile: bool,
    /// Connected and disconnected account records.
    pub accounts: usize,
    /// Included provider credentials.
    pub credentials: usize,
    /// Project records.
    pub projects: usize,
    /// Resource records.
    pub resources: usize,
    /// Ignored resources preserved in the transfer.
    pub ignored_resources: usize,
    /// Resource relationships.
    pub links: usize,
    /// Historical events included.
    pub activity: usize,
    /// Whether older activity was omitted by the export cap.
    pub activity_truncated: bool,
    /// Inspection never installs the contents; always false.
    pub restored: bool,
}

/// Completed local installation, separate from provider-access verification.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct TransferRestoreResult {
    /// Local installation completed.
    pub restored: bool,
    /// Imported projects start in watching mode.
    pub mode: Mode,
    /// Durable journal identifier.
    pub restore_id: String,
    /// Provider credentials written and read back locally.
    pub credentials_staged: usize,
    /// False until a separate provider check succeeds.
    pub provider_access_verified: bool,
}

/// Recent restore attempts and explicit credential-retention behavior.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct TransferRecovery {
    /// Newest attempts first.
    pub attempts: Vec<crate::application::data::transfer::payload::TransferRestoreStatus>,
    /// Maximum number returned.
    pub limit: usize,
    /// Human-readable retention policy.
    pub credential_cleanup: String,
}

/// A journal was abandoned without deleting potentially late credential writes.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct TransferAbandoned {
    /// Journal identifier.
    pub restore_id: String,
    /// Journal abandonment completed.
    pub abandoned: bool,
    /// Always false; credentials remain retained.
    pub credentials_deleted: bool,
}

/// Readiness for local clients; reaching this route means the owner IPC listener is serving.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct HealthDto {
    /// Daemon build version, checked against the CLI before enrollment.
    pub version: String,
    /// Process identity for owner-local diagnostics.
    pub pid: u32,
}

/// Identity and restart state of a running daemon, separate from general readiness.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct DaemonStatus {
    /// Installed daemon version.
    pub version: String,
    /// Current process ID, for diagnostics only.
    pub pid: u32,
    /// Random process instance identity used to reject stale restart requests.
    pub instance_id: String,
    /// Launched in service-manager mode; the manager is responsible for replacement after exit.
    pub supervised: bool,
    /// An owner restart has been accepted; completion still requires a new instance.
    pub restart_requested: bool,
    /// The exact command a coding tool runs to reach this daemon over MCP:
    /// the `hoist` binary beside this daemon, its state folder when it is not
    /// the default one, then `mcp`. Empty when the binary can't be found.
    #[serde(default)]
    pub mcp_command: Vec<String>,
}

/// Owner request to drain work and replace exactly one observed daemon instance.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct RestartRequest {
    /// Instance identity returned by daemon status.
    pub instance_id: String,
    /// Owner has arranged a service manager to take over an independently started daemon.
    #[serde(default)]
    pub handoff: bool,
}

/// Profile request and response; a missing profile is HTTP 404 during first run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ProfileDto {
    /// Display name, 1–80 characters after trimming.
    pub name: String,
    /// Avatar initials, 1–4 characters after trimming.
    pub initials: String,
}

impl From<crate::domain::Profile> for ProfileDto {
    fn from(profile: crate::domain::Profile) -> Self {
        Self {
            name: profile.name().into(),
            initials: profile.initials().into(),
        }
    }
}

/// Stable error body without internal causes or submitted values.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ErrorDto {
    /// Final failed provider response metadata, if known.
    #[serde(default)]
    pub provider_response: Option<ProviderFailureEvidence>,
    /// Required environment names when known; absent in older responses.
    #[serde(default)]
    pub missing_secrets: Vec<String>,
    /// Machine-readable reason.
    pub code: String,
    /// Plain-language next step.
    pub message: String,
    /// Conservative recovery advice, when there is any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recovery: Option<super::recovery::RecoveryAdvice>,
}

impl ErrorDto {
    /// Preserves typed recovery inputs from application failures without parsing messages.
    pub fn from_engine(error: &crate::application::ports::catalog::EngineError) -> Self {
        let mut dto = Self::new(error.code(), &error.to_string());
        dto.provider_response = error.provider_response().cloned();
        dto.missing_secrets = error.missing_secrets().to_vec();
        dto
    }

    /// Renders the stable code and recovery step for terminal clients.
    pub fn explanation(&self) -> String {
        let mut explanation = match &self.recovery {
            Some(recovery) => format!(
                "[{}] {}\nNext step: {}",
                self.code, self.message, recovery.next_step
            ),
            None => format!("[{}] {}", self.code, self.message),
        };
        if !self.missing_secrets.is_empty() {
            explanation.push_str(&format!("\nRequired names: {:?}", self.missing_secrets));
        }
        if let Some(response) = &self.provider_response {
            explanation.push_str(&format!("\nProvider HTTP status: {}", response.http_status));
            if let (Some(header), Some(id)) = (&response.request_id_header, &response.request_id) {
                explanation.push_str(&format!("\nProvider reference: {header:?} {id:?}"));
            }
        }
        explanation
    }

    /// Creates an error with code-based guidance that never incorporates raw details.
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            provider_response: None,
            missing_secrets: Vec::new(),
            code: code.into(),
            message: message.into(),
            recovery: Some(super::recovery::advice(code)),
        }
    }
}

/// A durable server-sent event; id is encoded as text to preserve integer precision.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct EventDto {
    /// Where to resume from; also sent as the SSE event id.
    pub id: String,
    /// RFC 3339 UTC time.
    pub at: String,
    /// Responsible person or agent.
    pub actor: String,
    /// Project identifier when an event belongs to a project.
    pub project_id: Option<String>,
    /// The record the event is about: a deployment for `deploy.*`, an approval
    /// for `approval.*`, and likewise for agents, resources, domains and
    /// accounts. Null when no single record applies or the event predates
    /// this field; never parse the summary instead.
    #[serde(default)]
    pub subject_id: Option<String>,
    /// Stable event type, also sent in the SSE event field.
    pub kind: String,
    /// Description with no submitted profile or credential values.
    pub summary: String,
}

/// `GET /activity`: committed events, newest first.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ActivityPage {
    /// At most 50 events.
    pub items: Vec<EventDto>,
    /// Pass as `cursor` for older events; null on the last page.
    pub next_cursor: Option<String>,
}

impl From<crate::application::Event> for EventDto {
    fn from(event: crate::application::Event) -> Self {
        Self {
            id: event.id.to_string(),
            at: event.at,
            actor: event.actor,
            project_id: event.project_id,
            subject_id: event.subject_id,
            kind: event.kind,
            summary: event.summary,
        }
    }
}

/// A health check result distinguishes unavailable functionality from healthy checks.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The check succeeded.
    Ok,
    /// The user must take action.
    ActionNeeded,
    /// Not yet implemented or deliberately not checked.
    NotChecked,
}

/// One diagnostic with a concrete next step where needed.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DoctorCheck {
    /// Stable check identifier.
    pub name: String,
    /// Result of this check.
    pub status: CheckStatus,
    /// Human-readable diagnostic without secret values.
    pub message: String,
}

/// How this Hoista is doing, and, when asked, whether each account's token still works.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DoctorReport {
    /// Bounded current-process HTTP observations.
    pub network: Option<crate::application::ports::network::NetworkReport>,
    /// Bounded account verification page when an engine is available.
    #[serde(default)]
    pub accounts: Option<Page<AccountDiagnostic>>,
    /// Engine version.
    pub version: String,
    /// daemon or direct, depending on the state owner.
    pub mode: String,
    /// Individual checks, including explicitly unchecked integrations.
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// Whether no check needs action. Unchecked integrations don't count as failures.
    pub fn healthy(&self) -> bool {
        self.checks
            .iter()
            .all(|check| check.status != CheckStatus::ActionNeeded)
    }
}

/// Root of the generated wire contract schema.
#[derive(JsonSchema)]
#[expect(dead_code, reason = "its fields are only read by the schema generator")]
struct Contract {
    transfer_export: TransferExportResult,
    transfer_inspection: TransferInspection,
    transfer_restore: TransferRestoreResult,
    transfer_recovery: TransferRecovery,
    transfer_abandoned: TransferAbandoned,
    diagnostic_report: crate::application::observe::diagnostic_report::DiagnosticReport,
    diagnostic_receipt: crate::application::observe::diagnostic_report::DiagnosticReceipt,
    profile: ProfileDto,
    health: HealthDto,
    daemon_status: DaemonStatus,
    restart_request: RestartRequest,
    error: ErrorDto,
    event: EventDto,
    activity_page: ActivityPage,
    doctor: DoctorReport,
    account: Account,
    account_choice: AccountChoice,
    custom_domain: CustomDomain,
    approval_display: super::approval_display::ApprovalDisplay,
    project: Project,
    environment: Environment,
    environments: Vec<Environment>,
    project_map: ProjectMap,
    binding_refresh: crate::application::engine::BindingRefresh,
    project_page: Page<Project>,
    imported_project: ImportedProject,
    import_proposals: Page<ImportProposal>,
    description_resolution: crate::application::projects::description::DescriptionResolution,
    attention: AttentionFeed,
    attention_query: AttentionQuery,
    project_overview: ProjectOverview,
    project_totals: crate::application::engine::ProjectTotals,
    provider_details: crate::application::ports::providers::ProviderDetails,
    provider_metrics: crate::application::ports::providers::ProviderMetrics,
    provider_repository: crate::application::ports::providers::ProviderRepository,
    source_preview: crate::application::deploys::planning::SourcePreview,
    sign_in_start: crate::application::engine::SignInStart,
    project_list_query: ProjectListQuery,
    resource: Resource,
    resource_detail: ResourceDetail,
    log_history: LogHistory,
    log_history_query: LogHistoryQuery,
    discovery: DiscoveryResult,
    link: Link,
    deploy: Deploy,
    deployment_request: DeploymentRequest,
    environment_key_recovery:
        crate::application::environments::admission_keys::EnvironmentKeyRecovery,
    deploy_options: DeployOptions,
    environment_variable: EnvironmentVariable,
    environment_deploy_request: EnvironmentDeployRequest,
    public_environment_request: crate::domain::environment::PublicEnvironmentRequest,
    public_environment_prefill: crate::domain::environment::PublicEnvironmentPrefill,
    agent: Agent,
    approval: Approval,
    deletion_preview: DeletionPreview,
    deletion_confirmation: DeletionConfirmation,
    database_archive: DatabaseArchive,
    database_observation: DatabaseObservation,
    database_operations: crate::domain::database_observation::DatabaseOperationHistory,
    log_capture: LogCapture,
    log_frame: LogFrame,
    usage: Usage,
    settings: Settings,
    settings_patch: SettingsPatch,
    capabilities: Capabilities,
    operation_page: Page<crate::domain::model::Operation>,
    webhooks: Vec<crate::domain::model::Webhook>,
    webhook_deliveries: Vec<crate::domain::model::WebhookDelivery>,
    plan: Plan,
    backup: Backup,
}

/// Generates JSON Schema from the same Rust types serialized by the API.
pub fn json_schema() -> schemars::Schema {
    schemars::schema_for!(Contract)
}

/// Generates TypeScript declarations from the serialized Rust types.
pub fn typescript() -> String {
    [
        TransferExportResult::decl(),
        TransferInspection::decl(),
        TransferRestoreResult::decl(),
        TransferRecovery::decl(),
        TransferAbandoned::decl(),
        crate::application::data::transfer::payload::TransferRestoreRecord::decl(),
        crate::application::data::transfer::payload::TransferRestoreStatus::decl(),
        crate::domain::map_layout::MapPosition::decl(),
        crate::domain::map_layout::MapLayout::decl(),
        crate::application::engine::BindingRefresh::decl(),
        crate::domain::environment::PublicEnvironmentRequest::decl(),
        crate::domain::environment::PublicEnvironmentValue::decl(),
        crate::domain::environment::PublicEnvironmentUnavailableReason::decl(),
        crate::domain::environment::PublicEnvironmentUnavailable::decl(),
        crate::domain::environment::PublicEnvironmentPrefill::decl(),
        Provider::decl(),
        Mode::decl(),
        ResourceKind::decl(),
        ResourceStatus::decl(),
        DeployStatus::decl(),
        StepStatus::decl(),
        AccountStatus::decl(),
        ApprovalDecision::decl(),
        GatedAction::decl(),
        AgentTransport::decl(),
        Money::decl(),
        Account::decl(),
        AccountChoice::decl(),
        AccountDiagnostic::decl(),
        AccountDiagnosticStatus::decl(),
        CustomDomain::decl(),
        super::approval_display::ApprovalDisplay::decl(),
        super::approval_display::ApprovalDisplayTarget::decl(),
        super::approval_display::ApprovalDisplayDetail::decl(),
        DomainObservation::decl(),
        DomainDnsRecord::decl(),
        DomainDnsRecordType::decl(),
        DomainDnsPurpose::decl(),
        DomainReadiness::decl(),
        Project::decl(),
        ProjectMap::decl(),
        MapGraph::decl(),
        MapNode::decl(),
        MapEdge::decl(),
        MapEdgeKind::decl(),
        MapEdgeOrigin::decl(),
        Page::<Project>::decl(),
        ImportedProject::decl(),
        ImportEvidenceKind::decl(),
        ImportEvidence::decl(),
        ImportProposal::decl(),
        crate::application::projects::description::DescriptionCandidate::decl(),
        crate::application::projects::description::DescriptionResolution::decl(),
        AttentionFeed::decl(),
        AttentionNotice::decl(),
        AttentionQuery::decl(),
        ProjectOverview::decl(),
        Environment::decl(),
        crate::application::engine::ProjectTotals::decl(),
        crate::application::ports::providers::ProviderDetails::decl(),
        crate::application::engine::SignInStart::decl(),
        crate::application::ports::providers::ProviderDeployment::decl(),
        crate::application::ports::providers::ProviderDeployState::decl(),
        crate::application::ports::providers::ProviderDomain::decl(),
        crate::application::ports::providers::ProviderReference::decl(),
        crate::application::ports::providers::ProviderConfig::decl(),
        crate::application::ports::providers::ProviderBill::decl(),
        crate::application::engine::AccountBill::decl(),
        crate::application::engine::AccountBilling::decl(),
        crate::application::ports::providers::ProviderPlanUsage::decl(),
        crate::application::ports::providers::ProviderTraffic::decl(),
        crate::application::ports::providers::ProviderMetrics::decl(),
        crate::application::ports::providers::ProviderRepository::decl(),
        crate::application::deploys::planning::SourcePreview::decl(),
        crate::application::ports::providers::ProviderChart::decl(),
        crate::application::ports::providers::MetricUnit::decl(),
        crate::application::ports::providers::MetricSummary::decl(),
        crate::application::ports::providers::ProviderSeries::decl(),
        crate::application::ports::providers::ProviderPoint::decl(),
        crate::application::ports::providers::ProviderErrorCount::decl(),
        crate::application::ports::providers::ProviderRequest::decl(),
        crate::application::ports::providers::ProviderVolume::decl(),
        crate::application::ports::providers::ProviderTcpProxy::decl(),
        crate::application::ports::providers::VariableAddress::decl(),
        crate::application::ports::providers::ProviderQuickAction::decl(),
        crate::application::ports::providers::ProviderDomainState::decl(),
        crate::application::ports::providers::ProviderDnsRecord::decl(),
        crate::application::ports::providers::ProviderDnsCheck::decl(),
        ProjectListQuery::decl(),
        ProjectSort::decl(),
        ProjectStats::decl(),
        SourceGit::decl(),
        DailyTraffic::decl(),
        Resource::decl(),
        ResourceDetail::decl(),
        ResourceRelationship::decl(),
        RecordedResourceRelease::decl(),
        LogHistory::decl(),
        LogHistoryQuery::decl(),
        SourceProvenance::decl(),
        SourceObservation::decl(),
        DiscoverySummary::decl(),
        DiscoveryResult::decl(),
        DiscoveryWarning::decl(),
        Link::decl(),
        DeployStep::decl(),
        DeployChannel::decl(),
        DeployOptions::decl(),
        EnvironmentVariable::decl(),
        EnvironmentChange::decl(),
        EnvironmentDeployRequest::decl(),
        ProviderFailureEvidence::decl(),
        DeployFailure::decl(),
        Deploy::decl(),
        DeploymentRequest::decl(),
        crate::application::environments::admission_keys::EnvironmentKeyState::decl(),
        crate::application::environments::admission_keys::EnvironmentKeyRecovery::decl(),
        Agent::decl(),
        crate::domain::model::AgentGrant::decl(),
        crate::domain::model::AgentGrantInput::decl(),
        crate::domain::model::ProductionRule::decl(),
        crate::domain::model::AgentException::decl(),
        crate::domain::model::ResourceGroup::decl(),
        crate::domain::model::Restriction::decl(),
        crate::domain::model::ServerRecord::decl(),
        crate::domain::model::Operation::decl(),
        crate::domain::model::OperationState::decl(),
        crate::domain::model::Webhook::decl(),
        crate::domain::model::DeliveryState::decl(),
        crate::domain::model::WebhookDelivery::decl(),
        crate::application::ports::backups::BackupFile::decl(),
        crate::application::engine::EnrollmentRequest::decl(),
        crate::application::engine::EnrollmentStatus::decl(),
        crate::application::engine::EnrollmentPoll::decl(),
        super::catalog::ConnectionAnswer::decl(),
        Approval::decl(),
        DeletionResource::decl(),
        DeletionDomain::decl(),
        DeletionPreview::decl(),
        DeletionConfirmation::decl(),
        DatabaseArchive::decl(),
        DatabaseObservation::decl(),
        crate::domain::database_observation::DatabaseOperation::decl(),
        crate::domain::database_observation::DatabaseOperationHistory::decl(),
        DatabaseBranch::decl(),
        DatabaseCompute::decl(),
        LogLevel::decl(),
        LogEntry::decl(),
        LogFrame::decl(),
        LogCapture::decl(),
        DatabaseIdentity::decl(),
        DatabaseTarget::decl(),
        SecretMetadata::decl(),
        Usage::decl(),
        Settings::decl(),
        SettingsPatch::decl(),
        Capabilities::decl(),
        VaultPlace::decl(),
        VaultInfo::decl(),
        ResourcePlan::decl(),
        Plan::decl(),
        PrismaMigration::decl(),
        Source::decl(),
        Release::decl(),
        ResourceRelease::decl(),
        Backup::decl(),
        ProfileDto::decl(),
        HealthDto::decl(),
        DaemonStatus::decl(),
        RestartRequest::decl(),
        super::recovery::RetryDisposition::decl(),
        super::recovery::RecoveryAdvice::decl(),
        ErrorDto::decl(),
        crate::application::ports::network::NetworkOutcome::decl(),
        crate::application::ports::network::HostObservation::decl(),
        crate::application::ports::network::NetworkReport::decl(),
        crate::application::observe::diagnostic_report::DiagnosticReport::decl(),
        crate::application::observe::diagnostic_report::DiagnosticIssue::decl(),
        crate::application::observe::diagnostic_report::DiagnosticAccount::decl(),
        crate::application::observe::diagnostic_report::DiagnosticResource::decl(),
        crate::application::observe::diagnostic_report::DiagnosticProject::decl(),
        crate::application::observe::diagnostic_report::DiagnosticDeploy::decl(),
        crate::application::observe::diagnostic_report::DiagnosticStep::decl(),
        crate::application::observe::diagnostic_report::DiagnosticReceipt::decl(),
        EventDto::decl(),
        ActivityPage::decl(),
        CheckStatus::decl(),
        DoctorCheck::decl(),
        DoctorReport::decl(),
    ]
    .into_iter()
    .map(|declaration| format!("export {declaration}\n"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::ErrorDto;
    use crate::{application::ports::catalog::EngineError, domain::model::ProviderFailureEvidence};

    #[test]
    fn provider_errors_show_the_status_and_reference_but_nothing_else() {
        let error = EngineError::Provider {
            evidence: Some(ProviderFailureEvidence {
                http_status: 503,
                request_id: Some("0123456789abcdef-LHR".into()),
                request_id_header: Some("cf-ray".into()),
            }),
            code: "provider.unavailable".into(),
            message: "Cloudflare is unavailable right now.".into(),
        };
        let dto = ErrorDto::from_engine(&error);
        let explanation = dto.explanation();
        assert!(explanation.contains("Provider HTTP status: 503"));
        assert!(explanation.contains("0123456789abcdef-LHR"));
        let encoded = serde_json::to_string(&dto).unwrap();
        assert!(encoded.contains("provider.unavailable"));
        assert!(encoded.contains("0123456789abcdef-LHR"));
    }
}
