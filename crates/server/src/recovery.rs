//! Conservative recovery guidance shared by owner API and MCP errors.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What must happen before the caller considers resubmitting an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum RetryDisposition {
    /// Read current local and provider state; the previous result may be uncertain.
    InspectFirst,
    /// Correct the input or prerequisite, then review current state before retrying.
    AfterCorrection,
    /// The owner must explicitly decide; an agent must not unblock itself.
    OwnerDecision,
}

/// Advice only: never an instruction to automatically replay a mutation.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct RecoveryAdvice {
    /// Recovery prerequisite, not a guarantee that replay is idempotent.
    pub retry: RetryDisposition,
    /// Plain-language action without provider bodies, paths or submitted values.
    pub next_step: String,
}

/// Maps stable codes, never raw messages, to conservative recovery instructions.
pub fn advice(code: &str) -> RecoveryAdvice {
    use RetryDisposition::{AfterCorrection, InspectFirst, OwnerDecision};
    let (retry, next_step) = match code {
        "request.invalid" => (
            AfterCorrection,
            "Correct the request using the documented schema. Review the plan before submitting it again.",
        ),
        "provider.token_expired" | "provider.token_scope" => (
            OwnerDecision,
            "Run hoist accounts, then hoist accounts check ACCOUNT. Give it a new token with the permissions it needs: hoist accounts rotate ACCOUNT. Never paste a token into an agent chat.",
        ),
        "approval.required" => (
            OwnerDecision,
            "The owner allows or denies that request in Hoista's app. An expired approval requires a new request.",
        ),
        "agent.disabled" => (
            OwnerDecision,
            "The owner can review agent access and use hoist agents resume. Do not resume access automatically.",
        ),
        "app.watching" => (
            OwnerDecision,
            "Review the project and its resources. Only the owner should opt into managed mode before requesting changes.",
        ),
        "deploy.secret_missing" | "deploy.environment_input_lost" => (
            AfterCorrection,
            "Supply the required values again through the local environment flow and review a fresh deployment plan. Secret values cannot be recovered from local state.",
        ),
        "deploy.environment_source_stale" => (
            AfterCorrection,
            "Inspect the current source, create a fresh plan and resupply the environment values before deploying.",
        ),
        "state.unavailable" => (
            InspectFirst,
            "Run hoist doctor and check available disk space and state-directory access. Preserve the database and its WAL files; do not delete state to repair this error. Inspect the operation status before retrying.",
        ),
        "provider.rate_limited" => (
            InspectFirst,
            "Wait for the provider's rate limit to clear. Inspect the operation and hosted resources before retrying a change; a failed response does not prove that nothing changed.",
        ),
        "provider.timeout"
        | "deploy.rollback_failed"
        | "deploy.resource_recovery_failed"
        | "schedule.recovery_failed" => (
            InspectFirst,
            "Inspect the deployment, recorded resources and provider state before retrying. The hosted outcome may be uncertain; do not recreate or delete resources automatically.",
        ),
        "provider.not_supported" => (
            AfterCorrection,
            "Choose a supported provider capability or use the provider's documented management flow. Repeating the same request will not add support.",
        ),
        "daemon.instance_changed" => (
            InspectFirst,
            "Read hoist daemon status again. Confirm whether replacement already completed before requesting another restart.",
        ),
        _ => (
            InspectFirst,
            "Inspect the current operation status and recent activity before retrying. If the result is uncertain, reconcile hosted resources first; do not automatically replay a change.",
        ),
    };
    RecoveryAdvice {
        retry,
        next_step: next_step.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_errors_remain_readable_and_new_errors_explain_recovery() {
        use crate::contracts::ErrorDto;
        let old: ErrorDto =
            serde_json::from_str(r#"{"code":"old.error","message":"Try inspection."}"#)
                .expect("legacy fixture");
        assert!(old.recovery.is_none());
        assert!(old.missing_secrets.is_empty());
        let missing = ErrorDto::from_engine(
            &crate::application::ports::catalog::EngineError::MissingSecrets(vec![
                "API_KEY".into(),
            ]),
        );
        assert_eq!(
            serde_json::to_value(&missing).expect("typed missing input")["missing_secrets"],
            serde_json::json!(["API_KEY"])
        );
        assert!(missing.explanation().contains("API_KEY"));
        assert_eq!(old.explanation(), "[old.error] Try inspection.");
        let current = ErrorDto::new("provider.timeout", "The request timed out.");
        let encoded = serde_json::to_value(&current).expect("error fixture");
        assert_eq!(encoded["recovery"]["retry"], "inspect_first");
        assert!(
            current
                .explanation()
                .contains("hosted outcome may be uncertain")
        );
    }

    #[test]
    fn unknown_and_ambiguous_failures_never_recommend_blind_retry() {
        for code in [
            "unknown.secret-value",
            "provider.timeout",
            "state.unavailable",
            "deploy.rollback_failed",
            "state.conflict",
        ] {
            let advice = advice(code);
            assert_eq!(advice.retry, RetryDisposition::InspectFirst);
            assert!(!advice.next_step.contains("secret-value"));
        }
        for code in [
            "approval.required",
            "agent.disabled",
            "app.watching",
            "provider.token_scope",
        ] {
            assert_eq!(advice(code).retry, RetryDisposition::OwnerDecision);
        }
    }
}
