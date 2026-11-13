//! Wrangler's JSONC, parsed by `hoista-system`, with the message Hoista shows
//! when it is invalid. Callers limit the input size.

use crate::application::ports::catalog::EngineError;

pub(super) fn parse(input: &[u8]) -> Result<serde_json::Value, EngineError> {
    hoista_system::files::parse_jsonc(input).map_err(|_| {
        EngineError::Invalid(
            "Wrangler JSONC must contain valid JSON with optional comments and trailing commas.",
        )
    })
}
