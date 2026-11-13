//! The diagnostic report the person can save and share for support. Built
//! from what Hoista has saved only: no provider is called, and no token,
//! name, address or log text goes in.

use crate::ports::catalog::{CatalogRepository, EngineError, Page};
use crate::{Provider, domain::model::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

const FORMAT_VERSION: u32 = 1;
/// The most accounts, resources and projects in one report.
const MAX_ITEMS: usize = 100;
const MAX_DEPLOYS_PER_PROJECT: usize = 10;
const MAX_STEPS_PER_DEPLOY: usize = 16;
/// Hex characters of hash kept in each reference.
const REFERENCE_HASH_CHARS: usize = 24;

/// The step names that go into a report; any other becomes `other`.
const KNOWN_STEPS: [&str; 8] = [
    "detect",
    "plan",
    "secrets",
    "leak_check",
    "build",
    "release",
    "verify",
    "automatic_rollback",
];

/// The failure codes that go into a report; any other becomes `unclassified`.
const KNOWN_FAILURES: [&str; 18] = [
    "request.invalid",
    "state.conflict",
    "state.unavailable",
    "app.watching",
    "deploy.secret_missing",
    "deploy.secret_leak",
    "deploy.cancelled",
    "deploy.environment_input_lost",
    "deploy.environment_source_stale",
    "deploy.health_failed",
    "provider.timeout",
    "provider.rate_limited",
    "provider.token_expired",
    "provider.token_scope",
    "provider.not_supported",
    "provider.invalid_response",
    "provider.rejected",
    "provider.cli_failed",
];

/// A support report. Every id in it is hashed with a salt made for this
/// report alone, so it means nothing outside it.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticReport {
    /// The report's format.
    pub format_version: u32,
    /// The version of Hoista that made it.
    pub engine_version: String,
    /// When the report was made. Not when any provider was last checked.
    pub generated_at: String,
    /// The operating system and processor Hoista was built for.
    pub platform: String,
    /// What the report leaves out, and how to read it.
    pub limitations: Vec<String>,
    /// Sections that couldn't be read. An empty list there doesn't mean there was nothing.
    pub collection_errors: Vec<DiagnosticIssue>,
    /// Up to 100 accounts, without their names, provider ids or scopes.
    pub accounts: Vec<DiagnosticAccount>,
    /// Whether there were more accounts than that.
    pub accounts_truncated: bool,
    /// Up to 100 resources, without their provider ids or addresses.
    pub resources: Vec<DiagnosticResource>,
    /// Whether there were more resources than that.
    pub resources_truncated: bool,
    /// Up to 100 projects, each with up to its ten newest deploys.
    pub projects: Vec<DiagnosticProject>,
    /// Whether there were more projects than that.
    pub projects_truncated: bool,
}

/// A section that couldn't be read. No error text or paths.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticIssue {
    /// `accounts`, `resources`, `projects` or `deployments`.
    pub section: String,
    /// The project, when only its deploys couldn't be read.
    pub reference: Option<String>,
    /// What went wrong, as a code.
    pub code: String,
}

/// An account as last saved. Its token isn't checked again.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticAccount {
    /// Its reference in this report.
    pub reference: String,
    /// Its provider.
    pub provider: Provider,
    /// Its status when last saved.
    pub status: AccountStatus,
}

/// A resource as last saved, without the provider's ids.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticResource {
    /// Its reference in this report.
    pub reference: String,
    /// Its project's reference, when it's in one.
    pub project: Option<String>,
    /// Its account's reference.
    pub account: String,
    /// Its provider.
    pub provider: Provider,
    /// What it is.
    pub kind: ResourceKind,
    /// Its status when last saved.
    pub status: ResourceStatus,
    /// Whether Hoista manages it or only watches it.
    pub mode: Mode,
}

/// A project and its newest deployments.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticProject {
    /// Its reference in this report. Never its name or folder.
    pub reference: String,
    /// Whether Hoista manages it or only watches it.
    pub mode: Mode,
    /// Its newest deploys.
    pub deployments: Vec<DiagnosticDeploy>,
    /// Whether it has older deploys than these.
    pub older_deployments_omitted: bool,
}

/// One deploy: its steps' names and statuses, never log text.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticDeploy {
    /// Its reference in this report.
    pub reference: String,
    /// How it ended. Says nothing about whether it's healthy now.
    pub status: DeployStatus,
    /// Which app started it, without saying who.
    pub channel: DeployChannel,
    /// When it started (UTC); null when the saved time isn't valid.
    pub started_at: Option<String>,
    /// Up to its first 16 steps.
    pub steps: Vec<DiagnosticStep>,
    /// Whether it had more steps than that.
    pub steps_truncated: bool,
    /// Why it failed, as a code; any code not on Hoista's list is `unclassified`.
    pub failure_code: Option<String>,
    /// Whether the release had started when it failed; null when it didn't fail.
    pub release_started: Option<bool>,
    /// Whether undoing the failed deploy failed too. `false` doesn't prove
    /// everything was put back. Null when it didn't fail.
    pub compensation_failed: Option<bool>,
}

/// One deploy step, without its message.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticStep {
    /// Its name, or `other` for a name not on Hoista's list.
    pub name: String,
    /// How it went.
    pub status: StepStatus,
}

/// A reference to `id` that only means something within this report.
fn reference(salt: &str, kind: &str, id: &str) -> String {
    let hash = format!("{:x}", Sha256::digest(format!("{salt}:{kind}:{id}")));
    format!("{kind}-{}", &hash[..REFERENCE_HASH_CHARS])
}

fn step(name: &str) -> String {
    if KNOWN_STEPS.contains(&name) {
        name.into()
    } else {
        "other".into()
    }
}

fn code(value: &str) -> String {
    if KNOWN_FAILURES.contains(&value) {
        value.into()
    } else {
        "unclassified".into()
    }
}

/// The page, or an empty one with the failure noted in `issues`.
fn section<T>(
    result: Result<Page<T>, EngineError>,
    name: &str,
    reference: Option<String>,
    issues: &mut Vec<DiagnosticIssue>,
) -> (Page<T>, bool) {
    match result {
        Ok(page) => (page, false),
        Err(_) => {
            issues.push(DiagnosticIssue {
                section: name.into(),
                reference,
                code: "state.unavailable".into(),
            });
            (
                Page {
                    items: Vec::new(),
                    next_cursor: None,
                },
                true,
            )
        }
    }
}

/// Builds the report from saved state. Calls no provider and reads no
/// token; the caller should give up on it after a while. Sections that
/// can't be read are listed in the report.
pub async fn collect(repository: &dyn CatalogRepository) -> DiagnosticReport {
    let salt = uuid::Uuid::new_v4().to_string();
    let mut issues = Vec::new();
    let (accounts, accounts_failed) = section(
        repository.accounts(None).await,
        "accounts",
        None,
        &mut issues,
    );
    let (resources, resources_failed) = section(
        repository.resources(None).await,
        "resources",
        None,
        &mut issues,
    );
    let (projects, projects_failed) = section(
        repository.projects(None).await,
        "projects",
        None,
        &mut issues,
    );
    let truncated = |failed: bool, more: bool, count: usize| failed || more || count > MAX_ITEMS;
    let mut output = DiagnosticReport {
        format_version: FORMAT_VERSION,
        engine_version: crate::VERSION.into(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        limitations: vec![
            "Saved observations only; collection is not a transaction-wide snapshot or a live health probe.".into(),
            "Names, raw IDs, paths, URLs, actors, environment names/values, credentials, raw logs and error messages are excluded. References change for every report.".into(),
            "Deployment steps are a sanitized execution trail, not provider/build log content. Unknown failure codes are unclassified.".into(),
            "At most 100 projects/accounts/resources, ten attempts per project and 16 steps per attempt. Omitted history is explicitly marked. Reports are never uploaded automatically.".into(),
        ],
        collection_errors: issues,
        accounts_truncated: truncated(
            accounts_failed,
            accounts.next_cursor.is_some(),
            accounts.items.len(),
        ),
        accounts: accounts
            .items
            .into_iter()
            .take(MAX_ITEMS)
            .map(|account| DiagnosticAccount {
                reference: reference(&salt, "account", &account.id),
                provider: account.provider,
                status: account.status,
            })
            .collect(),
        resources_truncated: truncated(
            resources_failed,
            resources.next_cursor.is_some(),
            resources.items.len(),
        ),
        resources: resources
            .items
            .into_iter()
            .take(MAX_ITEMS)
            .map(|resource| DiagnosticResource {
                reference: reference(&salt, "resource", &resource.id),
                project: resource
                    .project_id
                    .map(|id| reference(&salt, "project", &id)),
                account: reference(&salt, "account", &resource.account_id),
                provider: resource.provider,
                kind: resource.kind,
                status: resource.status,
                mode: resource.mode,
            })
            .collect(),
        projects_truncated: truncated(
            projects_failed,
            projects.next_cursor.is_some(),
            projects.items.len(),
        ),
        projects: Vec::new(),
    };
    for project in projects.items.into_iter().take(MAX_ITEMS) {
        let (deploys, failed) = section(
            repository.deploys(&project.id, None).await,
            "deployments",
            Some(reference(&salt, "project", &project.id)),
            &mut output.collection_errors,
        );
        let omitted = failed
            || deploys.next_cursor.is_some()
            || deploys.items.len() > MAX_DEPLOYS_PER_PROJECT;
        let deployments = deploys
            .items
            .into_iter()
            .take(MAX_DEPLOYS_PER_PROJECT)
            .map(|deploy| DiagnosticDeploy {
                reference: reference(&salt, "deploy", &deploy.id),
                status: deploy.status,
                channel: deploy.channel,
                started_at: chrono::DateTime::parse_from_rfc3339(&deploy.started_at)
                    .ok()
                    .map(|at| at.with_timezone(&chrono::Utc).to_rfc3339()),
                steps_truncated: deploy.steps.len() > MAX_STEPS_PER_DEPLOY,
                steps: deploy
                    .steps
                    .into_iter()
                    .take(MAX_STEPS_PER_DEPLOY)
                    .map(|recorded| DiagnosticStep {
                        name: step(&recorded.name),
                        status: recorded.status,
                    })
                    .collect(),
                failure_code: deploy.failure.as_ref().map(|failure| code(&failure.code)),
                release_started: deploy
                    .failure
                    .as_ref()
                    .map(|failure| failure.release_started),
                compensation_failed: deploy
                    .failure
                    .as_ref()
                    .map(|failure| failure.compensation_failed),
            })
            .collect();
        output.projects.push(DiagnosticProject {
            reference: reference(&salt, "project", &project.id),
            mode: project.mode,
            deployments,
            older_deployments_omitted: omitted,
        });
    }
    output
}

/// Where a saved report is. The path itself isn't in the report.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct DiagnosticReceipt {
    /// The saved file, for the person to look at or share.
    pub path: String,
    /// Its size in bytes.
    pub bytes: u32,
    /// The SHA-256 of the file.
    pub sha256: String,
}
