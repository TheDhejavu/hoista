//! Errors for a provider answer that's missing something Hoista needs.

use crate::application::ports::catalog::EngineError;
use serde_json::Value;

/// A provider's answer was incomplete or didn't add up.
pub(crate) fn invalid() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.invalid_response".into(),
        message: "The provider returned incomplete or inconsistent resource data. Retry discovery before making changes.".into(),
    }
}

/// [`invalid`], naming the provider.
pub(crate) fn incomplete(provider: &str) -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.invalid_response".into(),
        message: format!("{provider} returned incomplete data. Refresh and try again."),
    }
}

/// The text at `key`, which must be there and not empty.
pub(crate) fn field(value: &Value, key: &str) -> Result<String, EngineError> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or_else(invalid)
}
