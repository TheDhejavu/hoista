//! Requests that change a project's variables, and reading back the ones a
//! browser can see anyway. A secret value is never sent back out.

use super::{Secret, model::DeployChannel};
use schemars::JsonSchema;
use serde::Deserialize;
use ts_rs::TS;

/// One variable to set across a project.
#[derive(Debug, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentChange {
    /// The variable's name. Each name once per request.
    pub name: String,
    /// Its new value, at most 16 KiB. Held in memory that's wiped after use.
    #[schemars(with = "String")]
    #[ts(type = "string")]
    pub value: Secret,
}

/// Set variables across every running resource in a project, then deploy.
#[derive(Debug, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDeployRequest {
    /// A UUID that makes the request safe to retry: sending the same key with
    /// the same values returns the first attempt.
    pub request_key: Option<String>,
    /// The plan revision the person looked at. If the plan has changed since,
    /// nothing is kept or queued.
    #[ts(type = "number | null")]
    pub expected_plan_revision: Option<i64>,
    /// The deploy plan to use.
    pub plan_id: String,
    /// 1 to 64 changes, with at most 64 KiB of values in all.
    pub changes: Vec<EnvironmentChange>,
    /// What the change is, in the person's words.
    pub title: Option<String>,
    /// Which app sent it. Hoista Desktop when not given.
    pub channel: Option<DeployChannel>,
}

/// Read the public variables (the ones built into what browsers download)
/// from the project's source, to fill them in for the person.
#[derive(Debug, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PublicEnvironmentRequest {
    /// The plan for the project's source on this computer.
    pub plan_id: String,
    /// The plan revision on screen. A changed plan is refused.
    #[ts(type = "number")]
    pub expected_plan_revision: i64,
}

/// A public variable's value from the source. Deliberately has no `Debug`, so
/// it can't end up in a log.
#[derive(serde::Serialize, JsonSchema, TS)]
pub struct PublicEnvironmentValue {
    /// A name the plan lists as public.
    pub name: String,
    /// Its value in the source, which passed the secret check.
    pub value: String,
    /// Whether it matches what was last deployed, as far as Hoista recorded it.
    pub matches_recorded_value: bool,
}

/// Why a public variable can't be filled in from the source.
#[derive(Debug, serde::Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PublicEnvironmentUnavailableReason {
    /// The source doesn't set it.
    MissingFromSource,
    /// The source sets it to something other than what was deployed.
    SourceDiffersFromRecorded,
    /// The provider sets it itself.
    ManagedBinding,
}

/// A public variable left empty, and why. Never its value.
#[derive(Debug, serde::Serialize, JsonSchema, TS)]
pub struct PublicEnvironmentUnavailable {
    /// Its name.
    pub name: String,
    /// Why it's left empty.
    pub reason: PublicEnvironmentUnavailableReason,
}

/// The public variables as the source has them. Not proof of what's running:
/// Hoista didn't read them back from the provider or the built site.
#[derive(serde::Serialize, JsonSchema, TS)]
pub struct PublicEnvironmentPrefill {
    /// The plan read.
    pub plan_id: String,
    /// Its revision.
    #[ts(type = "number")]
    pub plan_revision: i64,
    /// Always `true`: these come from the source only.
    pub source_only: bool,
    /// The values found. Private variables are never here.
    pub values: Vec<PublicEnvironmentValue>,
    /// The ones left empty, and why.
    pub unavailable: Vec<PublicEnvironmentUnavailable>,
}
