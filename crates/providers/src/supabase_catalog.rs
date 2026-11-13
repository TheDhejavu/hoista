//! Supabase's Management API, run through the shared REST runner from the
//! catalog generated from its OpenAPI spec (`supabase/api_ops.json`).
//!
//! Operations that take or return secret values are left out of the catalog,
//! and answers are scrubbed again anyway.

use super::{api_catalog, http::ProviderHttp};
use crate::{application::ports::catalog::EngineError, domain::Secret};
use reqwest::Method;
use serde_json::Value;

/// The project and edge function a resource reference names:
/// `ref`, `ref/function:slug` or `ref/bucket:id`.
pub(super) fn context(reference: &str) -> (Option<&str>, Option<&str>) {
    let (project, rest) = reference
        .split_once('/')
        .map_or((reference, None), |(project, rest)| (project, Some(rest)));
    let function = rest.and_then(|rest| rest.strip_prefix("function:"));
    ((!project.is_empty()).then_some(project), function)
}

/// Runs one catalog operation.
///
/// # Errors
/// Unknown operations, unknown or missing arguments and provider failures.
pub(super) async fn run(
    http: &ProviderHttp,
    token: &Secret,
    reference: Option<&str>,
    name: &str,
    args: Value,
) -> Result<Value, EngineError> {
    let manifest = super::supabase::manifest();
    let op = manifest.api_operation(name).ok_or(EngineError::Invalid(
        "Supabase has no such operation in Hoista. Call list_provider_operations for the list.",
    ))?;
    let base = manifest.api.as_ref().map_or("", |api| api.base.as_str());
    let (project, function) = reference.map(context).unwrap_or((None, None));
    let mut known: Vec<(&str, &str)> = Vec::new();
    known.extend(project.map(|project| ("project", project)));
    known.extend(function.map(|function| ("function", function)));
    let request = api_catalog::request(op, base, &known, args)?;

    // Asking for a history table that isn't there writes an ERROR line into
    // the project's Postgres log, so look before asking.
    if name == "listMigrationHistory"
        && let Some(project) = request.segments.get(1)
    {
        let probe = http
            .json(
                crate::Provider::Supabase,
                token,
                Method::POST,
                &["projects", project, "database", "query", "read-only"],
                &[],
                Some(&serde_json::json!({
                    "query": "select to_regclass('supabase_migrations.schema_migrations') is not null as present",
                })),
            )
            .await;
        if let Ok(value) = &probe
            && value[0]["present"] == Value::Bool(false)
        {
            return Ok(Value::Array(vec![]));
        }
    }
    let answer = api_catalog::send(http, crate::Provider::Supabase, token, &request).await;
    // A project that has never had migrations applied with the Supabase CLI
    // has no history table: that's an empty history, not a failure.
    if name == "listMigrationHistory" && never_migrated(&answer) {
        return Ok(Value::Array(vec![]));
    }
    let mut answer = answer?;
    // Secrets: their names only, never their values or digests.
    if name == "listAllSecrets"
        && let Value::Array(items) = &mut answer
    {
        for item in items {
            if let Value::Object(map) = item {
                map.retain(|key, _| key == "name" || key == "updated_at");
            }
        }
    }
    api_catalog::clean(&mut answer, token)?;
    Ok(answer)
}

/// Whether an answer says the CLI's migration table isn't there.
fn never_migrated(answer: &Result<Value, EngineError>) -> bool {
    let said = match answer {
        Ok(value) => value["message"]
            .as_str()
            .or_else(|| value["error"].as_str())
            .unwrap_or_default()
            .to_owned(),
        Err(EngineError::Provider { message, .. }) => message.clone(),
        Err(_) => String::new(),
    };
    said.contains("schema_migrations") && said.contains("does not exist")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::providers::ProviderRisk;

    #[test]
    fn the_catalog_loads_and_never_offers_secret_operations() {
        let ops = super::super::supabase::manifest().operations();
        assert!(ops.len() > 100, "catalog has {} operations", ops.len());
        for hidden in [
            "getProjectApiKeys",
            "createProjectApiKey",
            "bulkCreateSecrets",
            "updateDatabasePassword",
            "getPgsodiumConfig",
            "createLoginRole",
        ] {
            assert!(
                !ops.iter().any(|op| op.name == hidden),
                "{hidden} must not be offered"
            );
        }
        for op in ops.iter().filter(|op| !op.name.starts_with("supabase ")) {
            for arg in &op.args {
                assert!(
                    !api_catalog::secret_key(&arg.name),
                    "{} takes a secret-looking argument {}",
                    op.name,
                    arg.name
                );
            }
        }
        let risk = |name: &str| ops.iter().find(|op| op.name == name).map(|op| op.risk);
        assert_eq!(risk("getProject"), Some(ProviderRisk::Read));
        assert_eq!(risk("readOnlyQuery"), Some(ProviderRisk::Read));
        assert_eq!(risk("restartProject"), Some(ProviderRisk::Change));
        assert_eq!(risk("runQuery"), Some(ProviderRisk::Destroy));
        assert_eq!(risk("deleteProject"), Some(ProviderRisk::Destroy));
        assert_eq!(risk("restorePitrBackup"), Some(ProviderRisk::Destroy));
    }

    #[test]
    fn references_name_the_project_and_function() {
        assert_eq!(context("abc"), (Some("abc"), None));
        assert_eq!(context("abc/function:hello"), (Some("abc"), Some("hello")));
        assert_eq!(context("abc/bucket:avatars"), (Some("abc"), None));
    }
}
