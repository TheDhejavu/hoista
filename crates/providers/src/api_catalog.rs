//! Running operations from a provider's generated REST catalog, and scrubbing
//! secrets from every answer (Railway's GraphQL catalog shares the scrubbing).
//!
//! Callers pick an operation and its arguments, never the method or path.

use super::http::ProviderHttp;
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{ProviderOperationArg, ProviderOperationInfo, ProviderRisk},
    },
    domain::{Secret, leaks::LeakScanner},
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value};

/// One REST operation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RestOperation {
    pub name: String,
    area: String,
    pub risk: ProviderRisk,
    description: String,
    method: String,
    path: String,
    args: Vec<RestArg>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestArg {
    name: String,
    #[serde(rename = "in")]
    place: String,
    required: bool,
    context: Option<String>,
    schema: Value,
}

#[derive(Deserialize)]
pub(super) struct RestCatalog {
    pub operations: Vec<RestOperation>,
}

const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
const PLACES: [&str; 4] = ["path", "query", "body", "body_root"];

impl RestOperation {
    /// Whether Hoista fills in one of its arguments from `context`
    /// (`bucket`, `project`…).
    pub(super) fn aimed_at(&self, context: &str) -> bool {
        self.args
            .iter()
            .any(|arg| arg.context.as_deref() == Some(context))
    }

    /// Whether the operation takes this argument.
    pub(super) fn takes(&self, key: &str) -> bool {
        self.args.iter().any(|arg| arg.name == key)
    }

    /// Refuses an operation whose address could be changed by a caller, or
    /// that takes a secret.
    pub(super) fn check(&self) -> Result<(), &'static str> {
        if !METHODS.contains(&self.method.as_str()) {
            return Err("unknown method");
        }
        if !self.path.starts_with('/') || self.path.contains("..") || self.path.contains('?') {
            return Err("the path must be a plain address");
        }
        for piece in self.path.split('/') {
            if let Some(param) = piece.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
                if !self
                    .args
                    .iter()
                    .any(|arg| arg.name == param && arg.place == "path")
                {
                    return Err("a part of the address isn't one of its arguments");
                }
            } else if piece.contains(['{', '}']) {
                return Err("the path has a broken placeholder");
            }
        }
        for arg in &self.args {
            if !PLACES.contains(&arg.place.as_str()) {
                return Err("an argument goes somewhere unknown");
            }
            if arg.place != "path" && secret_key(&arg.name) {
                return Err("it takes a secret");
            }
        }
        Ok(())
    }
}

/// The catalog as callers see it.
pub(super) fn rest_operations(operations: &[RestOperation]) -> Vec<ProviderOperationInfo> {
    operations
        .iter()
        .map(|op| ProviderOperationInfo {
            name: op.name.clone(),
            area: op.area.clone(),
            risk: op.risk,
            request: op.method.to_ascii_uppercase(),
            description: op.description.clone(),
            args: op
                .args
                .iter()
                .map(|arg| ProviderOperationArg {
                    name: arg.name.clone(),
                    required: arg.required && arg.context.is_none(),
                    filled_in: arg.context.clone(),
                    schema: arg.schema.clone(),
                })
                .collect(),
        })
        .collect()
}

/// The address and request one REST operation would send.
pub(super) struct Request {
    pub method: Method,
    pub segments: Vec<String>,
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
}

/// Checks the arguments and builds the request, without sending it.
///
/// # Errors
/// Unknown or missing arguments and names that would change the address.
pub(super) fn request(
    op: &RestOperation,
    base: &str,
    context: &[(&str, &str)],
    args: Value,
) -> Result<Request, EngineError> {
    let name = &op.name;
    let mut given = match args {
        Value::Null => Map::new(),
        Value::Object(map) => map,
        _ => return Err(EngineError::Invalid("Give arguments as an object.")),
    };
    if let Some(unknown) = given
        .keys()
        .find(|key| !op.args.iter().any(|arg| &arg.name == *key))
    {
        return Err(EngineError::Provider {
            evidence: None,
            code: "provider.unknown_argument".into(),
            message: format!("{name} has no argument {unknown}."),
        });
    }
    for arg in &op.args {
        let known = arg.context.as_deref().and_then(|wanted| {
            context
                .iter()
                .find(|(key, _)| *key == wanted)
                .map(|(_, value)| *value)
        });
        // What Hoista fills in from the resource always wins: an agent can't
        // aim an operation at another project, team or account.
        if let Some(value) = known {
            if given
                .get(&arg.name)
                .is_some_and(|said| said.as_str() != Some(value))
            {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "provider.argument_filled_in".into(),
                    message: format!(
                        "Hoista fills in {} from the resource; leave it out.",
                        arg.name
                    ),
                });
            }
            given.insert(arg.name.clone(), Value::String(value.to_owned()));
        }
    }
    let missing: Vec<&str> = op
        .args
        .iter()
        .filter(|arg| arg.required && !given.contains_key(&arg.name))
        .map(|arg| arg.name.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(EngineError::Provider {
            evidence: None,
            code: "provider.missing_argument".into(),
            message: format!("{name} needs {}.", missing.join(", ")),
        });
    }

    // The address, from the fixed path with its resources filled in.
    let mut segments: Vec<String> = Vec::new();
    // Drop the prefix the provider's base address already has, such as `/v1`.
    let path = op.path.strip_prefix(base).unwrap_or(&op.path);
    for piece in path.trim_start_matches('/').split('/') {
        if let Some(param) = piece.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
            let value = given.get(param).map(text).unwrap_or_default();
            if value.is_empty() || value.contains(['/', '?', '#']) || value == ".." || value == "."
            {
                return Err(EngineError::Invalid(
                    "Give each name in the address as plain text.",
                ));
            }
            segments.push(value);
        } else {
            segments.push(piece.to_owned());
        }
    }
    let mut query: Vec<(String, String)> = Vec::new();
    let mut body = Map::new();
    let mut root = None;
    for arg in &op.args {
        let Some(value) = given.get(&arg.name) else {
            continue;
        };
        match arg.place.as_str() {
            "query" => match value {
                Value::Array(items) => {
                    query.extend(items.iter().map(|item| (arg.name.clone(), text(item))));
                }
                other => query.push((arg.name.clone(), text(other))),
            },
            "body" => {
                body.insert(arg.name.clone(), value.clone());
            }
            "body_root" => root = Some(value.clone()),
            _ => {}
        }
    }
    let body = root.or_else(|| (!body.is_empty()).then_some(Value::Object(body)));
    let method = Method::from_bytes(op.method.as_bytes()).map_err(|_| EngineError::Store)?;
    Ok(Request {
        method,
        segments,
        query,
        body,
    })
}

/// Sends a built request. The answer is not scrubbed yet: callers that
/// adjust it first call [`clean`] after.
///
/// # Errors
/// Provider failures.
pub(super) async fn send(
    http: &ProviderHttp,
    provider: Provider,
    token: &Secret,
    request: &Request,
) -> Result<Value, EngineError> {
    let path: Vec<&str> = request.segments.iter().map(String::as_str).collect();
    let pairs: Vec<(&str, &str)> = request
        .query
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    http.json(
        provider,
        token,
        request.method.clone(),
        &path,
        &pairs,
        request.body.as_ref(),
    )
    .await
}

/// Runs one operation from a manifest's API catalog on the resource and
/// account `context` names, and returns the answer, secrets removed.
///
/// # Errors
/// Unknown operations, invalid arguments and provider failures.
pub(super) async fn run_manifest(
    http: &ProviderHttp,
    provider: Provider,
    manifest: &super::manifest::Manifest,
    token: &Secret,
    context: &[(&str, &str)],
    name: &str,
    args: Value,
) -> Result<Value, EngineError> {
    let op = manifest.api_operation(name).ok_or(EngineError::Invalid(
        "This provider has no such operation in Hoista. Call list_provider_operations for the list.",
    ))?;
    let base = manifest.api.as_ref().map_or("", |api| api.base.as_str());
    let request = request(op, base, context, args)?;
    let mut answer = send(http, provider, token, &request).await?;
    clean(&mut answer, token)?;
    Ok(answer)
}

/// Removes secrets from an answer before anyone sees it.
///
/// # Errors
/// The leak scanner failing to start.
pub(super) fn clean(answer: &mut Value, token: &Secret) -> Result<(), EngineError> {
    let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
    scrub(answer, &scanner, token);
    Ok(())
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Removes anything that could be a secret: secret-looking keys, the values
/// of environment variables (names stay), and secrets inside strings.
pub(super) fn scrub(value: &mut Value, scanner: &LeakScanner, token: &Secret) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| !secret_key(key));
            for (key, item) in map.iter_mut() {
                if matches!(
                    key.as_str(),
                    "variables" | "sharedVariables" | "env" | "environmentVariables"
                ) && let Value::Object(vars) = item
                {
                    for var in vars.values_mut() {
                        *var = Value::Null;
                    }
                    continue;
                }
                scrub(item, scanner, token);
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub(item, scanner, token);
            }
        }
        Value::String(text) => {
            *text = scanner.redact(text, std::slice::from_ref(token));
        }
        _ => {}
    }
}

/// Field names that hold a secret, in snake, kebab or camel case:
/// `smtp_pass`, `external_github_secret`, `sms_twilio_auth_token`,
/// `apiKey`, `jwt_secret`, `private_jwk`, `db-password`.
pub(super) fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['_', '-'], "");
    [
        "password",
        "secret",
        "token",
        "credential",
        "privatekey",
        "accesskey",
        "apikey",
        "connectionstring",
        "cookie",
        "privatejwk",
        "rootkey",
    ]
    .iter()
    .any(|word| key.contains(word))
        || (key.ends_with("pass") && !key.ends_with("bypass"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(path: &str, args: Value) -> RestOperation {
        serde_json::from_value(serde_json::json!({
            "name": "x",
            "area": "a",
            "risk": "read",
            "description": "d",
            "method": "GET",
            "path": path,
            "args": args,
        }))
        .unwrap()
    }

    #[test]
    fn addresses_are_fixed_and_filled_in_from_the_resource() {
        let get = op(
            "/v1/projects/{ref}/functions/{function_slug}",
            serde_json::json!([
                {
                    "name": "ref",
                    "in": "path",
                    "required": true,
                    "context": "project",
                    "schema": {},
                },
                {
                    "name": "function_slug",
                    "in": "path",
                    "required": true,
                    "context": "function",
                    "schema": {},
                },
                {"name": "limit", "in": "query", "required": false, "context": null, "schema": {}},
            ]),
        );
        assert!(get.check().is_ok());
        let built = request(
            &get,
            "/v1",
            &[("project", "abc"), ("function", "hello")],
            serde_json::json!({"limit": 5}),
        )
        .unwrap();
        assert_eq!(built.segments, ["projects", "abc", "functions", "hello"]);
        assert_eq!(built.query, [("limit".to_owned(), "5".to_owned())]);
        // Giving the value Hoista fills in is fine; another project is refused.
        assert!(
            request(
                &get,
                "/v1",
                &[("project", "abc"), ("function", "hello")],
                serde_json::json!({"ref": "abc"}),
            )
            .is_ok()
        );
        let other = request(
            &get,
            "/v1",
            &[("project", "abc"), ("function", "hello")],
            serde_json::json!({"ref": "someone-elses"}),
        );
        assert!(matches!(
            other,
            Err(EngineError::Provider { ref code, .. }) if code == "provider.argument_filled_in"
        ));
        // A name that would change the address is refused.
        assert!(
            request(
                &get,
                "/v1",
                &[("project", "abc")],
                serde_json::json!({"function_slug": "../keys"})
            )
            .is_err()
        );
        assert!(
            request(
                &get,
                "/v1",
                &[("project", "abc")],
                serde_json::json!({"function_slug": "a/b"})
            )
            .is_err()
        );
        assert!(request(&get, "/v1", &[], serde_json::json!({"nope": 1})).is_err());
        assert!(
            request(&get, "/v1", &[], Value::Null).is_err(),
            "missing project"
        );
    }

    #[test]
    fn operations_that_could_move_or_take_secrets_are_refused() {
        assert!(op("/v1/{x}", serde_json::json!([])).check().is_err());
        assert!(op("/v1/a/../b", serde_json::json!([])).check().is_err());
        assert!(op("/v1/a?b=1", serde_json::json!([])).check().is_err());
        assert!(
            op("/v1/a", serde_json::json!([{"name": "db_pass", "in": "body", "required": false, "context": null, "schema": {}}]))
                .check()
                .is_err()
        );
    }

    #[test]
    fn secret_fields_and_variable_values_are_dropped() {
        let scanner = LeakScanner::new().unwrap();
        let token = Secret::new("sbp_example".into());
        let mut value = serde_json::json!({
            "site_url": "https://sam.dev",
            "smtp_pass": "x",
            "external_github_secret": "y",
            "sms_twilio_auth_token": "z",
            "apiKey": "k",
            "bypass_cidr_checks": true,
            "env": {"DATABASE_URL": "postgres://x"},
            "external_github_client_id": "public",
        });
        scrub(&mut value, &scanner, &token);
        assert_eq!(
            value,
            serde_json::json!({
                "site_url": "https://sam.dev",
                "bypass_cidr_checks": true,
                "env": {"DATABASE_URL": null},
                "external_github_client_id": "public",
            })
        );
    }
}
