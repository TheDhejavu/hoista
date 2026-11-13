//! Reading a project's files during a deploy, with the messages Hoista shows
//! when one can't be read.

use crate::application::ports::catalog::EngineError;
use std::path::Path;

pub(crate) fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, EngineError> {
    hoista_system::files::read_bounded(path, limit).map_err(|error| match error {
        hoista_system::files::FileError::TooLarge => {
            EngineError::Invalid("A source configuration file exceeds the inspection limit.")
        }
        _ => EngineError::Invalid("Could not read a project source file."),
    })
}

pub(crate) fn parse_jsonc(input: &[u8]) -> Result<serde_json::Value, EngineError> {
    hoista_system::files::parse_jsonc(input).map_err(|_| {
        EngineError::Invalid(
            "Wrangler JSONC must contain valid JSON with optional comments and trailing commas.",
        )
    })
}
