//! Cloudflare: finds Workers, Pages projects, D1 databases, KV namespaces, R2
//! buckets and domains (zones), and deploys, configures and rolls back Workers.

use super::http::{ProviderHttp, focus_bill};
use crate::{
    Provider,
    application::ports::{catalog::EngineError, providers::*},
    domain::{Secret, model::*},
};
use base64::Engine as _;
use reqwest::{
    Method,
    multipart::{Form, Part as FormPart},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Cloudflare's adapter, using only Cloudflare's documented API.
pub struct Cloudflare {
    http: ProviderHttp,
}

/// Cloudflare's entry in the provider list.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Cloudflare,
    logo: LOGO,
    capabilities: Capabilities {
        bills: true,
        provider_variables: true,
        deploy: true,
        logs: true,
        metrics: true,
        usage: true,
        rollback: true,
        domains: true,
        secrets: true,
        discover: true,
        browser_sign_in: CLIENT_ID.is_some(),
        provider_operations: true,
        provider_details: true,
        delete: true,
        new_from_folder: true,
        new_database: true,
        new_bucket: true,
        plan_deploy: true,
        cron: true,
        dns_zones: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://api.cloudflare.com/client/v4/",
        hosts: &["api.cloudflare.com", "dash.cloudflare.com"],
        explains_refusals: false,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["accounts"],
            query: &[("per_page", "50")],
            page: Some("page"),
            cursor: None,
        },
        rows: &["result"],
        item: &[],
        id: "id",
        pages: super::accounts::Pages::Counted(&["result_info", "total_pages"]),
    },
    adapter: |http| std::sync::Arc::new(Cloudflare::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// Limits on what a Worker's settings may hold.
const MAX_SCHEDULES: usize = 100;
const MAX_BINDINGS: usize = 500;
const MAX_SETTING_CHARS: usize = 256;
const MAX_NAME_CHARS: usize = 128;
/// How much of a `workers.dev` subdomain comes from the account's name.
const MAX_SUBDOMAIN_BASE_CHARS: usize = 40;
/// The most of a Wrangler config, and of a Worker's module, that's read.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_MODULE_BYTES: u64 = 10 * 1024 * 1024;
/// Scripts checked at once for a `workers.dev` address.
const PARALLEL_LOOKUPS: usize = 6;

/// Hoista's Cloudflare OAuth app (a self-managed OAuth client): a public client
/// with PKCE and no secret. On Cloudflare's page the person picks the accounts
/// to share and can switch off optional scopes. `offline_access` returns a
/// refresh token. Registered with `http://127.0.0.1:{47131,47132}/callback`.
const CLIENT_ID: Option<&str> = option_env!("HOISTA_CLOUDFLARE_CLIENT_ID");

/// Cloudflare's dashboard.
const DASHBOARD: &str = "https://dash.cloudflare.com";
/// Where Cloudflare's sign-in page sends people.
const OAUTH_AUTHORIZE_URL: &str = "https://dash.cloudflare.com/oauth2/auth";
/// Where Cloudflare's sign-in codes and refresh tokens are exchanged.
const OAUTH_TOKEN_URL: &str = "https://dash.cloudflare.com/oauth2/token";

/// A Worker's address on the account's `workers.dev` subdomain.
fn workers_dev_url(script: &str, subdomain: &str) -> String {
    format!("https://{script}.{subdomain}.workers.dev")
}

fn oauth_app() -> Option<crate::application::ports::providers::ProviderOAuth> {
    CLIENT_ID.map(
        |client_id| crate::application::ports::providers::ProviderOAuth {
            authorize_url: OAUTH_AUTHORIZE_URL,
            client_id,
            // The client's scopes, exactly as registered at Cloudflare. The first
            // five are required; the rest can be switched off on the consent page.
            scopes: "offline_access workers-scripts.edit workers-tail.read workers-observability.write account-settings.read account-analytics.read workers-kv-storage.write workers-r2.write workers-routes.write d1.write page.write query-cache.write workers-ci.write containers.write pipelines.write queues.write vectorize.write ai.write secrets-store.write browser-rendering.write email-routing-address.write email-routing-rule.write zone.read dns.write ssl-and-certificates.write cache.purge",
            ports: &[47131, 47132],
            extra: &[],
            redirect_host: "127.0.0.1",
            logo: LOGO,
        },
    )
}

/// Cloudflare's mark, for Hoista's sign-in pages.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M16.5088 16.8447c.1475-.5068.0908-.9707-.1553-1.3154-.2246-.3164-.6045-.499-1.0615-.5205l-8.6592-.1123a.1559.1559 0 0 1-.1333-.0713c-.0283-.042-.0351-.0986-.021-.1553.0278-.084.1123-.1484.2036-.1562l8.7359-.1123c1.0351-.0489 2.1601-.8868 2.5537-1.9136l.499-1.3013c.0215-.0561.0293-.1128.0147-.168-.5625-2.5463-2.835-4.4453-5.5499-4.4453-2.5039 0-4.6284 1.6177-5.3876 3.8614-.4927-.3658-1.1187-.5625-1.794-.499-1.2026.119-2.1665 1.083-2.2861 2.2856-.0283.31-.0069.6128.0635.894C1.5683 13.171 0 14.7754 0 16.752c0 .1748.0142.3515.0352.5273.0141.083.0844.1475.1689.1475h15.9814c.0909 0 .1758-.0645.2032-.1553l.12-.4268zm2.7568-5.5634c-.0771 0-.1611 0-.2383.0112-.0566 0-.1054.0415-.127.0976l-.3378 1.1744c-.1475.5068-.0918.9707.1543 1.3164.2256.3164.6055.498 1.0625.5195l1.8437.1133c.0557 0 .1055.0263.1329.0703.0283.043.0351.1074.0214.1562-.0283.084-.1132.1485-.204.1553l-1.921.1123c-1.041.0488-2.1582.8867-2.5527 1.914l-.1406.3585c-.0283.0713.0215.1416.0986.1416h6.5977c.0771 0 .1474-.0489.169-.126.1122-.4082.1757-.837.1757-1.2803 0-2.6025-2.125-4.727-4.7344-4.727" fill="#F38020"/></svg>"##;

/// A fake of the provider for the conformance suite: enough to connect and
/// find one resource.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "acct-1",
    resource: "worker:res-1",
    kind: crate::domain::model::ResourceKind::Web,
    routes: &[
        super::conformance::Route {
            path: "/accounts?per_page",
            body: "",
            answer: r#"{"success":true,"result":[{"id":"acct-1","name":"Sam"}],"result_info":{"total_pages":1}}"#,
        },
        super::conformance::Route {
            path: "/user/tokens/verify",
            body: "",
            answer: r#"{"success":true,"result":{"id":"tok-1","status":"active"}}"#,
        },
        super::conformance::Route {
            path: "/user/tokens/tok-1",
            body: "",
            answer: r#"{"success":true,"result":{"id":"tok-1","status":"active","policies":[]}}"#,
        },
        super::conformance::Route {
            path: "/r2/buckets",
            body: "",
            answer: r#"{"success":true,"result":{"buckets":[]},"result_info":{"cursor":""}}"#,
        },
        super::conformance::Route {
            path: "/workers/scripts",
            body: "",
            answer: r#"{"success":true,"result":[{"id":"res-1"}],"result_info":{"total_pages":1}}"#,
        },
    ],
    empty: r#"{"success":true,"result":[],"result_info":{"total_pages":1,"cursor":""}}"#,
};

impl Cloudflare {
    /// The account's workers.dev subdomain, set up when it has none yet, so
    /// Workers can have a public address and cron schedules without a visit
    /// to Cloudflare's dashboard. A new one is named after `wanted` (the
    /// project or Worker), with a short random ending if Cloudflare says that name is
    /// taken. Returns the subdomain and whether it was just made.
    async fn workers_subdomain(
        &self,
        token: &Secret,
        account: &str,
        wanted: &str,
    ) -> Result<(String, bool), EngineError> {
        match self
            .call(
                token,
                Method::GET,
                &["accounts", account, "workers", "subdomain"],
                &[],
                None,
            )
            .await
        {
            Ok(value) => {
                if let Some(subdomain) = value["subdomain"].as_str().filter(|name| valid_slug(name))
                {
                    return Ok((subdomain.to_owned(), false));
                }
            }
            Err(EngineError::Provider { ref code, .. }) if code == "provider.not_found" => {}
            Err(error) => return Err(error),
        }
        let mut base: String = wanted
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        base.truncate(MAX_SUBDOMAIN_BASE_CHARS);
        let base = base.trim_end_matches('-');
        let base = if base.is_empty() { "workers" } else { base };
        for attempt in 0..4 {
            let name = if attempt == 0 {
                base.to_owned()
            } else {
                let random = uuid::Uuid::new_v4().simple().to_string();
                format!("{base}-{}", &random[..5])
            };
            match self
                .call(
                    token,
                    Method::PUT,
                    &["accounts", account, "workers", "subdomain"],
                    &[],
                    Some(&json!({ "subdomain": name })),
                )
                .await
            {
                Ok(value) => {
                    let made = value["subdomain"].as_str().unwrap_or(&name).to_owned();
                    return Ok((made, true));
                }
                // Taken (Cloudflare refuses a name someone else has): try another.
                Err(EngineError::Provider { ref code, .. }) if code == "provider.rejected" => {}
                Err(error) => return Err(error),
            }
        }
        Err(EngineError::Conflict(
            "Cloudflare didn't accept a workers.dev name for this account. Try again in a minute.",
        ))
    }

    async fn read_schedules(
        &self,
        token: &Secret,
        account: &str,
        script: &str,
    ) -> Result<Vec<String>, EngineError> {
        let value = self
            .call(
                token,
                Method::GET,
                &[
                    "accounts",
                    account,
                    "workers",
                    "scripts",
                    script,
                    "schedules",
                ],
                &[],
                None,
            )
            .await?;
        let rows = value
            .get("schedules")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        if rows.len() > 100 {
            return Err(limit());
        }
        let mut schedules = Vec::new();
        for row in rows {
            let cron = text(row, "cron")?;
            if cron.is_empty()
                || cron.len() > MAX_SETTING_CHARS
                || cron.chars().any(char::is_control)
                || schedules.iter().any(|value| value == cron)
            {
                return Err(invalid());
            }
            schedules.push(cron.to_owned());
        }
        Ok(schedules)
    }

    async fn write_schedules(
        &self,
        token: &Secret,
        account: &str,
        script: &str,
        schedules: &[String],
    ) -> Result<(), EngineError> {
        if schedules.len() > MAX_SCHEDULES
            || schedules.iter().any(|value| {
                value.is_empty()
                    || value.len() > MAX_SETTING_CHARS
                    || value.chars().any(char::is_control)
            })
        {
            return Err(invalid());
        }
        let rows: Vec<_> = schedules.iter().map(|cron| json!({"cron":cron})).collect();
        self.call(
            token,
            Method::PUT,
            &[
                "accounts",
                account,
                "workers",
                "scripts",
                script,
                "schedules",
            ],
            &[],
            Some(&json!(rows)),
        )
        .await?;
        Ok(())
    }

    /// An adapter on the shared provider HTTP client.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    /// Uploads a one-module Worker as a new version and serves it, keeping
    /// the secrets and variables already set on it, then sets its
    /// workers.dev route as its configuration says.
    async fn upload_plain_worker(
        &self,
        token: &Secret,
        account: &str,
        script: &str,
        upload: &PlainWorker,
    ) -> Result<(), EngineError> {
        if !valid_slug(script) {
            return Err(invalid());
        }
        let metadata = json!({
            "main_module": upload.file_name,
            "compatibility_date": upload.compatibility_date,
            "compatibility_flags": upload.compatibility_flags,
            "bindings": [],
            "keep_bindings": ["secret_text", "plain_text"],
        });
        let form = Form::new()
            .part(
                "metadata",
                FormPart::text(metadata.to_string())
                    .mime_str("application/json")
                    .map_err(|_| invalid())?,
            )
            .part(
                upload.file_name.clone(),
                FormPart::bytes(upload.module.clone())
                    .file_name(upload.file_name.clone())
                    .mime_str("application/javascript+module")
                    .map_err(|_| invalid())?,
            );
        self.http
            .multipart(
                Provider::Cloudflare,
                token,
                Method::PUT,
                &["accounts", account, "workers", "scripts", script],
                &[],
                form,
            )
            .await?;
        self.call(
            token,
            Method::POST,
            &[
                "accounts",
                account,
                "workers",
                "scripts",
                script,
                "subdomain",
            ],
            &[],
            Some(&json!({"enabled": upload.workers_dev, "previews_enabled": false})),
        )
        .await?;
        Ok(())
    }

    /// Whether the Worker's workers.dev route is turned on; false when
    /// Cloudflare won't say.
    async fn script_on_workers_dev(&self, token: &Secret, account: &str, script: &str) -> bool {
        self.call(
            token,
            Method::GET,
            &[
                "accounts",
                account,
                "workers",
                "scripts",
                script,
                "subdomain",
            ],
            &[],
            None,
        )
        .await
        .ok()
        .and_then(|value| value["enabled"].as_bool())
        .unwrap_or(false)
    }

    async fn call(
        &self,
        token: &Secret,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value, EngineError> {
        let value = self
            .http
            .json(Provider::Cloudflare, token, method, path, query, body)
            .await?;
        Ok(value.get("result").cloned().unwrap_or(value))
    }

    /// Every row of a paged list, `size` rows a page (each list has its own
    /// limit: Pages projects take at most 10).
    async fn pages(
        &self,
        token: &Secret,
        path: &[&str],
        size: usize,
    ) -> Result<Vec<Value>, EngineError> {
        let mut all = Vec::new();
        let per_page = size.to_string();
        for page in 1..=1000 {
            let number = page.to_string();
            let value = self
                .http
                .json(
                    Provider::Cloudflare,
                    token,
                    Method::GET,
                    path,
                    &[("page", &number), ("per_page", &per_page)],
                    None,
                )
                .await?;
            let rows = value
                .get("result")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            if all.len() + rows.len() > MAX_DISCOVERED_RESOURCES {
                return Err(limit());
            }
            all.extend(rows.iter().cloned());
            let pages = value
                .pointer("/result_info/total_pages")
                .and_then(Value::as_u64);
            if pages.is_some_and(|pages| page >= pages) || (pages.is_none() && rows.len() < size) {
                return Ok(all);
            }
        }
        Err(limit())
    }
}

fn invalid() -> EngineError {
    super::answers::incomplete("Cloudflare")
}

fn limit() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.pagination_limit".into(),
        message: "This account exceeds the discovery limit. Import a narrower set of resources."
            .into(),
    }
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, EngineError> {
    value.get(field).and_then(Value::as_str).ok_or_else(invalid)
}

/// A Worker folder that needs no bundling: its configuration names only the
/// Worker and its runtime, and its entry is one ES module that imports
/// nothing.
#[derive(Debug)]
struct PlainWorker {
    file_name: String,
    module: Vec<u8>,
    compatibility_date: String,
    compatibility_flags: Vec<String>,
    workers_dev: bool,
}

/// Reads a folder as a [`PlainWorker`], or `None` when it needs Wrangler:
/// bindings, assets, routes or other settings in its configuration, an
/// entry that imports anything, or one that isn't a module.
fn plain_worker(folder: &std::path::Path) -> Result<Option<PlainWorker>, EngineError> {
    use crate::files::bounded_read;
    let configs: Vec<_> = ["wrangler.toml", "wrangler.json", "wrangler.jsonc"]
        .iter()
        .map(|file| folder.join(file))
        .filter(|path| path.is_file())
        .collect();
    let [config] = configs.as_slice() else {
        return Ok(None);
    };
    let bytes = bounded_read(config, MAX_CONFIG_BYTES)?;
    let value: Value = match config.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => {
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return Ok(None);
            };
            let Ok(parsed) = toml::from_str::<toml::Value>(text) else {
                return Ok(None);
            };
            serde_json::to_value(parsed).map_err(|_| EngineError::Store)?
        }
        Some("jsonc") => match crate::files::parse_jsonc(&bytes) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        },
        _ => match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        },
    };
    let Some(settings) = value.as_object() else {
        return Ok(None);
    };
    const PLAIN: [&str; 7] = [
        "$schema",
        "name",
        "main",
        "compatibility_date",
        "compatibility_flags",
        "workers_dev",
        "account_id",
    ];
    if settings.keys().any(|key| !PLAIN.contains(&key.as_str())) {
        return Ok(None);
    }
    let Some(main) = settings.get("main").and_then(Value::as_str) else {
        return Ok(None);
    };
    let entry = std::path::Path::new(main);
    if entry.is_absolute()
        || entry
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || !matches!(
            entry.extension().and_then(|ext| ext.to_str()),
            Some("js" | "mjs")
        )
    {
        return Ok(None);
    }
    let path = folder.join(entry);
    if !path.is_file() {
        return Ok(None);
    }
    let module = bounded_read(&path, MAX_MODULE_BYTES)?;
    let Ok(text) = std::str::from_utf8(&module) else {
        return Ok(None);
    };
    if !is_lone_module(text) {
        return Ok(None);
    }
    let Some(file_name) = entry.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    Ok(Some(PlainWorker {
        file_name: file_name.to_owned(),
        module,
        compatibility_date: settings
            .get("compatibility_date")
            .and_then(Value::as_str)
            .filter(|date| date.len() == 10)
            .unwrap_or("2026-11-01")
            .to_owned(),
        compatibility_flags: settings
            .get("compatibility_flags")
            .and_then(Value::as_array)
            .map(|flags| {
                flags
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        workers_dev: settings
            .get("workers_dev")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    }))
}

/// True for an ES module with a default export that imports nothing, so it
/// runs as uploaded. Any `import` or `require` means it needs bundling.
fn is_lone_module(text: &str) -> bool {
    let code: String = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let imports = code.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import ")
            || line.starts_with("import{")
            || line.starts_with("import\"")
            || line.starts_with("import'")
            || (line.starts_with("export ") && line.contains(" from "))
    }) || code.contains("import(")
        || code.contains("require(");
    !imports && code.contains("export default")
}

/// One Pages deployment as a provider deployment.
fn pages_deployment(
    account: &str,
    project: &str,
    row: &Value,
) -> Option<crate::application::ports::providers::ProviderDeployment> {
    use crate::application::ports::providers::{ProviderDeployState, ProviderDeployment};
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let stage = &row["latest_stage"];
    let status = stage["status"].as_str().unwrap_or("idle");
    let stage_name = stage["name"].as_str().unwrap_or("");
    let state = match (stage_name, status) {
        (_, "failure") => ProviderDeployState::Failed,
        (_, "canceled" | "skipped") => ProviderDeployState::Stopped,
        ("deploy", "success") => ProviderDeployState::Succeeded,
        _ => ProviderDeployState::InProgress,
    };
    let meta = &row["deployment_trigger"]["metadata"];
    let id = text(&row["id"])?;
    Some(ProviderDeployment {
        provider_page: Some(format!("{DASHBOARD}/{account}/pages/view/{project}/{id}")),
        status: format!("{stage_name} {status}").trim().to_owned(),
        state,
        created_at: text(&row["created_on"])?,
        url: text(&row["url"]),
        commit: text(&meta["commit_hash"]),
        message: text(&meta["commit_message"]),
        branch: text(&meta["branch"]),
        repo: match (
            row["source"]["config"]["owner"].as_str(),
            row["source"]["config"]["repo_name"].as_str(),
        ) {
            (Some(owner), Some(name)) => Some(format!("{owner}/{name}")),
            _ => None,
        },
        finished_at: matches!(
            state,
            ProviderDeployState::Succeeded | ProviderDeployState::Failed
        )
        .then(|| text(&stage["ended_on"]))
        .flatten(),
        can_roll_back: None,
        author: None,
        id,
    })
}

/// One Worker deployment as a provider deployment. The newest is the one serving.
fn worker_deployment(
    account: &str,
    script: &str,
    newest: bool,
    row: &Value,
) -> Option<crate::application::ports::providers::ProviderDeployment> {
    use crate::application::ports::providers::{ProviderDeployState, ProviderDeployment};
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let notes = &row["annotations"];
    Some(ProviderDeployment {
        id: text(&row["id"])?,
        status: if newest { "active" } else { "replaced" }.to_owned(),
        state: ProviderDeployState::Succeeded,
        created_at: text(&row["created_on"])?,
        url: None,
        commit: None,
        message: text(&notes["workers/message"])
            .or_else(|| text(&notes["workers/triggered_by"]))
            .or_else(|| text(&row["source"])),
        branch: None,
        repo: None,
        // A Worker deployment is live as soon as Cloudflare records it.
        finished_at: text(&row["created_on"]),
        can_roll_back: None,
        author: text(&row["author_email"]),
        provider_page: Some(format!(
            "{DASHBOARD}/{account}/workers/services/view/{script}/production/deployments"
        )),
    })
}

fn worker(reference: &str) -> Result<&str, EngineError> {
    reference
        .strip_prefix("worker:")
        .filter(|name| valid_slug(name))
        .ok_or(EngineError::Invalid(
            "Expected a Cloudflare Worker resource.",
        ))
}

#[async_trait::async_trait]
impl ProviderAdapter for Cloudflare {
    fn provider(&self) -> Provider {
        Provider::Cloudflare
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// Workers report their bindings.
    fn reports_bindings(&self, reference: &str) -> bool {
        reference.starts_with("worker:")
    }

    /// Cloudflare lists a Worker the same way whether it serves requests or
    /// only runs on a cron schedule.
    fn keeps_known_role(&self, reference: &str) -> bool {
        reference.starts_with("worker:")
    }

    /// A sign-in's scopes are Cloudflare's OAuth names (`workers-scripts.edit`);
    /// a pasted token's are permission names, which aren't checked here.
    fn missing_grant(
        &self,
        granted: &[String],
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Option<&'static str> {
        use crate::application::ports::providers::ProviderNew;
        if !granted.iter().any(|scope| scope == "offline_access") {
            return None;
        }
        let (needed, missing) = match what {
            ProviderNew::Folder { .. } => (
                "workers-scripts.edit",
                "This Cloudflare sign-in didn't grant Workers Scripts Edit, so Cloudflare refuses new Workers. Reconnect Cloudflare from Accounts, choose this account on Cloudflare's page, and leave Workers Scripts on.",
            ),
            ProviderNew::Bucket { .. } => (
                "workers-r2.write",
                "This Cloudflare sign-in didn't grant Workers R2 Storage. Reconnect Cloudflare from Accounts and leave R2 on, on Cloudflare's page.",
            ),
            ProviderNew::Database { .. } => (
                "d1.write",
                "This Cloudflare sign-in didn't grant D1. Reconnect Cloudflare from Accounts and leave D1 on, on Cloudflare's page.",
            ),
            _ => return None,
        };
        (!granted.iter().any(|scope| scope == needed)).then_some(missing)
    }

    /// A Worker from a folder (with a wrangler.toml, .jsonc or .json, under
    /// the given name), a D1 database, or an R2 bucket. A Worker with nothing to
    /// bundle is uploaded directly; anything else is deployed with Wrangler.
    async fn provider_add(
        &self,
        token: &Secret,
        external_id: &str,
        project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        use crate::application::ports::providers::ProviderNew;
        match *what {
            ProviderNew::Folder { name, folder } => {
                if !["wrangler.toml", "wrangler.jsonc", "wrangler.json"]
                    .iter()
                    .any(|file| folder.join(file).is_file())
                {
                    return Err(EngineError::Invalid(
                        "Wrangler deploys a folder with a wrangler.toml or wrangler.jsonc. Add one, then try again.",
                    ));
                }
                // The Worker's address and cron schedules need the account's
                // workers.dev subdomain.
                let (subdomain, made) = self.workers_subdomain(token, external_id, project).await?;
                let set_up = if made {
                    format!(
                        "Set up {subdomain}.workers.dev for this Cloudflare account and deployed"
                    )
                } else {
                    "Deployed".to_owned()
                };
                // A Worker that is one module with nothing to bundle goes
                // straight up through Cloudflare's API; anything else is
                // Wrangler's to bundle and deploy.
                if let Some(upload) = plain_worker(folder)? {
                    self.upload_plain_worker(token, external_id, name, &upload)
                        .await?;
                    let address = if upload.workers_dev {
                        format!(", at https://{name}.{subdomain}.workers.dev")
                    } else {
                        ", with its workers.dev address turned off as its configuration says"
                            .to_owned()
                    };
                    return Ok((
                        vec![format!("worker:{name}")],
                        format!(
                            "{set_up} the Worker {name} from its folder through Cloudflare's API{address}"
                        ),
                    ));
                }
                let manifest = super::manifest::builtin("wrangler");
                let cli = manifest.cli.as_ref().ok_or(EngineError::Store)?;
                super::cli::run(
                    cli,
                    &self.http,
                    token,
                    super::cli::Aim {
                        context: &[("account", external_id), ("worker", name)],
                        folder: Some(folder),
                    },
                    "wrangler deploy",
                    Value::Object(serde_json::Map::new()),
                )
                .await?;
                Ok((
                    vec![format!("worker:{name}")],
                    format!(
                        "{set_up} the Worker {name} from its folder with Wrangler, at https://{name}.{subdomain}.workers.dev"
                    ),
                ))
            }
            ProviderNew::Database {
                name,
                engine,
                region,
            } => {
                if !matches!(engine, "sqlite" | "d1") {
                    return Err(EngineError::Invalid(
                        "Cloudflare's database is D1 (SQLite): give engine sqlite.",
                    ));
                }
                let mut body = serde_json::json!({"name": name});
                if let Some(region) = region {
                    body["primary_location_hint"] = region.into();
                }
                let value = self
                    .call(
                        token,
                        Method::POST,
                        &["accounts", external_id, "d1", "database"],
                        &[],
                        Some(&body),
                    )
                    .await?;
                let id = text(&value, "uuid")?;
                Ok((
                    vec![format!("d1:{id}")],
                    format!("Made the D1 database {name} on Cloudflare"),
                ))
            }
            ProviderNew::Bucket { name } => {
                self.call(
                    token,
                    Method::POST,
                    &["accounts", external_id, "r2", "buckets"],
                    &[],
                    Some(&serde_json::json!({"name": name})),
                )
                .await?;
                Ok((
                    vec![format!("r2:default:{name}")],
                    format!("Made the R2 bucket {name} on Cloudflare"),
                ))
            }
            _ => Err(crate::application::ports::providers::unsupported()),
        }
    }

    /// A new project from a folder: the same as adding one to a project.
    async fn provider_new_from_folder(
        &self,
        token: &Secret,
        external_id: &str,
        name: &str,
        folder: &std::path::Path,
    ) -> Result<(Vec<String>, String), EngineError> {
        self.provider_add(
            token,
            external_id,
            name,
            None,
            &crate::application::ports::providers::ProviderNew::Folder { name, folder },
        )
        .await
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        super::manifest::builtin("wrangler").operations()
    }

    fn prepare_tools(&self) {
        if let Some(cli) = super::manifest::builtin("wrangler").cli.as_ref() {
            super::cli::prepare(cli, &self.http);
        }
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        super::manifest::builtin("wrangler").actions.clone()
    }

    async fn provider_operation(
        &self,
        token: &Secret,
        external_id: &str,
        reference: Option<&str>,
        folder: Option<&std::path::Path>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        let manifest = super::manifest::builtin("wrangler");
        // Everything is aimed at the account, and at the resource it's run
        // on: a Worker, a Pages project, an R2 bucket or a domain (zone).
        let mut context = vec![("account", external_id)];
        let reference = reference.unwrap_or_default();
        if let Ok(name) = worker(reference) {
            context.push(("worker", name));
        } else if let Some(name) = reference.strip_prefix("pages:") {
            context.push(("pages_project", name));
        } else if let Some(zone) = reference.strip_prefix("zone:") {
            context.push(("zone", zone));
        } else if let Some(id) = reference.strip_prefix("d1:") {
            context.push(("d1", id));
        } else if let Some(id) = reference.strip_prefix("kv:") {
            context.push(("kv", id));
        } else if let Some((jurisdiction, bucket)) = reference
            .strip_prefix("r2:")
            .and_then(|rest| rest.split_once(':'))
        {
            context.push(("bucket", bucket));
            // Cloudflare's API reaches a bucket outside the default
            // jurisdiction only with a header this catalog doesn't send.
            if jurisdiction != "default"
                && manifest
                    .api_operation(name)
                    .is_some_and(|op| op.aimed_at("bucket"))
            {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "provider.not_supported".into(),
                    message: format!(
                        "This bucket is kept in the {jurisdiction} jurisdiction; use Wrangler's r2 commands for it."
                    ),
                });
            }
        }
        if manifest.api_operation(name).is_some() {
            return super::api_catalog::run_manifest(
                &self.http,
                Provider::Cloudflare,
                manifest,
                token,
                &context,
                name,
                args,
            )
            .await;
        }
        let cli = manifest.cli.as_ref().ok_or(EngineError::Store)?;
        if !super::cli::has(cli, name) {
            return Err(EngineError::Invalid(
                "This provider has no such operation in Hoista. Call list_provider_operations for the list.",
            ));
        }
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

    /// What Cloudflare says about a watched Worker, Pages project, R2 bucket
    /// or domain: there and serving, still being set up, or gone.
    async fn observe_resource(
        &self,
        token: &Secret,
        external_id: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let reference = resource.provider_ref.as_str();
        let gone = |error: EngineError| match error {
            EngineError::Provider { ref code, .. } if code == "provider.not_found" => {
                Ok(Observed {
                    status: ResourceStatus::Stopped,
                    url: None,
                    kind: None,
                })
            }
            error => Err(error),
        };
        if let Some(zone) = reference.strip_prefix("zone:") {
            if zone.is_empty() || !zone.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(invalid());
            }
            let value = match self
                .call(token, Method::GET, &["zones", zone], &[], None)
                .await
            {
                Ok(value) => value,
                Err(error) => return gone(error),
            };
            // Cloudflare's zone states: active once its name servers point at
            // Cloudflare; pending or initializing until then.
            let status = match value.get("status").and_then(Value::as_str) {
                Some("active") => ResourceStatus::Live,
                Some("pending" | "initializing") => ResourceStatus::Deploying,
                Some("moved" | "deactivated" | "deleted") => ResourceStatus::Stopped,
                _ => ResourceStatus::Unknown,
            };
            return Ok(Observed {
                status,
                url: None,
                kind: Some(ResourceKind::Domain),
            });
        }
        if let Some(rest) = reference.strip_prefix("r2:") {
            let (jurisdiction, name) = rest.split_once(':').ok_or_else(invalid)?;
            return match self.r2_bucket(token, external_id, jurisdiction, name).await {
                Ok(_) => Ok(Observed {
                    status: ResourceStatus::Live,
                    url: None,
                    kind: Some(ResourceKind::Bucket),
                }),
                Err(error) => gone(error),
            };
        }
        // A D1 database or KV namespace is there or gone.
        for (prefix, path) in [
            ("d1:", ["d1", "database"]),
            ("kv:", ["storage", "kv/namespaces"]),
        ] {
            let Some(id) = reference.strip_prefix(prefix) else {
                continue;
            };
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return Err(invalid());
            }
            let mut at = vec!["accounts", external_id];
            at.extend(path.iter().flat_map(|part| part.split('/')));
            at.push(id);
            return match self.call(token, Method::GET, &at, &[], None).await {
                Ok(_) => Ok(Observed {
                    status: ResourceStatus::Live,
                    url: None,
                    kind: Some(ResourceKind::Database),
                }),
                Err(error) => gone(error),
            };
        }
        if let Some(name) = reference.strip_prefix("pages:") {
            if !valid_slug(name) {
                return Err(invalid());
            }
            let value = match self
                .call(
                    token,
                    Method::GET,
                    &["accounts", external_id, "pages", "projects", name],
                    &[],
                    None,
                )
                .await
            {
                Ok(value) => value,
                Err(error) => return gone(error),
            };
            return Ok(Observed {
                status: ResourceStatus::Live,
                url: value
                    .get("subdomain")
                    .and_then(Value::as_str)
                    .map(|host| format!("https://{host}")),
                kind: None,
            });
        }
        let script = worker(reference)?;
        match self
            .call(
                token,
                Method::GET,
                &[
                    "accounts",
                    external_id,
                    "workers",
                    "scripts",
                    script,
                    "settings",
                ],
                &[],
                None,
            )
            .await
        {
            Ok(_) => {
                // Its workers.dev address only when that route is on for it.
                let url = if self.script_on_workers_dev(token, external_id, script).await {
                    self.call(
                        token,
                        Method::GET,
                        &["accounts", external_id, "workers", "subdomain"],
                        &[],
                        None,
                    )
                    .await
                    .ok()
                    .and_then(|value| value["subdomain"].as_str().map(str::to_owned))
                    .filter(|sub| valid_slug(sub))
                    .map(|sub| workers_dev_url(script, &sub))
                } else {
                    None
                };
                Ok(Observed {
                    status: ResourceStatus::Live,
                    url,
                    kind: None,
                })
            }
            Err(error) => gone(error),
        }
    }

    /// A Worker or Pages project as Cloudflare keeps it: deployments,
    /// variable and secret names, its workers.dev or pages.dev address and
    /// custom domains, schedules and build settings.
    async fn provider_details(
        &self,
        token: &Secret,
        external_id: &str,
        reference: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        use crate::application::ports::providers::{
            ProviderConfig, ProviderDetails, ProviderDomain, ProviderDomainState,
        };
        let domain = |name: String, custom: bool, working: bool| ProviderDomain {
            name,
            custom,
            state: Some(if working {
                ProviderDomainState::Working
            } else {
                ProviderDomainState::Pending
            }),
            records: vec![],
            port: None,
            certificate: None,
            dns: vec![],
        };
        let mut details = ProviderDetails {
            deployments: vec![],
            variable_names: None,
            shared_variable_names: None,
            domains: vec![],
            references: vec![],
            sleeps: None,
            config: None,
            volumes: vec![],
            tcp_proxies: vec![],
            addresses: vec![],
            private_domain: None,
        };
        if let Some(name) = reference.strip_prefix("pages:") {
            if !valid_slug(name) {
                return Err(invalid());
            }
            let rows = self
                .call(
                    token,
                    Method::GET,
                    &[
                        "accounts",
                        external_id,
                        "pages",
                        "projects",
                        name,
                        "deployments",
                    ],
                    &[("per_page", "25")],
                    None,
                )
                .await?;
            details.deployments = rows
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| pages_deployment(external_id, name, row))
                .collect();
            if let Ok(project) = self
                .call(
                    token,
                    Method::GET,
                    &["accounts", external_id, "pages", "projects", name],
                    &[],
                    None,
                )
                .await
            {
                let mut names: Vec<String> =
                    project["deployment_configs"]["production"]["env_vars"]
                        .as_object()
                        .map(|vars| vars.keys().cloned().collect())
                        .unwrap_or_default();
                names.sort();
                details.variable_names = Some(names);
                if let Some(sub) = project["subdomain"].as_str() {
                    details.domains.push(domain(sub.to_owned(), false, true));
                }
                details.domains.extend(
                    project["domains"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|host| !host.ends_with(".pages.dev"))
                        .map(|host| domain(host.to_owned(), true, true)),
                );
                let build = &project["build_config"];
                let text = |value: &Value| {
                    value
                        .as_str()
                        .filter(|said| !said.is_empty())
                        .map(str::to_owned)
                };
                let source = &project["source"]["config"];
                details.config = Some(ProviderConfig {
                    repo: match (source["owner"].as_str(), source["repo_name"].as_str()) {
                        (Some(owner), Some(repo)) => Some(format!("{owner}/{repo}")),
                        _ => None,
                    },
                    build_command: text(&build["build_command"]),
                    root_directory: text(&build["root_dir"]),
                    ..ProviderConfig::default()
                });
            }
            return Ok(details);
        }
        if !reference.starts_with("worker:") {
            return Ok(details);
        }
        let script = worker(reference)?;
        let value = self
            .call(
                token,
                Method::GET,
                &[
                    "accounts",
                    external_id,
                    "workers",
                    "scripts",
                    script,
                    "deployments",
                ],
                &[],
                None,
            )
            .await?;
        details.deployments = value["deployments"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, row)| worker_deployment(external_id, script, index == 0, row))
            .collect();
        // Variables and secrets are the Worker's text bindings, names only.
        if let Ok(settings) = self
            .call(
                token,
                Method::GET,
                &[
                    "accounts",
                    external_id,
                    "workers",
                    "scripts",
                    script,
                    "settings",
                ],
                &[],
                None,
            )
            .await
        {
            let mut names: Vec<String> = settings["bindings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|binding| {
                    matches!(
                        binding["type"].as_str(),
                        Some("plain_text" | "secret_text" | "json")
                    )
                })
                .filter_map(|binding| binding["name"].as_str().map(str::to_owned))
                .collect();
            names.sort();
            names.dedup();
            details.variable_names = Some(names);
        }
        if self.script_on_workers_dev(token, external_id, script).await
            && let Ok(sub) = self
                .call(
                    token,
                    Method::GET,
                    &["accounts", external_id, "workers", "subdomain"],
                    &[],
                    None,
                )
                .await
            && let Some(sub) = sub["subdomain"].as_str().filter(|sub| valid_slug(sub))
        {
            details
                .domains
                .push(domain(format!("{script}.{sub}.workers.dev"), false, true));
        }
        if let Ok(custom) = self
            .call(
                token,
                Method::GET,
                &["accounts", external_id, "workers", "domains"],
                &[("service", script)],
                None,
            )
            .await
        {
            details.domains.extend(
                custom
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|row| {
                        row["service"]
                            .as_str()
                            .is_none_or(|service| service == script)
                    })
                    .filter_map(|row| row["hostname"].as_str())
                    .map(|host| domain(host.to_owned(), true, true)),
            );
        }
        if let Ok(schedules) = self.read_schedules(token, external_id, script).await
            && !schedules.is_empty()
        {
            details.config = Some(ProviderConfig {
                cron_schedule: Some(schedules.join(", ")),
                ..ProviderConfig::default()
            });
        }
        Ok(details)
    }

    /// What Cloudflare has billed this account so far this billing period,
    /// from its Billable Usage API (self-serve accounts; the token needs
    /// Billing Read). Updated daily by Cloudflare.
    async fn provider_bill(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Option<crate::application::ports::providers::ProviderBill>, EngineError> {
        let rows = self
            .call(
                token,
                Method::GET,
                &["accounts", external_id, "billable-usage"],
                &[],
                None,
            )
            .await?;
        let Some(rows) = rows.as_array() else {
            return Ok(None);
        };
        Ok(focus_bill(rows, "ContractedCost"))
    }

    fn oauth(&self) -> Option<crate::application::ports::providers::ProviderOAuth> {
        oauth_app()
    }

    async fn oauth_token(
        &self,
        grant: crate::application::ports::providers::OAuthGrant<'_>,
    ) -> Result<crate::application::ports::providers::OAuthTokens, EngineError> {
        let client_id = CLIENT_ID.ok_or_else(super::oauth::not_set_up)?;
        super::oauth::public_token(&self.http, OAUTH_TOKEN_URL, client_id, grant).await
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        super::accounts::list(&self.http, Provider::Cloudflare, token).await
    }

    async fn verify(&self, token: &Secret, external_id: &str) -> Result<Vec<String>, EngineError> {
        if external_id.is_empty() {
            return Err(EngineError::Invalid("A Cloudflare account id is required."));
        }
        let (verified, account_owned) = match self
            .call(token, Method::GET, &["user", "tokens", "verify"], &[], None)
            .await
        {
            Ok(value) => (value, false),
            Err(EngineError::Provider { code, .. })
                if matches!(
                    code.as_str(),
                    "provider.token_expired" | "provider.token_scope" | "provider.rejected"
                ) =>
            {
                match self
                    .call(
                        token,
                        Method::GET,
                        &["accounts", external_id, "tokens", "verify"],
                        &[],
                        None,
                    )
                    .await
                {
                    Ok(value) => (value, true),
                    // A sign-in (OAuth) token isn't an API token, so neither
                    // verify address knows it. Reading the account's Workers
                    // proves access; Cloudflare's consent page set the rest.
                    // A truly expired token fails this read as well.
                    Err(EngineError::Provider { code, .. })
                        if matches!(
                            code.as_str(),
                            "provider.token_expired" | "provider.token_scope" | "provider.rejected"
                        ) =>
                    {
                        self.call(
                            token,
                            Method::GET,
                            &["accounts", external_id, "workers", "scripts"],
                            &[],
                            None,
                        )
                        .await?;
                        return Ok(vec![
                            "Workers Scripts Read (verified by request)".into(),
                            "Signed in with Cloudflare: permissions as chosen on Cloudflare's page"
                                .into(),
                        ]);
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        if verified.get("status").and_then(Value::as_str) != Some("active") {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.token_expired".into(),
                message: "Cloudflare reports this token is not active. Create a new token.".into(),
            });
        }
        // A read proves access to this account without creating anything.
        self.call(
            token,
            Method::GET,
            &["accounts", external_id, "workers", "scripts"],
            &[],
            None,
        )
        .await?;
        let token_id = text(&verified, "id")?;
        let policy_path = if account_owned {
            vec!["accounts", external_id, "tokens", token_id]
        } else {
            vec!["user", "tokens", token_id]
        };
        let policy = match self.call(token, Method::GET, &policy_path, &[], None).await {
            Ok(policy) => policy,
            Err(EngineError::Provider { code, .. })
                if matches!(code.as_str(), "provider.token_scope" | "provider.rejected") =>
            {
                // Reading the token's own policies needs API Tokens Read, which a
                // token may lack. That says nothing about Workers write or zone
                // access, so those are reported as unverified.
                return Ok(vec![
                    "Workers Scripts Read (verified by request)".into(),
                    "Permission metadata unavailable; write and domain permissions are unverified"
                        .into(),
                ]);
            }
            Err(error) => return Err(error),
        };
        let policies = policy
            .get("policies")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        let resource = format!("com.cloudflare.api.account.{external_id}");
        let mut scopes = Vec::new();
        for policy in policies {
            if policy.get("effect").and_then(Value::as_str) != Some("allow") {
                continue;
            }
            let resources = policy
                .get("resources")
                .and_then(Value::as_object)
                .ok_or_else(invalid)?;
            if !resources.contains_key(&resource)
                && !resources.contains_key("com.cloudflare.api.account.*")
            {
                continue;
            }
            if let Some(groups) = policy.get("permission_groups").and_then(Value::as_array) {
                scopes.extend(
                    groups
                        .iter()
                        .filter_map(|group| group.get("name").and_then(Value::as_str))
                        .map(str::to_owned),
                );
            }
        }
        scopes.sort();
        scopes.dedup();
        scopes.push("Workers Scripts Read (verified by request)".into());
        Ok(scopes)
    }

    async fn resource_bindings(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
    ) -> Result<Vec<crate::application::ports::providers::ResourceBinding>, EngineError> {
        let value = self
            .call(
                token,
                Method::GET,
                &[
                    "accounts",
                    account,
                    "workers",
                    "scripts",
                    worker(reference)?,
                    "settings",
                ],
                &[],
                None,
            )
            .await?;
        let rows = value
            .get("bindings")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        if rows.len() > MAX_BINDINGS {
            return Err(limit());
        }
        let mut names = std::collections::BTreeSet::new();
        let mut bindings = Vec::new();
        for row in rows {
            let name = text(row, "name")?;
            if !valid_env_name(name) || !names.insert(name) {
                return Err(invalid());
            }
            let kind = text(row, "type")?;
            let target_reference = match kind {
                "service"
                    if row
                        .get("environment")
                        .is_none_or(|v| v.is_null() || v.as_str() == Some("production")) =>
                {
                    let service = text(row, "service")?;
                    if !valid_slug(service) {
                        return Err(invalid());
                    }
                    Some(format!("worker:{service}"))
                }
                "r2_bucket" => {
                    let bucket = text(row, "bucket_name")?;
                    if bucket.is_empty()
                        || bucket.len() > MAX_SETTING_CHARS
                        || bucket.chars().any(char::is_control)
                    {
                        return Err(invalid());
                    }
                    let jurisdiction = match row.get("jurisdiction") {
                        None | Some(Value::Null) => "default",
                        Some(Value::String(v)) => v.as_str(),
                        _ => return Err(invalid()),
                    };
                    if !matches!(
                        jurisdiction,
                        "default" | "eu" | "us" | "fedramp" | "fedramp-high"
                    ) {
                        None
                    } else {
                        Some(format!("r2:{jurisdiction}:{bucket}"))
                    }
                }
                _ => None,
            };
            bindings.push(crate::application::ports::providers::ResourceBinding {
                name: name.into(),
                target_reference,
            });
        }
        Ok(bindings)
    }

    async fn discover(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let report = self.discover_report(token, external_id).await?;
        if !report.warnings.is_empty() {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.discovery_incomplete".into(),
                message: "Cloudflare discovery has inaccessible resource families. Use the discovery report to inspect its coverage.".into(),
            });
        }
        Ok(report.resources)
    }

    async fn discover_report(
        &self,
        token: &Secret,
        external_id: &str,
    ) -> Result<DiscoveryReport, EngineError> {
        let mut report = DiscoveryReport {
            resources: vec![],
            warnings: vec![],
        };
        for (family, endpoint, size) in [("workers", "scripts", 100), ("pages", "projects", 10)] {
            let rows = match self
                .pages(token, &["accounts", external_id, family, endpoint], size)
                .await
            {
                Ok(rows) => rows,
                // A product the sign-in didn't include, or one Cloudflare
                // refuses, is noted and the scan goes on to the next. An ended
                // sign-in or a malformed answer still stops it.
                Err(error)
                    if matches!(
                        error.code(),
                        "provider.token_scope" | "provider.rejected" | "provider.not_found"
                    ) =>
                {
                    let message = if error.code() == "provider.token_scope" {
                        format!(
                            "Cloudflare denied access to {family}. Grant read access and scan again; existing local resources have been retained."
                        )
                    } else {
                        format!(
                            "Cloudflare couldn't list {family}. The rest was scanned; existing local resources have been retained."
                        )
                    };
                    report.warnings.push(DiscoveryWarning {
                        resource_type: family.into(),
                        code: error.code().into(),
                        message,
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            // A Worker's own address is on the account's workers.dev
            // subdomain; read once, never made here.
            let workers_dev = if family == "workers" && !rows.is_empty() {
                self.call(
                    token,
                    Method::GET,
                    &["accounts", external_id, "workers", "subdomain"],
                    &[],
                    None,
                )
                .await
                .ok()
                .and_then(|value| value["subdomain"].as_str().map(str::to_owned))
                .filter(|name| valid_slug(name))
            } else {
                None
            };
            // Only Workers with their workers.dev route turned on answer there.
            let on_workers_dev: std::collections::HashSet<String> = if workers_dev.is_some() {
                use futures_util::StreamExt;
                let names: Vec<String> = rows
                    .iter()
                    .filter_map(|row| row["id"].as_str())
                    .filter(|name| valid_slug(name))
                    .map(str::to_owned)
                    .collect();
                futures_util::stream::iter(names)
                    .map(|name| async move {
                        self.script_on_workers_dev(token, external_id, &name)
                            .await
                            .then_some(name)
                    })
                    .buffer_unordered(PARALLEL_LOOKUPS)
                    .filter_map(|name| async move { name })
                    .collect()
                    .await
            } else {
                std::collections::HashSet::new()
            };
            for value in rows {
                let name = text(&value, if family == "workers" { "id" } else { "name" })?;
                let url = if family == "pages" {
                    value
                        .get("subdomain")
                        .and_then(Value::as_str)
                        .map(|host| format!("https://{host}"))
                } else {
                    workers_dev
                        .as_deref()
                        .filter(|_| on_workers_dev.contains(name))
                        .map(|sub| workers_dev_url(name, sub))
                };
                report.resources.push(RemoteResource {
                    reference: format!(
                        "{}:{name}",
                        if family == "workers" {
                            "worker"
                        } else {
                            "pages"
                        }
                    ),
                    name: name.into(),
                    kind: ResourceKind::Web,
                    region: None,
                    status: ResourceStatus::Unknown,
                    url,
                    group: None,
                });
                if report.resources.len() > MAX_DISCOVERED_RESOURCES {
                    return Err(limit());
                }
            }
        }
        // D1 databases and KV namespaces, each its own resource.
        for (family, path, size) in [
            ("d1", ["d1", "database", ""], 100),
            ("kv", ["storage", "kv", "namespaces"], 100),
        ] {
            let mut at = vec!["accounts", external_id];
            at.extend(path.iter().filter(|part| !part.is_empty()));
            let rows = match self.pages(token, &at, size).await {
                Ok(rows) => rows,
                Err(error)
                    if matches!(
                        error.code(),
                        "provider.token_scope" | "provider.rejected" | "provider.not_found"
                    ) =>
                {
                    report.warnings.push(DiscoveryWarning {
                        resource_type: family.into(),
                        code: error.code().into(),
                        message: format!(
                            "Cloudflare couldn't list {}. The rest was scanned; existing local resources have been retained.",
                            if family == "d1" {
                                "D1 databases"
                            } else {
                                "KV namespaces"
                            }
                        ),
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            for value in rows {
                let (id, name) = if family == "d1" {
                    (text(&value, "uuid")?, text(&value, "name")?)
                } else {
                    (text(&value, "id")?, text(&value, "title")?)
                };
                if id.is_empty()
                    || id.len() > 64
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return Err(invalid());
                }
                report.resources.push(RemoteResource {
                    reference: format!("{family}:{id}"),
                    name: name.chars().take(MAX_NAME_CHARS).collect(),
                    kind: ResourceKind::Database,
                    region: None,
                    status: ResourceStatus::Live,
                    url: None,
                    group: None,
                });
                if report.resources.len() > MAX_DISCOVERED_RESOURCES {
                    return Err(limit());
                }
            }
        }
        self.discover_inventory(token, external_id, &mut report)
            .await?;
        Ok(report)
    }

    async fn deploy(
        &self,
        token: &Secret,
        account: &str,
        resource: &ResourcePlan,
        artifact: &Artifact,
    ) -> Result<Release, EngineError> {
        if !matches!(resource.kind, ResourceKind::Web | ResourceKind::Job)
            || !valid_slug(&resource.name)
        {
            return Err(EngineError::Invalid(
                "Cloudflare web deploys need a valid lowercase Worker name.",
            ));
        }
        if resource
            .schedules
            .as_ref()
            .is_some_and(|values| !crate::domain::schedules::valid(values))
            || (resource.kind == ResourceKind::Job && artifact.worker.is_none())
            || (resource.schedules.as_ref().is_some_and(|v| !v.is_empty())
                && artifact.worker.is_none())
        {
            return Err(EngineError::Invalid(
                "Cron triggers require valid UTC expressions and a Worker module with a scheduled handler.",
            ));
        }
        if artifact.assets.is_empty() && artifact.worker.is_none() {
            return Err(EngineError::Invalid(
                "The build produced neither public assets nor a Worker module.",
            ));
        }
        let script = match &resource.provider_ref {
            Some(reference) => worker(reference)?,
            None => resource.name.as_str(),
        };
        if resource.provider_ref.is_none() {
            match self
                .call(
                    token,
                    Method::GET,
                    &[
                        "accounts", account, "workers", "scripts", script, "settings",
                    ],
                    &[],
                    None,
                )
                .await
            {
                Err(EngineError::Provider { code, .. }) if code == "provider.not_found" => {}
                Err(error) => return Err(error),
                Ok(_) => {
                    return Err(EngineError::Conflict(
                        "A Worker already uses this name. Import it and explicitly enable management before redeploying it.",
                    ));
                }
            }
        }
        let previous_schedules = if resource.provider_ref.is_some() {
            self.read_schedules(token, account, script).await?
        } else {
            vec![]
        };
        let asset_config = if artifact.assets.is_empty() {
            None
        } else {
            let mut manifest = serde_json::Map::new();
            let mut hashes = std::collections::BTreeMap::new();
            for asset in &artifact.assets {
                let digest = format!("{:x}", Sha256::digest(&asset.bytes));
                let hash = digest[..32].to_owned();
                manifest.insert(
                    asset.path.clone(),
                    json!({"hash":hash,"size":asset.bytes.len()}),
                );
                hashes.insert(hash, asset);
            }
            let session = self
                .call(
                    token,
                    Method::POST,
                    &[
                        "accounts",
                        account,
                        "workers",
                        "scripts",
                        script,
                        "assets-upload-session",
                    ],
                    &[],
                    Some(&json!({"manifest":manifest})),
                )
                .await?;
            let upload = Secret::new(text(&session, "jwt")?.into());
            let buckets = session
                .get("buckets")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            let mut completion = if buckets.is_empty() {
                Some(Secret::new(upload.expose().into()))
            } else {
                None
            };
            for bucket in buckets {
                let mut form = Form::new();
                for hash in bucket.as_array().ok_or_else(invalid)? {
                    let hash = hash.as_str().ok_or_else(invalid)?;
                    let asset = hashes.get(hash).ok_or_else(invalid)?;
                    let data = base64::engine::general_purpose::STANDARD.encode(&asset.bytes);
                    let content = FormPart::text(data)
                        .file_name(hash.to_owned())
                        .mime_str(&asset.content_type)
                        .map_err(|_| invalid())?;
                    form = form.part(hash.to_owned(), content);
                }
                let uploaded = self
                    .http
                    .multipart(
                        Provider::Cloudflare,
                        &upload,
                        Method::POST,
                        &["accounts", account, "workers", "assets", "upload"],
                        &[("base64", "true")],
                        form,
                    )
                    .await?;
                if let Some(jwt) = uploaded
                    .pointer("/result/jwt")
                    .or_else(|| uploaded.get("jwt"))
                    .and_then(Value::as_str)
                {
                    completion = Some(Secret::new(jwt.into()));
                }
            }
            let completion = completion.ok_or_else(invalid)?;
            let mut asset_config = json!({"jwt":completion.expose()});
            if artifact.worker.is_none() {
                asset_config["config"] = json!({"not_found_handling":"single-page-application"});
            }
            Some(asset_config)
        };
        let module = artifact.worker.clone().unwrap_or_else(|| {
            b"export default {fetch(request,env){return env.ASSETS.fetch(request)}}".to_vec()
        });
        let mut bindings = Vec::new();
        if asset_config.is_some() {
            bindings.push(json!({"name":"ASSETS","type":"assets"}));
        }
        bindings.extend(
            artifact.secrets.iter().map(
                |(name, value)| json!({"name":name,"type":"secret_text","text":value.expose()}),
            ),
        );
        let mut metadata = json!({
            "main_module": artifact.worker_name,
            "compatibility_date": "2026-11-01",
            "compatibility_flags": ["nodejs_compat"],
            "bindings": bindings,
            "keep_bindings": ["secret_text"],
        });
        if let Some(config) = asset_config {
            metadata["assets"] = config;
        }
        let mut form = Form::new()
            .part(
                "metadata",
                FormPart::text(metadata.to_string())
                    .mime_str("application/json")
                    .map_err(|_| invalid())?,
            )
            .part(
                artifact.worker_name.clone(),
                FormPart::bytes(module)
                    .file_name(artifact.worker_name.clone())
                    .mime_str("application/javascript+module")
                    .map_err(|_| invalid())?,
            );
        for module in &artifact.worker_modules {
            let name = module.path.trim_start_matches('/').to_owned();
            let mime = if name.ends_with(".wasm") {
                "application/wasm"
            } else if name.ends_with(".js") || name.ends_with(".mjs") {
                "application/javascript+module"
            } else {
                &module.content_type
            };
            form = form.part(
                name.clone(),
                FormPart::bytes(module.bytes.clone())
                    .file_name(name)
                    .mime_str(mime)
                    .map_err(|_| invalid())?,
            );
        }
        let version = if resource.provider_ref.is_none() {
            self.http
                .multipart(
                    Provider::Cloudflare,
                    token,
                    Method::PUT,
                    &["accounts", account, "workers", "scripts", script],
                    &[],
                    form,
                )
                .await?;
            let deployed = self
                .call(
                    token,
                    Method::GET,
                    &[
                        "accounts",
                        account,
                        "workers",
                        "scripts",
                        script,
                        "deployments",
                    ],
                    &[("per_page", "1")],
                    None,
                )
                .await?;
            deployed
                .pointer("/deployments/0/versions/0/version_id")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?
                .to_owned()
        } else {
            let result = self
                .http
                .multipart(
                    Provider::Cloudflare,
                    token,
                    Method::POST,
                    &[
                        "accounts", account, "workers", "scripts", script, "versions",
                    ],
                    &[],
                    form,
                )
                .await?;
            let version = result
                .pointer("/result/id")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?
                .to_owned();
            self.call(token,Method::POST,&["accounts",account,"workers","scripts",script,"deployments"],&[],Some(&json!({"strategy":"percentage","versions":[{"version_id":version,"percentage":100}]}))).await?;
            version
        };
        // A new account has no workers.dev subdomain; Workers need one for
        // a public address and for cron schedules.
        self.workers_subdomain(token, account, script).await?;
        self.call(
            token,
            Method::POST,
            &[
                "accounts",
                account,
                "workers",
                "scripts",
                script,
                "subdomain",
            ],
            &[],
            Some(&json!({"enabled":resource.kind != ResourceKind::Job,"previews_enabled":false})),
        )
        .await?;
        let url = if resource.kind == ResourceKind::Job {
            None
        } else {
            let domain = self
                .call(
                    token,
                    Method::GET,
                    &["accounts", account, "workers", "subdomain"],
                    &[],
                    None,
                )
                .await?;
            let subdomain = text(&domain, "subdomain")?;
            if !valid_slug(subdomain) {
                return Err(invalid());
            }
            Some(workers_dev_url(script, subdomain))
        };
        if let Some(schedules) = &resource.schedules
            && let Err(error) = self
                .write_schedules(token, account, script, schedules)
                .await
        {
            if self
                .write_schedules(token, account, script, &previous_schedules)
                .await
                .is_err()
            {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: "schedule.recovery_failed".into(),
                    message: "Cron update and recovery failed. Inspect the Worker triggers before retrying.".into(),
                });
            }
            return Err(error);
        }
        Ok(Release {
            schedules: Some(resource.schedules.clone().unwrap_or(previous_schedules)),
            reference: format!("worker:{script}"),
            version,
            url,
        })
    }

    async fn health(
        &self,
        token: &Secret,
        external_id: &str,
        release: &Release,
    ) -> Result<(), EngineError> {
        if release.url.is_none() {
            self.call(
                token,
                Method::GET,
                &[
                    "accounts",
                    external_id,
                    "workers",
                    "scripts",
                    worker(&release.reference)?,
                    "settings",
                ],
                &[],
                None,
            )
            .await?;
            return Ok(());
        }
        self.http
            .health(release.url.as_deref().ok_or_else(invalid)?)
            .await
    }

    async fn rollback(
        &self,
        token: &Secret,
        account: &str,
        release: &Release,
    ) -> Result<Release, EngineError> {
        let name = worker(&release.reference)?;
        self.call(
            token,
            Method::POST,
            &[
                "accounts",
                account,
                "workers",
                "scripts",
                name,
                "deployments",
            ],
            &[],
            Some(&json!({
                "strategy": "percentage",
                "versions": [{"version_id": release.version, "percentage": 100}],
            })),
        )
        .await?;
        if let Some(schedules) = &release.schedules {
            self.write_schedules(token, account, name, schedules)
                .await?;
        }
        Ok(release.clone())
    }

    async fn set_secret(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        if !valid_env_name(name) {
            return Err(EngineError::Invalid(
                "Use a valid environment variable name.",
            ));
        }
        self.call(
            token,
            Method::PUT,
            &[
                "accounts",
                account,
                "workers",
                "scripts",
                worker(reference)?,
                "secrets",
            ],
            &[],
            Some(&json!({"name":name,"text":value.expose(),"type":"secret_text"})),
        )
        .await?;
        Ok(())
    }

    // A watched Worker's variables are its secrets.
    async fn set_provider_variable(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        name: &str,
        value: &Secret,
    ) -> Result<(), EngineError> {
        self.set_secret(token, account, reference, name, value)
            .await
    }

    async fn set_provider_variables(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        values: &[(String, Secret)],
        shared: bool,
        _redeploy: bool,
    ) -> Result<(), EngineError> {
        if shared {
            return Err(crate::application::ports::providers::unsupported());
        }
        for (name, value) in values {
            self.set_secret(token, account, reference, name, value)
                .await?;
        }
        Ok(())
    }

    async fn delete_provider_variable(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        self.delete_secret(token, account, reference, name).await
    }

    async fn delete_secret(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        if !valid_env_name(name) {
            return Err(EngineError::Invalid(
                "Use a valid environment variable name.",
            ));
        }
        self.call(
            token,
            Method::DELETE,
            &[
                "accounts",
                account,
                "workers",
                "scripts",
                worker(reference)?,
                "secrets",
                name,
            ],
            &[],
            None,
        )
        .await?;
        Ok(())
    }

    async fn log_history(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        query: &crate::domain::logs::LogHistoryQuery,
    ) -> Result<crate::domain::logs::LogHistory, EngineError> {
        history::read(self, token, account, worker(reference)?, query).await
    }

    async fn provider_metrics(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        _step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        if !(reference.starts_with("worker:") || reference.starts_with("zone:")) {
            return Err(crate::application::ports::providers::unsupported());
        }
        metrics::read(self, token, account, reference, from, to).await
    }

    async fn open_logs(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        query: &str,
    ) -> Result<Box<dyn LogSession>, EngineError> {
        logs::open(self, token, account, worker(reference)?, query).await
    }

    async fn usage(
        &self,
        token: &Secret,
        account: &str,
        resource: &Resource,
        day: &str,
    ) -> Result<Usage, EngineError> {
        usage::read(self, token, account, resource, day).await
    }

    async fn configure_domain(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        hostname: &str,
        zone_name: &str,
        create_zone: bool,
    ) -> Result<crate::domain::domains::DomainObservation, EngineError> {
        self.configure_hostname(token, account, reference, hostname, zone_name, create_zone)
            .await
    }

    async fn detach_domain(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
        domain_id: Option<&str>,
        hostname: &str,
    ) -> Result<(), EngineError> {
        self.detach_hostname(token, account, reference, domain_id, hostname)
            .await
    }

    /// Deletes a Worker, a Pages project, a D1 database, an R2 bucket (in
    /// the default jurisdiction; it must be empty) or a domain's zone.
    async fn delete(
        &self,
        token: &Secret,
        account: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let named = |name: &str| {
            !name.is_empty()
                && name.len() <= MAX_NAME_CHARS
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        let path: Vec<&str> = if reference.starts_with("worker:") {
            vec![
                "accounts",
                account,
                "workers",
                "scripts",
                worker(reference)?,
            ]
        } else if let Some(name) = reference.strip_prefix("pages:").filter(|n| named(n)) {
            vec!["accounts", account, "pages", "projects", name]
        } else if let Some(id) = reference.strip_prefix("d1:").filter(|n| named(n)) {
            vec!["accounts", account, "d1", "database", id]
        } else if let Some(id) = reference.strip_prefix("kv:").filter(|n| named(n)) {
            vec!["accounts", account, "storage", "kv", "namespaces", id]
        } else if let Some(name) = reference.strip_prefix("r2:default:").filter(|n| named(n)) {
            vec!["accounts", account, "r2", "buckets", name]
        } else if let Some(zone) = reference.strip_prefix("zone:").filter(|n| named(n)) {
            vec!["zones", zone]
        } else {
            return Err(crate::application::ports::providers::unsupported());
        };
        self.call(token, Method::DELETE, &path, &[], None).await?;
        Ok(())
    }
}

mod domains;

mod history;
mod logs;
mod metrics;
mod usage;

mod inventory;

#[cfg(test)]
mod grant_tests {
    use super::*;
    use crate::application::ports::providers::{ProviderAdapter, ProviderNew};

    #[test]
    fn a_lone_module_worker_goes_through_the_api() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("wrangler.toml"),
            "name = \"hello\"\nmain = \"src/index.js\"\ncompatibility_date = \"2026-10-01\"\n",
        )
        .unwrap();
        std::fs::create_dir(folder.path().join("src")).unwrap();
        std::fs::write(
            folder.path().join("src/index.js"),
            "// says hello\nexport default { fetch() { return new Response('hi') } }\n",
        )
        .unwrap();
        let plain = plain_worker(folder.path()).unwrap().unwrap();
        assert_eq!(plain.file_name, "index.js");
        assert_eq!(plain.compatibility_date, "2026-10-01");
        assert!(plain.workers_dev);
        // An import means bundling, so Wrangler.
        std::fs::write(
            folder.path().join("src/index.js"),
            "import { Hono } from 'hono'\nexport default new Hono()\n",
        )
        .unwrap();
        assert!(plain_worker(folder.path()).unwrap().is_none());
        // So do bindings.
        std::fs::write(
            folder.path().join("src/index.js"),
            "export default { fetch() { return new Response('hi') } }\n",
        )
        .unwrap();
        std::fs::write(
            folder.path().join("wrangler.toml"),
            "name = \"hello\"\nmain = \"src/index.js\"\n[[kv_namespaces]]\nbinding = \"KV\"\nid = \"abc\"\n",
        )
        .unwrap();
        assert!(plain_worker(folder.path()).unwrap().is_none());
    }

    #[test]
    fn pages_and_worker_deployments_read_their_state() {
        use crate::application::ports::providers::ProviderDeployState;
        let live = pages_deployment("acct", "site", &serde_json::json!({
            "id": "d1", "created_on": "2026-11-01T10:00:00Z", "url": "https://d1.site.pages.dev",
            "latest_stage": {"name": "deploy", "status": "success", "ended_on": "2026-11-01T10:02:00Z"},
            "deployment_trigger": {"metadata": {"branch": "main", "commit_hash": "abc", "commit_message": "Ship it"}}
        })).unwrap();
        assert!(matches!(live.state, ProviderDeployState::Succeeded));
        assert_eq!(live.branch.as_deref(), Some("main"));
        let building = pages_deployment(
            "acct",
            "site",
            &serde_json::json!({
                "id": "d2",
                "created_on": "2026-11-01T11:00:00Z",
                "latest_stage": {"name": "build", "status": "active"},
            }),
        )
        .unwrap();
        assert!(matches!(building.state, ProviderDeployState::InProgress));
        let worker = worker_deployment(
            "acct",
            "api",
            true,
            &serde_json::json!({
                "id": "w1",
                "created_on": "2026-11-01T12:00:00Z",
                "source": "wrangler",
                "author_email": "sam@example.com",
                "annotations": {"workers/message": "Deploy from Hoista"},
            }),
        )
        .unwrap();
        assert_eq!(worker.message.as_deref(), Some("Deploy from Hoista"));
    }

    #[test]
    fn a_sign_in_without_workers_edit_is_caught_before_anything_is_sent() {
        let cloudflare = Cloudflare::new(ProviderHttp::new().expect("http"));
        let worker = ProviderNew::Folder {
            name: "edge",
            folder: std::path::Path::new("/tmp"),
        };
        let scopes = |scopes: &[&str]| -> Vec<String> {
            scopes.iter().map(|scope| (*scope).to_owned()).collect()
        };
        assert!(
            cloudflare
                .missing_grant(&scopes(&["offline_access", "workers-tail.read"]), &worker)
                .is_some_and(|said| said.contains("Workers Scripts Edit"))
        );
        assert!(
            cloudflare
                .missing_grant(
                    &scopes(&["offline_access", "workers-scripts.edit"]),
                    &worker
                )
                .is_none()
        );
        // A pasted token's permission names aren't judged here.
        assert!(
            cloudflare
                .missing_grant(
                    &scopes(&["Workers Scripts Read (verified by request)"]),
                    &worker
                )
                .is_none()
        );
    }
}
