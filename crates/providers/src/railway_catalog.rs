//! Railway's public API operations, from the generated `railway_ops.json`.
//!
//! Callers pick an operation and its arguments, never the GraphQL text.
//! Secret fields are stripped from answers, and operations that read or take
//! secret values are left out; the variable tools handle those.

use super::{api_catalog::scrub, http::ProviderHttp, railway};
use crate::{
    application::ports::{
        catalog::EngineError,
        providers::{ProviderOperationArg, ProviderOperationInfo, ProviderRisk},
    },
    domain::{Secret, leaks::LeakScanner},
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Catalog {
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
struct Operation {
    name: String,
    area: String,
    risk: ProviderRisk,
    description: String,
    args: Vec<Arg>,
    input_context: Vec<InputContext>,
    document: String,
}

#[derive(Deserialize)]
struct Arg {
    name: String,
    required: bool,
    context: Option<String>,
    schema: Value,
}

#[derive(Deserialize)]
struct InputContext {
    arg: String,
    field: String,
    context: String,
}

fn catalog() -> &'static [Operation] {
    static CATALOG: OnceLock<Vec<Operation>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str::<Catalog>(include_str!("railway_ops.json"))
            .map(|catalog| catalog.operations)
            .unwrap_or_default()
    })
}

/// The catalog as callers see it.
pub(super) fn operations() -> Vec<ProviderOperationInfo> {
    catalog()
        .iter()
        .map(|op| ProviderOperationInfo {
            name: op.name.clone(),
            area: op.area.clone(),
            risk: op.risk,
            request: if op.document.trim_start().starts_with("mutation") {
                "Mutation".into()
            } else {
                "Query".into()
            },
            description: op.description.clone(),
            args: op
                .args
                .iter()
                .map(|arg| ProviderOperationArg {
                    name: arg.name.clone(),
                    required: arg.required && arg.context.is_none(),
                    filled_in: arg.context.clone(),
                    schema: filled_fields(&arg.schema, &arg.name, &op.input_context),
                })
                .collect(),
        })
        .collect()
}

/// An input object's schema with the fields Hoista fills in marked
/// `x-filled-in` (with what fills them) and no longer required, so a form
/// asks only for the rest.
fn filled_fields(schema: &Value, arg: &str, context: &[InputContext]) -> Value {
    let mut schema = schema.clone();
    let filled: Vec<&InputContext> = context.iter().filter(|item| item.arg == arg).collect();
    if filled.is_empty() {
        return schema;
    }
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for item in &filled {
            if let Some(Value::Object(field)) = properties.get_mut(&item.field) {
                field.insert("x-filled-in".into(), Value::String(item.context.clone()));
            }
        }
    }
    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|name| {
            !filled
                .iter()
                .any(|item| name.as_str() == Some(item.field.as_str()))
        });
    }
    schema
}

/// Ids Hoista knows for the resource an operation runs on.
#[derive(Default)]
struct Context {
    project: Option<String>,
    environment: Option<String>,
    service: Option<String>,
    workspace: Option<String>,
}

impl Context {
    fn get(&self, name: &str) -> Option<&String> {
        match name {
            "project" => self.project.as_ref(),
            "environment" => self.environment.as_ref(),
            "service" => self.service.as_ref(),
            "workspace" => self.workspace.as_ref(),
            _ => None,
        }
    }
}

/// Runs one catalog operation.
///
/// # Errors
/// Unknown operations, unknown or missing arguments, invalid references and
/// provider failures.
pub(super) async fn run(
    http: &ProviderHttp,
    token: &Secret,
    workspace: &str,
    reference: Option<&str>,
    name: &str,
    args: Value,
) -> Result<Value, EngineError> {
    let op = catalog()
        .iter()
        .find(|op| op.name == name)
        .ok_or(EngineError::Invalid(
            "Railway has no such operation in Hoista. Call list_provider_operations for the list.",
        ))?;
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
    let mut context = Context {
        workspace: (!workspace.is_empty()).then(|| workspace.to_owned()),
        ..Context::default()
    };
    if let Some(reference) = reference {
        let (project, service) = railway::split(reference)?;
        context.project = Some(project.to_owned());
        context.service = Some(service.to_owned());
        let wants_environment = op.args.iter().any(|arg| {
            arg.context.as_deref() == Some("environment") && !given.contains_key(&arg.name)
        }) || op
            .input_context
            .iter()
            .any(|item| item.context == "environment");
        if wants_environment {
            context.environment = environment_of(http, token, reference).await?;
        }
    }
    for arg in &op.args {
        if let Some(kind) = &arg.context
            && !given.contains_key(&arg.name)
            && let Some(value) = context.get(kind)
        {
            given.insert(arg.name.clone(), Value::String(value.clone()));
        }
    }
    for item in &op.input_context {
        let required = op
            .args
            .iter()
            .any(|arg| arg.name == item.arg && arg.required);
        if !given.contains_key(&item.arg) && !required {
            continue;
        }
        let entry = given
            .entry(item.arg.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(input) = entry
            && !input.contains_key(&item.field)
            && let Some(value) = context.get(&item.context)
        {
            input.insert(item.field.clone(), Value::String(value.clone()));
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
    let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
    let variables = Value::Object(given);
    // A screen asks for the same reads together and again on every visit;
    // answer a repeat from the last 30 seconds instead of asking Railway again.
    let read = op.document.starts_with("query");
    let key = read.then(|| answer_key(token, name, &variables));
    if let Some(key) = &key
        && let Some(answer) = recent_answer(key)
    {
        return Ok(answer);
    }
    // Railway refuses some fields to some sign-ins, and one refused field
    // fails a whole read. Drop the fields it names and ask again; remember
    // them, so the next read leaves them out from the start.
    let mut document = without(&op.document, &refused(name));
    let mut value = Value::Null;
    for _ in 0..5 {
        value = http
            .graphql_raw(
                crate::Provider::Railway,
                token,
                &serde_json::json!({ "query": document, "variables": variables }),
            )
            .await?;
        let errors = value["errors"].as_array().cloned().unwrap_or_default();
        if errors.is_empty() || !value["data"][name].is_null() {
            break;
        }
        let fields: Vec<String> = errors
            .iter()
            .filter_map(|error| error["path"].as_array())
            .filter(|path| path.len() > 1)
            .filter_map(|path| path.iter().rev().find_map(Value::as_str).map(str::to_owned))
            .filter(|field| field != name)
            .collect();
        let next = without(&document, &fields);
        if fields.is_empty() || next == document {
            break;
        }
        remember(name, &fields);
        document = next;
    }
    if op.document.starts_with("mutation") {
        railway::forget_reads();
        forget_answers();
    }
    let failed = value["errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty());
    if failed && value["data"][name].is_null() {
        let message = value["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_ERRORS_SHOWN)
            .filter_map(|error| error["message"].as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(railway::explain_refusal(EngineError::Provider {
            evidence: None,
            code: "provider.refused".into(),
            message: scanner.redact(
                &format!("Railway refused {name}: {message}"),
                std::slice::from_ref(token),
            ),
        }));
    }
    let mut answer = value["data"][name].clone();
    scrub(&mut answer, &scanner, token);
    if let Some(key) = key {
        keep_answer(key, &answer);
    }
    Ok(answer)
}

/// How long a read's answer is reused.
/// Errors passed on from one refused query, and answers kept for reuse.
const MAX_ERRORS_SHOWN: usize = 3;
const MAX_ANSWERS: usize = 512;
const ANSWER_REUSE: std::time::Duration = std::time::Duration::from_secs(30);
/// How long a service's environment is remembered; it changes only when the
/// service moves, and a change clears it.
const ENVIRONMENT_REUSE: std::time::Duration = std::time::Duration::from_secs(600);

type Remembered<T> = std::sync::Mutex<std::collections::HashMap<[u8; 32], (std::time::Instant, T)>>;

fn answers() -> &'static Remembered<Value> {
    static ANSWERS: OnceLock<Remembered<Value>> = OnceLock::new();
    ANSWERS.get_or_init(Default::default)
}

fn environments() -> &'static Remembered<Option<String>> {
    static ENVIRONMENTS: OnceLock<Remembered<Option<String>>> = OnceLock::new();
    ENVIRONMENTS.get_or_init(Default::default)
}

/// Keyed by the token too, so one account never sees another's answer.
fn answer_key(token: &Secret, name: &str, variables: &Value) -> [u8; 32] {
    use sha2::Digest;
    let mut digest = sha2::Sha256::new();
    digest.update(token.expose().as_bytes());
    digest.update([0]);
    digest.update(name.as_bytes());
    digest.update([0]);
    digest.update(variables.to_string().as_bytes());
    digest.finalize().into()
}

fn recent_answer(key: &[u8; 32]) -> Option<Value> {
    if cfg!(test) {
        return None;
    }
    let answers = answers().lock().ok()?;
    let (at, value) = answers.get(key)?;
    (at.elapsed() < ANSWER_REUSE).then(|| value.clone())
}

fn keep_answer(key: [u8; 32], answer: &Value) {
    if let Ok(mut answers) = answers().lock() {
        answers.retain(|_, (at, _)| at.elapsed() < ANSWER_REUSE);
        if answers.len() < MAX_ANSWERS {
            answers.insert(key, (std::time::Instant::now(), answer.clone()));
        }
    }
}

/// Forgets remembered answers and environments, after any change.
pub(super) fn forget_answers() {
    if let Ok(mut answers) = answers().lock() {
        answers.clear();
    }
    if let Ok(mut environments) = environments().lock() {
        environments.clear();
    }
}

/// The environment a service's newest deployment runs in, remembered for a while.
pub(super) async fn environment_of(
    http: &ProviderHttp,
    token: &Secret,
    reference: &str,
) -> Result<Option<String>, EngineError> {
    let key = answer_key(token, "environment", &Value::String(reference.to_owned()));
    if !cfg!(test)
        && let Ok(remembered) = environments().lock()
        && let Some((at, environment)) = remembered.get(&key)
        && at.elapsed() < ENVIRONMENT_REUSE
    {
        return Ok(environment.clone());
    }
    let environment = railway::latest(http, token, reference)
        .await?
        .and_then(|latest| latest.environment_id);
    if let Ok(mut remembered) = environments().lock() {
        remembered.insert(key, (std::time::Instant::now(), environment.clone()));
    }
    Ok(environment)
}

fn refused_fields() -> &'static std::sync::Mutex<std::collections::HashMap<String, Vec<String>>> {
    static REFUSED: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>> =
        OnceLock::new();
    REFUSED.get_or_init(Default::default)
}

fn refused(name: &str) -> Vec<String> {
    refused_fields()
        .lock()
        .ok()
        .and_then(|map| map.get(name).cloned())
        .unwrap_or_default()
}

fn remember(name: &str, fields: &[String]) {
    if let Ok(mut map) = refused_fields().lock() {
        let list = map.entry(name.to_owned()).or_default();
        for field in fields {
            if !list.contains(field) && list.len() < 64 {
                list.push(field.clone());
            }
        }
    }
}

/// The document without these fields (and what they select), wherever they
/// appear inside the operation's own selection.
fn without(document: &str, fields: &[String]) -> String {
    let mut out = document.to_owned();
    for field in fields {
        if field.is_empty() || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        // The operation's field opens the second `{`; only what follows is a selection.
        let Some(body) = out.match_indices('{').nth(1).map(|(at, _)| at + 1) else {
            continue;
        };
        let bytes = out.as_bytes();
        let mut result = String::with_capacity(out.len());
        let mut i = 0;
        while i < bytes.len() {
            let starts = i >= body
                && out[i..].starts_with(field.as_str())
                && matches!(bytes.get(i - 1), Some(b' ' | b'{'))
                && matches!(bytes.get(i + field.len()), None | Some(b' ' | b'}'));
            if starts {
                let mut j = i + field.len();
                if out[j..].trim_start().starts_with('{') {
                    j += out[j..].find('{').unwrap_or(0);
                    let mut depth = 0;
                    while j < bytes.len() {
                        match bytes[j] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    j += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                }
                i = j;
                continue;
            }
            result.push(bytes[i] as char);
            i += 1;
        }
        out = result;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filled_in_input_fields_are_marked_and_not_required() {
        let op = operations()
            .into_iter()
            .find(|op| op.name == "serviceInstanceLimitsUpdate")
            .expect("in the catalog");
        let schema = &op.args[0].schema;
        assert_eq!(
            schema["properties"]["serviceId"]["x-filled-in"],
            serde_json::json!("service")
        );
        assert!(
            schema["properties"]["memoryGB"]
                .get("x-filled-in")
                .is_none()
        );
        assert!(
            !schema["required"]
                .as_array()
                .is_some_and(|names| names.iter().any(|name| name == "serviceId"))
        );
    }

    #[test]
    fn the_catalog_loads_and_never_offers_secret_operations() {
        let ops = operations();
        assert!(ops.len() > 150, "catalog has {} operations", ops.len());
        for hidden in [
            "variables",
            "bucketS3Credentials",
            "generateShellToken",
            "variablesForServiceDeployment",
        ] {
            assert!(
                !ops.iter().any(|op| op.name == hidden),
                "{hidden} must not be offered"
            );
        }
        let delete = ops
            .iter()
            .find(|op| op.name == "projectDelete")
            .expect("projectDelete");
        assert_eq!(delete.risk, ProviderRisk::Destroy);
        let cancel = ops
            .iter()
            .find(|op| op.name == "deploymentCancel")
            .expect("deploymentCancel");
        assert_eq!(cancel.risk, ProviderRisk::Change);
        let metrics = ops
            .iter()
            .find(|op| op.name == "httpDurationMetrics")
            .expect("metrics");
        assert!(
            metrics
                .args
                .iter()
                .any(|arg| arg.name == "serviceId" && !arg.required)
        );
        assert!(
            metrics
                .args
                .iter()
                .any(|arg| arg.name == "startDate" && arg.required)
        );
        // Every document is a single fixed operation Hoista wrote.
        for op in catalog() {
            assert!(
                op.document.starts_with("query Hoista_")
                    || op.document.starts_with("mutation Hoista_")
            );
            assert!(!op.document.to_ascii_lowercase().contains("password"));
        }
    }

    #[test]
    fn refused_fields_come_out_of_the_document() {
        let doc = "query Hoista_me { me { avatar id isAdmin profile { bio website } name } }";
        assert_eq!(
            without(doc, &["isAdmin".into(), "profile".into()]),
            "query Hoista_me { me { avatar id   name } }"
        );
        // The operation's own field is never removed.
        assert_eq!(without(doc, &["me".into()]), doc);
    }

    #[test]
    fn answers_lose_secret_fields_and_variable_values() {
        let scanner = LeakScanner::new().expect("scanner");
        let token = Secret::new("tok-abc-123-xyz-secretvalue".into());
        let mut value = serde_json::json!({
            "name": "api",
            "accessKey": "AKIA...",
            "config": {"variables": {"API_KEY": "sk_live_1234567890abcdef", "PORT": "3000"}},
            "note": "token tok-abc-123-xyz-secretvalue leaked",
        });
        scrub(&mut value, &scanner, &token);
        assert!(value.get("accessKey").is_none());
        assert_eq!(value["config"]["variables"]["API_KEY"], Value::Null);
        assert!(value["config"]["variables"].get("PORT").is_some());
        assert!(
            !value["note"]
                .as_str()
                .unwrap_or("")
                .contains("tok-abc-123-xyz-secretvalue")
        );
    }
}
