export type TransferExportResult = { 
/**
 * Absolute output path.
 */
path: string, 
/**
 * The encrypted file's size in bytes. Small enough for JavaScript to hold exactly.
 */
bytes: number, 
/**
 * SHA-256 of the encrypted file.
 */
sha256: string, 
/**
 * Whether the containing directory was synchronized after publication.
 */
durable: boolean, 
/**
 * Recovery guidance if publication succeeded but durability is uncertain.
 */
next_step?: string, };
export type TransferInspection = { 
/**
 * Payload format version.
 */
version: number, 
/**
 * Snapshot time.
 */
created_at: string, 
/**
 * Whether a profile is included.
 */
has_profile: boolean, 
/**
 * Connected and disconnected account records.
 */
accounts: number, 
/**
 * Included provider credentials.
 */
credentials: number, 
/**
 * Project records.
 */
projects: number, 
/**
 * Resource records.
 */
resources: number, 
/**
 * Ignored resources preserved in the transfer.
 */
ignored_resources: number, 
/**
 * Resource relationships.
 */
links: number, 
/**
 * Historical events included.
 */
activity: number, 
/**
 * Whether older activity was omitted by the export cap.
 */
activity_truncated: boolean, 
/**
 * Inspection never installs the contents; always false.
 */
restored: boolean, };
export type TransferRestoreResult = { 
/**
 * Local installation completed.
 */
restored: boolean, 
/**
 * Imported projects start in watching mode.
 */
mode: Mode, 
/**
 * Durable journal identifier.
 */
restore_id: string, 
/**
 * Provider credentials written and read back locally.
 */
credentials_staged: number, 
/**
 * False until a separate provider check succeeds.
 */
provider_access_verified: boolean, };
export type TransferRecovery = { 
/**
 * Newest attempts first.
 */
attempts: Array<TransferRestoreStatus>, 
/**
 * Maximum number returned.
 */
limit: number, 
/**
 * Human-readable retention policy.
 */
credential_cleanup: string, };
export type TransferAbandoned = { 
/**
 * Journal identifier.
 */
restore_id: string, 
/**
 * Journal abandonment completed.
 */
abandoned: boolean, 
/**
 * Always false; credentials remain retained.
 */
credentials_deleted: boolean, };
export type TransferRestoreRecord = { 
/**
 * The restore's id. Never reused.
 */
id: string, 
/**
 * When it started.
 */
created_at: string, 
/**
 * The keychain entries it writes, chosen before any is written.
 */
credential_references: Array<string>, };
export type TransferRestoreStatus = { 
/**
 * The restore and its keychain entries.
 */
record: TransferRestoreRecord, 
/**
 * `staging`, `installed` or `abandoned`.
 */
status: string, };
export type MapPosition = { 
/**
 * The node's id, as the project map gives it.
 */
node_id: string, 
/**
 * Across.
 */
x: number, 
/**
 * Down.
 */
y: number, };
export type MapLayout = { 
/**
 * Goes up by one on every save. Send back the one you read; a save made
 * on an older one is refused, so two windows can't overwrite each other.
 */
revision: number, 
/**
 * Every saved position. Saving an empty list goes back to the automatic layout.
 */
positions: Array<MapPosition>, };
export type BindingRefresh = { 
/**
 * The resource whose bindings were read.
 */
resource_id: string, 
/**
 * Bindings resolved to resources in a project, on the same account.
 */
resolved: number, 
/**
 * Bindings left unknown: unsupported, pointing outside a project, missing or ambiguous.
 */
unresolved: number, 
/**
 * When the read finished.
 */
observed_at: string, 
/**
 * Always false: bindings aren't every possible dependency.
 */
relationships_complete: boolean, };
export type PublicEnvironmentRequest = { 
/**
 * The plan for the project's source on this computer.
 */
plan_id: string, 
/**
 * The plan revision on screen. A changed plan is refused.
 */
expected_plan_revision: number, };
export type PublicEnvironmentValue = { 
/**
 * A name the plan lists as public.
 */
name: string, 
/**
 * Its value in the source, which passed the secret check.
 */
value: string, 
/**
 * Whether it matches what was last deployed, as far as Hoista recorded it.
 */
matches_recorded_value: boolean, };
export type PublicEnvironmentUnavailableReason = "missing_from_source" | "source_differs_from_recorded" | "managed_binding";
export type PublicEnvironmentUnavailable = { 
/**
 * Its name.
 */
name: string, 
/**
 * Why it's left empty.
 */
reason: PublicEnvironmentUnavailableReason, };
export type PublicEnvironmentPrefill = { 
/**
 * The plan read.
 */
plan_id: string, 
/**
 * Its revision.
 */
plan_revision: number, 
/**
 * Always `true`: these come from the source only.
 */
source_only: boolean, 
/**
 * The values found. Private variables are never here.
 */
values: Array<PublicEnvironmentValue>, 
/**
 * The ones left empty, and why.
 */
unavailable: Array<PublicEnvironmentUnavailable>, };
export type Provider = "cloudflare" | "fly" | "railway" | "vercel" | "neon" | "supabase" | "render" | "resend" | "runpod" | "modal" | "github";
export type Mode = "managed" | "watching";
export type ResourceKind = "web" | "service" | "job" | "database" | "bucket" | "email" | "domain";
export type ResourceStatus = "live" | "sleeping" | "idle" | "failing" | "paused" | "deploying" | "not_deployed" | "stopped" | "unknown";
export type DeployStatus = "queued" | "running" | "live" | "failed" | "rolled_back";
export type StepStatus = "pending" | "running" | "succeeded" | "failed" | "skipped";
export type AccountStatus = "connected" | "expired" | "missing_scopes" | "disconnected";
export type ApprovalDecision = "pending" | "allowed" | "declined" | "expired" | "consumed";
export type GatedAction = "connect_agent" | "delete_project" | "delete_resource" | "delete_domain" | "connect_account" | "provider_change";
export type AgentTransport = "stdio" | "link";
export type Money = { 
/**
 * Amount in the currency’s smallest unit.
 */
cents: number, 
/**
 * ISO 4217 currency code, such as `USD`.
 */
currency: string, };
export type Account = { 
/**
 * How the last look for its resources went; null before the first.
 */
last_discovery: DiscoverySummary | null, 
/**
 * Its id in Hoista.
 */
id: string, 
/**
 * Its provider.
 */
provider: Provider, 
/**
 * Its name, as shown.
 */
label: string, 
/**
 * The provider's id for the account, team or organization.
 */
external_id: string, 
/**
 * What the token was checked to be allowed to do, when connected.
 */
scopes: Array<string>, 
/**
 * Whether its token works.
 */
status: AccountStatus, 
/**
 * When it was connected (RFC 3339, UTC).
 */
connected_at: string, 
/**
 * Another account whose saved sign-in this one shares: one provider sign-in
 * can cover several workspaces, and its refresh token works only once,
 * so they keep one copy. Null when the account holds its own.
 */
credential: string | null, 
/**
 * The token lives in 1Password: Hoista keeps only its `op://` reference.
 */
in_1password: boolean, };
export type AccountChoice = { 
/**
 * Provider account ID or organization slug consumed by the adapter.
 */
external_id: string, 
/**
 * Its name at the provider, checked before it's shown.
 */
name: string, };
export type AccountDiagnostic = { 
/**
 * Account id.
 */
account_id: string, 
/**
 * The provider.
 */
provider: Provider, 
/**
 * The account's label in Hoista.
 */
label: string, 
/**
 * Result of this check; the saved account status is not changed.
 */
status: AccountDiagnosticStatus, 
/**
 * Permissions the check saw; not a full list of what the token can write.
 */
verified_scopes: Array<string>, 
/**
 * RFC 3339 time the check finished; null when it didn't run or the account is
 * disconnected.
 */
checked_at: string | null, 
/**
 * What to do next. Never the provider's own error text.
 */
message: string, };
export type AccountDiagnosticStatus = "verified" | "credential_missing" | "expired" | "missing_scopes" | "unavailable" | "timed_out" | "disconnected" | "not_checked";
export type CustomDomain = { 
/**
 * Its id in Hoista.
 */
id: string, 
/**
 * The project.
 */
project_id: string, 
/**
 * The resource that answers on it.
 */
resource_id: string, 
/**
 * The domain, in lowercase ASCII, without `https://` or a path.
 */
hostname: string, 
/**
 * What the provider last said about it.
 */
observed: DomainObservation, 
/**
 * When that was (RFC 3339, UTC).
 */
checked_at: string, };
export type ApprovalDisplay = { 
/**
 * Approval identity used for the decision request.
 */
id: string, 
/**
 * Requesting agent identity.
 */
agent_id: string, 
/**
 * Registered display name, or identity when unavailable.
 */
agent_name: string, 
/**
 * Execution semantics shown alongside the agent identity.
 */
agent_detail: string, 
/**
 * Exact server-side action this request authorizes.
 */
action: GatedAction, 
/**
 * Human-readable summary.
 */
title: string, 
/**
 * Original request explanation, rendered as text.
 */
reason: string, 
/**
 * Reviewed resource and hostname effects.
 */
targets: Array<ApprovalDisplayTarget>, 
/**
 * Exact before/after settings for non-resource changes.
 */
details: Array<ApprovalDisplayDetail>, 
/**
 * Scope and execution behavior after permission is granted.
 */
if_allowed: string, 
/**
 * Effect of denying this request.
 */
if_declined: string, 
/**
 * Exact name the UI requires before permitting Allow.
 */
confirm_text: string | null, 
/**
 * Saved deletion fingerprint and backup policy for the decision request.
 */
confirmation: DeletionConfirmation | null, 
/**
 * Reason Desktop cannot offer Allow; decline remains possible.
 */
allow_blocked: string | null, 
/**
 * RFC3339 creation timestamp.
 */
created_at: string, 
/**
 * RFC3339 expiry timestamp.
 */
expires_at: string, 
/**
 * Null for pending; otherwise allow, decline, expired or consumed.
 */
decision: string | null, };
export type ApprovalDisplayTarget = { 
/**
 * Kind of effect, such as deletion or disruption.
 */
label: string, 
/**
 * Display name from Hoista's own records, not from the agent.
 */
name: string, 
/**
 * Provider serving this resource.
 */
provider: Provider, };
export type ApprovalDisplayDetail = { 
/**
 * Human-readable setting label.
 */
label: string, 
/**
 * The value as saved when the approval was asked for.
 */
value: string, };
export type DomainObservation = { 
/**
 * The DNS zone's id at the provider. Empty when the provider doesn't run the domain's DNS.
 */
zone_id: string, 
/**
 * The DNS zone's name: the provider's, or the one the person chose.
 */
zone_name: string, 
/**
 * The nameservers the domain has to use, as the provider says.
 */
nameservers: Array<string>, 
/**
 * Where the domain is registered, when the provider says. It can be out of date.
 */
registrar: string | null, 
/**
 * Whether DNS is set up, by the provider's own check.
 */
dns: DomainReadiness, 
/**
 * Whether the HTTPS certificate is ready. DNS can be ready before it is.
 */
https: DomainReadiness, 
/**
 * The provider's id for the domain.
 */
provider_domain_id: string | null, 
/**
 * The provider's id for its certificate, when known.
 */
certificate_id: string | null, 
/**
 * DNS records the provider suggests. Hoista shows them; it never adds them itself.
 */
dns_records: Array<DomainDnsRecord>, 
/**
 * Anything the person should know, such as a step only they can do.
 */
warnings: Array<string>, };
export type DomainDnsRecord = { 
/**
 * The record's full name.
 */
name: string, 
/**
 * Its type.
 */
record_type: DomainDnsRecordType, 
/**
 * Its value: an address, a CNAME target or TXT text, without quotes.
 */
value: string, 
/**
 * What it's for.
 */
purpose: DomainDnsPurpose, };
export type DomainDnsRecordType = "A" | "AAAA" | "CNAME" | "TXT";
export type DomainDnsPurpose = "routing" | "acme_challenge" | "ownership";
export type DomainReadiness = "active" | "pending" | "failing" | "unknown";
export type Project = { 
/**
 * Its id.
 */
id: string, 
/**
 * Display name.
 */
name: string, 
/**
 * Where its code is: a folder, or a repository at a commit.
 */
source: Source, 
/**
 * Managed or watched.
 */
mode: Mode, 
/**
 * RFC 3339 UTC creation time.
 */
created_at: string, 
/**
 * The Hoista environment it lives in.
 */
environment_id: string, };
export type ProjectMap = { 
/**
 * Saved canvas positions; positions of unknown or removed nodes are kept but not drawn.
 */
layout: MapLayout, 
/**
 * Linked source, or the unlinked state.
 */
source: Source, 
/**
 * The project's resources.
 */
nodes: Array<Resource>, 
/**
 * Recorded resource links.
 */
edges: Array<Link>, 
/**
 * Source, resource and custom-domain nodes with typed edges.
 */
graph: MapGraph, };
export type MapGraph = { 
/**
 * Nodes. Ids are prefixed by node type, so a source, a resource and a domain never clash.
 */
nodes: Array<MapNode>, 
/**
 * Relationships recorded locally, including ones no longer seen.
 */
edges: Array<MapEdge>, 
/**
 * False until the provider's relationships have all been discovered.
 */
relationships_complete: boolean, };
export type MapNode = { "type": "source", 
/**
 * Node id.
 */
id: string, 
/**
 * The linked folder or repository.
 */
source: Source, } | { "type": "resource", 
/**
 * Node id.
 */
id: string, 
/**
 * The resource: provider, kind, status and project.
 */
resource: Resource, 
/**
 * Latest recorded daily usage; not live traffic, and null rather than zero when unknown.
 */
latest_usage: Usage | null, 
/**
 * Last recorded release schedules; null means unknown, empty means none.
 */
schedules: Array<string> | null, } | { "type": "domain", 
/**
 * Node id.
 */
id: string, 
/**
 * Where it is attached, and what the provider last reported.
 */
domain: CustomDomain, };
export type MapEdge = { 
/**
 * Edge id.
 */
id: string, 
/**
 * Source node identifier.
 */
from: string, 
/**
 * Destination node identifier.
 */
to: string, 
/**
 * What the edge means.
 */
kind: MapEdgeKind, 
/**
 * Variable or binding name, hostname, or deployment label.
 */
via: string, 
/**
 * Where it came from.
 */
origin: MapEdgeOrigin, 
/**
 * The provider no longer shows this relationship; it is kept for review.
 */
missing: boolean, 
/**
 * The deployment behind a `deploys` edge; null for other edges.
 */
deploy_id: string | null, };
export type MapEdgeKind = "deploys" | "route" | "dependency";
export type MapEdgeOrigin = "deployment" | "custom_domain" | "manual" | "derived";
export type Page<T> = { 
/**
 * Up to 100 records, in the same order every time.
 */
items: Array<T>, 
/**
 * Cursor for the next page, or null on the last page.
 */
next_cursor: string | null, };
export type ImportedProject = { 
/**
 * New local project metadata.
 */
project: Project, 
/**
 * The assigned resources, each with the mode chosen for it.
 */
resources: Array<Resource>, };
export type ImportEvidenceKind = "shared_name" | "shared_hostname" | "same_project";
export type ImportEvidence = { 
/**
 * What matched.
 */
kind: ImportEvidenceKind, 
/**
 * Shared name stem or exact hostname; never a URL, connection string or secret.
 */
value: string, };
export type ImportProposal = { 
/**
 * Id derived from the evidence and the exact members, so it is the same each time.
 */
id: string, 
/**
 * Suggested name; never used without the person's choice.
 */
label: string, 
/**
 * Unassigned resource ids to review. Importing checks the selection again.
 */
resource_ids: Array<string>, 
/**
 * Evidence explaining this proposal.
 */
evidence: ImportEvidence, 
/**
 * True when suggestions overlap or two members have the same provider and kind; ask the
 * person to choose.
 */
ambiguous: boolean, };
export type DescriptionCandidate = { 
/**
 * Resource id, to select it.
 */
id: string, 
/**
 * Name as the provider shows it.
 */
name: string, 
/**
 * The provider.
 */
provider: Provider, 
/**
 * Resource role.
 */
kind: ResourceKind, 
/**
 * Local account id, to tell apart resources on different accounts.
 */
account_id: string, 
/**
 * The project it is in now; grouping never moves it out of another project. `app_id` is
 * the older name.
 */
project_id: string | null, 
/**
 * The provider's reference for it; part of what the review token covers.
 */
provider_ref: string, 
/**
 * The id, name, provider project name or hostname that matched. Not proof of a dependency.
 */
matched: string, };
export type DescriptionResolution = { 
/**
 * Up to 100 candidates to review and choose from.
 */
candidates: Array<DescriptionCandidate>, 
/**
 * True when matches on different providers or of different kinds overlap; nothing is chosen
 * automatically.
 */
ambiguous: boolean, 
/**
 * Token for exactly this result, to pass back when grouping. Grants no access.
 */
review_token: string, 
/**
 * Always true: a description never groups resources without a choice.
 */
selection_required: boolean, };
export type AttentionFeed = { 
/**
 * Pending approvals from this approval page.
 */
approvals: Array<Approval>, 
/**
 * Approvals cursor, even when the filtered page is empty.
 */
next_cursor: string | null, 
/**
 * Notices for this page of projects; they don't claim savings or current health.
 */
notices: Array<AttentionNotice>, 
/**
 * Project scan cursor, even for pages without notices.
 */
project_next_cursor: string | null, };
export type AttentionNotice = { "kind": "deployment_failed", 
/**
 * Owning project.
 */
project_id: string, 
/**
 * Failed attempt.
 */
deploy_id: string, } | { "kind": "resource_failing", 
/**
 * Owning project.
 */
project_id: string, 
/**
 * The failing resource.
 */
resource_id: string, } | { "kind": "idle_resource", 
/**
 * Owning project.
 */
project_id: string, 
/**
 * Resource to review; this is not permission to pause it.
 */
resource_id: string, 
/**
 * First full UTC day checked.
 */
since: string, 
/**
 * Last full UTC day checked.
 */
through: string, 
/**
 * Number of full days with usage reported.
 */
days: number, };
export type AttentionQuery = { 
/**
 * Approvals cursor.
 */
cursor: string | null, 
/**
 * Project scan cursor. An empty page can still have one.
 */
project_cursor: string | null, };
export type ProjectOverview = { 
/**
 * Project identity and source.
 */
project: Project, 
/**
 * Last observed aggregate resource state, independently of deploy failures.
 */
status: ResourceStatus, 
/**
 * All associated resources.
 */
resources: Array<Resource>, 
/**
 * Newest finished deployment, including failures that left the live version
 * unchanged. A deploy still running doesn't hide the last result.
 */
last_deploy: Deploy | null, 
/**
 * Usage as reported, with gaps left null.
 */
stats: ProjectStats, 
/**
 * For a folder source that is a clone of a GitHub repository: the
 * repository, branch and commit, read from the folder itself.
 */
source_git: SourceGit | null, };
export type Environment = { 
/**
 * Stable id; `env_default` for the default one.
 */
id: string, 
/**
 * Lowercase name with letters, digits and hyphens, unique among environments.
 */
name: string, 
/**
 * RFC 3339 UTC creation time.
 */
created_at: string, };
export type ProjectTotals = { 
/**
 * Projects in the catalog.
 */
projects: number, 
/**
 * Projects whose aggregate resource status is live.
 */
live: number, 
/**
 * Projects whose aggregate resource status is failing.
 */
failing: number, 
/**
 * Distinct providers hosting at least one grouped resource.
 */
providers: number, };
export type ProviderDetails = { 
/**
 * Newest first, at most 20.
 */
deployments: Array<ProviderDeployment>, 
/**
 * Variable names only, sorted. Null when the provider does not report them.
 */
variable_names: Array<string> | null, 
/**
 * Names of the provider project's shared variables in this environment, never
 * their values; null when the provider doesn't say.
 */
shared_variable_names: Array<string> | null, 
/**
 * Addresses the provider serves this resource on.
 */
domains: Array<ProviderDomain>, 
/**
 * Variables that point at another service on the same provider (Railway's
 * `${{Postgres.DATABASE_URL}}`): how this resource connects to others.
 */
references: Array<ProviderReference>, 
/**
 * Whether the provider puts it to sleep when nobody uses it (Railway's
 * Serverless setting). Null when the provider doesn't say.
 */
sleeps: boolean | null, 
/**
 * How the provider builds and runs it, as far as the provider says.
 */
config: ProviderConfig | null, 
/**
 * Disks attached to it.
 */
volumes: Array<ProviderVolume>, 
/**
 * Public TCP endpoints (for databases and non-HTTP services).
 */
tcp_proxies: Array<ProviderTcpProxy>, 
/**
 * The name other services in the same environment reach it by, for
 * example Railway's `postgres.railway.internal`. Null when the provider
 * doesn't say or it has none.
 */
private_domain: string | null, 
/**
 * Where its variables point outside this provider: the host name (and the
 * user name, when the address has one) of each address a variable holds,
 * such as another provider's database. Never the value itself, a password
 * or anything after the host.
 */
addresses: Array<VariableAddress>, };
export type SignInStart = { 
/**
 * Pass to finish the sign-in.
 */
id: string, 
/**
 * The provider's authorization page to open in the browser.
 */
url: string, 
/**
 * Only for a relayed sign-in: the port on the person's computer the
 * browser comes back to, at `/callback`, for the app to forward.
 */
relay_port?: number, };
export type ProviderDeployment = { 
/**
 * Provider deployment id.
 */
id: string, 
/**
 * The provider's own status word, such as `SUCCESS` or `CRASHED`.
 */
status: string, 
/**
 * Hoista's reading of that status.
 */
state: ProviderDeployState, 
/**
 * RFC 3339 creation time.
 */
created_at: string, 
/**
 * Public URL of this deployment, when the provider gives one.
 */
url: string | null, 
/**
 * Commit hash, when deployed from a repository.
 */
commit: string | null, 
/**
 * Commit message or deploy note.
 */
message: string | null, 
/**
 * Branch, when deployed from a repository.
 */
branch: string | null, 
/**
 * Repository, such as `owner/name`, when deployed from one.
 */
repo: string | null, 
/**
 * RFC 3339 time its build finished, live or failed; null while it is
 * still building, and when the provider can't say (a later status, such as
 * being replaced, moves the provider's own timestamp).
 */
finished_at: string | null, 
/**
 * Whether the provider says it can go back to this deployment; null when the
 * provider doesn't say.
 */
can_roll_back: boolean | null, 
/**
 * Who made the change it deploys, as the provider records it (the commit's
 * author); null when the provider doesn't say.
 */
author: string | null, 
/**
 * The deployment's own page in the provider's dashboard, when the provider
 * gives one or its address can be built from the ids.
 */
provider_page: string | null, };
export type ProviderDeployState = "succeeded" | "failed" | "in_progress" | "stopped";
export type ProviderDomain = { 
/**
 * Hostname.
 */
name: string, 
/**
 * True for the person's own domain, false for the provider's generated one.
 */
custom: boolean, 
/**
 * Whether it serves yet, when the provider reports it.
 */
state: ProviderDomainState | null, 
/**
 * DNS records the provider still needs, while it waits for them.
 */
records: Array<ProviderDnsRecord>, 
/**
 * The port traffic goes to, when the provider says.
 */
port: number | null, 
/**
 * The certificate in a few words, such as `Valid` or `Issuing`, for
 * the person's own domains; null for the provider's own addresses.
 */
certificate: string | null, 
/**
 * Every DNS record the domain uses, with what the DNS host has now.
 */
dns: Array<ProviderDnsCheck>, };
export type ProviderReference = { 
/**
 * The variable's name, such as `DATABASE_URL`.
 */
variable: string, 
/**
 * The service it refers to, by its name on the provider.
 */
service: string, };
export type ProviderConfig = { 
/**
 * The repository it deploys from, `owner/name`.
 */
repo: string | null, 
/**
 * The image it runs, when it runs an image.
 */
image: string | null, 
/**
 * The command that starts it.
 */
start_command: string | null, 
/**
 * The command that builds it.
 */
build_command: string | null, 
/**
 * The folder inside the repository it builds from.
 */
root_directory: string | null, 
/**
 * The cron schedule it runs on, when it is a job.
 */
cron_schedule: string | null, 
/**
 * The path checked before traffic is sent to it.
 */
healthcheck_path: string | null, 
/**
 * How many copies run.
 */
replicas: number | null, 
/**
 * The region it runs in.
 */
region: string | null, 
/**
 * When it restarts after exiting, as the provider names it.
 */
restart_policy: string | null, };
export type ProviderBill = { 
/**
 * Billed so far this period.
 */
billed: Money, 
/**
 * RFC 3339 start of the billing period, when the provider says.
 */
period_start: string | null, 
/**
 * RFC 3339 end of the billing period, when the provider says.
 */
period_end: string | null, 
/**
 * The provider's soft usage limit (a warning), if set.
 */
soft_limit: Money | null, 
/**
 * The provider's hard usage limit (services stop), if set.
 */
hard_limit: Money | null, 
/**
 * Whether the account is over its limit.
 */
over_limit: boolean, 
/**
 * The provider's id for the paying customer, needed to change its usage
 * limits (Railway's `customer.id`). An id, not a secret.
 */
customer_id: string | null, };
export type AccountBill = { 
/**
 * Local account id.
 */
account_id: string, 
/**
 * The provider.
 */
provider: Provider, 
/**
 * The account's label in Hoista.
 */
label: string, 
/**
 * What the provider has billed so far this period.
 */
bill: ProviderBill, };
export type AccountBilling = { 
/**
 * Local account id.
 */
account_id: string, 
/**
 * The provider.
 */
provider: Provider, 
/**
 * The account's label in Hoista.
 */
label: string, 
/**
 * What the provider has billed so far this period, when it says.
 */
bill: ProviderBill | null, 
/**
 * Plan usage, for providers that count usage but don't share money.
 */
plan_usage: Array<ProviderPlanUsage>, 
/**
 * Why there is no bill, in a sentence, when there isn't one.
 */
note: string | null, };
export type ProviderPlanUsage = { 
/**
 * What is counted, in the provider's words, such as `Emails this month`.
 */
label: string, 
/**
 * How much was used.
 */
used: number, 
/**
 * The plan's limit, when it has one.
 */
limit: number | null, 
/**
 * The unit, such as `emails`, `hours` or `GB`.
 */
unit: string, };
export type ProviderTraffic = { 
/**
 * Start of the range, RFC 3339.
 */
from: string, 
/**
 * End of the range, RFC 3339.
 */
to: string, 
/**
 * Requests served. Null when there were more than Hoista reads in one
 * pass, so a partial count is never shown as the total.
 */
requests: number | null, 
/**
 * Responses with a 4xx status, counted the same way.
 */
client_errors: number | null, 
/**
 * Responses with a 5xx status, counted the same way.
 */
server_errors: number | null, 
/**
 * Median response time in milliseconds, of the requests read.
 */
p50_ms: number | null, 
/**
 * 95th percentile response time in milliseconds.
 */
p95_ms: number | null, 
/**
 * 99th percentile response time in milliseconds.
 */
p99_ms: number | null, 
/**
 * The most frequent failing requests, most frequent first.
 */
top_errors: Array<ProviderErrorCount>, 
/**
 * The newest requests, newest first, at most 200.
 */
recent: Array<ProviderRequest>, };
export type ProviderMetrics = { 
/**
 * Start of the range, RFC 3339.
 */
from: string, 
/**
 * End of the range, RFC 3339.
 */
to: string, 
/**
 * The charts, in the order to show them.
 */
charts: Array<ProviderChart>, 
/**
 * What the provider couldn't answer, in a sentence, such as a chart its
 * plan doesn't include. Null when everything was read.
 */
note: string | null, };
export type ProviderRepository = { 
/**
 * `owner/name`.
 */
full_name: string, 
/**
 * The branch it builds from by default, such as `main`.
 */
default_branch: string, 
/**
 * Only people given access can see it.
 */
private: boolean, 
/**
 * When something was last pushed to it, RFC 3339.
 */
pushed_at: string | null, 
/**
 * Its page.
 */
url: string, 
/**
 * Its one-line description, when it has one.
 */
description: string | null, };
export type SourcePreview = { 
/**
 * The folder, as given.
 */
path: string, 
/**
 * The framework Hoista recognised, when it did.
 */
framework: string | null, 
/**
 * Static, edge, job, service or supabase, when recognised.
 */
shape: string | null, 
/**
 * From the lockfile.
 */
package_manager: string | null, 
/**
 * The build command, when there is one.
 */
build_command: Array<string>, 
/**
 * Environment variable names the code reads; never values.
 */
env_names: Array<string>, 
/**
 * Things to know, in plain words.
 */
warnings: Array<string>, 
/**
 * Set when Hoista didn't recognise the project, saying why.
 */
unrecognised: string | null, };
export type ProviderChart = { 
/**
 * A stable id for the chart, such as `requests`.
 */
id: string, 
/**
 * Its title, such as "Requests".
 */
label: string, 
/**
 * What the values measure.
 */
unit: MetricUnit, 
/**
 * How the chart's headline number is worked out from its points.
 */
summary: MetricSummary, 
/**
 * The lines, at most a handful.
 */
series: Array<ProviderSeries>, };
export type MetricUnit = "count" | "milliseconds" | "percent" | "bytes" | "cores" | "seconds";
export type MetricSummary = "total" | "average" | "latest";
export type ProviderSeries = { 
/**
 * What the line is, such as "5xx" or "p95".
 */
name: string, 
/**
 * Its points, oldest first.
 */
points: Array<ProviderPoint>, };
export type ProviderPoint = { 
/**
 * When, RFC 3339.
 */
at: string, 
/**
 * The value, in the chart's unit.
 */
value: number, };
export type ProviderErrorCount = { 
/**
 * The HTTP status.
 */
status: number, 
/**
 * The HTTP method.
 */
method: string, 
/**
 * The path, without its query string.
 */
path: string, 
/**
 * How many times.
 */
count: number, };
export type ProviderRequest = { 
/**
 * RFC 3339 time.
 */
at: string, 
/**
 * The HTTP method.
 */
method: string, 
/**
 * The path, without its query string (it may carry secrets).
 */
path: string, 
/**
 * The HTTP status.
 */
status: number, 
/**
 * Total time in milliseconds.
 */
duration_ms: number | null, 
/**
 * The provider's request id, for support.
 */
request_id: string, };
export type ProviderVolume = { 
/**
 * The provider's id, for removing it.
 */
id: string, 
/**
 * Its name at the provider.
 */
name: string, 
/**
 * Where it is mounted, such as `/data`.
 */
mount_path: string | null, 
/**
 * The provider's id for this volume on this resource, which backups use.
 */
instance_id: string | null, 
/**
 * Space used now, in MB, when the provider reports it.
 */
used_mb: bigint | null, 
/**
 * Space it has, in MB, when the provider reports it.
 */
size_mb: bigint | null, 
/**
 * Where it is, such as `europe-west4`.
 */
region: string | null, };
export type ProviderTcpProxy = { 
/**
 * The provider's id, for removing it.
 */
id: string, 
/**
 * Connect to `domain:proxy_port`.
 */
domain: string, 
/**
 * The public port.
 */
proxy_port: number, 
/**
 * The port it listens on.
 */
application_port: number, };
export type VariableAddress = { 
/**
 * The variable's name, such as `SUPABASE_URL`.
 */
variable: string, 
/**
 * The host name in the address.
 */
host: string, 
/**
 * The user name in the address, when it has one.
 */
user: string | null, };
export type ProviderQuickAction = { 
/**
 * The button's words, such as `Apply migrations`.
 */
label: string, 
/**
 * The kind of resource it's for; none for every kind.
 */
on: ResourceKind | null, 
/**
 * The operation it runs, from the provider's operation list.
 */
operation: string, 
/**
 * What it does, in a sentence.
 */
description: string, 
/**
 * Its arguments.
 */
args: Record<string, unknown>, 
/**
 * Arguments for a first run that changes nothing (a dry run), where
 * the operation has one; shown before the real run.
 */
try_first: Record<string, unknown> | null, 
/**
 * Only for resources whose provider reference starts with one of these
 * (`pod:` on Runpod); every resource of its kind when empty.
 */
refs?: Array<string>, };
export type ProviderDomainState = "working" | "pending" | "failed";
export type ProviderDnsRecord = { 
/**
 * Record type, such as `CNAME`.
 */
kind: string, 
/**
 * Full name the record goes on.
 */
name: string, 
/**
 * The value it must have.
 */
value: string, };
export type ProviderDnsCheck = { 
/**
 * Record type, such as `CNAME`.
 */
kind: string, 
/**
 * Full name the record goes on.
 */
name: string, 
/**
 * The value it must have.
 */
value: string, 
/**
 * What the DNS host serves now, when anything.
 */
current: string | null, 
/**
 * Whether it matches.
 */
ok: boolean, };
export type ProjectListQuery = { 
/**
 * Case-insensitive substring of the project name, at most 80 characters.
 */
q: string | null, 
/**
 * Include projects with a resource on this provider.
 */
provider: Provider | null, 
/**
 * Filter by observed aggregate state.
 */
status: ResourceStatus | null, 
/**
 * Managed or watching projects.
 */
mode: Mode | null, 
/**
 * Only projects in this environment (its id).
 */
environment: string | null, 
/**
 * Ordering; newest by default.
 */
sort: ProjectSort, 
/**
 * The `next_cursor` of the page before, passed back as it is.
 */
cursor: string | null, };
export type ProjectSort = "newest" | "name";
export type ProjectStats = { 
/**
 * Seven UTC days ending today, including today's partial day.
 */
traffic: Array<DailyTraffic>, 
/**
 * Seven-day request total; null if any day or request-serving resource is missing.
 */
requests: number | null, 
/**
 * Errors divided by requests, as a fraction. Null for missing data or zero requests.
 */
error_rate: number | null, 
/**
 * Milliseconds. Never inferred from a median or averaged daily percentiles.
 */
p99: number | null, 
/**
 * UTC date of the latest recorded usage observation.
 */
last_observed_day: string | null, };
export type SourceGit = { 
/**
 * `owner/name` on GitHub.
 */
repository: string, 
/**
 * The branch checked out, when one is.
 */
branch: string | null, 
/**
 * The commit checked out.
 */
commit: string | null, 
/**
 * True for a folder the system clears on its own, such as /tmp.
 */
temporary: boolean, };
export type DailyTraffic = { 
/**
 * UTC calendar day.
 */
day: string, 
/**
 * Sum across request-serving resources, or null when coverage is incomplete.
 */
requests: number | null, 
/**
 * Sum across the same resources, or null when unavailable.
 */
errors: number | null, };
export type Resource = { 
/**
 * Its id.
 */
id: string, 
/**
 * Its project, or null until it's put in one.
 */
project_id: string | null, 
/**
 * What it is.
 */
kind: ResourceKind, 
/**
 * Provider adapter.
 */
provider: Provider, 
/**
 * Connected account id.
 */
account_id: string, 
/**
 * What the provider calls it; Hoista doesn't read anything into it.
 */
provider_ref: string, 
/**
 * Display name.
 */
name: string, 
/**
 * Its region, when the provider has regions.
 */
region: string | null, 
/**
 * How it's doing, as last checked.
 */
status: ResourceStatus, 
/**
 * Managed or watched.
 */
mode: Mode, 
/**
 * Its public address, when it has one.
 */
url: string | null, 
/**
 * The provider's grouping it belongs to, by name: a Railway project, for
 * example. Null when the provider has none or it isn't known yet.
 */
group: string | null, };
export type ResourceDetail = { 
/**
 * The resource, with the status the provider last reported.
 */
resource: Resource, 
/**
 * Owning project; null for an ungrouped discovery candidate.
 */
project: Project | null, 
/**
 * Its account, without credentials.
 */
account: Account, 
/**
 * Outgoing edges: resources this resource depends on.
 */
dependencies: Array<ResourceRelationship>, 
/**
 * Incoming edges: recorded consumers of this resource.
 */
used_by: Array<ResourceRelationship>, 
/**
 * Recorded custom domains attached to this resource.
 */
domains: Array<CustomDomain>, 
/**
 * The latest recorded day of usage (UTC); not a month-to-date total.
 */
latest_usage: Usage | null, 
/**
 * The latest deployment that released this resource.
 */
latest_recorded_release: RecordedResourceRelease | null, 
/**
 * Always false: links at the provider are not all discovered yet.
 */
relationships_complete: boolean, };
export type ResourceRelationship = { 
/**
 * The link: direction, binding, where it came from, and whether it is still seen.
 */
link: Link, 
/**
 * The resource at the other end.
 */
resource: Resource, 
/**
 * Its project, if it is in one.
 */
project: Project | null, };
export type RecordedResourceRelease = { 
/**
 * The deployment that made this release.
 */
deploy_id: string, 
/**
 * The project's deployment number, counting failed ones.
 */
attempt_version: number, 
/**
 * How the deployment ended; it can have failed after this resource was released.
 */
status: DeployStatus, 
/**
 * Person or agent that started the attempt.
 */
started_by: string, 
/**
 * RFC 3339 start time.
 */
started_at: string, 
/**
 * RFC 3339 finish time, if finished.
 */
finished_at: string | null, 
/**
 * The provider's release recorded for this resource.
 */
release: ResourceRelease, };
export type LogHistory = { 
/**
 * Up to 100 lines, with secrets taken out.
 */
items: Array<LogEntry>, 
/**
 * Pass this back for the next page. The last page can be empty.
 */
next_cursor: string | null, 
/**
 * What share of lines the provider searched, when it says.
 */
query_sampling_level: number | null, 
/**
 * Always `false`: sampling, expiry and turned-off logging can leave lines out.
 */
coverage_complete: boolean, };
export type LogHistoryQuery = { 
/**
 * From this time (RFC 3339), no earlier than 1970.
 */
from: string, 
/**
 * To this time (RFC 3339): after `from`, and at most 31 days later.
 */
to: string, 
/**
 * Text to look for, matched as written. At most 256 bytes.
 */
query: string, 
/**
 * From the last page's `next_cursor`, with the same resource and times.
 */
cursor: string | null, };
export type SourceProvenance = { 
/**
 * The commit at `HEAD`, or `null` for a folder outside Git or a repository with no commits.
 */
git_commit: string | null, 
/**
 * Whether the project's folder had changes not yet committed. `null` outside Git.
 */
git_dirty: boolean | null, 
/**
 * SHA-256 of the list of copied files (paths, contents and whether each is executable).
 * Not a Git tree hash.
 */
snapshot_sha256: string, 
/**
 * Which version of that list's format the hash was taken over.
 */
snapshot_format: string, 
/**
 * How many files were copied. `.env` and generated files aren't copied or counted.
 */
snapshot_files: number, };
export type SourceObservation = { "kind": "local_git", 
/**
 * The commit at `HEAD`, or `null` when there are no commits yet.
 */
commit: string | null, 
/**
 * Whether the project's folder had uncommitted changes.
 */
dirty: boolean, } | { "kind": "local_folder" } | { "kind": "remote_source" } | { "kind": "unavailable" };
export type DiscoverySummary = { 
/**
 * Completion time of this scan attempt.
 */
observed_at: string, 
/**
 * Whether all supported inventory families completed successfully.
 */
complete: boolean, 
/**
 * Number observed in this scan; null if it did not finish.
 */
observed_resources: number | null, 
/**
 * Coverage gaps or a generic failed-scan explanation.
 */
warnings: Array<DiscoveryWarning>, };
export type DiscoveryResult = { 
/**
 * Resources observed and saved locally without changing provider resources.
 */
items: Array<Resource>, 
/**
 * True only when every supported discovery family was inspected successfully.
 */
complete: boolean, 
/**
 * Inaccessible families; absence is not proof that those resources do not exist.
 */
warnings: Array<DiscoveryWarning>, };
export type DiscoveryWarning = { 
/**
 * Provider resource family, such as workers or pages.
 */
resource_type: string, 
/**
 * Stable machine-readable failure code.
 */
code: string, 
/**
 * Safe explanation without provider response bodies.
 */
message: string, };
export type Link = { 
/**
 * Its id.
 */
id: string, 
/**
 * The resource that uses the other.
 */
from_resource: string, 
/**
 * The resource it uses.
 */
to_resource: string, 
/**
 * How: the variable, route or binding that connects them.
 */
via: string, 
/**
 * Whether Hoista found it at the provider, rather than the person drawing it.
 */
derived: boolean, 
/**
 * Whether a link Hoista found earlier has since gone.
 */
missing: boolean, };
export type DeployStep = { 
/**
 * The step's name, such as `build`.
 */
name: string, 
/**
 * Where it stands.
 */
status: StepStatus, 
/**
 * What happened, with secrets taken out.
 */
message: string | null, };
export type DeployChannel = "desktop" | "cli" | "mcp";
export type DeployOptions = { 
/**
 * A UUID that makes it safe to retry: sending the same key again returns
 * the first attempt. A deploy with a key must also give `expected_plan_revision`.
 */
request_key: string | null, 
/**
 * The plan revision the person looked at; refused if the plan has changed. Without it, the current plan is used.
 */
expected_plan_revision: number | null, 
/**
 * What the change is, in the person's words: 1 to 160 characters.
 */
title: string | null, 
/**
 * Which app asked. An agent's deploys are always recorded as MCP.
 */
channel: DeployChannel | null, };
export type EnvironmentVariable = { 
/**
 * It was changed in Hoista; the project's `.env` files haven't caught up, so they mustn't overwrite it.
 */
requires_source_sync: boolean, 
/**
 * The resource it's set on.
 */
resource_id: string, 
/**
 * Its name.
 */
name: string, 
/**
 * When it was last set.
 */
set_at: string, 
/**
 * Who set it last, when known.
 */
set_by: string | null, 
/**
 * The provider, when the provider sets it itself.
 */
managed_by: Provider | null, 
/**
 * Whether it's built into what browsers download, so anyone can read it.
 */
browser_visible: boolean, };
export type EnvironmentChange = { 
/**
 * The variable's name. Each name once per request.
 */
name: string, 
/**
 * Its new value, at most 16 KiB. Held in memory that's wiped after use.
 */
value: string, };
export type EnvironmentDeployRequest = { 
/**
 * A UUID that makes the request safe to retry: sending the same key with
 * the same values returns the first attempt.
 */
request_key: string | null, 
/**
 * The plan revision the person looked at. If the plan has changed since,
 * nothing is kept or queued.
 */
expected_plan_revision: number | null, 
/**
 * The deploy plan to use.
 */
plan_id: string, 
/**
 * 1 to 64 changes, with at most 64 KiB of values in all.
 */
changes: Array<EnvironmentChange>, 
/**
 * What the change is, in the person's words.
 */
title: string | null, 
/**
 * Which app sent it. Hoista Desktop when not given.
 */
channel: DeployChannel | null, };
export type ProviderFailureEvidence = { 
/**
 * The HTTP status.
 */
http_status: number, 
/**
 * The provider's id for the request, for their support, when it gives one.
 */
request_id: string | null, 
/**
 * The header that id came from.
 */
request_id_header: string | null, };
export type DeployFailure = { 
/**
 * The provider's last failed answer, when the failure was one.
 */
provider_response: ProviderFailureEvidence | null, 
/**
 * Variables the project needs that weren't set. Names only.
 */
missing_secrets: Array<string>, 
/**
 * The error code, such as `build.failed`.
 */
code: string, 
/**
 * What went wrong, in plain words, with secrets taken out.
 */
message: string, 
/**
 * The steps running when it failed; empty when it failed before starting.
 */
steps: Array<string>, 
/**
 * Whether it had started releasing, so something may have changed at the provider.
 */
release_started: boolean, 
/**
 * Whether undoing the partial release failed. `false` doesn't prove it was all undone.
 */
compensation_failed: boolean, };
export type Deploy = { 
/**
 * Why it failed; null when it didn't, or Hoista stopped before it could tell.
 */
failure: DeployFailure | null, 
/**
 * The variables given for this deploy only, by name. Their values are never saved.
 */
environment_keys: Array<string>, 
/**
 * What the change is, in the person's words. Never taken from a commit message.
 */
title: string | null, 
/**
 * Which app it was asked for from.
 */
channel: DeployChannel, 
/**
 * The agent's name when it asked, so renaming it later doesn't rewrite history.
 */
started_by_name: string | null, 
/**
 * The project's release number (v1, v2, …); null until it's live, and for failed deploys.
 */
release_version: number | null, 
/**
 * What the source copy held; null when copying didn't finish.
 */
provenance: SourceProvenance | null, 
/**
 * A first look at the source before it was copied; null while queued, or
 * when it stopped before looking.
 */
source_observation: SourceObservation | null, 
/**
 * Whether the person stopped it. It stops at the next point where stopping is safe.
 */
cancel_requested: boolean, 
/**
 * The plan it ran.
 */
plan_id: string, 
/**
 * What it released at each provider, to roll back to later.
 */
releases: Array<ResourceRelease>, 
/**
 * Its id.
 */
id: string, 
/**
 * The project.
 */
project_id: string, 
/**
 * Counts up with each deploy of the project, failed ones included.
 */
version: number, 
/**
 * The source it deployed.
 */
source: Source, 
/**
 * The person, or the agent's id.
 */
started_by: string, 
/**
 * Where it stands.
 */
status: DeployStatus, 
/**
 * Its steps, in order.
 */
steps: Array<DeployStep>, 
/**
 * The address it was checked to answer on.
 */
url: string | null, 
/**
 * When it started (RFC 3339, UTC).
 */
started_at: string, 
/**
 * When it finished.
 */
finished_at: string | null, 
/**
 * How long it took, in milliseconds.
 */
duration_ms: number | null, 
/**
 * For a rollback, the deploy it went back to.
 */
rollback_of: string | null, };
export type DeploymentRequest = { 
/**
 * The key, as a lowercase UUID. Each caller's keys are their own.
 */
request_key: string, 
/**
 * Who sent it.
 */
actor: string, 
/**
 * The project, kept even if the project has since been deleted.
 */
project_id: string, 
/**
 * The deploy it started, kept even if that deploy's history was deleted.
 */
deploy_id: string, 
/**
 * The deploy as it stands now, or null if its history was deleted. Null
 * still means the request was accepted.
 */
deployment: Deploy | null, };
export type EnvironmentKeyState = "not_initialized" | "available" | "missing" | "changed" | "unavailable";
export type EnvironmentKeyRecovery = { 
/**
 * Where the key stood when checked. It can change after.
 */
state: EnvironmentKeyState, 
/**
 * The keychain entry holding the key, to check or restore; null before setup.
 */
active_reference: string | null, 
/**
 * Keychain entries of setups that didn't finish, up to sixteen.
 */
pending_references: Array<string>, 
/**
 * How many unfinished setups the person retired. Their entries are kept.
 */
retired_attempts: number, 
/**
 * Pass this to `--retry-setup`, so it only acts on what this report
 * showed. Grants nothing on its own.
 */
review_token: string, 
/**
 * Whether `--retry-setup` can be used: there are unfinished setups and no key.
 */
retry_setup_allowed: boolean, 
/**
 * What to do next.
 */
next_step: string, };
export type Agent = { 
/**
 * Its id.
 */
id: string, 
/**
 * The name it connected with.
 */
name: string, 
/**
 * How it connects.
 */
transport: AgentTransport, 
/**
 * What it may do: `apps:read`, `apps:write`, `apps:create`.
 */
scopes: Array<string>, 
/**
 * When it last did anything (RFC 3339, UTC).
 */
last_seen: string, 
/**
 * Whether the person cut it off.
 */
revoked: boolean, 
/**
 * What it may touch beyond its scopes: which projects and environments,
 * tools it may never use, and when the grant ends.
 */
grant: AgentGrant, };
export type AgentGrant = { 
/**
 * Project ids it may touch; null means every project, including new ones.
 */
projects: Array<string> | null, 
/**
 * Hoista environment ids whose projects it may touch; null means every
 * environment, including new ones.
 */
environment_ids: Array<string> | null, 
/**
 * Tool names it may never call, whatever its scopes.
 */
deny: Array<string>, 
/**
 * RFC 3339 UTC end of the grant; null means no end date (inactivity
 * expiry still applies).
 */
expires_at: string | null, 
/**
 * Raised on every change, so a decision can name the policy it used.
 */
version: number, 
/**
 * Full access: what would need the person's OK (deleting or removing
 * anything, connecting an account) goes ahead without
 * asking. Off for "Ask for approval": Hoista asks in the agent first.
 */
without_asking: boolean, 
/**
 * Stricter rules by Hoista environment id: what it may do to projects
 * in that environment, on top of its access.
 */
environment_rules: { [key in string]?: ProductionRule }, 
/**
 * Narrower access for some resources, on top of everything above. An
 * exception only ever narrows; the strictest one that applies wins.
 */
exceptions: Array<AgentException>, };
export type AgentGrantInput = { 
/**
 * Project ids, or null for every project.
 */
projects: Array<string> | null, 
/**
 * Hoista environment ids, or null for every environment.
 */
environment_ids: Array<string> | null, 
/**
 * Tool names it may never call.
 */
deny: Array<string>, 
/**
 * RFC 3339 end, or null for none.
 */
expires_at: string | null, 
/**
 * What needs the person's OK goes ahead without asking each time.
 */
without_asking: boolean, 
/**
 * Stricter rules by Hoista environment id.
 */
environment_rules: { [key in string]?: ProductionRule }, 
/**
 * Narrower access for some resources.
 */
exceptions: Array<AgentException>, };
export type ProductionRule = "always_ask" | "view_only";
export type AgentException = { 
/**
 * A group of resources; set this or `resource_id`.
 */
group: ResourceGroup | null, 
/**
 * With a group: the project it applies in, or null for every project.
 * With a resource: the project it's in, for showing it.
 */
project_id: string | null, 
/**
 * One resource; set this or `group`.
 */
resource_id: string | null, 
/**
 * How far it narrows access.
 */
access: Restriction, };
export type ResourceGroup = "services" | "databases" | "storage" | "domains";
export type Restriction = "always_ask" | "view_only" | "no_access";
export type ServerRecord = { 
/**
 * The provider it runs on; none for one connected to by its address.
 */
provider: Provider | null, 
/**
 * The account it was made with; none for one connected to.
 */
account_id: string | null, 
/**
 * `https://host`.
 */
url: string, 
/**
 * The provider's reference for its service; none for one connected to.
 */
reference: string | null, 
/**
 * The published image it runs, pinned by digest; none when built from a repository.
 */
image?: string, 
/**
 * RFC 3339 UTC.
 */
created_at: string, };
export type Operation = { 
/**
 * Its id, for checking it.
 */
id: string, 
/**
 * The provider it was sent to.
 */
provider: Provider, 
/**
 * The provider account's id.
 */
account: string, 
/**
 * What kind of change, such as `create` or `set_variable`.
 */
kind: string, 
/**
 * What it was for, in plain words, with no secret values.
 */
target: string, 
/**
 * Where it stands.
 */
state: OperationState, 
/**
 * Whether repeating it could make a second one; such changes wait while
 * an identical one is unclear.
 */
repeats: boolean, 
/**
 * RFC 3339 UTC.
 */
started_at: string, 
/**
 * RFC 3339 UTC; when it finished or was checked.
 */
finished_at: string | null, 
/**
 * What the provider said when it didn't work, redacted.
 */
message: string | null, };
export type OperationState = "running" | "done" | "failed" | "unclear" | "happened" | "did_not_happen";
export type Webhook = { 
/**
 * Its id.
 */
id: string, 
/**
 * `https://…`, a public address.
 */
url: string, 
/**
 * Event kinds it gets, such as `deploy.succeeded`, or a family ending in
 * `.*` such as `deploy.*`. Empty means every kind.
 */
events: Array<string>, 
/**
 * Only this project's events, when set.
 */
project_id: string | null, 
/**
 * RFC 3339 UTC.
 */
created_at: string, };
export type DeliveryState = "pending" | "delivered" | "failed";
export type WebhookDelivery = { 
/**
 * Its id.
 */
id: string, 
/**
 * The webhook.
 */
webhook_id: string, 
/**
 * The event, as in `GET /events`.
 */
event_id: string, 
/**
 * The event's kind.
 */
event_kind: string, 
/**
 * Where it stands.
 */
state: DeliveryState, 
/**
 * Tries so far.
 */
attempts: number, 
/**
 * RFC 3339 UTC: the next try, while pending.
 */
next_attempt_at: string, 
/**
 * The receiver's last HTTP status, if it answered.
 */
last_status: number | null, 
/**
 * Why the last try didn't work, in plain words.
 */
last_error: string | null, };
export type BackupFile = { 
/**
 * Its name, such as `backup-20261104T150211Z.json`.
 */
name: string, 
/**
 * RFC 3339 UTC: when it was saved.
 */
at: string, 
/**
 * Its size.
 */
bytes: bigint, };
export type EnrollmentRequest = { 
/**
 * Request id.
 */
id: string, 
/**
 * The name the tool gave. Hoista cannot verify it.
 */
name: string, 
/**
 * How it connects.
 */
transport: AgentTransport, 
/**
 * Short code the bridge shows, to match the request with the tool.
 */
code: string, 
/**
 * RFC 3339 UTC.
 */
created_at: string, 
/**
 * RFC 3339 UTC; after this it can't be allowed.
 */
expires_at: string, 
/**
 * Where it stands.
 */
status: EnrollmentStatus, 
/**
 * Whether the tool can show Hoista's questions (MCP elicitation);
 * unknown when the bridge didn't say.
 */
can_ask: boolean | null, };
export type EnrollmentStatus = "pending" | "allowed" | "denied" | "expired" | "collected";
export type EnrollmentPoll = { 
/**
 * Where it stands.
 */
status: EnrollmentStatus, 
/**
 * The new credential, given once when allowed.
 */
credential: string | null, };
export type ConnectionAnswer = { 
/**
 * Allow or deny.
 */
allow: boolean, 
/**
 * What it may do when allowed; defaults to read only.
 */
scopes: Array<string> | null, 
/**
 * Which projects and environments it may touch; defaults to every project.
 */
grant: AgentGrantInput | null, 
/**
 * Keep the earlier connections from the same tool on this computer;
 * by default the new one replaces them.
 */
keep_earlier: boolean, };
export type Approval = { 
/**
 * Its id.
 */
id: string, 
/**
 * The agent asking.
 */
agent_id: string, 
/**
 * What it wants to do.
 */
action: GatedAction, 
/**
 * What it wants to do it to: a resource, or a hash of the exact operation.
 */
target: string, 
/**
 * For a delete, everything it would remove.
 */
deletion: DeletionPreview | null, 
/**
 * The agent's reason, shown to the person.
 */
reason: string, 
/**
 * When it asked.
 */
created_at: string, 
/**
 * When it lapses, 15 minutes later.
 */
expires_at: string, 
/**
 * Where it stands.
 */
decision: ApprovalDecision, };
export type DeletionResource = { 
/**
 * Its id in Hoista.
 */
id: string, 
/**
 * Its project, which can be another project when the resource is shared.
 */
project_id: string | null, 
/**
 * Its name.
 */
name: string, 
/**
 * What it is. A database's data is gone for good unless it's copied first.
 */
kind: ResourceKind, 
/**
 * Its provider.
 */
provider: Provider, 
/**
 * The account it's in.
 */
account_id: string, 
/**
 * What the provider calls it: exactly what will be deleted there.
 */
provider_ref: string, 
/**
 * Its public address, so the person sees what stops answering.
 */
url: string | null, };
export type DeletionDomain = { 
/**
 * Its id in Hoista.
 */
id: string, 
/**
 * The domain.
 */
hostname: string, 
/**
 * The resource it points at now.
 */
resource_id: string, 
/**
 * Its DNS zone at the provider, to check the domain is still the person's.
 */
zone_id: string, };
export type DeletionPreview = { 
/**
 * `DeleteProject` or `DeleteResource`.
 */
action: GatedAction, 
/**
 * The project or resource being deleted.
 */
target: string, 
/**
 * Its project.
 */
project_id: string, 
/**
 * The newest deploy when this was shown. A newer deploy means showing it again.
 */
latest_deploy_id: string | null, 
/**
 * The name the person types to confirm.
 */
confirmation_name: string, 
/**
 * What gets deleted, in the order it will be: databases last.
 */
resources: Array<DeletionResource>, 
/**
 * Resources that stay but depend on something being deleted.
 */
affected_resources: Array<DeletionResource>, 
/**
 * Every database on each database resource being deleted.
 */
databases: Array<DatabaseTarget>, 
/**
 * Custom domains that stop pointing anywhere. The domains themselves stay.
 */
domains: Array<DeletionDomain>, 
/**
 * The map links that touch what's being deleted, including ones marked missing.
 */
links: Array<Link>, 
/**
 * Why it can't be deleted right now. None doesn't mean the data is saved anywhere.
 */
blockers: Array<string>, 
/**
 * Whether this build of Hoista can copy a database before deleting it.
 */
database_export_available: boolean, 
/**
 * A hash of all of the above. The delete is refused if anything has changed since.
 */
fingerprint: string, };
export type DeletionConfirmation = { 
/**
 * Copy the databases before deleting. On unless the person turns it off,
 * which only they can, and only when deleting a whole project.
 */
backup_databases: boolean, 
/**
 * The name they typed, exactly as shown.
 */
confirmation_name: string, 
/**
 * The preview's `fingerprint`.
 */
fingerprint: string, };
export type DatabaseArchive = { 
/**
 * The copy's id.
 */
id: string, 
/**
 * The database resource it was taken from.
 */
resource_id: string, 
/**
 * The provider's branch the database is on.
 */
branch_id: string, 
/**
 * The database's name.
 */
database_name: string, 
/**
 * Where the file is on this computer. It holds the app's data, so treat it like the database.
 */
path: string, 
/**
 * Its size in bytes.
 */
bytes: number, 
/**
 * SHA-256 of the whole file.
 */
sha256: string, 
/**
 * When it finished (RFC 3339).
 */
completed_at: string, };
export type DatabaseObservation = { 
/**
 * When both calls had finished (RFC 3339).
 */
observed_at: string, 
/**
 * The branches that exist now. Deleted ones are left out.
 */
branches: Array<DatabaseBranch>, 
/**
 * The computes.
 */
computes: Array<DatabaseCompute>, 
/**
 * How many times it woke up, when the provider reports it.
 */
wake_count: number | null, };
export type DatabaseOperation = { 
/**
 * The provider's id. The same one can come back on the next page.
 */
id: string, 
/**
 * The branch, which may no longer exist.
 */
branch_id: string | null, 
/**
 * The compute, which may no longer exist.
 */
compute_id: string | null, 
/**
 * What was done, in the provider's words, such as `start_compute`.
 */
action: string, 
/**
 * How it went. `failed` can still be retried by the provider.
 */
status: string, 
/**
 * When the provider started it.
 */
created_at: string, 
/**
 * When it last changed, which isn't necessarily when it finished.
 */
updated_at: string, };
export type DatabaseOperationHistory = { 
/**
 * When the provider answered (RFC 3339).
 */
observed_at: string, 
/**
 * Up to 100 operations.
 */
items: Array<DatabaseOperation>, 
/**
 * Pass this to get the next page. `null` means the provider gave no next page.
 */
next_cursor: string | null, 
/**
 * How many computes finished starting on this page alone.
 */
completed_starts_in_page: number, 
/**
 * Always `false`: this is never the whole history.
 */
complete_history: boolean, };
export type DatabaseBranch = { 
/**
 * The provider's id for it.
 */
id: string, 
/**
 * Its name.
 */
name: string, 
/**
 * The branch it was made from, which may since have been deleted.
 */
parent_id: string | null, 
/**
 * The provider's word for its state. Whether it's running is on the compute.
 */
state: string, 
/**
 * Whether the provider marks it as the default branch.
 */
is_default: boolean | null, 
/**
 * How much data it holds, in bytes. Not what's billed for storage.
 */
logical_size_bytes: number | null, };
export type DatabaseCompute = { 
/**
 * The provider's id for it.
 */
id: string, 
/**
 * The branch it serves.
 */
branch_id: string, 
/**
 * The provider's kind, such as `read_write` or `read_only`.
 */
kind: string, 
/**
 * Its state. `idle` means asleep, not broken.
 */
state: string, 
/**
 * The state it's changing to, while it changes.
 */
pending_state: string | null, 
/**
 * Its region.
 */
region: string, 
/**
 * The fewest compute units it's set to use. A setting, not a measurement.
 */
min_compute_units: number | null, 
/**
 * The most compute units it's set to use. A setting, not a measurement.
 */
max_compute_units: number | null, 
/**
 * How long it waits idle before sleeping, in seconds. `0` is the
 * provider's default; `-1` means it never sleeps.
 */
suspend_timeout_seconds: number | null, };
export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogEntry = { 
/**
 * When it happened, as the provider says. `null` when it doesn't say.
 */
timestamp: string | null, 
/**
 * When Hoista received it.
 */
observed_at: string, 
/**
 * How serious it is.
 */
level: LogLevel, 
/**
 * Where in the resource it came from, such as the console or a request.
 */
source: string, 
/**
 * The text, at most 8 KiB.
 */
message: string, };
export type LogFrame = { "type": "ready", 
/**
 * When it connected.
 */
started_at: string, 
/**
 * Always false: live tails carry no history.
 */
historical: boolean, } | { "type": "log", 
/**
 * The log entry.
 */
entry: LogEntry, } | { "type": "end", 
/**
 * Why it ended, as a code.
 */
reason: string, 
/**
 * Whether the tail was deleted at the provider.
 */
cleanup_complete: boolean, };
export type LogCapture = { 
/**
 * Up to 100 redacted log entries.
 */
items: Array<LogEntry>, 
/**
 * Always false: this never fills in past logs.
 */
historical: boolean, 
/**
 * When the tail connected; null if it failed to connect.
 */
started_at: string | null, 
/**
 * When capture and cleanup at the provider finished.
 */
finished_at: string, 
/**
 * Why it ended, such as a limit, a disconnect or a missing permission.
 */
end_reason: string, 
/**
 * Whether the tail was deleted at the provider.
 */
cleanup_complete: boolean, 
/**
 * Always null: live captures can't be resumed.
 */
next_cursor: string | null, };
export type DatabaseIdentity = { 
/**
 * The provider's branch.
 */
branch_id: string, 
/**
 * The database's name on that branch.
 */
database_name: string, 
/**
 * The role that owns it, which Hoista connects as to make a copy.
 */
owner_name: string, };
export type DatabaseTarget = { 
/**
 * The resource.
 */
resource_id: string, 
/**
 * The database at the provider.
 */
database: DatabaseIdentity, };
export type SecretMetadata = { 
/**
 * It was changed in Hoista, and the project's `.env` files haven't caught up yet.
 */
requires_source_sync: boolean, 
/**
 * Who set it last; null when it came in with an import.
 */
set_by: string | null, 
/**
 * The provider sets this one itself, such as a database address. Not the person's to replace.
 */
managed_by: Provider | null, 
/**
 * The project.
 */
project_id: string, 
/**
 * The resource it's set on.
 */
resource_id: string, 
/**
 * Its name.
 */
name: string, 
/**
 * SHA-256 of the value.
 */
fingerprint: string, 
/**
 * When it was last set (RFC 3339, UTC).
 */
set_at: string, };
export type Usage = { 
/**
 * The resource.
 */
resource_id: string, 
/**
 * The day (UTC).
 */
day: string, 
/**
 * Requests, when the provider counts them.
 */
requests: number | null, 
/**
 * Errors, when the provider counts them.
 */
errors: number | null, 
/**
 * Seconds of compute, when the provider counts them.
 */
compute_seconds: number | null, 
/**
 * Bytes stored, when the provider says.
 */
storage_bytes: number | null, 
/**
 * Bytes sent out, when the provider says.
 */
egress_bytes: number | null, };
export type Settings = { 
/**
 * Whether agents may connect and make requests. Off pauses every agent.
 */
agents_enabled: boolean, 
/**
 * After how many days without use a resource counts as idle.
 */
idle_days: number, 
/**
 * After how many days without use an agent's access ends.
 */
agent_expiry_days: number, 
/**
 * Whether Hoista may check for new versions.
 */
check_updates: boolean, 
/**
 * Whether Hoista may prepare crash reports. The person sees each one before it's sent.
 */
crash_reports: boolean, 
/**
 * Also open Hoista's consent page on this computer's private network
 * address. Off unless the person turns it on; follows the setting and
 * the current network while Hoista runs.
 */
network_page: boolean, 
/**
 * Serve MCP over HTTP on `127.0.0.1:8788`, for web agents such as
 * ChatGPT through a tunnel or proxy the person runs. On unless the person
 * turns it off; nothing outside this computer reaches it without one.
 */
mcp_http: boolean, 
/**
 * The access Hoista's consent page starts on for a new agent: `view`,
 * `ask` or `full`. The person still picks it on the page.
 */
default_access: string, 
/**
 * Build projects even where they can't be isolated from the rest of this
 * computer. Off unless the person turns it on.
 */
unisolated_builds: boolean, 
/**
 * Hours between automatic backups; 0 means none.
 */
backup_every_hours: number, 
/**
 * How many automatic backups are kept.
 */
backup_keep: number, 
/**
 * Days of activity, finished operations and webhook deliveries kept; at least 30.
 */
history_days: number, 
/**
 * Hoista put on a server from here, if any.
 */
server: ServerRecord | null, };
export type SettingsPatch = { 
/**
 * Allow or temporarily pause agent connections and requests.
 */
agents_enabled: boolean | null, 
/**
 * Idle suggestion threshold in days.
 */
idle_days: number | null, 
/**
 * Inactivity expiry in days.
 */
agent_expiry_days: number | null, 
/**
 * Let Hoista check for new versions.
 */
check_updates: boolean | null, 
/**
 * Allow local crash-report preparation.
 */
crash_reports: boolean | null, 
/**
 * Open the consent page on the private network address too.
 */
network_page: boolean | null, 
/**
 * Serve MCP over HTTP on this computer.
 */
mcp_http: boolean | null, 
/**
 * The access the consent page starts on: `view`, `ask` or `full`.
 */
default_access: string | null, 
/**
 * Build projects even where they can't be isolated.
 */
unisolated_builds: boolean | null, 
/**
 * Hours between automatic backups: 0, 6, 24 or 168.
 */
backup_every_hours: number | null, 
/**
 * How many automatic backups to keep, 1 to 100.
 */
backup_keep: number | null, 
/**
 * Days of history kept: 30, 90, 365 or 1095.
 */
history_days: number | null, };
export type Capabilities = { 
/**
 * Deploys project code.
 */
deploy: boolean, 
/**
 * Reads logs.
 */
logs: boolean, 
/**
 * Reads metrics.
 */
metrics: boolean, 
/**
 * Reads daily usage.
 */
usage: boolean, 
/**
 * Rolls back to an earlier release.
 */
rollback: boolean, 
/**
 * Pauses and resumes resources.
 */
pause: boolean, 
/**
 * Manages custom domains.
 */
domains: boolean, 
/**
 * Sets environment variables and secrets.
 */
secrets: boolean, 
/**
 * Lists existing resources in the account.
 */
discover: boolean, 
/**
 * Can run Hoista itself as a remote, from the published image.
 */
server: boolean, 
/**
 * Reads a watched resource's settings, deployments and addresses as the
 * provider keeps them (the provider tab).
 */
provider_details: boolean, 
/**
 * Lists the provider's deployments of a watched resource, and redeploys or
 * rolls back through the provider's own actions.
 */
provider_deploys: boolean, 
/**
 * Reads and changes a watched resource's variables at the provider.
 */
provider_variables: boolean, 
/**
 * Runs the provider's own API operations by name (see `provider_operations`).
 */
provider_operations: boolean, 
/**
 * Uploads a folder for the provider to build and deploy.
 */
upload: boolean, 
/**
 * Creates a new project at the provider from a repository.
 */
new_project: boolean, 
/**
 * Creates a new project at the provider from a folder on this computer,
 * with the provider's own command-line tool.
 */
new_from_folder: boolean, 
/**
 * Creates a database of its own at the provider (a Postgres project, or a
 * database next to a project's services).
 */
new_database: boolean, 
/**
 * Creates an object storage bucket at the provider.
 */
new_bucket: boolean, 
/**
 * Adds a domain to send email from.
 */
new_email_domain: boolean, 
/**
 * Runs a container image as a new resource (a service, or a GPU pod).
 */
new_from_image: boolean, 
/**
 * Reports what the provider has billed so far this period.
 */
bills: boolean, 
/**
 * Connects by signing in on the provider's own page, instead of a pasted token.
 */
browser_sign_in: boolean, 
/**
 * Deletes a resource at the provider (a provider project, service, Worker, bucket or
 * domain), when the person confirms it by name.
 */
delete: boolean, 
/**
 * Hoista builds a project folder and deploys it here from a plan
 * (`plan_deploy`).
 */
plan_deploy: boolean, 
/**
 * Runs what Hoista deploys as a long-running container built from the
 * project's Dockerfile, which Hoista writes for a Node project without one.
 */
containers: boolean, 
/**
 * Runs a deployed web or job resource on exact UTC cron schedules.
 */
cron: boolean, 
/**
 * Where Hoista creates a project's Postgres when its plan needs one.
 */
project_database: boolean, 
/**
 * Creates the DNS zone for a domain it adds, when asked to.
 */
dns_zones: boolean, };
export type VaultPlace = "keychain" | "sealed_file";
export type VaultInfo = { 
/**
 * Where entries are kept.
 */
kept_in: VaultPlace, 
/**
 * Where each provider token itself lives (`built-in`: in that place).
 */
token_store: string, 
/**
 * Where the key that seals the file comes from, on a remote.
 */
key_source?: string, 
/**
 * A token may be a 1Password reference (`op://…`), read when needed.
 */
references: boolean, };
export type ResourcePlan = { 
/**
 * Cron schedules (UTC) to set. Null leaves the provider's alone; an empty list removes them.
 */
schedules: Array<string> | null, 
/**
 * The resource it deploys over, when there is one. Hoista fills this in.
 */
provider_ref: string | null, 
/**
 * Resource name.
 */
name: string, 
/**
 * Resource type.
 */
kind: ResourceKind, 
/**
 * Selected provider.
 */
provider: Provider, 
/**
 * Connected account to use.
 */
account_id: string, 
/**
 * Requested location.
 */
region: string | null, 
/**
 * Names of resources released first.
 */
depends_on: Array<string>, };
export type Plan = { 
/**
 * Goes up with every edit. Send back the one you read; an edit made on an older one is refused.
 */
revision: number, 
/**
 * Plan id.
 */
id: string, 
/**
 * Source analyzed.
 */
source: Source, 
/**
 * Detected framework.
 */
framework: string, 
/**
 * Detected package manager.
 */
package_manager: string | null, 
/**
 * Executable and arguments; never evaluated by a shell.
 */
build_command: Array<string>, 
/**
 * Relative directory containing public assets.
 */
output_directory: string, 
/**
 * Required environment names only.
 */
env_names: Array<string>, 
/**
 * Authentication names generated in memory for new resources when no value was supplied.
 */
generated_secret_names: Array<string>, 
/**
 * Optional checked-in database migration workflow; never run by code rollback.
 */
prisma_migration: PrismaMigration | null, 
/**
 * Planned resources in dependency order.
 */
resources: Array<ResourcePlan>, 
/**
 * Concrete deployment constraints.
 */
warnings: Array<string>, };
export type PrismaMigration = { 
/**
 * Relative schema file inside the source snapshot.
 */
schema: string, 
/**
 * Relative directory containing migration history.
 */
migrations: string, };
export type Source = { "kind": "unlinked" } | { "kind": "folder", 
/**
 * Absolute path to the project source.
 */
path: string, } | { "kind": "github", 
/**
 * Owner/repository, without credentials or a URL query.
 */
repository: string, 
/**
 * Full commit SHA.
 */
commit: string, };
export type Release = { 
/**
 * The cron schedules to put back with this release; null leaves the current ones alone.
 */
schedules: Array<string> | null, 
/**
 * Provider resource reference.
 */
reference: string, 
/**
 * The provider's version id or image digest, which never changes.
 */
version: string, 
/**
 * Public URL, if applicable.
 */
url: string | null, };
export type ResourceRelease = { 
/**
 * Local resource identifier.
 */
resource_id: string, 
/**
 * Planned resource name.
 */
name: string, 
/**
 * What the provider released.
 */
release: Release, };
export type Backup = { 
/**
 * The backup format's version.
 */
version: number, 
/**
 * Connected accounts, without their tokens. Each one is reconnected after a restore.
 */
accounts: Array<Account>, 
/**
 * Projects and where their source is.
 */
projects: Array<Project>, 
/**
 * The resources in those projects.
 */
resources: Array<Resource>, 
/**
 * The links between resources on the project map.
 */
links: Array<Link>, 
/**
 * Custom domains and the resources they point at.
 */
custom_domains: Array<CustomDomain>, 
/**
 * Settings.
 */
settings: Settings, 
/**
 * Environments other than `default`.
 */
environments: Array<Environment>, };
export type ProfileDto = { 
/**
 * Display name, 1–80 characters after trimming.
 */
name: string, 
/**
 * Avatar initials, 1–4 characters after trimming.
 */
initials: string, };
export type HealthDto = { 
/**
 * Daemon build version, checked against the CLI before enrollment.
 */
version: string, 
/**
 * Process identity for owner-local diagnostics.
 */
pid: number, };
export type DaemonStatus = { 
/**
 * Installed daemon version.
 */
version: string, 
/**
 * Current process ID, for diagnostics only.
 */
pid: number, 
/**
 * Random process instance identity used to reject stale restart requests.
 */
instance_id: string, 
/**
 * Launched in service-manager mode; the manager is responsible for replacement after exit.
 */
supervised: boolean, 
/**
 * An owner restart has been accepted; completion still requires a new instance.
 */
restart_requested: boolean, 
/**
 * The exact command a coding tool runs to reach this daemon over MCP:
 * the `hoist` binary beside this daemon, its state folder when it is not
 * the default one, then `mcp`. Empty when the binary can't be found.
 */
mcp_command: Array<string>, };
export type RestartRequest = { 
/**
 * Instance identity returned by daemon status.
 */
instance_id: string, 
/**
 * Owner has arranged a service manager to take over an independently started daemon.
 */
handoff: boolean, };
export type RetryDisposition = "inspect_first" | "after_correction" | "owner_decision";
export type RecoveryAdvice = { 
/**
 * Recovery prerequisite, not a guarantee that replay is idempotent.
 */
retry: RetryDisposition, 
/**
 * Plain-language action without provider bodies, paths or submitted values.
 */
next_step: string, };
export type ErrorDto = { 
/**
 * Final failed provider response metadata, if known.
 */
provider_response: ProviderFailureEvidence | null, 
/**
 * Required environment names when known; absent in older responses.
 */
missing_secrets: Array<string>, 
/**
 * Machine-readable reason.
 */
code: string, 
/**
 * Plain-language next step.
 */
message: string, 
/**
 * Conservative recovery advice, when there is any.
 */
recovery?: RecoveryAdvice, };
export type NetworkOutcome = "in_flight" | "response" | "transport_failure" | "cancelled";
export type HostObservation = { 
/**
 * The host name, such as a provider's API or a deployed app's address.
 */
host: string, 
/**
 * When the latest request started (RFC 3339, UTC).
 */
last_attempt_at: string, 
/**
 * Requests since Hoista started or this entry was last dropped.
 */
attempts: number, 
/**
 * How the latest request went.
 */
outcome: NetworkOutcome, 
/**
 * The latest HTTP status, when there was one.
 */
http_status: number | null, };
export type NetworkReport = { 
/**
 * The provider API hosts Hoista may call, whether or not it has.
 */
allowed_api_hosts: Array<string>, 
/**
 * Up to 64 hosts contacted in the last 24 hours.
 */
recent_hosts: Array<HostObservation>, 
/**
 * What the report leaves out, and how long it keeps entries.
 */
limitations: Array<string>, };
export type DiagnosticReport = { 
/**
 * The report's format.
 */
format_version: number, 
/**
 * The version of Hoista that made it.
 */
engine_version: string, 
/**
 * When the report was made. Not when any provider was last checked.
 */
generated_at: string, 
/**
 * The operating system and processor Hoista was built for.
 */
platform: string, 
/**
 * What the report leaves out, and how to read it.
 */
limitations: Array<string>, 
/**
 * Sections that couldn't be read. An empty list there doesn't mean there was nothing.
 */
collection_errors: Array<DiagnosticIssue>, 
/**
 * Up to 100 accounts, without their names, provider ids or scopes.
 */
accounts: Array<DiagnosticAccount>, 
/**
 * Whether there were more accounts than that.
 */
accounts_truncated: boolean, 
/**
 * Up to 100 resources, without their provider ids or addresses.
 */
resources: Array<DiagnosticResource>, 
/**
 * Whether there were more resources than that.
 */
resources_truncated: boolean, 
/**
 * Up to 100 projects, each with up to its ten newest deploys.
 */
projects: Array<DiagnosticProject>, 
/**
 * Whether there were more projects than that.
 */
projects_truncated: boolean, };
export type DiagnosticIssue = { 
/**
 * `accounts`, `resources`, `projects` or `deployments`.
 */
section: string, 
/**
 * The project, when only its deploys couldn't be read.
 */
reference: string | null, 
/**
 * What went wrong, as a code.
 */
code: string, };
export type DiagnosticAccount = { 
/**
 * Its reference in this report.
 */
reference: string, 
/**
 * Its provider.
 */
provider: Provider, 
/**
 * Its status when last saved.
 */
status: AccountStatus, };
export type DiagnosticResource = { 
/**
 * Its reference in this report.
 */
reference: string, 
/**
 * Its project's reference, when it's in one.
 */
project: string | null, 
/**
 * Its account's reference.
 */
account: string, 
/**
 * Its provider.
 */
provider: Provider, 
/**
 * What it is.
 */
kind: ResourceKind, 
/**
 * Its status when last saved.
 */
status: ResourceStatus, 
/**
 * Whether Hoista manages it or only watches it.
 */
mode: Mode, };
export type DiagnosticProject = { 
/**
 * Its reference in this report. Never its name or folder.
 */
reference: string, 
/**
 * Whether Hoista manages it or only watches it.
 */
mode: Mode, 
/**
 * Its newest deploys.
 */
deployments: Array<DiagnosticDeploy>, 
/**
 * Whether it has older deploys than these.
 */
older_deployments_omitted: boolean, };
export type DiagnosticDeploy = { 
/**
 * Its reference in this report.
 */
reference: string, 
/**
 * How it ended. Says nothing about whether it's healthy now.
 */
status: DeployStatus, 
/**
 * Which app started it, without saying who.
 */
channel: DeployChannel, 
/**
 * When it started (UTC); null when the saved time isn't valid.
 */
started_at: string | null, 
/**
 * Up to its first 16 steps.
 */
steps: Array<DiagnosticStep>, 
/**
 * Whether it had more steps than that.
 */
steps_truncated: boolean, 
/**
 * Why it failed, as a code; any code not on Hoista's list is `unclassified`.
 */
failure_code: string | null, 
/**
 * Whether the release had started when it failed; null when it didn't fail.
 */
release_started: boolean | null, 
/**
 * Whether undoing the failed deploy failed too. `false` doesn't prove
 * everything was put back. Null when it didn't fail.
 */
compensation_failed: boolean | null, };
export type DiagnosticStep = { 
/**
 * Its name, or `other` for a name not on Hoista's list.
 */
name: string, 
/**
 * How it went.
 */
status: StepStatus, };
export type DiagnosticReceipt = { 
/**
 * The saved file, for the person to look at or share.
 */
path: string, 
/**
 * Its size in bytes.
 */
bytes: number, 
/**
 * The SHA-256 of the file.
 */
sha256: string, };
export type EventDto = { 
/**
 * Where to resume from; also sent as the SSE event id.
 */
id: string, 
/**
 * RFC 3339 UTC time.
 */
at: string, 
/**
 * Responsible person or agent.
 */
actor: string, 
/**
 * Project identifier when an event belongs to a project.
 */
project_id: string | null, 
/**
 * The record the event is about: a deployment for `deploy.*`, an approval
 * for `approval.*`, and likewise for agents, resources, domains and
 * accounts. Null when no single record applies or the event predates
 * this field; never parse the summary instead.
 */
subject_id: string | null, 
/**
 * Stable event type, also sent in the SSE event field.
 */
kind: string, 
/**
 * Description with no submitted profile or credential values.
 */
summary: string, };
export type ActivityPage = { 
/**
 * At most 50 events.
 */
items: Array<EventDto>, 
/**
 * Pass as `cursor` for older events; null on the last page.
 */
next_cursor: string | null, };
export type CheckStatus = "ok" | "action_needed" | "not_checked";
export type DoctorCheck = { 
/**
 * Stable check identifier.
 */
name: string, 
/**
 * Result of this check.
 */
status: CheckStatus, 
/**
 * Human-readable diagnostic without secret values.
 */
message: string, };
export type DoctorReport = { 
/**
 * Bounded current-process HTTP observations.
 */
network: NetworkReport | null, 
/**
 * Bounded account verification page when an engine is available.
 */
accounts: Page<AccountDiagnostic> | null, 
/**
 * Engine version.
 */
version: string, 
/**
 * daemon or direct, depending on the state owner.
 */
mode: string, 
/**
 * Individual checks, including explicitly unchecked integrations.
 */
checks: Array<DoctorCheck>, };
