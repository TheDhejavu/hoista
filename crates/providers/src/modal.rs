//! Modal: serverless GPU apps and their volumes. Modal has no REST API, so
//! Hoista speaks gRPC to `api.modal.com` like Modal's own client, checking
//! messages against Modal's API descriptor (`modal/api.desc`).
//!
//! References: `app:<environment>:<app id>`, `volume:<environment>:<volume id>`.

use super::http::ProviderHttp;
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{
            Observed, ProviderAdapter, ProviderBill, ProviderOperationArg, ProviderOperationInfo,
            ProviderRisk, ProviderSeries, RemoteResource,
        },
    },
    domain::{Secret, model::*},
};
use prost::Message as _;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, MethodDescriptor,
    SerializeOptions,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

/// Modal's API adapter.
pub struct Modal {
    http: ProviderHttp,
}

const API: &str = "https://api.modal.com/";
/// A gRPC frame's header: a compression flag, then a four-byte length.
const FRAME_HEADER_BYTES: usize = 5;
const MAX_NAME_CHARS: usize = 128;
const MAX_ERROR_CHARS: usize = 240;
const MAX_SEARCH_CHARS: usize = 256;
const MAX_LOG_LINES: usize = 100;
const MAX_DEPLOYMENTS: usize = 20;
/// Files named when more than one could be the app.
const MAX_FILES_NAMED: usize = 5;
/// Functions whose web address is looked up for one app.
const MAX_URL_LOOKUPS: usize = 20;
/// The largest Python file read when looking for an app's name.
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
/// Chart buckets Modal accepts: a minute to a day.
const MIN_BUCKET_SECS: u32 = 60;
const MAX_BUCKET_SECS: u32 = 86_400;
/// How deep an operation's input schema is described before it's just "object".
const MAX_SCHEMA_DEPTH: usize = 3;
const SERVICE: &str = "modal.client.ModalClient";
/// The Modal client version Hoista speaks as: the package its descriptor
/// came from.
const CLIENT_VERSION: &str = "1.6.3";

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M4.89 5.57 0 14.002l2.521 4.4h5.05l4.396-7.718 4.512 7.709 4.996.037L24 14.057l-4.857-8.452-5.073-.015-2.076 3.598L9.94 5.57Zm.837.729h3.787l1.845 3.252H7.572Zm9.189.021 3.803.012 4.228 7.355-3.736-.027zm-9.82.346L6.94 9.914l-4.209 7.389-1.892-3.3Zm9.187.014 4.297 7.343-1.892 3.282-4.3-7.344zm-6.713 3.6h3.79l-4.212 7.394H3.361Zm11.64 4.109 3.74.027-1.893 3.281-3.74-.027z" fill="currentColor"/></svg>"##;

/// Modal's entry in the provider list. A token reaches one workspace, so the
/// adapter answers account choices itself.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Modal,
    logo: LOGO,
    capabilities: Capabilities {
        discover: true,
        provider_operations: true,
        bills: true,
        delete: true,
        logs: true,
        metrics: true,
        upload: true,
        new_from_folder: true,
        provider_details: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: API,
        hosts: &["api.modal.com"],
        explains_refusals: true,
        sign_in: super::SignIn::Headers(sign_in),
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["modal.client.ModalClient", "TokenInfoGet"],
            query: &[],
            page: None,
            cursor: None,
        },
        rows: &[],
        item: &[],
        id: "workspace_id",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(Modal::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of the provider for the conformance suite, in Modal's messages as
/// JSON: enough to connect and find one app.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: "ak-conformance5f2b as-conformance5f2b",
    account: "ws-1",
    resource: "app:main:ap-1",
    kind: crate::domain::model::ResourceKind::Service,
    routes: &[
        super::conformance::Route {
            path: "/TokenInfoGet",
            body: "",
            answer: r#"{"workspace_id":"ws-1","workspace_name":"sam"}"#,
        },
        super::conformance::Route {
            path: "/EnvironmentList",
            body: "",
            answer: r#"{"items":[{"name":"main","default":true}]}"#,
        },
        super::conformance::Route {
            path: "/AppList",
            body: "",
            answer: r#"{"apps":[{"app_id":"ap-1","name":"llm-server","state":"APP_STATE_DEPLOYED","n_running_tasks":1}]}"#,
        },
        super::conformance::Route {
            path: "/VolumeList",
            body: "",
            answer: r#"{"items":[]}"#,
        },
        super::conformance::Route {
            path: "/WorkspaceBillingSummary",
            body: "",
            answer: r#"{"metered_cost":"1.50"}"#,
        },
    ],
    empty: "{}",
};

fn invalid() -> EngineError {
    super::answers::incomplete("Modal")
}

fn pool() -> Option<&'static DescriptorPool> {
    static POOL: OnceLock<Option<DescriptorPool>> = OnceLock::new();
    POOL.get_or_init(|| DescriptorPool::decode(include_bytes!("modal/api.desc").as_ref()).ok())
        .as_ref()
}

fn method(name: &str) -> Result<MethodDescriptor, EngineError> {
    pool()
        .and_then(|pool| pool.get_service_by_name(SERVICE))
        .and_then(|service| service.methods().find(|method| method.name() == name))
        .ok_or(EngineError::Invalid(
            "Modal has no such operation in Hoista. Call list_provider_operations for the list.",
        ))
}

/// Modal's ids, names and environment names: letters, digits, `-`, `_`, `.`.
fn plain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME_CHARS
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Modal takes the token's ID and secret in two headers of its own.
fn sign_in(token: &Secret) -> Result<super::SignInHeaders, EngineError> {
    let (id, secret) = credentials(token)?;
    Ok(vec![
        ("x-modal-token-id", id),
        ("x-modal-token-secret", secret),
    ])
}

/// The token ID and secret in whatever the person pasted.
pub(crate) fn credentials(token: &Secret) -> Result<(String, String), EngineError> {
    let words = token
        .expose()
        .split(|c: char| c.is_whitespace() || c == ':' || c == '=' || c == ',')
        .filter(|word| !word.is_empty());
    let mut id = None;
    let mut secret = None;
    for word in words {
        let word = word.trim_matches(|c| c == '"' || c == '\'');
        if word.starts_with("ak-") && plain(word) {
            id = Some(word.to_owned());
        } else if word.starts_with("as-") && plain(word) {
            secret = Some(word.to_owned());
        }
    }
    match (id, secret) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => Err(EngineError::Provider {
            evidence: None,
            code: "provider.token_expired".into(),
            message: "Paste both parts of the Modal token: the ID (ak-…) and the secret (as-…)."
                .into(),
        }),
    }
}

/// One gRPC message, framed: no compression, then its length.
fn frame(message: &[u8]) -> Result<Vec<u8>, EngineError> {
    let length = u32::try_from(message.len()).map_err(|_| invalid())?;
    let mut out = Vec::with_capacity(message.len() + FRAME_HEADER_BYTES);
    out.push(0);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(message);
    Ok(out)
}

/// The messages in a gRPC body, at most `limit` of them.
fn frames(body: &[u8], limit: usize) -> Result<Vec<&[u8]>, EngineError> {
    let mut out = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        if rest.len() < FRAME_HEADER_BYTES || rest[0] != 0 || out.len() >= limit {
            return Err(invalid());
        }
        let length = usize::try_from(u32::from_be_bytes([rest[1], rest[2], rest[3], rest[4]]))
            .map_err(|_| invalid())?;
        let end = 5usize.checked_add(length).ok_or_else(invalid)?;
        let message = rest.get(5..end).ok_or_else(invalid)?;
        out.push(message);
        rest = &rest[end..];
    }
    Ok(out)
}

fn to_json(message: &DynamicMessage) -> Result<Value, EngineError> {
    message
        .serialize_with_options(
            serde_json::value::Serializer,
            &SerializeOptions::new().use_proto_field_name(true),
        )
        .map_err(|_| invalid())
}

impl Modal {
    /// Shares the allow-listed HTTP boundary.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    /// Calls one of Modal's methods with a request in JSON (proto field
    /// names), checked against its message type, and returns the answer as
    /// JSON: one message, or a list of them for a streamed answer.
    async fn call(&self, token: &Secret, name: &str, request: Value) -> Result<Value, EngineError> {
        let method = method(name)?;
        if method.is_client_streaming() {
            return Err(EngineError::Invalid("Hoista can't send Modal a stream."));
        }
        let message = DynamicMessage::deserialize(method.input(), request).map_err(|error| {
            EngineError::Provider {
                evidence: None,
                code: "provider.invalid_argument".into(),
                message: format!("{name}: {error}")
                    .chars()
                    .take(MAX_ERROR_CHARS)
                    .collect(),
            }
        })?;
        let address = format!("{API}{SERVICE}/{name}");
        // The conformance fixture speaks Modal's messages as JSON, signed
        // with its token like any provider.
        #[cfg(test)]
        if self.http.is_fixture() {
            let body = serde_json::to_vec(&to_json(&message)?).map_err(|_| invalid())?;
            let answer = self
                .http
                .grpc(Provider::Modal, token, &address, &[], body)
                .await?;
            let value: Value = serde_json::from_slice(&answer).map_err(|_| invalid())?;
            let decoded =
                DynamicMessage::deserialize(method.output(), value).map_err(|_| invalid())?;
            return to_json(&decoded);
        }
        let headers = [
            ("x-modal-client-type", "1".to_owned()),
            ("x-modal-client-version", CLIENT_VERSION.to_owned()),
        ];
        let body = frame(&message.encode_to_vec())?;
        let answer = self
            .http
            .grpc(Provider::Modal, token, &address, &headers, body)
            .await?;
        let mut out = Vec::new();
        for part in frames(&answer, 1000)? {
            let decoded = DynamicMessage::decode(method.output(), part).map_err(|_| invalid())?;
            out.push(to_json(&decoded)?);
        }
        if method.is_server_streaming() {
            Ok(Value::Array(out))
        } else {
            out.into_iter().next().ok_or_else(invalid)
        }
    }

    /// The workspace the token belongs to, and its name.
    async fn workspace(&self, token: &Secret) -> Result<(String, String), EngineError> {
        let info = self.call(token, "TokenInfoGet", json!({})).await?;
        let id = info["workspace_id"]
            .as_str()
            .filter(|id| plain(id))
            .ok_or_else(invalid)?;
        let name = info["workspace_name"].as_str().unwrap_or(id);
        Ok((id.to_owned(), name.to_owned()))
    }

    async fn environments(&self, token: &Secret) -> Result<Vec<String>, EngineError> {
        let list = self.call(token, "EnvironmentList", json!({})).await?;
        let names: Vec<String> = list["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["name"].as_str())
                    .filter(|name| plain(name))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        // A workspace always has its default environment, `main`.
        Ok(if names.is_empty() {
            vec!["main".to_owned()]
        } else {
            names
        })
    }

    /// An app's functions with their web addresses: the layout names them,
    /// and a deployed app's function says its address when looked up by
    /// name (as `modal.Function.from_name` does).
    async fn functions_with_urls(
        &self,
        token: &Secret,
        environment: &str,
        app: &str,
        app_name: &str,
    ) -> Result<Vec<(String, String, Option<String>)>, EngineError> {
        let mut found = self.functions(token, app).await?;
        for (name, _, url) in found.iter_mut().take(MAX_URL_LOOKUPS) {
            if url.is_some() {
                continue;
            }
            if let Ok(answer) = self
                .call(
                    token,
                    "FunctionGet",
                    json!({"app_name": app_name, "object_tag": name, "environment_name": environment}),
                )
                .await
            {
                *url = answer["handle_metadata"]["web_url"]
                    .as_str()
                    .filter(|address| address.starts_with("https://"))
                    .map(str::to_owned);
            }
        }
        Ok(found)
    }

    /// An app's deployed name, as `FunctionGet` looks it up.
    async fn app_name(
        &self,
        token: &Secret,
        environment: &str,
        app: &str,
    ) -> Result<String, EngineError> {
        self.apps(token, environment)
            .await?
            .iter()
            .find(|found| found["app_id"].as_str() == Some(app))
            .and_then(|found| {
                found["name"]
                    .as_str()
                    .or_else(|| found["description"].as_str())
            })
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .ok_or(EngineError::NotFound)
    }

    /// An app's functions: name, id and web address (for a web endpoint),
    /// from its layout.
    async fn functions(
        &self,
        token: &Secret,
        app: &str,
    ) -> Result<Vec<(String, String, Option<String>)>, EngineError> {
        let layout = self
            .call(token, "AppGetLayout", json!({"app_id": app}))
            .await?;
        let objects = layout["app_layout"]["objects"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut found: Vec<(String, String, Option<String>)> = layout["app_layout"]["function_ids"]
            .as_object()
            .map(|map| {
                map.iter()
                    .filter_map(|(name, id)| Some((name.clone(), id.as_str()?.to_owned())))
                    .filter(|(_, id)| plain(id))
                    .map(|(name, id)| {
                        let url = objects
                            .iter()
                            .find(|object| object["object_id"].as_str() == Some(id.as_str()))
                            .and_then(|object| {
                                object["function_handle_metadata"]["web_url"].as_str()
                            })
                            .filter(|url| url.starts_with("https://"))
                            .map(str::to_owned);
                        (name, id, url)
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        Ok(found)
    }

    /// The workspace's default environment, where Modal puts what isn't
    /// aimed at another.
    async fn default_environment(&self, token: &Secret) -> Result<String, EngineError> {
        let list = self.call(token, "EnvironmentList", json!({})).await?;
        let items = list["items"].as_array().cloned().unwrap_or_default();
        let name = items
            .iter()
            .find(|item| item["default"].as_bool() == Some(true))
            .or_else(|| items.first())
            .and_then(|item| item["name"].as_str())
            .filter(|name| plain(name))
            .unwrap_or("main");
        Ok(name.to_owned())
    }

    /// Runs one of Modal's own commands where `environment` and `app` say.
    async fn command(
        &self,
        token: &Secret,
        environment: &str,
        app: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        let cli = manifest().cli.as_ref().ok_or(EngineError::Invalid(
            "Modal's command line isn't available in Hoista.",
        ))?;
        let mut context = vec![("environment", environment)];
        context.extend(app.map(|id| ("app", id)));
        super::cli::run(
            cli,
            &self.http,
            token,
            super::cli::Aim {
                context: &context,
                folder,
            },
            name,
            args,
        )
        .await
    }

    /// Deploys the app defined in `folder` as `app_name`, then finds its id.
    async fn deploy_folder(
        &self,
        token: &Secret,
        environment: &str,
        app_name: &str,
        folder: &std::path::Path,
    ) -> Result<String, EngineError> {
        let entry = entry_file(folder, app_name)?;
        self.command(
            token,
            environment,
            None,
            Some(folder),
            "modal deploy",
            json!({"app_ref": entry, "name": app_name}),
        )
        .await?;
        let apps = self.apps(token, environment).await?;
        apps.iter()
            .find(|app| {
                app["name"].as_str() == Some(app_name)
                    || app["description"].as_str() == Some(app_name)
            })
            .and_then(|app| app["app_id"].as_str())
            .map(|id| format!("app:{environment}:{id}"))
            .ok_or_else(|| EngineError::Provider {
                evidence: None,
                code: "provider.not_found".into(),
                message: format!("Modal deployed {app_name}, but Hoista can't see it in the {environment} environment yet."),
            })
    }

    async fn apps(&self, token: &Secret, environment: &str) -> Result<Vec<Value>, EngineError> {
        let list = self
            .call(token, "AppList", json!({"environment_name": environment}))
            .await?;
        Ok(list["apps"].as_array().cloned().unwrap_or_default())
    }
}

fn manifest() -> &'static super::manifest::Manifest {
    super::manifest::builtin("modal")
}

/// Folders a Modal app's code is never in.
const SKIPPED: [&str; 6] = [
    "node_modules",
    "venv",
    "__pycache__",
    "site-packages",
    "build",
    "dist",
];

/// The Python file in `folder` that defines the Modal app, as `modal deploy`
/// takes it: the only one that makes a `modal.App`, or the one of several
/// that names `app_name`. Looks two folders deep.
fn entry_file(folder: &std::path::Path, app_name: &str) -> Result<String, EngineError> {
    fn walk(
        root: &std::path::Path,
        at: &std::path::Path,
        depth: usize,
        found: &mut Vec<(String, String)>,
    ) {
        let Ok(entries) = std::fs::read_dir(at) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth < 2 && !name.starts_with('.') && !SKIPPED.contains(&name.as_str()) {
                    walk(root, &path, depth + 1, found);
                }
            } else if kind.is_file()
                && name.ends_with(".py")
                && entry
                    .metadata()
                    .is_ok_and(|meta| meta.len() < MAX_SOURCE_BYTES)
                && let Ok(text) = std::fs::read_to_string(&path)
                && text.contains("modal")
                && text.contains("App(")
                && let Ok(relative) = path.strip_prefix(root)
            {
                let relative = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                found.push((relative, text));
            }
        }
    }
    let mut found = Vec::new();
    walk(folder, folder, 0, &mut found);
    let named = |text: &str| {
        text.contains(&format!("\"{app_name}\"")) || text.contains(&format!("'{app_name}'"))
    };
    match found.as_slice() {
        [] => Err(EngineError::Provider {
            evidence: None,
            code: "provider.needs_folder".into(),
            message: "No Python file in this folder makes a Modal app (modal.App). Add one, or link the folder that has it.".into(),
        }),
        [(only, _)] => Ok(only.clone()),
        several => {
            let matching: Vec<&String> = several
                .iter()
                .filter(|(_, text)| named(text))
                .map(|(path, _)| path)
                .collect();
            if let [one] = matching.as_slice() {
                return Ok((*one).clone());
            }
            let names: Vec<&str> = several
                .iter()
                .take(MAX_FILES_NAMED)
                .map(|(path, _)| path.as_str())
                .collect();
            Err(EngineError::Provider {
                evidence: None,
                code: "provider.invalid_argument".into(),
                message: format!(
                    "More than one file makes a Modal app ({}). Run the operation modal deploy with app_ref set to the one you mean.",
                    names.join(", ")
                ),
            })
        }
    }
}

/// An app's state as a resource status. A deployed app with nothing
/// running has scaled to zero, which is Modal's normal rest.
fn app_status(app: &Value) -> Option<ResourceStatus> {
    let running = app["n_running_tasks"].as_i64().unwrap_or(0) > 0;
    match app["state"].as_str().unwrap_or_default() {
        "APP_STATE_DEPLOYED" if running => Some(ResourceStatus::Live),
        "APP_STATE_DEPLOYED" => Some(ResourceStatus::Sleeping),
        "APP_STATE_EPHEMERAL" | "APP_STATE_DETACHED" | "APP_STATE_DETACHED_DISCONNECTED" => {
            Some(ResourceStatus::Live)
        }
        "APP_STATE_INITIALIZING" => Some(ResourceStatus::Deploying),
        "APP_STATE_DISABLED" => Some(ResourceStatus::Paused),
        // Stopped apps are gone; derived ones belong to another app.
        _ => None,
    }
}

fn app_resource(environment: &str, app: &Value) -> Option<RemoteResource> {
    let id = app["app_id"].as_str().filter(|id| plain(id))?;
    let name = app["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .or_else(|| app["description"].as_str().filter(|text| !text.is_empty()))
        .unwrap_or(id);
    Some(RemoteResource {
        reference: format!("app:{environment}:{id}"),
        name: name.chars().take(MAX_NAME_CHARS).collect(),
        kind: ResourceKind::Service,
        region: None,
        status: app_status(app)?,
        url: None,
        group: None,
    })
}

/// A Modal metric unit as Hoista's, with how its headline is worked out.
fn unit_of(
    unit: &str,
) -> (
    crate::application::ports::providers::MetricUnit,
    crate::application::ports::providers::MetricSummary,
) {
    use crate::application::ports::providers::{MetricSummary as S, MetricUnit as U};
    match unit.to_ascii_lowercase().as_str() {
        "percent" | "%" | "ratio" => (U::Percent, S::Average),
        "seconds" | "s" | "sec" => (U::Seconds, S::Average),
        "milliseconds" | "ms" => (U::Milliseconds, S::Average),
        "bytes" | "b" => (U::Bytes, S::Average),
        "cores" | "cpu" => (U::Cores, S::Average),
        _ => (U::Count, S::Total),
    }
}

/// `input_count` → `Input count`.
fn words_of(measure: &str) -> String {
    let text = measure.replace(['_', '-'], " ");
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// A reference's kind, environment and id.
fn split(reference: &str) -> Result<(&str, &str, &str), EngineError> {
    let mut parts = reference.splitn(3, ':');
    let (Some(kind), Some(environment), Some(id)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    if !matches!(kind, "app" | "volume") || !plain(environment) || !plain(id) {
        return Err(invalid());
    }
    Ok((kind, environment, id))
}

/// One operation of Hoista's Modal catalog (`modal/ops.json`).
#[derive(Deserialize)]
struct Op {
    name: String,
    area: String,
    risk: ProviderRisk,
    description: String,
    /// Fields Hoista fills in from the resource: `app_id` ← `app`.
    #[serde(default)]
    context: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Catalog {
    operations: Vec<Op>,
}

fn catalog() -> &'static [Op] {
    static CATALOG: OnceLock<Vec<Op>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str::<Catalog>(include_str!("modal/ops.json"))
            .map(|catalog| catalog.operations)
            .unwrap_or_default()
            .into_iter()
            // Only methods the descriptor has, answered once or as a stream.
            .filter(|op| method(&op.name).is_ok_and(|method| !method.is_client_streaming()))
            .collect()
    })
}

/// A field's JSON schema, from its protobuf type.
fn schema(field: &FieldDescriptor, depth: usize) -> Value {
    let one = match field.kind() {
        Kind::Bool => json!({"type": "boolean"}),
        Kind::String => json!({"type": "string"}),
        Kind::Bytes => json!({"type": "string", "description": "Base64"}),
        Kind::Double | Kind::Float => json!({"type": "number"}),
        Kind::Enum(values) => json!({
            "type": "string",
            "enum": values.values().map(|value| value.name().to_owned()).collect::<Vec<_>>(),
        }),
        Kind::Message(message) if message.full_name() == "google.protobuf.Timestamp" => {
            json!({"type": "string", "format": "date-time"})
        }
        Kind::Message(message) => object(&message, depth + 1),
        _ => json!({"type": "integer"}),
    };
    if field.is_list() {
        json!({"type": "array", "items": one})
    } else {
        one
    }
}

fn object(message: &MessageDescriptor, depth: usize) -> Value {
    if depth > MAX_SCHEMA_DEPTH || message.is_map_entry() {
        return json!({"type": "object"});
    }
    let properties: Map<String, Value> = message
        .fields()
        .map(|field| (field.name().to_owned(), schema(&field, depth)))
        .collect();
    json!({"type": "object", "properties": properties})
}

#[async_trait::async_trait]
impl ProviderAdapter for Modal {
    fn provider(&self) -> Provider {
        Provider::Modal
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        let (id, name) = self.workspace(token).await?;
        Ok(vec![AccountChoice {
            external_id: id,
            name,
        }])
    }

    async fn verify(&self, token: &Secret, workspace: &str) -> Result<Vec<String>, EngineError> {
        if !plain(workspace) {
            return Err(EngineError::Invalid("A Modal workspace is required."));
        }
        if self.workspace(token).await?.0 != workspace {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.token_scope".into(),
                message: "This token is for a different Modal workspace than the one connected. \
                          Connect it as a new account."
                    .into(),
            });
        }
        Ok(vec!["Reads the workspace (verified by request)".into()])
    }

    async fn discover(
        &self,
        token: &Secret,
        _workspace: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        for environment in self.environments(token).await? {
            for app in self.apps(token, &environment).await? {
                let Some(mut resource) = app_resource(&environment, &app) else {
                    continue;
                };
                // A deployed app's web endpoint is its address.
                if app["state"].as_str() == Some("APP_STATE_DEPLOYED")
                    && let Some(id) = app["app_id"].as_str()
                    && let Ok(functions) = self
                        .functions_with_urls(token, &environment, id, &resource.name)
                        .await
                {
                    resource.url = functions.into_iter().find_map(|(_, _, url)| url);
                }
                resources.push(resource);
            }
            let volumes = self
                .call(
                    token,
                    "VolumeList",
                    json!({"environment_name": environment}),
                )
                .await?;
            for volume in volumes["items"].as_array().cloned().unwrap_or_default() {
                let Some(id) = volume["volume_id"].as_str().filter(|id| plain(id)) else {
                    continue;
                };
                resources.push(RemoteResource {
                    reference: format!("volume:{environment}:{id}"),
                    name: volume["label"]
                        .as_str()
                        .unwrap_or(id)
                        .chars()
                        .take(MAX_NAME_CHARS)
                        .collect(),
                    kind: ResourceKind::Bucket,
                    region: None,
                    status: ResourceStatus::Live,
                    url: None,
                    group: None,
                });
            }
            if resources.len() > MAX_DISCOVERED_RESOURCES {
                return Err(invalid());
            }
        }
        Ok(resources)
    }

    /// An app is as its state says; a stopped one is gone.
    async fn observe_resource(
        &self,
        token: &Secret,
        _workspace: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let (kind, environment, id) = split(&resource.provider_ref)?;
        if kind == "volume" {
            let volumes = self
                .call(
                    token,
                    "VolumeList",
                    json!({"environment_name": environment}),
                )
                .await?;
            let there = volumes["items"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["volume_id"] == id));
            return Ok(Observed {
                status: if there {
                    ResourceStatus::Live
                } else {
                    ResourceStatus::Stopped
                },
                url: None,
                kind: Some(ResourceKind::Bucket),
            });
        }
        let status = self
            .apps(token, environment)
            .await?
            .iter()
            .find(|app| app["app_id"] == id)
            .and_then(app_status)
            .unwrap_or(ResourceStatus::Stopped);
        Ok(Observed {
            status,
            url: None,
            kind: Some(ResourceKind::Service),
        })
    }

    /// Stops an app, or deletes a volume.
    async fn delete(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let (kind, environment, id) = split(reference)?;
        if kind == "volume" {
            self.call(
                token,
                "VolumeDelete",
                json!({"volume_id": id, "environment_name": environment}),
            )
            .await?;
        } else {
            self.call(
                token,
                "AppStop",
                json!({"app_id": id, "source": "APP_STOP_SOURCE_CLI"}),
            )
            .await?;
        }
        Ok(())
    }

    /// The workspace's metered cost this calendar month, as Modal reports it.
    async fn provider_bill(
        &self,
        token: &Secret,
        _workspace: &str,
    ) -> Result<Option<ProviderBill>, EngineError> {
        use chrono::{Datelike, TimeZone};
        let now = chrono::Utc::now();
        let start = chrono::Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single()
            .unwrap_or(now)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let summary = self
            .call(
                token,
                "WorkspaceBillingSummary",
                json!({"start_timestamp": start}),
            )
            .await?;
        let Some(cost) = summary["metered_cost"]
            .as_str()
            .and_then(|text| text.parse::<f64>().ok())
        else {
            return Ok(None);
        };
        if !cost.is_finite() || cost < 0.0 {
            return Err(invalid());
        }
        let cents = crate::units::cents(cost);
        Ok(Some(ProviderBill {
            billed: Money {
                cents,
                currency: "USD".into(),
            },
            period_start: Some(start),
            period_end: None,
            soft_limit: None,
            hard_limit: None,
            over_limit: false,
            customer_id: None,
        }))
    }

    /// The app's logs in the range, as Modal keeps them: what its
    /// containers printed to stdout and stderr, and Modal's own notes.
    async fn log_history(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        use crate::domain::logs::{LogEntry, LogHistory, LogLevel};
        query.milliseconds().map_err(EngineError::Invalid)?;
        let (kind, _, id) = split(reference)?;
        if kind != "app" {
            return Err(crate::application::ports::providers::unsupported());
        }
        let mut request = json!({
            "app_id": id,
            "since": query.from,
            "until": query.to,
            "limit": MAX_LOG_LINES,
        });
        if !query.query.trim().is_empty() {
            request["search_text"] =
                Value::String(query.query.chars().take(MAX_SEARCH_CHARS).collect());
        }
        let answer = self.call(token, "AppFetchLogs", request).await?;
        let scanner = crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
        let observed = chrono::Utc::now().to_rfc3339();
        let mut items = Vec::new();
        for batch in answer["batches"].as_array().into_iter().flatten() {
            for log in batch["items"].as_array().into_iter().flatten() {
                let Some(text) = log["data"].as_str().filter(|text| !text.trim().is_empty()) else {
                    continue;
                };
                let message = crate::domain::logs::clipped(
                    scanner.redact(text.trim_end(), std::slice::from_ref(token)),
                );
                let stream = log["file_descriptor"].as_str().unwrap_or_default();
                let level = if message.contains("Traceback") || message.contains("Error") {
                    LogLevel::Error
                } else if stream == "FILE_DESCRIPTOR_STDERR" {
                    LogLevel::Warn
                } else {
                    LogLevel::Info
                };
                let timestamp = log["timestamp"]
                    .as_f64()
                    .and_then(crate::units::from_unix_seconds)
                    .map(|at| at.to_rfc3339());
                items.push(LogEntry {
                    timestamp,
                    observed_at: observed.clone(),
                    level,
                    source: match stream {
                        "FILE_DESCRIPTOR_STDERR" => "stderr",
                        "FILE_DESCRIPTOR_INFO" => "modal",
                        _ => "stdout",
                    }
                    .into(),
                    message,
                });
            }
        }
        items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        items.truncate(100);
        Ok(LogHistory {
            items,
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: false,
        })
    }

    /// The app's functions' metrics as Modal measures them: one chart per
    /// measure, one line per function (at most eight functions).
    async fn provider_metrics(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::stamp;
        let (kind, _, id) = split(reference)?;
        if kind != "app" {
            return Err(crate::application::ports::providers::unsupported());
        }
        let layout = self
            .call(token, "AppGetLayout", json!({"app_id": id}))
            .await?;
        let functions: Vec<(String, String)> = layout["app_layout"]["function_ids"]
            .as_object()
            .map(|map| {
                map.iter()
                    .filter_map(|(name, id)| Some((name.clone(), id.as_str()?.to_owned())))
                    .filter(|(_, id)| plain(id))
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
        let mut by_measure: std::collections::BTreeMap<String, (String, Vec<ProviderSeries>)> =
            std::collections::BTreeMap::new();
        let mut missing = Vec::new();
        for (name, function) in &functions {
            let answer = self
                .call(
                    token,
                    "FunctionGetMetrics",
                    json!({
                        "function_id": function,
                        "since": stamp(from),
                        "until": stamp(to),
                        "bucket_secs": step_seconds.clamp(MIN_BUCKET_SECS, MAX_BUCKET_SECS),
                    }),
                )
                .await;
            let Ok(answer) = answer else {
                missing.push(name.as_str());
                continue;
            };
            for series in answer["series"].as_array().into_iter().flatten() {
                let Some(measure) = series["name"].as_str() else {
                    continue;
                };
                let points = super::metrics::points(
                    series["points"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|point| {
                            Some((
                                super::metrics::time_of(&point["timestamp"])?,
                                point["value"].as_f64().unwrap_or(0.0),
                            ))
                        }),
                );
                let entry = by_measure.entry(measure.to_owned()).or_insert_with(|| {
                    (
                        series["unit"].as_str().unwrap_or_default().to_owned(),
                        Vec::new(),
                    )
                });
                entry.1.push(super::metrics::line(name, points));
            }
        }
        let charts = by_measure
            .into_iter()
            .map(|(measure, (unit, lines))| {
                let (unit, summary) = unit_of(&unit);
                super::metrics::chart(&measure, &words_of(&measure), unit, summary, lines)
            })
            .collect();
        Ok(super::metrics::metrics(from, to, charts, &missing, "Modal"))
    }

    fn provider_operations(&self) -> Vec<ProviderOperationInfo> {
        let mut operations: Vec<ProviderOperationInfo> = catalog()
            .iter()
            .filter_map(|op| {
                let method = method(&op.name).ok()?;
                Some(ProviderOperationInfo {
                    name: op.name.clone(),
                    area: op.area.clone(),
                    risk: op.risk,
                    request: if method.is_server_streaming() {
                        "RPC stream".into()
                    } else {
                        "RPC".into()
                    },
                    description: op.description.clone(),
                    args: method
                        .input()
                        .fields()
                        .map(|field| ProviderOperationArg {
                            name: field.name().to_owned(),
                            required: false,
                            filled_in: op.context.get(field.name()).cloned(),
                            schema: schema(&field, 0),
                        })
                        .collect(),
                })
            })
            .collect();
        operations.extend(manifest().operations());
        operations
    }

    fn prepare_tools(&self) {
        if let Some(cli) = manifest().cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        manifest().actions.clone()
    }

    async fn provider_details(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        use crate::application::ports::providers::{
            ProviderDeployState, ProviderDeployment, ProviderDetails, ProviderDomain,
            ProviderDomainState,
        };
        let (kind, environment, id) = split(reference)?;
        if kind != "app" {
            return Err(crate::application::ports::providers::unsupported());
        }
        let app_name = self.app_name(token, environment, id).await?;
        let history = self
            .call(token, "AppDeploymentHistory", json!({"app_id": id}))
            .await?;
        let mut rows = history["app_deployment_histories"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        rows.sort_by_key(|row| std::cmp::Reverse(row["version"].as_u64().unwrap_or(0)));
        let newest = rows.first().and_then(|row| row["version"].as_u64());
        let deployments = rows
            .iter()
            .take(MAX_DEPLOYMENTS)
            .filter_map(|row| {
                let version = row["version"].as_u64()?;
                let at = row["deployed_at"]
                    .as_f64()
                    .and_then(crate::units::from_unix_seconds)?;
                let live = Some(version) == newest;
                Some(ProviderDeployment {
                    id: format!("v{version}"),
                    status: if live {
                        "DEPLOYED".into()
                    } else {
                        "REPLACED".into()
                    },
                    // Each went live; the older ones were later replaced.
                    state: ProviderDeployState::Succeeded,
                    created_at: at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    url: None,
                    commit: row["commit_info"]["commit_hash"]
                        .as_str()
                        .filter(|hash| !hash.is_empty())
                        .map(str::to_owned),
                    message: row["tag"]
                        .as_str()
                        .filter(|tag| !tag.is_empty())
                        .map(|tag| format!("Tag {tag}"))
                        .or_else(|| Some(format!("Version {version}"))),
                    branch: row["commit_info"]["branch"]
                        .as_str()
                        .filter(|branch| !branch.is_empty())
                        .map(str::to_owned),
                    repo: None,
                    finished_at: Some(at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                    can_roll_back: row["rollback_allowed"].as_bool(),
                    author: row["deployed_by"]
                        .as_str()
                        .filter(|by| !by.is_empty())
                        .map(str::to_owned),
                    provider_page: None,
                })
            })
            .collect();
        let domains = self
            .functions_with_urls(token, environment, id, &app_name)
            .await?
            .into_iter()
            .filter_map(|(_, _, url)| url)
            .map(|url| ProviderDomain {
                name: url
                    .trim_start_matches("https://")
                    .trim_end_matches('/')
                    .to_owned(),
                custom: false,
                state: Some(ProviderDomainState::Working),
                records: Vec::new(),
                port: None,
                certificate: None,
                dns: Vec::new(),
            })
            .collect();
        Ok(ProviderDetails {
            deployments,
            variable_names: None,
            shared_variable_names: None,
            domains,
            references: Vec::new(),
            sleeps: Some(true),
            config: None,
            volumes: Vec::new(),
            tcp_proxies: Vec::new(),
            private_domain: None,
            addresses: Vec::new(),
        })
    }

    async fn provider_upload(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: &str,
        folder: &std::path::Path,
        _message: Option<&str>,
    ) -> Result<String, EngineError> {
        let (kind, environment, id) = split(reference)?;
        if kind != "app" {
            return Err(EngineError::Invalid(
                "Only a Modal app deploys from a folder.",
            ));
        }
        let apps = self.apps(token, environment).await?;
        let name = apps
            .iter()
            .find(|app| app["app_id"].as_str() == Some(id))
            .and_then(|app| app["name"].as_str().or_else(|| app["description"].as_str()))
            .filter(|name| plain(name))
            .ok_or(EngineError::NotFound)?
            .to_owned();
        self.deploy_folder(token, environment, &name, folder)
            .await?;
        Ok(format!("Deployed {name} to Modal from its folder"))
    }

    async fn provider_new_from_folder(
        &self,
        token: &Secret,
        _workspace: &str,
        name: &str,
        folder: &std::path::Path,
    ) -> Result<(Vec<String>, String), EngineError> {
        let environment = self.default_environment(token).await?;
        let reference = self
            .deploy_folder(token, &environment, name, folder)
            .await?;
        Ok((
            vec![reference],
            format!("Deployed {name} to Modal's {environment} environment from its folder"),
        ))
    }

    /// One of Modal's methods from Hoista's catalog, aimed at the app or
    /// volume: its id and environment are filled in, and can't be pointed
    /// elsewhere.
    async fn provider_operation(
        &self,
        token: &Secret,
        _workspace: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        if manifest()
            .cli
            .as_ref()
            .is_some_and(|cli| super::cli::has(cli, name))
        {
            if name == "modal deploy" && reference.is_none() {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "provider.needs_folder".into(),
                    message: "modal deploy here redeploys an app Hoista already watches, from its project's folder. For a first deploy, call create_project_at_provider with folder (a new project) or project_id (an empty project linked to a folder).".into(),
                });
            }
            let (environment, app) = match reference {
                Some(reference) => {
                    let (kind, environment, id) = split(reference)?;
                    (environment.to_owned(), (kind == "app").then_some(id))
                }
                None => (self.default_environment(token).await?, None),
            };
            return self
                .command(token, &environment, app, folder, name, args)
                .await;
        }
        let op = catalog()
            .iter()
            .find(|op| op.name == name)
            .ok_or(EngineError::Invalid(
                "Modal has no such operation in Hoista. Call list_provider_operations for the list.",
            ))?;
        let mut given = match args {
            Value::Null => Map::new(),
            Value::Object(map) => map,
            _ => return Err(EngineError::Invalid("Give arguments as an object.")),
        };
        let (kind, environment, id) = match reference {
            Some(reference) => {
                let (kind, environment, id) = split(reference)?;
                (Some(kind), Some(environment), Some(id))
            }
            None => (None, None, None),
        };
        for (field, fills) in &op.context {
            let value = match fills.as_str() {
                "environment" => environment,
                "app" if kind == Some("app") => id,
                "volume" if kind == Some("volume") => id,
                _ => None,
            };
            if let Some(value) = value {
                given.insert(field.clone(), Value::String(value.to_owned()));
            }
        }
        let input = method(name)?.input();
        // A function of the app it runs on, when the app has just one.
        if input.get_field_by_name("function_id").is_some()
            && given
                .get("function_id")
                .is_none_or(|value| value.as_str().is_none_or(str::is_empty))
            && let (Some("app"), Some(app)) = (kind, id)
        {
            let functions = self.functions(token, app).await?;
            match functions.as_slice() {
                [(_, only, _)] => {
                    given.insert("function_id".into(), Value::String(only.clone()));
                }
                [] => {}
                several => {
                    let names: Vec<String> = several
                        .iter()
                        .map(|(name, id, _)| format!("{name} ({id})"))
                        .collect();
                    return Err(EngineError::Provider {
                        evidence: None,
                        code: "provider.invalid_argument".into(),
                        message: format!(
                            "This app has more than one function. Give function_id: {}.",
                            names.join(", ")
                        ),
                    });
                }
            }
        }
        // Modal needs a window and a size where Hoista's own views give none:
        // the last hour, 100 lines, 5-minute buckets.
        let now = chrono::Utc::now();
        for (field, value) in [
            ("limit", json!(100)),
            (
                "since",
                json!((now - chrono::Duration::hours(1)).to_rfc3339()),
            ),
            ("until", json!(now.to_rfc3339())),
            ("bucket_secs", json!(300)),
            // Billing summaries start this month; Modal refuses more than a
            // year back, which is where an empty start lands.
            (
                "start_timestamp",
                json!(
                    chrono::Datelike::with_day(&now.date_naive(), 1)
                        .and_then(|day| day.and_hms_opt(0, 0, 0))
                        .map(|start| start.and_utc().to_rfc3339())
                        .unwrap_or_else(|| now.to_rfc3339())
                ),
            ),
        ] {
            if input.get_field_by_name(field).is_some()
                && given
                    .get(field)
                    .is_none_or(|given| given.is_null() || given == &json!(0))
                && name != "AppList"
            {
                given.insert(field.into(), value);
            }
        }
        let mut answer = self.call(token, name, Value::Object(given)).await?;
        super::api_catalog::clean(&mut answer, token)?;
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_found_in_what_modal_prints() {
        let pasted = Secret::new(
            "modal token set --token-id ak-AbC123 --token-secret as-XyZ789 --profile=sam".into(),
        );
        assert_eq!(
            credentials(&pasted).unwrap(),
            ("ak-AbC123".to_owned(), "as-XyZ789".to_owned())
        );
        assert_eq!(
            credentials(&Secret::new("ak-1:as-2".into())).unwrap(),
            ("ak-1".to_owned(), "as-2".to_owned())
        );
        assert!(credentials(&Secret::new("ak-only".into())).is_err());
    }

    #[test]
    fn messages_are_framed_and_unframed_as_grpc_does() {
        let body = [
            frame(b"one").unwrap(),
            frame(b"").unwrap(),
            frame(b"three").unwrap(),
        ]
        .concat();
        let parts = frames(&body, 10).unwrap();
        assert_eq!(parts, vec![&b"one"[..], &b""[..], &b"three"[..]]);
        assert!(frames(&body[..body.len() - 1], 10).is_err());
        assert!(frames(&body, 2).is_err());
    }

    #[test]
    fn the_app_file_is_found_in_the_folder() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        assert!(entry_file(root, "llm").is_err());
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".venv/lib")).unwrap();
        std::fs::write(
            root.join(".venv/lib/x.py"),
            "import modal\napp = modal.App('llm')",
        )
        .unwrap();
        std::fs::write(root.join("README.md"), "modal.App(").unwrap();
        std::fs::write(
            root.join("src/serve.py"),
            "import modal\napp = modal.App(\"llm\")",
        )
        .unwrap();
        assert_eq!(entry_file(root, "llm").unwrap(), "src/serve.py");
        std::fs::write(
            root.join("train.py"),
            "import modal\napp = modal.App('trainer')",
        )
        .unwrap();
        assert_eq!(entry_file(root, "llm").unwrap(), "src/serve.py");
        assert_eq!(entry_file(root, "trainer").unwrap(), "train.py");
        assert!(entry_file(root, "other").is_err());
    }

    #[test]
    fn requests_and_answers_follow_modals_own_messages() {
        let method = method("AppList").unwrap();
        let request =
            DynamicMessage::deserialize(method.input(), json!({"environment_name": "main"}))
                .unwrap();
        assert!(!request.encode_to_vec().is_empty());
        assert!(DynamicMessage::deserialize(method.input(), json!({"nope": 1})).is_err());
        let answer = DynamicMessage::deserialize(
            method.output(),
            json!({
                "apps": [
                    {
                        "app_id": "ap-1",
                        "name": "llm",
                        "state": "APP_STATE_DEPLOYED",
                        "n_running_tasks": 2,
                    },
                ],
            }),
        )
        .unwrap();
        let round =
            DynamicMessage::decode(method.output(), answer.encode_to_vec().as_slice()).unwrap();
        let value = to_json(&round).unwrap();
        let found = app_resource("main", &value["apps"][0]).unwrap();
        assert_eq!(found.reference, "app:main:ap-1");
        assert_eq!(found.status, ResourceStatus::Live);
        assert_eq!(
            app_status(&json!({"state":"APP_STATE_DEPLOYED"})),
            Some(ResourceStatus::Sleeping)
        );
        assert_eq!(app_status(&json!({"state":"APP_STATE_STOPPED"})), None);
    }

    #[test]
    fn the_catalog_is_aimed_at_the_resource() {
        let modal = Modal::new(ProviderHttp::new().unwrap());
        let ops = modal.provider_operations();
        assert!(ops.len() >= 20, "{} operations", ops.len());
        let stop = ops.iter().find(|op| op.name == "AppStop").unwrap();
        assert_eq!(stop.risk, ProviderRisk::Destroy);
        assert_eq!(
            stop.args
                .iter()
                .find(|arg| arg.name == "app_id")
                .unwrap()
                .filled_in
                .as_deref(),
            Some("app")
        );
        assert!(ops.iter().all(|op| !op.name.contains("Token")));
        assert!(split("app:main:ap-1").is_ok());
        assert!(split("app:main:../x").is_err());
        assert!(split("machine:main:x").is_err());
    }
}
