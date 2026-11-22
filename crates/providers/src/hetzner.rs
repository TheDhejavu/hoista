//! Hetzner Cloud servers and volumes in one Hetzner project, through the
//! Hetzner Cloud API.
//!
//! A token belongs to one project, and the API never names it, so Hoista
//! names the account itself (after a hash of the token) and the person names it.

use super::http::ProviderHttp;
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{Observed, ProviderAdapter, RemoteResource},
    },
    domain::{Secret, model::*},
};
use reqwest::Method;
use serde_json::Value;
use std::time::Duration;

mod setup;

/// Hetzner Cloud's API adapter, for the project its token belongs to.
pub struct Hetzner {
    http: ProviderHttp,
}

const MAX_TEXT_CHARS: usize = 256;
/// Hetzner's largest page.
const PER_PAGE: &str = "50";
const MAX_PAGES: usize = 200;
/// Hetzner keeps metrics at no finer than a minute.
const MIN_STEP_SECONDS: u32 = 60;
/// How often, and how many times, Hoista checks on something Hetzner is doing.
const POLL_EVERY: Duration = Duration::from_secs(3);
const MAX_POLLS: usize = 40;
/// How long a server gets to shut down by itself before its power is cut.
const MAX_SHUTDOWN_POLLS: usize = 30;

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path fill="currentColor" fill-rule="evenodd" d="M0 0h24v24H0zM5.5 5v14h3.25v-5.4h6.5V19h3.25V5h-3.25v5.4h-6.5V5z"/></svg>"##;

/// Hetzner's entry in the provider list. The API has no list of projects, so
/// the adapter answers account choices itself.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Hetzner,
    logo: LOGO,
    capabilities: Capabilities {
        discover: true,
        provider_details: true,
        provider_operations: true,
        metrics: true,
        pause: true,
        delete: true,
        new_server: true,
        server_access: true,
        ..Capabilities::NONE
    },
    cli: None,
    api: super::ProviderApi {
        base: "https://api.hetzner.cloud/v1/",
        hosts: &["api.hetzner.cloud"],
        explains_refusals: true,
        sign_in: super::SignIn::Bearer,
    },
    accounts: None,
    adapter: |http| std::sync::Arc::new(Hetzner::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of the provider for the conformance suite: one server with a
/// volume. More specific paths come first, since the first match answers.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "hz-50878cfb5a906fbd",
    resource: "server:42",
    kind: ResourceKind::Server,
    routes: &[
        super::conformance::Route {
            path: "/servers/42/metrics",
            body: "",
            answer: r#"{"metrics":{"start":"2026-11-01T00:00:00Z","end":"2026-11-01T01:00:00Z","step":60,"time_series":{"cpu":{"values":[[1761955200,"12.5"],[1761955260,"7"]]},"network.0.bandwidth.in":{"values":[[1761955200,"2048"]]}}}}"#,
        },
        super::conformance::Route {
            path: "/servers/42",
            body: "",
            answer: SERVER,
        },
        super::conformance::Route {
            path: "/servers?",
            body: "",
            answer: concat!(
                r#"{"servers":["#,
                include_str!("hetzner/fake_server.json"),
                r#"],"meta":{"pagination":{"page":1,"per_page":50,"next_page":null,"last_page":1,"total_entries":1}}}"#
            ),
        },
        super::conformance::Route {
            path: "/volumes?",
            body: "",
            answer: r#"{"volumes":[{"id":7,"name":"data","size":100,"server":42,"status":"available","location":{"name":"fsn1","city":"Falkenstein"},"linux_device":"/dev/disk/by-id/scsi-0HC_Volume_7","created":"2026-10-01T10:00:00+00:00"}],"meta":{"pagination":{"page":1,"per_page":50,"next_page":null,"last_page":1,"total_entries":1}}}"#,
        },
    ],
    empty: r#"{"meta":{"pagination":{"page":1,"per_page":50,"next_page":null,"last_page":1,"total_entries":0}}}"#,
};

#[cfg(test)]
const SERVER: &str = concat!(
    r#"{"server":"#,
    include_str!("hetzner/fake_server.json"),
    "}"
);

fn invalid() -> EngineError {
    super::answers::incomplete("Hetzner")
}

/// The account a token reaches. Hetzner never names a token's project, so
/// the account is named after the token: the same token always gives the
/// same account, and a hash of it says nothing about the token. A rotated
/// token keeps the account it was added to.
fn account_id(token: &Secret) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::new()
        .chain_update(b"hoista hetzner account\n")
        .chain_update(token.expose().as_bytes())
        .finalize();
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("hz-{hex}")
}

/// An account id as [`account_id`] makes them.
fn plain_account(id: &str) -> bool {
    id.len() == 19 && id.starts_with("hz-") && id[3..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// What a reference names: a server or a volume, by Hetzner's numeric id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Server,
    Volume,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Self::Server => "servers",
            Self::Volume => "volumes",
        }
    }
}

fn split(reference: &str) -> Result<(Kind, &str), EngineError> {
    let (kind, id) = reference.split_once(':').ok_or_else(invalid)?;
    let kind = match kind {
        "server" => Kind::Server,
        "volume" => Kind::Volume,
        _ => return Err(invalid()),
    };
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    Ok((kind, id))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= MAX_TEXT_CHARS)
        .map(str::to_owned)
}

/// A server's status as a resource status. A server that's off is stopped
/// until someone starts it; Hetzner still bills it.
fn server_status(status: &str) -> ResourceStatus {
    match status {
        "running" => ResourceStatus::Live,
        "initializing" | "starting" | "rebuilding" | "migrating" => ResourceStatus::Deploying,
        "stopping" | "off" | "deleting" => ResourceStatus::Stopped,
        _ => ResourceStatus::Unknown,
    }
}

fn volume_status(status: &str) -> ResourceStatus {
    match status {
        "available" => ResourceStatus::Live,
        "creating" => ResourceStatus::Deploying,
        _ => ResourceStatus::Unknown,
    }
}

/// Where a server or volume is: the city, or the location's code.
fn location(item: &Value) -> Option<String> {
    let at = if item["location"].is_object() {
        &item["location"]
    } else {
        &item["datacenter"]["location"]
    };
    text(at, "city").or_else(|| text(at, "name"))
}

/// The server's public addresses. Hetzner gives a server an IPv6 network;
/// the server answers on its first address.
fn public_ips(server: &Value) -> Vec<String> {
    let net = &server["public_net"];
    let v4 = text(&net["ipv4"], "ip");
    let v6 = text(&net["ipv6"], "ip").map(|network| match network.strip_suffix("::/64") {
        Some(prefix) => format!("{prefix}::1"),
        None => network,
    });
    v4.into_iter().chain(v6).collect()
}

fn resource(kind: Kind, item: &Value) -> Result<RemoteResource, EngineError> {
    let id = item["id"].as_u64().ok_or_else(invalid)?;
    let name = text(item, "name").unwrap_or_else(|| id.to_string());
    let status = item["status"].as_str().unwrap_or_default();
    Ok(RemoteResource {
        reference: format!("{}:{id}", kind_word(kind)),
        name,
        kind: match kind {
            Kind::Server => ResourceKind::Server,
            Kind::Volume => ResourceKind::Bucket,
        },
        region: location(item),
        status: match kind {
            Kind::Server => server_status(status),
            Kind::Volume => volume_status(status),
        },
        url: None,
        // Servers Hoista made carry their project; Hetzner's own grouping.
        group: text(&item["labels"], setup::PROJECT_LABEL),
    })
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Server => "server",
        Kind::Volume => "volume",
    }
}

/// Hetzner's metric series as charts: CPU, disk and network.
fn server_charts(
    value: &Value,
) -> Vec<Option<crate::application::ports::providers::ProviderChart>> {
    use super::metrics::{chart, line, number, points, stamp};
    use crate::application::ports::providers::{MetricSummary as S, MetricUnit as U};
    let series = &value["metrics"]["time_series"];
    let line_of = |key: &str, name: &str| {
        let values = series[key]["values"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        line(
            name,
            points(values.iter().filter_map(|pair| {
                let seconds = pair[0].as_f64()?;
                #[expect(clippy::cast_possible_truncation, reason = "Unix seconds fit")]
                let at = chrono::DateTime::from_timestamp(seconds as i64, 0)?;
                Some((stamp(at), number(&pair[1])?))
            })),
        )
    };
    vec![
        chart(
            "cpu",
            "CPU",
            U::Percent,
            S::Average,
            vec![line_of("cpu", "CPU")],
        ),
        chart(
            "network",
            "Network per second",
            U::Bytes,
            S::Average,
            vec![
                line_of("network.0.bandwidth.in", "In"),
                line_of("network.0.bandwidth.out", "Out"),
            ],
        ),
        chart(
            "disk",
            "Disk per second",
            U::Bytes,
            S::Average,
            vec![
                line_of("disk.0.bandwidth.read", "Read"),
                line_of("disk.0.bandwidth.write", "Written"),
            ],
        ),
        chart(
            "disk_operations",
            "Disk operations per second",
            U::Count,
            S::Average,
            vec![
                line_of("disk.0.iops.read", "Reads"),
                line_of("disk.0.iops.write", "Writes"),
            ],
        ),
    ]
}

impl Hetzner {
    /// Shares the allow-listed HTTP boundary.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    async fn call(
        &self,
        token: &Secret,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value, EngineError> {
        self.http
            .json(Provider::Hetzner, token, method, path, query, body)
            .await
    }

    /// Every item of one of Hetzner's lists, page by page.
    async fn every(&self, token: &Secret, kind: Kind) -> Result<Vec<Value>, EngineError> {
        let key = kind.path();
        let mut all = Vec::new();
        let mut page = 1_u64;
        for _ in 0..MAX_PAGES {
            let page_text = page.to_string();
            let value = self
                .call(
                    token,
                    Method::GET,
                    &[key],
                    &[("page", &page_text), ("per_page", PER_PAGE)],
                    None,
                )
                .await?;
            all.extend(value[key].as_array().into_iter().flatten().cloned());
            match value["meta"]["pagination"]["next_page"].as_u64() {
                Some(next) if next > page => page = next,
                Some(_) => return Err(invalid()),
                None => return Ok(all),
            }
        }
        Err(invalid())
    }

    async fn one(&self, token: &Secret, kind: Kind, id: &str) -> Result<Value, EngineError> {
        let value = self
            .call(token, Method::GET, &[kind.path(), id], &[], None)
            .await?;
        let item = &value[kind_word(kind)];
        if item.is_object() {
            Ok(item.clone())
        } else {
            Err(invalid())
        }
    }

    /// Waits for one of Hetzner's actions (a power change, a detach) to finish.
    async fn finished(&self, token: &Secret, answer: &Value) -> Result<(), EngineError> {
        let id = answer["action"]["id"]
            .as_u64()
            .ok_or_else(invalid)?
            .to_string();
        for _ in 0..MAX_POLLS {
            let value = self
                .call(token, Method::GET, &["actions", &id], &[], None)
                .await?;
            match value["action"]["status"].as_str() {
                Some("success") => return Ok(()),
                Some("error") => {
                    return Err(EngineError::Provider {
                        evidence: None,
                        code: "provider.rejected".into(),
                        message: value["action"]["error"]["message"].as_str().map_or_else(
                            || "Hetzner couldn't finish the change.".to_owned(),
                            |said| format!("Hetzner couldn't finish the change: {said}"),
                        ),
                    });
                }
                _ => tokio::time::sleep(POLL_EVERY).await,
            }
        }
        Err(EngineError::Provider {
            evidence: None,
            code: "provider.timeout".into(),
            message: "Hetzner is still working on the change. Check the server in a minute.".into(),
        })
    }

    async fn server_action(
        &self,
        token: &Secret,
        id: &str,
        action: &str,
    ) -> Result<(), EngineError> {
        let answer = self
            .call(
                token,
                Method::POST,
                &["servers", id, "actions", action],
                &[],
                None,
            )
            .await?;
        self.finished(token, &answer).await
    }

    /// The volumes attached to server `id`.
    async fn volumes_of(&self, token: &Secret, id: &str) -> Result<Vec<Value>, EngineError> {
        Ok(self
            .every(token, Kind::Volume)
            .await?
            .into_iter()
            .filter(|volume| {
                volume["server"]
                    .as_u64()
                    .map(|on| on.to_string())
                    .as_deref()
                    == Some(id)
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Hetzner {
    fn provider(&self) -> Provider {
        Provider::Hetzner
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// The one project the token belongs to, once the token works.
    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        self.call(token, Method::GET, &["servers"], &[("per_page", "1")], None)
            .await?;
        Ok(vec![AccountChoice {
            external_id: account_id(token),
            name: "Hetzner project".into(),
        }])
    }

    /// The token still works. Hetzner can't say which project it belongs to,
    /// so a token from another project isn't caught here.
    async fn verify(&self, token: &Secret, account: &str) -> Result<Vec<String>, EngineError> {
        if !plain_account(account) {
            return Err(EngineError::Invalid("A Hetzner project is required."));
        }
        self.call(token, Method::GET, &["servers"], &[("per_page", "1")], None)
            .await?;
        Ok(vec![
            "Reaches the project's servers (verified by request)".into(),
        ])
    }

    async fn discover(
        &self,
        token: &Secret,
        _account: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        let mut resources = Vec::new();
        for kind in [Kind::Server, Kind::Volume] {
            for item in self.every(token, kind).await? {
                resources.push(resource(kind, &item)?);
            }
        }
        if resources.len() > MAX_DISCOVERED_RESOURCES {
            return Err(invalid());
        }
        Ok(resources)
    }

    async fn observe_resource(
        &self,
        token: &Secret,
        _account: &str,
        resource: &Resource,
    ) -> Result<Observed, EngineError> {
        let (kind, id) = split(&resource.provider_ref)?;
        let status = match self.one(token, kind, id).await {
            Ok(item) => {
                let said = item["status"].as_str().unwrap_or_default();
                match kind {
                    Kind::Server => server_status(said),
                    Kind::Volume => volume_status(said),
                }
            }
            Err(EngineError::Provider { code, .. }) if code == "provider.not_found" => {
                ResourceStatus::Stopped
            }
            Err(error) => return Err(error),
        };
        Ok(Observed {
            status,
            url: None,
            kind: None,
        })
    }

    /// A server's image, location, addresses and volumes as Hetzner keeps them.
    async fn provider_details(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
    ) -> Result<crate::application::ports::providers::ProviderDetails, EngineError> {
        use crate::application::ports::providers::{
            ProviderConfig, ProviderDetails, ProviderDomain, ProviderDomainState, ProviderVolume,
        };
        let (kind, id) = split(reference)?;
        let item = self.one(token, kind, id).await?;
        let volume = |volume: &Value| -> Option<ProviderVolume> {
            Some(ProviderVolume {
                id: format!("volume:{}", volume["id"].as_u64()?),
                name: text(volume, "name")?,
                mount_path: text(volume, "linux_device"),
                instance_id: None,
                used_mb: None,
                size_mb: volume["size"].as_u64().map(|gb| gb * 1024),
                region: location(volume),
            })
        };
        let (config, volumes, domains, public_ips) = match kind {
            Kind::Server => {
                let image = &item["image"];
                let config = ProviderConfig {
                    repo: None,
                    image: text(image, "description").or_else(|| text(image, "name")),
                    start_command: None,
                    build_command: None,
                    root_directory: None,
                    cron_schedule: None,
                    healthcheck_path: None,
                    replicas: None,
                    region: location(&item),
                    restart_policy: None,
                };
                let volumes = self
                    .volumes_of(token, id)
                    .await?
                    .iter()
                    .filter_map(volume)
                    .collect();
                // Hetzner's reverse DNS name points at the server.
                let domains = text(&item["public_net"]["ipv4"], "dns_ptr")
                    .map(|name| ProviderDomain {
                        name,
                        custom: false,
                        state: Some(ProviderDomainState::Working),
                        records: Vec::new(),
                        port: None,
                        certificate: None,
                        dns: Vec::new(),
                    })
                    .into_iter()
                    .collect();
                (Some(config), volumes, domains, public_ips(&item))
            }
            Kind::Volume => (
                None,
                volume(&item).into_iter().collect(),
                Vec::new(),
                Vec::new(),
            ),
        };
        Ok(ProviderDetails {
            deployments: Vec::new(),
            variable_names: None,
            shared_variable_names: None,
            domains,
            references: Vec::new(),
            sleeps: None,
            config,
            volumes,
            tcp_proxies: Vec::new(),
            private_domain: None,
            addresses: Vec::new(),
            public_ips,
        })
    }

    /// Hetzner's CPU, disk and network measurements for a server.
    async fn provider_metrics(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        step_seconds: u32,
    ) -> Result<crate::application::ports::providers::ProviderMetrics, EngineError> {
        use super::metrics::stamp;
        let (kind, id) = split(reference)?;
        if kind != Kind::Server {
            return Ok(super::metrics::metrics(
                from,
                to,
                Vec::new(),
                &[],
                "Hetzner",
            ));
        }
        let (start, end) = (stamp(from), stamp(to));
        let step = step_seconds.max(MIN_STEP_SECONDS).to_string();
        let value = self
            .call(
                token,
                Method::GET,
                &["servers", id, "metrics"],
                &[
                    ("type", "cpu,disk,network"),
                    ("start", &start),
                    ("end", &end),
                    ("step", &step),
                ],
                None,
            )
            .await?;
        Ok(super::metrics::metrics(
            from,
            to,
            server_charts(&value),
            &[],
            "Hetzner",
        ))
    }

    /// Shuts the server down, and cuts its power if it hasn't stopped by
    /// itself after a while. Hetzner keeps billing a server that's off.
    async fn pause(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let (Kind::Server, id) = split(reference)? else {
            return Err(EngineError::Invalid("Only a server can be turned off."));
        };
        self.server_action(token, id, "shutdown").await?;
        for _ in 0..MAX_SHUTDOWN_POLLS {
            if self.one(token, Kind::Server, id).await?["status"] == "off" {
                return Ok(());
            }
            tokio::time::sleep(POLL_EVERY).await;
        }
        self.server_action(token, id, "poweroff").await
    }

    async fn resume(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
    ) -> Result<ResourceStatus, EngineError> {
        let (Kind::Server, id) = split(reference)? else {
            return Err(EngineError::Invalid("Only a server can be turned on."));
        };
        self.server_action(token, id, "poweron").await?;
        Ok(ResourceStatus::Deploying)
    }

    /// Makes a server, set up so Hoista can reach it and nobody else can
    /// sign in.
    async fn provider_add(
        &self,
        token: &Secret,
        _account: &str,
        project: &str,
        _beside: Option<&str>,
        what: &crate::application::ports::providers::ProviderNew<'_>,
    ) -> Result<(Vec<String>, String), EngineError> {
        let Some(new) = setup::NewServer::from(what, project)? else {
            return Err(crate::application::ports::providers::unsupported());
        };
        self.make_server(token, &new).await
    }

    /// Deletes a server, whose volumes stay (each is deleted on its own),
    /// with the sign-in key and firewall Hoista made for it; or a volume,
    /// detached from its server first.
    async fn delete(
        &self,
        token: &Secret,
        _account: &str,
        reference: &str,
    ) -> Result<(), EngineError> {
        let (kind, id) = split(reference)?;
        if kind == Kind::Server {
            let server = self.one(token, kind, id).await?;
            let answer = self
                .call(token, Method::DELETE, &["servers", id], &[], None)
                .await?;
            self.finished(token, &answer).await?;
            // The server is gone either way; a key or firewall left behind
            // costs nothing and can be removed in the Console.
            if self.remove_made_with(token, &server).await.is_err() {
                tracing::warn!(
                    "couldn't remove the SSH key and firewall made for a deleted Hetzner server"
                );
            }
            return Ok(());
        }
        if self.one(token, kind, id).await?["server"].is_u64() {
            let answer = self
                .call(
                    token,
                    Method::POST,
                    &["volumes", id, "actions", "detach"],
                    &[],
                    None,
                )
                .await?;
            self.finished(token, &answer).await?;
        }
        self.call(token, Method::DELETE, &[kind.path(), id], &[], None)
            .await?;
        Ok(())
    }

    fn provider_operations(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderOperationInfo> {
        super::manifest::builtin("hetzner").operations()
    }

    fn provider_quick_actions(
        &self,
    ) -> Vec<crate::application::ports::providers::ProviderQuickAction> {
        super::manifest::builtin("hetzner").actions.clone()
    }

    /// Hetzner's API, aimed at the server or volume.
    async fn provider_operation(
        &self,
        token: &Secret,
        _account: &str,
        reference: Option<&str>,
        _folder: Option<&std::path::Path>,
        name: &str,
        args: Value,
    ) -> Result<Value, EngineError> {
        let mut context = Vec::new();
        if let Some(reference) = reference {
            let (kind, id) = split(reference)?;
            context.push((kind_word(kind), id));
        }
        super::api_catalog::run_manifest(
            &self.http,
            Provider::Hetzner,
            super::manifest::builtin("hetzner"),
            token,
            &context,
            name,
            args,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_token_always_names_the_same_account() {
        let token = Secret::new(super::super::conformance::TOKEN.into());
        assert_eq!(account_id(&token), FAKE.account);
        assert!(plain_account(&account_id(&token)));
        assert_ne!(
            account_id(&token),
            account_id(&Secret::new("another-token".into()))
        );
        assert!(!account_id(&token).contains(token.expose()));
        assert!(!plain_account("hz-../../etc"));
    }

    #[test]
    fn references_name_a_server_or_a_volume() {
        assert_eq!(split("server:42").unwrap(), (Kind::Server, "42"));
        assert_eq!(split("volume:7").unwrap(), (Kind::Volume, "7"));
        assert!(split("server:4/2").is_err());
        assert!(split("firewall:1").is_err());
        assert!(split("42").is_err());
    }

    #[test]
    fn a_server_reads_as_its_status_place_and_addresses() {
        let server: Value = serde_json::from_str(SERVER).unwrap();
        let found = resource(Kind::Server, &server["server"]).unwrap();
        assert_eq!(found.reference, "server:42");
        assert_eq!(found.kind, ResourceKind::Server);
        assert_eq!(found.status, ResourceStatus::Live);
        assert_eq!(found.region.as_deref(), Some("Falkenstein"));
        assert_eq!(
            public_ips(&server["server"]),
            ["203.0.113.10", "2001:db8:1234::1"]
        );
        assert_eq!(server_status("off"), ResourceStatus::Stopped);
        assert_eq!(server_status("starting"), ResourceStatus::Deploying);
    }

    #[test]
    fn metric_series_become_charts() {
        let value = serde_json::json!({"metrics": {"time_series": {
            "cpu": {"values": [[1_761_955_200.0, "12.5"], [1_761_955_260, "7"]]},
            "network.0.bandwidth.out": {"values": [[1_761_955_200, "100"]]},
        }}});
        let charts: Vec<_> = server_charts(&value).into_iter().flatten().collect();
        assert_eq!(charts.len(), 2);
        assert_eq!(charts[0].series[0].points.len(), 2);
        assert_eq!(charts[0].series[0].points[0].value, 12.5);
        assert_eq!(charts[1].series[0].name, "Out");
    }

    #[test]
    fn the_api_is_aimed_at_the_resource_and_keeps_access_out() {
        let manifest = super::super::manifest::builtin("hetzner");
        assert!(
            manifest
                .api_operation("getServer")
                .unwrap()
                .aimed_at("server")
        );
        assert!(
            manifest
                .api_operation("getVolume")
                .unwrap()
                .aimed_at("volume")
        );
        // Hoista makes and deletes servers itself, and nothing may reach a
        // server around it.
        for name in [
            "createServer",
            "deleteServer",
            "deleteVolume",
            "createSshKey",
            "requestConsoleForServer",
            "resetRootPasswordOfServer",
            "enableRescueModeForServer",
            "rebuildServerFromImage",
        ] {
            assert!(manifest.api_operation(name).is_none(), "{name}");
        }
    }
}
