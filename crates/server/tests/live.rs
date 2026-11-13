//! Explicitly opted-in, disposable provider acceptance. Never run ignored tests on PRs.

#![cfg(feature = "live")]

use hoista_server::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{Artifact, Asset, DiscoveryReport, ProviderAdapter, RemoteResource},
    },
    domain::{Secret, model::*},
    infrastructure::providers::{cloudflare::Cloudflare, fly::Fly, http::ProviderHttp, neon::Neon},
};
use serde_json::json;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    time::Duration,
};

type Outcome<T> = Result<T, String>;
const CONSENT: &str = "create-and-delete-disposable-resources";
struct Credentials {
    token: Secret,
    account: String,
}

struct Config {
    cloudflare: Credentials,
    fly: Credentials,
    neon: Credentials,
    journal_dir: PathBuf,
}

impl Config {
    fn load(read: impl Fn(&str) -> Option<String>) -> Outcome<Self> {
        if read("HOISTA_LIVE_CONFIRM").as_deref() != Some(CONSENT) {
            return Err(
                "Set HOISTA_LIVE_CONFIRM to the documented disposable-resource consent value."
                    .into(),
            );
        }
        let credentials = |token: &str, account: &str| -> Outcome<Credentials> {
            let value = read(token)
                .filter(|v| !v.trim().is_empty())
                .map(Secret::new)
                .ok_or_else(|| format!("Missing {token}."))?;
            let id = read(account)
                .filter(|v| valid_account(v))
                .ok_or_else(|| format!("Missing or invalid {account}."))?;
            Ok(Credentials {
                token: value,
                account: id,
            })
        };
        Ok(Self {
            cloudflare: credentials(
                "HOISTA_LIVE_CLOUDFLARE_TOKEN",
                "HOISTA_LIVE_CLOUDFLARE_ACCOUNT",
            )?,
            fly: credentials("HOISTA_LIVE_FLY_TOKEN", "HOISTA_LIVE_FLY_ORG")?,
            neon: credentials("HOISTA_LIVE_NEON_TOKEN", "HOISTA_LIVE_NEON_ORG")?,
            journal_dir: read("HOISTA_LIVE_JOURNAL_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("target/live-acceptance")),
        })
    }

    fn credentials(&self, provider: Provider) -> &Credentials {
        match provider {
            Provider::Cloudflare => &self.cloudflare,
            Provider::Fly => &self.fly,
            Provider::Neon => &self.neon,
            _ => unreachable!("fixed live targets"),
        }
    }
}

fn valid_account(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

struct Journal {
    file: File,
}

impl Journal {
    fn create(config: &Config, run: &str) -> Outcome<Self> {
        std::fs::create_dir_all(&config.journal_dir)
            .map_err(|_| "Cannot create live journal directory.")?;
        let path = config.journal_dir.join(format!("{run}.jsonl"));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|_| "Cannot create exclusive live journal.")?;
        #[cfg(unix)]
        File::open(&config.journal_dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "Cannot persist live journal directory entry.")?;
        Ok(Self { file })
    }

    fn record(&mut self, value: serde_json::Value) -> Outcome<()> {
        let data = serde_json::to_vec(&value).map_err(|_| "Cannot encode live journal.")?;
        self.file
            .write_all(&data)
            .and_then(|()| self.file.write_all(b"\n"))
            .and_then(|()| self.file.sync_all())
            .map_err(|_| "Cannot persist live journal.".into())
    }
}

struct Target {
    provider: Provider,
    name: String,
    kind: ResourceKind,
    attempted: bool,
    reference: Option<String>,
}

fn targets(run: &str) -> Vec<Target> {
    [
        (Provider::Cloudflare, "static", ResourceKind::Web),
        (Provider::Cloudflare, "worker", ResourceKind::Web),
        (Provider::Fly, "fly", ResourceKind::Service),
        (Provider::Neon, "db", ResourceKind::Database),
    ]
    .into_iter()
    .map(|(provider, suffix, kind)| Target {
        provider,
        name: format!("{run}-{suffix}"),
        kind,
        attempted: false,
        reference: None,
    })
    .collect()
}

fn adapter(http: &ProviderHttp, provider: Provider) -> Box<dyn ProviderAdapter> {
    match provider {
        Provider::Cloudflare => Box::new(Cloudflare::new(http.clone())),
        Provider::Fly => Box::new(Fly::new(http.clone())),
        Provider::Neon => Box::new(Neon::new(http.clone())),
        _ => unreachable!("fixed live targets"),
    }
}

async fn bounded<T>(
    step: &str,
    seconds: u64,
    future: impl std::future::Future<Output = Result<T, EngineError>>,
) -> Outcome<T> {
    tokio::time::timeout(Duration::from_secs(seconds), future)
        .await
        .map_err(|_| format!("{step} timed out; inspect the cleanup journal."))?
        .map_err(|_| format!("{step} failed; inspect the cleanup journal."))
}

fn matching<'a>(
    report: &'a DiscoveryReport,
    target: &Target,
) -> Outcome<Option<&'a RemoteResource>> {
    if report.warnings.iter().any(|warning| {
        target.provider != Provider::Cloudflare || warning.resource_type == "workers"
    }) {
        return Err("Required inventory coverage is incomplete.".into());
    }
    let mut matches = report.resources.iter().filter(|resource| {
        resource.name == target.name
            && resource.kind == target.kind
            && (target.provider != Provider::Cloudflare
                || resource.reference.starts_with("worker:"))
    });
    let found = matches.next();
    if matches.next().is_some() {
        return Err(
            "Multiple resources match a disposable identity; cleanup requires review.".into(),
        );
    }
    if let (Some(expected), Some(actual)) = (&target.reference, found)
        && expected != &actual.reference
    {
        return Err("Disposable resource identity changed; cleanup requires review.".into());
    }
    Ok(found)
}

fn artifact(worker: bool, marker: &str, directory: Option<String>) -> Artifact {
    Artifact {
        provenance: None,
        assets: if !worker && directory.is_none() {
            vec![Asset {
                path: "/index.html".into(),
                content_type: "text/html".into(),
                bytes: format!("<!doctype html><title>Hoista acceptance</title>{marker}")
                    .into_bytes(),
            }]
        } else {
            vec![]
        },
        worker: worker.then(|| {
            format!(
                "export default {{fetch() {{return new Response('{}', {{headers: {{'cache-control':'no-store'}}}})}}}}",
                marker
            )
            .into_bytes()
        }),
        worker_name: "worker.js".into(),
        worker_modules: vec![],
        image: directory,
        secrets: vec![],
    }
}

async fn expect_body(release: &Release, provider: Provider, marker: &str) -> Outcome<()> {
    let url = release
        .url
        .as_deref()
        .ok_or("Released runtime has no public URL.")?;
    let url = reqwest::Url::parse(url).map_err(|_| "Invalid runtime URL.")?;
    let suffix = match provider {
        Provider::Cloudflare => ".workers.dev",
        Provider::Fly => ".fly.dev",
        _ => return Err("Unexpected public runtime provider.".into()),
    };
    if url.scheme() != "https"
        || !url.host_str().is_some_and(|host| host.ends_with(suffix))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("Runtime URL is outside the disposable provider provider family.".into());
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot initialize runtime probe.")?;
    for _ in 0..12 {
        if let Ok(mut response) = client
            .get(url.clone())
            .header("cache-control", "no-cache")
            .send()
            .await
            && response.status().is_success()
        {
            let mut bytes = Vec::new();
            let complete = loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if bytes.len() + chunk.len() > 65_536 {
                            return Err("Runtime response exceeds fixture limit.".into());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    Ok(None) => break true,
                    Err(_) => break false,
                }
            };
            if complete && String::from_utf8_lossy(&bytes).contains(marker) {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    Err("Runtime did not serve the expected fixture revision.".into())
}

async fn exercise(
    config: &Config,
    http: &ProviderHttp,
    targets: &mut [Target],
    journal: &mut Journal,
    directory: &std::path::Path,
) -> Outcome<()> {
    let mut command = tokio::process::Command::new("flyctl");
    command
        .arg("version")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(Duration::from_secs(20), command.status())
        .await
        .map_err(|_| "flyctl prerequisite timed out.")?
        .map_err(|_| "Install flyctl before running live acceptance.")?;
    if !status.success() {
        return Err("flyctl prerequisite failed.".into());
    }
    // Complete all read-only preflight before creating any provider resources.
    for target in targets.iter() {
        let host = adapter(http, target.provider);
        let c = config.credentials(target.provider);
        bounded(
            "credential verification",
            120,
            host.verify(&c.token, &c.account),
        )
        .await?;
        let inventory = bounded(
            "preflight inventory",
            120,
            host.discover_report(&c.token, &c.account),
        )
        .await?;
        if matching(&inventory, target)?.is_some() {
            return Err("Disposable name already exists; nothing may be overwritten.".into());
        }
    }
    for target in targets.iter_mut() {
        let host = adapter(http, target.provider);
        let c = config.credentials(target.provider);
        journal.record(json!({
            "phase": "create_intent",
            "provider": target.provider,
            "account": c.account,
            "name": target.name,
        }))?;
        target.attempted = true;
        let mut plan = ResourcePlan {
            schedules: None,
            provider_ref: None,
            name: target.name.clone(),
            kind: target.kind,
            provider: target.provider,
            account_id: "live-fixture".into(),
            region: None,
            depends_on: vec![],
        };
        let worker = target.name.ends_with("-worker");
        let image =
            (target.provider == Provider::Fly).then(|| directory.to_string_lossy().into_owned());
        let first = bounded(
            "deploy",
            900,
            host.deploy(
                &c.token,
                &c.account,
                &plan,
                &artifact(worker, "hoista-live-v1", image),
            ),
        )
        .await?;
        target.reference = Some(first.reference.clone());
        journal.record(json!({
            "phase": "created",
            "provider": target.provider,
            "name": target.name,
            "reference": first.reference,
        }))?;
        bounded("health", 180, host.health(&c.token, &c.account, &first)).await?;
        if target.provider != Provider::Neon {
            expect_body(
                &first,
                target.provider,
                if target.provider == Provider::Fly {
                    "hoista-live-fly-v1"
                } else {
                    "hoista-live-v1"
                },
            )
            .await?;
        }
        if target.provider == Provider::Cloudflare {
            plan.provider_ref = Some(first.reference.clone());
            let second = bounded(
                "second revision",
                600,
                host.deploy(
                    &c.token,
                    &c.account,
                    &plan,
                    &artifact(worker, "hoista-live-v2", None),
                ),
            )
            .await?;
            expect_body(&second, target.provider, "hoista-live-v2").await?;
            let rollback =
                bounded("rollback", 300, host.rollback(&c.token, &c.account, &first)).await?;
            expect_body(&rollback, target.provider, "hoista-live-v1").await?;
        }
        journal
            .record(json!({"phase":"verified","provider":target.provider,"name":target.name}))?;
    }
    Ok(())
}

async fn cleanup_target(
    host: &dyn ProviderAdapter,
    c: &Credentials,
    target: &Target,
    delay: Duration,
) -> Outcome<()> {
    let mut deletion_observed = false;
    for attempt in 0..6 {
        let inventory = bounded(
            "cleanup inventory",
            120,
            host.discover_report(&c.token, &c.account),
        )
        .await?;
        if let Some(resource) = matching(&inventory, target)? {
            bounded(
                "cleanup deletion",
                120,
                host.delete(&c.token, &c.account, &resource.reference),
            )
            .await?;
            deletion_observed = true;
            break;
        }
        if target.reference.is_some() {
            deletion_observed = true;
            break;
        }
        if attempt < 5 {
            tokio::time::sleep(delay).await;
        }
    }
    // A timed-out create can finish remotely after our observation. Do not claim
    // cleanup when no returned identity or discovered-and-deleted identity exists.
    if !deletion_observed {
        return Err("Creation outcome is unresolved; inspect the retained journal.".into());
    }
    // Re-read to verify removal rather than trusting a DELETE response.
    for _ in 0..6 {
        tokio::time::sleep(delay).await;
        let inventory = bounded(
            "cleanup verification",
            120,
            host.discover_report(&c.token, &c.account),
        )
        .await?;
        if matching(&inventory, target)?.is_none() {
            return Ok::<(), String>(());
        }
    }
    Err("Disposable resource remains after deletion.".into())
}

async fn cleanup(
    config: &Config,
    http: &ProviderHttp,
    targets: &[Target],
    journal: &mut Journal,
) -> Outcome<()> {
    let mut failed = false;
    // Always continue to later targets even if one cleanup or journal write fails.
    for target in targets.iter().rev().filter(|target| target.attempted) {
        let host = adapter(http, target.provider);
        let c = config.credentials(target.provider);
        let result = tokio::time::timeout(
            Duration::from_secs(180),
            cleanup_target(host.as_ref(), c, target, Duration::from_secs(5)),
        )
        .await
        .unwrap_or_else(|_| Err("Target cleanup timed out; inspect the retained journal.".into()));
        failed |= result.is_err();
        failed |= journal
            .record(json!({
                "phase": if result.is_ok() {"cleanup_verified"} else {"cleanup_incomplete"},
                "provider": target.provider,
                "name": target.name,
                "reference": target.reference,
            }))
            .is_err();
    }
    if failed {
        Err("Cleanup is incomplete. Review every create_intent in the retained journal.".into())
    } else {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "creates billable disposable resources; requires explicit consent and dedicated Cloudflare, Fly and Neon credentials"]
async fn live_static_worker_fly_neon_and_cloudflare_rollback() -> Outcome<()> {
    let config = Config::load(|name| std::env::var(name).ok())?;
    let run = format!("hoista-live-{}", uuid::Uuid::new_v4().simple());
    let mut journal = Journal::create(&config, &run)?;
    let http = ProviderHttp::new().map_err(|_| "Cannot initialize provider HTTP.")?;
    let directory = tempfile::tempdir().map_err(|_| "Cannot prepare Fly fixture.")?;
    for name in ["Dockerfile", "server.cjs"] {
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/live-fly")
                .join(name),
            directory.path().join(name),
        )
        .map_err(|_| "Cannot copy Fly fixture.")?;
    }
    let mut targets = targets(&run);
    let result = tokio::time::timeout(
        Duration::from_secs(2100),
        exercise(&config, &http, &mut targets, &mut journal, directory.path()),
    )
    .await
    .unwrap_or_else(|_| Err("Live exercise deadline expired.".into()));
    // No assertion or early return is permitted between exercise and cleanup.
    let cleaned = tokio::time::timeout(
        Duration::from_secs(900),
        cleanup(&config, &http, &targets, &mut journal),
    )
    .await
    .unwrap_or_else(|_| Err("Cleanup deadline expired; inspect the retained journal.".into()));
    journal.record(json!({"phase":"finished","exercise_passed":result.is_ok(),"cleanup_passed":cleaned.is_ok()}))?;
    cleaned?;
    result
}

#[test]
fn live_consent_and_all_credentials_are_required_before_setup() {
    assert!(Config::load(|_| None).is_err());
    assert!(Config::load(|name| (name == "HOISTA_LIVE_CONFIRM").then(|| CONSENT.into())).is_err());
    for missing in [
        "HOISTA_LIVE_CLOUDFLARE_TOKEN",
        "HOISTA_LIVE_CLOUDFLARE_ACCOUNT",
        "HOISTA_LIVE_FLY_TOKEN",
        "HOISTA_LIVE_FLY_ORG",
        "HOISTA_LIVE_NEON_TOKEN",
        "HOISTA_LIVE_NEON_ORG",
    ] {
        assert!(
            Config::load(|name| {
                if name == missing {
                    None
                } else if name == "HOISTA_LIVE_CONFIRM" {
                    Some(CONSENT.into())
                } else {
                    Some("fixture".into())
                }
            })
            .is_err(),
            "missing credential/account must not be optional"
        );
    }
    for invalid in ["", "org/other", "org\nother", "https://example.com"] {
        assert!(!valid_account(invalid));
    }
}

#[test]
fn cleanup_requires_exact_unambiguous_identity_and_complete_required_inventory() {
    let mut targets = targets("hoista-live-fixture");
    let target = &mut targets[0];
    let resource = |name: &str, reference: &str| RemoteResource {
        name: name.into(),
        reference: reference.into(),
        kind: ResourceKind::Web,
        region: None,
        status: ResourceStatus::Unknown,
        url: None,
        group: None,
    };
    let mut report = DiscoveryReport {
        resources: vec![resource("other", "worker:other")],
        warnings: vec![],
    };
    assert!(matching(&report, target).unwrap().is_none());
    report
        .resources
        .push(resource(&target.name, "worker:exact"));
    assert!(matching(&report, target).unwrap().is_some());
    target.reference = Some("worker:different".into());
    assert!(matching(&report, target).is_err());
    target.reference = None;
    report
        .resources
        .push(resource(&target.name, "worker:duplicate"));
    assert!(matching(&report, target).is_err());
    report.resources.pop();
    report.warnings.push(DiscoveryWarning {
        resource_type: "workers".into(),
        code: "denied".into(),
        message: "fixture".into(),
    });
    assert!(matching(&report, target).is_err());
    report.warnings[0].resource_type = "r2".into();
    assert!(matching(&report, target).is_ok());
}

struct CleanupFixture {
    name: String,
    present: std::sync::atomic::AtomicBool,
    retain_after_delete: bool,
    deletes: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderAdapter for CleanupFixture {
    fn provider(&self) -> Provider {
        Provider::Cloudflare
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            deploy: false,
            discover: true,
            server: false,
            logs: false,
            metrics: false,
            usage: false,
            rollback: false,
            pause: false,
            domains: false,
            secrets: false,
            plan_deploy: true,
            cron: true,
            dns_zones: true,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(if self.present.load(std::sync::atomic::Ordering::SeqCst) {
            vec![RemoteResource {
                name: self.name.clone(),
                reference: "worker:fixture".into(),
                kind: ResourceKind::Web,
                region: None,
                status: ResourceStatus::Unknown,
                url: None,
                group: None,
            }]
        } else {
            vec![]
        })
    }

    async fn delete(&self, _: &Secret, _: &str, reference: &str) -> Result<(), EngineError> {
        assert_eq!(reference, "worker:fixture");
        self.deletes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if !self.retain_after_delete {
            self.present
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
}

#[tokio::test]
async fn cleanup_checks_removal_and_does_not_hide_uncertain_creation() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let target = targets("hoista-live-fixture").remove(0);
    let c = Credentials {
        token: Secret::new("fixture-token".into()),
        account: "fixture".into(),
    };
    for (present, retain, succeeds, deletes) in [
        (true, false, true, 1),
        (true, true, false, 1),
        (false, false, false, 0),
    ] {
        let host = CleanupFixture {
            name: target.name.clone(),
            present: AtomicBool::new(present),
            retain_after_delete: retain,
            deletes: AtomicUsize::new(0),
        };
        assert_eq!(
            cleanup_target(&host, &c, &target, Duration::ZERO)
                .await
                .is_ok(),
            succeeds
        );
        assert_eq!(host.deletes.load(Ordering::SeqCst), deletes);
    }
}

#[test]
fn journal_is_exclusive_retained_and_private_on_unix() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = Config::load(|name| {
        Some(if name == "HOISTA_LIVE_CONFIRM" {
            CONSENT.into()
        } else {
            "fixture".into()
        })
    })
    .unwrap();
    config.journal_dir = directory.path().to_owned();
    let mut journal = Journal::create(&config, "test-run").unwrap();
    journal
        .record(json!({"phase":"create_intent","name":"test-resource"}))
        .unwrap();
    drop(journal);
    assert!(Journal::create(&config, "test-run").is_err());
    let path = directory.path().join("test-run.jsonl");
    let contents = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(contents.trim()).unwrap()["phase"],
        "create_intent"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
