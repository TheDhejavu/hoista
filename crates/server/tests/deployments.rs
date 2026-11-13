//! Deploy tests on a real SQLite store, with fake providers and builds that behave
//! the same way every run.

use hoista_server::{
    Provider,
    application::{
        deploys::planning::Detection,
        engine::{Actor, Engine},
        ports::{
            build::{BuildService, SourceInspector},
            catalog::{CatalogRepository, EngineError},
            providers::{Artifact, ProviderAdapter, RemoteResource},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{paths::StatePaths, store::SqliteStore},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

struct Vault;
#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, _: &str) -> Result<Secret, EngineError> {
        Ok(Secret::new("fixture-token".into()))
    }

    async fn write(&self, _: &str, _: Secret) -> Result<(), EngineError> {
        Ok(())
    }

    async fn delete(&self, _: &str) -> Result<(), EngineError> {
        Ok(())
    }
}

struct Inspector;
#[async_trait::async_trait]
impl SourceInspector for Inspector {
    async fn detect(&self, _: &Source) -> Result<Detection, EngineError> {
        Ok(Detection {
            schedules: None,
            generated_secret_names: vec![],
            prisma_migration: None,
            framework: "fixture".into(),
            shape: "edge".into(),
            package_manager: None,
            build_command: vec![],
            output_directory: "dist".into(),
            env_names: vec![],
            postgres: false,
            warnings: vec![],
        })
    }
}

#[derive(Default)]
struct Builder {
    supplied: std::sync::Mutex<Vec<(String, String)>>,
    built: std::sync::Mutex<Vec<Vec<(String, String)>>>,
    migrations: AtomicUsize,
    migration_fails: AtomicBool,
    cleaned: AtomicUsize,
}

#[async_trait::async_trait]
impl BuildService for Builder {
    async fn secrets(&self, _: &Plan) -> Result<Vec<(String, Secret)>, EngineError> {
        Ok(self
            .supplied
            .lock()
            .expect("supplied")
            .iter()
            .map(|(name, value)| (name.clone(), Secret::new(value.clone())))
            .collect())
    }

    async fn build(
        &self,
        _: &Plan,
        _: &str,
        secrets: Vec<(String, Secret)>,
    ) -> Result<Artifact, EngineError> {
        self.built.lock().expect("built").push(
            secrets
                .iter()
                .map(|(name, value)| (name.clone(), value.expose().to_owned()))
                .collect(),
        );
        Ok(Artifact {
            provenance: Some(hoista_server::domain::provenance::SourceProvenance {
                git_commit: Some("a".repeat(40)),
                git_dirty: Some(false),
                snapshot_sha256: "b".repeat(64),
                snapshot_format: "hoista-source-v1".into(),
                snapshot_files: 1,
            }),
            assets: vec![],
            worker: Some(vec![]),
            worker_name: "index.mjs".into(),
            worker_modules: vec![],
            image: None,
            secrets: secrets
                .into_iter()
                .filter(|(name, _)| !browser_env(name))
                .collect(),
        })
    }

    async fn migrate(&self, _: &Plan, _: &str, database: &Secret) -> Result<(), EngineError> {
        assert_eq!(database.expose(), "direct-migration");
        self.migrations.fetch_add(1, Ordering::SeqCst);
        if self.migration_fails.load(Ordering::SeqCst) {
            return Err(EngineError::Conflict("Fixture migration failure"));
        }
        Ok(())
    }

    async fn cleanup(&self, _: &str) -> Result<(), EngineError> {
        self.cleaned.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct TestProvider {
    discovery_calls: AtomicUsize,
    discovery_fails: AtomicBool,
    secret_writes: AtomicUsize,
    log_closed: Arc<AtomicUsize>,
    log_flood: AtomicBool,
    partial_discovery: AtomicBool,
    extra_database: AtomicBool,
    deleted: std::sync::Mutex<Vec<String>>,
    targets: std::sync::Mutex<Vec<Option<String>>>,
    verification: AtomicUsize,
    verification_wait: AtomicBool,
    verification_entered: Notify,
    verification_continue: Notify,
    control_fails: AtomicBool,
    released: AtomicUsize,
    domains: AtomicUsize,
    detached: AtomicUsize,
    rolled_back: AtomicUsize,
    health_fails: AtomicBool,
    rollback_fails: AtomicBool,
    hold: AtomicBool,
    entered: Notify,
    continue_write: Notify,
}

#[async_trait::async_trait]
impl ProviderAdapter for TestProvider {
    async fn set_secret(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: &str,
        _: &Secret,
    ) -> Result<(), EngineError> {
        self.secret_writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn database_observation(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
    ) -> Result<hoista_server::domain::database_observation::DatabaseObservation, EngineError> {
        if self.hold.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.continue_write.notified().await;
        }
        Ok(
            hoista_server::domain::database_observation::DatabaseObservation {
                observed_at: chrono::Utc::now().to_rfc3339(),
                branches: vec![],
                computes: vec![],
                wake_count: None,
            },
        )
    }

    async fn log_history(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: &hoista_server::domain::logs::LogHistoryQuery,
    ) -> Result<hoista_server::domain::logs::LogHistory, EngineError> {
        if self.hold.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.continue_write.notified().await;
        }
        Ok(hoista_server::domain::logs::LogHistory {
            items: vec![],
            next_cursor: None,
            query_sampling_level: None,
            coverage_complete: false,
        })
    }

    async fn open_logs(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Box<dyn hoista_server::application::ports::providers::LogSession>, EngineError>
    {
        Ok(Box::new(FixtureTail {
            closed: self.log_closed.clone(),
            flood: self.log_flood.load(Ordering::SeqCst),
        }))
    }

    async fn database_inventory(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
    ) -> Result<Vec<hoista_server::domain::database_export::DatabaseIdentity>, EngineError> {
        Ok((0..if self.extra_database.load(Ordering::SeqCst) {
            3
        } else {
            2
        })
            .map(
                |index| hoista_server::domain::database_export::DatabaseIdentity {
                    branch_id: format!("branch-{index}"),
                    database_name: "project".into(),
                    owner_name: "owner".into(),
                },
            )
            .collect())
    }

    async fn database_export_connection(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: &hoista_server::domain::database_export::DatabaseIdentity,
    ) -> Result<Secret, EngineError> {
        Ok(Secret::new("postgresql://owner:private@host/app".into()))
    }

    async fn delete(&self, _: &Secret, _: &str, reference: &str) -> Result<(), EngineError> {
        self.deleted
            .lock()
            .expect("deleted")
            .push(reference.to_owned());
        Ok(())
    }

    fn provider(&self) -> Provider {
        Provider::Cloudflare
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            deploy: true,
            rollback: true,
            logs: true,
            metrics: false,
            usage: false,
            pause: true,
            domains: true,
            secrets: false,
            discover: false,
            server: false,
            plan_deploy: true,
            cron: true,
            dns_zones: true,
            ..Capabilities::default()
        }
    }

    fn reports_bindings(&self, reference: &str) -> bool {
        reference.starts_with("worker:")
    }

    fn keeps_known_role(&self, reference: &str) -> bool {
        reference.starts_with("worker:")
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        if self.verification_wait.load(Ordering::SeqCst) {
            self.verification_entered.notify_one();
            self.verification_continue.notified().await;
        }
        match self.verification.load(Ordering::SeqCst) {
            0 => Ok(vec!["verified read".into()]),
            4 => Ok(vec!["fixture-token".into()]),
            value => Err(EngineError::Provider {
                evidence: None,
                code: match value {
                    1 => "provider.token_expired",
                    2 => "provider.token_scope",
                    _ => "provider.unavailable",
                }
                .into(),
                message: "Fixture verification failure".into(),
            }),
        }
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }

    async fn discover_report(
        &self,
        token: &Secret,
        account: &str,
    ) -> Result<hoista_server::application::ports::providers::DiscoveryReport, EngineError> {
        use hoista_server::application::ports::providers::DiscoveryReport;
        self.discovery_calls.fetch_add(1, Ordering::SeqCst);
        if self.discovery_fails.load(Ordering::SeqCst) {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.unavailable".into(),
                message: "private-provider-response".into(),
            });
        }
        if self.partial_discovery.load(Ordering::SeqCst) {
            return Ok(DiscoveryReport {
                resources: vec![RemoteResource {
                    reference: "worker:new".into(),
                    name: "new".into(),
                    kind: ResourceKind::Web,
                    region: None,
                    status: ResourceStatus::Unknown,
                    url: None,
                    group: None,
                }],
                warnings: vec![DiscoveryWarning {
                    resource_type: "pages".into(),
                    code: "provider.token_scope".into(),
                    message: "Pages access denied".into(),
                }],
            });
        }
        Ok(DiscoveryReport {
            resources: self.discover(token, account).await?,
            warnings: vec![],
        })
    }

    async fn deploy(
        &self,
        _: &Secret,
        _: &str,
        resource: &ResourcePlan,
        _: &Artifact,
    ) -> Result<Release, EngineError> {
        self.targets
            .lock()
            .expect("targets")
            .push(resource.provider_ref.clone());
        self.entered.notify_one();
        if self.hold.load(Ordering::SeqCst) {
            self.continue_write.notified().await;
        }
        let number = self.released.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Release {
            schedules: None,
            reference: format!("worker:{}", resource.name),
            version: format!("v{number}"),
            url: Some("https://app.fixture.workers.dev".into()),
        })
    }

    async fn pause(&self, _: &Secret, _: &str, _: &str) -> Result<(), EngineError> {
        if self.control_fails.load(Ordering::SeqCst) {
            return Err(EngineError::Conflict("Fixture pause failure"));
        }
        Ok(())
    }

    async fn resume(&self, _: &Secret, _: &str, _: &str) -> Result<ResourceStatus, EngineError> {
        Ok(ResourceStatus::Live)
    }

    async fn configure_domain(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: &str,
        zone: &str,
        _: bool,
    ) -> Result<hoista_server::domain::domains::DomainObservation, EngineError> {
        use hoista_server::domain::domains::*;
        self.domains.fetch_add(1, Ordering::SeqCst);
        Ok(DomainObservation {
            dns_records: vec![],
            zone_id: "zone".into(),
            zone_name: zone.into(),
            nameservers: vec!["ada.ns.cloudflare.com".into()],
            registrar: None,
            dns: DomainReadiness::Pending,
            https: DomainReadiness::Unknown,
            provider_domain_id: None,
            certificate_id: None,
            warnings: vec![],
        })
    }

    async fn detach_domain(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> Result<(), EngineError> {
        self.detached.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn health(&self, _: &Secret, _: &str, release: &Release) -> Result<(), EngineError> {
        if release.version != "v1" && self.health_fails.load(Ordering::SeqCst) {
            return Err(EngineError::Provider {
                evidence: Some(ProviderFailureEvidence {
                    http_status: 503,
                    request_id: Some("0123456789abcdef-LHR".into()),
                    request_id_header: Some("cf-ray".into()),
                }),
                code: "deploy.health_failed".into(),
                message: "Fixture health failure.".into(),
            });
        }
        Ok(())
    }

    async fn rollback(
        &self,
        _: &Secret,
        _: &str,
        release: &Release,
    ) -> Result<Release, EngineError> {
        self.rolled_back.fetch_add(1, Ordering::SeqCst);
        if self.rollback_fails.load(Ordering::SeqCst) {
            return Err(EngineError::Conflict("Fixture rollback failure."));
        }
        Ok(release.clone())
    }
}

async fn finished(store: &SqliteStore, id: &str) -> Deploy {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let value = store
                .deploy(id)
                .await
                .expect("deployment read")
                .expect("deployment");
            if !matches!(value.status, DeployStatus::Queued | DeployStatus::Running) {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("deployment completed")
}

#[tokio::test]
async fn releases_health_failures_and_inflight_cancellation_preserve_recovery_evidence() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let first = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue first");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .expect("provider entered");
    let first = finished(&store, &first.id).await;
    assert_eq!(first.status, DeployStatus::Live);
    assert_eq!(first.releases[0].release.version, "v1");
    assert_eq!(
        first.provenance.as_ref().unwrap().git_commit,
        Some("a".repeat(40))
    );

    let map = serde_json::to_value(
        engine
            .projects()
            .project_map(&Actor::Person, &project.id)
            .await
            .unwrap(),
    )
    .unwrap();
    let graph = &map["graph"];
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"][0]["kind"], "deploys");
    assert_eq!(graph["edges"][0]["deploy_id"], first.id);
    assert_eq!(graph["edges"][0]["origin"], "deployment");
    assert!(graph["edges"][0]["traffic"].is_null());
    assert_eq!(graph["relationships_complete"], false);

    provider.health_fails.store(true, Ordering::SeqCst);
    let second = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue second");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .expect("provider entered");
    let second = finished(&store, &second.id).await;
    assert_eq!(second.status, DeployStatus::Failed);
    let failure = second.failure.as_ref().expect("durable failure evidence");
    assert_eq!(failure.code, "deploy.health_failed");
    let response = failure
        .provider_response
        .as_ref()
        .expect("persisted response evidence");
    assert_eq!(response.http_status, 503);
    assert_eq!(response.request_id.as_deref(), Some("0123456789abcdef-LHR"));
    assert_eq!(failure.steps, ["verify"]);
    assert!(failure.release_started);
    assert!(!failure.compensation_failed);
    assert_eq!(provider.rolled_back.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .resource(&first.releases[0].resource_id)
            .await
            .expect("resource")
            .expect("resource exists")
            .status,
        ResourceStatus::Live
    );

    provider.health_fails.store(false, Ordering::SeqCst);
    provider.hold.store(true, Ordering::SeqCst);
    let third = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue third");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .expect("provider entered");
    for enabled in [false, true] {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.settings().update_settings(
                &Actor::Person,
                SettingsPatch {
                    agents_enabled: Some(enabled),
                    ..Default::default()
                },
            ),
        )
        .await
        .expect("agent toggle is not blocked by provider write")
        .expect("settings");
    }
    // Cancellation must return while the provider write holds the mutation lock.
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        engine.deploys().cancel_deploy(&Actor::Person, &third.id),
    )
    .await
    .expect("cancellation not blocked")
    .expect("cancel");
    assert_eq!(provider.released.load(Ordering::SeqCst), 2);
    provider.continue_write.notify_one();
    let third = finished(&store, &third.id).await;
    assert_eq!(third.status, DeployStatus::Failed);
    assert!(third.cancel_requested);
    assert_eq!(third.releases[0].release.version, "v3");
    assert_eq!(provider.rolled_back.load(Ordering::SeqCst), 2);

    provider.hold.store(false, Ordering::SeqCst);
    provider.health_fails.store(true, Ordering::SeqCst);
    provider.rollback_fails.store(true, Ordering::SeqCst);
    let fourth = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue fourth");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .expect("provider entered");
    let fourth = finished(&store, &fourth.id).await;
    assert!(
        fourth
            .failure
            .as_ref()
            .expect("failure evidence")
            .compensation_failed
    );
    assert!(
        fourth
            .steps
            .iter()
            .any(|step| step.name == "automatic_rollback" && step.status == StepStatus::Failed)
    );
    provider.health_fails.store(false, Ordering::SeqCst);
    provider.rollback_fails.store(false, Ordering::SeqCst);
    provider.hold.store(true, Ordering::SeqCst);
    let fifth = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue fifth");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .expect("provider entered");
    stop.cancel();
    // Wait for the durable shutdown request; the held provider call must not be dropped.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !store
            .deploy(&fifth.id)
            .await
            .expect("read")
            .expect("attempt")
            .cancel_requested
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shutdown cancellation");
    assert!(!worker.is_finished());
    provider.continue_write.notify_one();
    let fifth = finished(&store, &fifth.id).await;
    assert_eq!(fifth.status, DeployStatus::Failed);
    assert_eq!(fifth.releases[0].release.version, "v5");
    worker
        .await
        .expect("joined worker")
        .expect("worker shutdown");
    assert_eq!(builder.cleaned.load(Ordering::SeqCst), 5);
    store.close().await;
}

#[tokio::test]
async fn custom_domains_reserve_ownership_obey_watching_and_survive_metadata_backup() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    );
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Watching,
        )
        .await
        .expect("project");
    for id in ["one", "two"] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: Some(project.id.clone()),
                kind: ResourceKind::Web,
                provider: Provider::Cloudflare,
                account_id: "account".into(),
                provider_ref: format!("worker:{id}"),
                name: id.into(),
                region: None,
                status: ResourceStatus::Live,
                mode: Mode::Watching,
                url: None,
                group: None,
            })
            .await
            .expect("resource");
    }
    assert!(
        engine
            .environments()
            .add_domain(
                &Actor::Person,
                &project.id,
                "one",
                "app.example.com",
                "example.com",
                true
            )
            .await
            .is_err()
    );
    assert_eq!(provider.domains.load(Ordering::SeqCst), 0);
    engine
        .projects()
        .set_mode(&project.id, Mode::Managed)
        .await
        .expect("manage");
    let domain = engine
        .environments()
        .add_domain(
            &Actor::Person,
            &project.id,
            "one",
            "APP.EXAMPLE.COM",
            "example.com",
            true,
        )
        .await
        .expect("domain");
    assert_eq!(domain.hostname, "app.example.com");
    assert!(
        engine
            .environments()
            .add_domain(
                &Actor::Person,
                &project.id,
                "two",
                "app.example.com",
                "example.com",
                true
            )
            .await
            .is_err()
    );
    assert_eq!(provider.domains.load(Ordering::SeqCst), 1);
    engine
        .environments()
        .refresh_domain(&domain.id)
        .await
        .expect("refresh");
    let backup = engine.data().export().await.expect("backup");
    assert_eq!(backup.custom_domains.len(), 1);
    let restored_paths = StatePaths::new(Some(dir.path().join("restored"))).expect("restore paths");
    let restored_store = SqliteStore::open(&restored_paths)
        .await
        .expect("restore store");
    let restored_engine = Engine::new(
        Arc::new(restored_store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    );
    restored_engine
        .data()
        .restore(backup)
        .await
        .expect("restore");
    let restored = restored_store
        .custom_domain(&domain.id)
        .await
        .expect("read restored")
        .expect("restored domain");
    assert_eq!(
        restored.observed.dns,
        hoista_server::domain::domains::DomainReadiness::Unknown
    );
    assert!(
        restored_engine
            .environments()
            .refresh_domain(&domain.id)
            .await
            .is_err()
    );
    restored_store.close().await;
    engine
        .projects()
        .set_mode(&project.id, Mode::Watching)
        .await
        .expect("watch");
    assert!(
        engine
            .environments()
            .refresh_domain(&domain.id)
            .await
            .is_err()
    );
    assert_eq!(provider.domains.load(Ordering::SeqCst), 2);
    engine
        .projects()
        .set_mode(&project.id, Mode::Managed)
        .await
        .expect("manage");
    engine
        .environments()
        .delete_domain(&Actor::Person, &domain.id, None)
        .await
        .expect("detach");
    assert_eq!(provider.detached.load(Ordering::SeqCst), 1);
    assert!(
        store
            .custom_domain(&domain.id)
            .await
            .expect("read")
            .is_none()
    );
    assert!(store.resource("one").await.expect("resource").is_some());
    assert_eq!(
        engine
            .deploys()
            .control_resource(&Actor::Person, "one", false)
            .await
            .expect("pause")
            .status,
        ResourceStatus::Paused
    );
    assert_eq!(
        engine
            .deploys()
            .control_resource(&Actor::Person, "one", true)
            .await
            .expect("resume")
            .status,
        ResourceStatus::Live
    );
    provider.control_fails.store(true, Ordering::SeqCst);
    assert!(
        engine
            .deploys()
            .control_resource(&Actor::Person, "one", false)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .resource("one")
            .await
            .expect("read")
            .expect("resource")
            .status,
        ResourceStatus::Unknown
    );
    engine
        .projects()
        .set_mode(&project.id, Mode::Watching)
        .await
        .expect("watch");
    assert!(
        engine
            .deploys()
            .control_resource(&Actor::Person, "one", true)
            .await
            .is_err()
    );
    engine
        .projects()
        .set_mode(&project.id, Mode::Managed)
        .await
        .expect("manage");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");
    engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue");
    assert!(
        engine
            .deploys()
            .control_resource(&Actor::Person, "one", true)
            .await
            .is_err()
    );

    store.close().await;
}

#[tokio::test]
async fn account_checks_distinguish_revocation_missing_scopes_and_network_outages() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    );
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec!["old".into()],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    provider.verification.store(3, Ordering::SeqCst);
    assert!(engine.accounts().check_account("account").await.is_err());
    assert_eq!(
        store
            .account("account")
            .await
            .expect("read")
            .expect("account")
            .status,
        AccountStatus::Connected
    );
    for (result, expected) in [
        (1, AccountStatus::Expired),
        (2, AccountStatus::MissingScopes),
        (0, AccountStatus::Connected),
    ] {
        provider.verification.store(result, Ordering::SeqCst);
        let account = engine
            .accounts()
            .check_account("account")
            .await
            .expect("check");
        assert_eq!(account.status, expected);
        assert_eq!(account.scopes.is_empty(), result != 0);
    }
    engine
        .accounts()
        .disconnect_account("account")
        .await
        .expect("disconnect");
    assert!(engine.accounts().check_account("account").await.is_err());
    assert_eq!(
        store
            .account("account")
            .await
            .expect("read")
            .expect("account")
            .status,
        AccountStatus::Disconnected
    );
    store.close().await;
}

struct PrismaInspector;
#[async_trait::async_trait]
impl SourceInspector for PrismaInspector {
    async fn detect(&self, source: &Source) -> Result<Detection, EngineError> {
        let mut value = Inspector.detect(source).await?;
        value.postgres = true;
        value.prisma_migration = Some(PrismaMigration {
            schema: "prisma/schema.prisma".into(),
            migrations: "prisma/migrations".into(),
        });
        Ok(value)
    }
}

struct Database;
#[async_trait::async_trait]
impl ProviderAdapter for Database {
    fn provider(&self) -> Provider {
        Provider::Neon
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            project_database: true,
            ..TestProvider::default().capabilities()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }

    async fn deploy(
        &self,
        _: &Secret,
        _: &str,
        _: &ResourcePlan,
        _: &Artifact,
    ) -> Result<Release, EngineError> {
        Ok(Release {
            schedules: None,
            reference: "project:database".into(),
            version: "database-version".into(),
            url: None,
        })
    }

    async fn connection_string(&self, _: &Secret, _: &str, _: &str) -> Result<Secret, EngineError> {
        Ok(Secret::new("pooled-runtime".into()))
    }

    async fn migration_connection_string(
        &self,
        _: &Secret,
        _: &str,
        _: &str,
    ) -> Result<Secret, EngineError> {
        Ok(Secret::new("direct-migration".into()))
    }

    async fn health(&self, _: &Secret, _: &str, _: &Release) -> Result<(), EngineError> {
        Ok(())
    }
}

#[tokio::test]
async fn prisma_failure_preserves_database_and_old_code_then_retry_links_runtime() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let builder = Arc::new(Builder::default());
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(PrismaInspector),
        vec![provider.clone(), Arc::new(Database)],
        builder.clone(),
    ));
    for (id, provider) in [("host", Provider::Cloudflare), ("database", Provider::Neon)] {
        store
            .put_account(&Account {
                last_discovery: None,
                id: id.into(),
                provider,
                label: id.into(),
                external_id: id.into(),
                scopes: vec![],
                status: AccountStatus::Connected,
                connected_at: "2026-11-02T00:00:00Z".into(),
                credential: None,
                in_1password: false,
            })
            .await
            .expect("account");
    }
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");
    let mut changed = plan.clone();
    changed.prisma_migration = None;
    assert!(
        engine
            .deploys()
            .edit_plan(&Actor::Person, changed)
            .await
            .is_err()
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let first = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue");
    let first = finished(&store, &first.id).await;
    assert_eq!(first.status, DeployStatus::Live);
    assert_eq!(builder.migrations.load(Ordering::SeqCst), 1);
    let binding = store
        .secrets(&project.id)
        .await
        .unwrap()
        .into_iter()
        .find(|value| value.name == "DATABASE_URL")
        .unwrap();
    assert_eq!(binding.managed_by, Some(Provider::Neon));
    assert_eq!(binding.set_by.as_deref(), Some("hoista"));
    assert_eq!(store.links(&project.id).await.expect("links").len(), 1);
    builder.migration_fails.store(true, Ordering::SeqCst);
    let failed = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue");
    let failed = finished(&store, &failed.id).await;
    assert_eq!(failed.status, DeployStatus::Failed);
    assert_eq!(failed.releases.len(), 1);
    assert_eq!(
        failed
            .steps
            .iter()
            .find(|s| s.name == "migrate")
            .expect("migration")
            .status,
        StepStatus::Failed
    );
    assert_eq!(provider.released.load(Ordering::SeqCst), 1);
    assert_eq!(provider.rolled_back.load(Ordering::SeqCst), 0);
    builder.migration_fails.store(false, Ordering::SeqCst);
    let retry = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("retry");
    assert_eq!(finished(&store, &retry.id).await.status, DeployStatus::Live);
    assert_eq!(store.links(&project.id).await.expect("links").len(), 1);
    let rollback = engine
        .deploys()
        .enqueue_rollback(&Actor::Person, &first.id)
        .await
        .expect("rollback");
    assert_eq!(
        finished(&store, &rollback.id).await.status,
        DeployStatus::RolledBack
    );
    assert_eq!(
        finished(&store, &rollback.id).await.provenance,
        first.provenance
    );
    assert_eq!(
        builder.migrations.load(Ordering::SeqCst),
        3,
        "rollback never migrates"
    );
    stop.cancel();
    worker.await.expect("worker").expect("shutdown");
}

struct AuthInspector;
#[async_trait::async_trait]
impl SourceInspector for AuthInspector {
    async fn detect(&self, source: &Source) -> Result<Detection, EngineError> {
        let mut value = Inspector.detect(source).await?;
        value.env_names = vec!["BETTER_AUTH_SECRET".into()];
        value.generated_secret_names = value.env_names.clone();
        Ok(value)
    }
}

#[tokio::test]
async fn generated_auth_is_memory_only_and_redeploy_does_not_rotate_it() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let builder = Arc::new(Builder::default());
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(AuthInspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "host".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");
    let stop = tokio_util::sync::CancellationToken::new();
    let first = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue");
    let second = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("requeue");
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    assert_eq!(finished(&store, &first.id).await.status, DeployStatus::Live);
    assert_eq!(
        store.secrets(&project.id).await.unwrap()[0]
            .set_by
            .as_deref(),
        Some("hoista")
    );
    let generated = builder.built.lock().expect("built")[0][0].1.clone();
    assert_eq!(generated.len(), 64);
    let fingerprint = format!("{:x}", Sha256::digest(generated.as_bytes()));
    assert_eq!(
        store.secrets(&project.id).await.expect("metadata")[0].fingerprint,
        fingerprint
    );
    assert_eq!(
        finished(&store, &second.id).await.status,
        DeployStatus::Live
    );
    assert_eq!(
        *provider.targets.lock().expect("targets"),
        vec![None, Some("worker:project".into())]
    );
    assert!(
        builder.built.lock().expect("built")[1].is_empty(),
        "redeploy preserves hosted key"
    );
    assert_eq!(
        store.secrets(&project.id).await.expect("metadata")[0].fingerprint,
        fingerprint
    );
    builder.supplied.lock().expect("supplied").push((
        "BETTER_AUTH_SECRET".into(),
        "explicit-user-supplied-secret-value".into(),
    ));
    let third = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("explicit value");
    assert_eq!(finished(&store, &third.id).await.status, DeployStatus::Live);
    assert_eq!(
        builder.built.lock().expect("built")[2][0].1,
        "explicit-user-supplied-secret-value"
    );
    let mut missing_plan = plan.clone();
    missing_plan.id = "missing-inputs-plan".into();
    missing_plan
        .env_names
        .extend(["Z_KEY".into(), "A_KEY".into()]);
    store
        .put_plan(&missing_plan)
        .await
        .expect("save missing-input plan");
    let missing = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &missing_plan.id)
        .await
        .expect("queue missing inputs");
    let failed = finished(&store, &missing.id).await;
    let failure = failed.failure.as_ref().expect("failure evidence");
    assert_eq!(failure.code, "deploy.secret_missing");
    assert_eq!(failure.missing_secrets, ["A_KEY", "Z_KEY"]);
    assert_eq!(failure.steps, ["secrets"]);
    assert!(!failure.release_started);
    assert_eq!(builder.built.lock().expect("built").len(), 3);
    builder.supplied.lock().expect("supplied").extend([
        ("A_KEY".into(), "private-a-repair-value".into()),
        ("Z_KEY".into(), "private-z-repair-value".into()),
    ]);
    let repaired = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &missing_plan.id)
        .await
        .expect("queue corrected inputs");
    let repaired = finished(&store, &repaired.id).await;
    assert_eq!(repaired.status, DeployStatus::Live);
    assert!(repaired.failure.is_none());
    let history = serde_json::to_string(&store.deploys(&project.id, None).await.expect("history"))
        .expect("history JSON");
    assert!(!history.contains("private-a-repair-value"));
    assert!(!history.contains("private-z-repair-value"));
    stop.cancel();
    worker.await.expect("worker").expect("shutdown");
    let changed = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queue identity check");
    let mut resource = store
        .resources(None)
        .await
        .expect("resources")
        .items
        .remove(0);
    resource.provider_ref = "worker:replacement".into();
    store
        .put_resource(&resource)
        .await
        .expect("external identity change");
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let changed = finished(&store, &changed.id).await;
    assert_eq!(changed.status, DeployStatus::Failed);
    assert_eq!(
        provider.released.load(Ordering::SeqCst),
        4,
        "identity conflict cannot release code"
    );
    assert_eq!(
        builder.built.lock().expect("builds").len(),
        4,
        "identity conflict fails before build"
    );
    stop.cancel();
    worker.await.expect("worker").expect("shutdown");
    store.close().await;
    for entry in walkdir::WalkDir::new(paths.root()) {
        let entry = entry.expect("entry");
        // The daemon's lock file stays locked while it runs; Windows refuses to read it.
        if entry.file_type().is_file() && entry.path().extension() != Some("lock".as_ref()) {
            let bytes = std::fs::read(entry.path())
                .unwrap_or_else(|error| panic!("read {}: {error}", entry.path().display()));
            assert!(
                !bytes
                    .windows(generated.len())
                    .any(|window| window == generated.as_bytes()),
                "generated value reached disk"
            );
        }
    }
}

#[tokio::test]
async fn partial_discovery_preserves_unobserved_resources_and_reports_coverage() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    provider.partial_discovery.store(true, Ordering::SeqCst);
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider],
        Arc::new(Builder::default()),
    );
    store
        .put_account(&Account {
            last_discovery: None,
            id: "host".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let existing = Resource {
        id: "pages".into(),
        project_id: None,
        kind: ResourceKind::Web,
        provider: Provider::Cloudflare,
        account_id: "host".into(),
        provider_ref: "pages:existing".into(),
        name: "existing".into(),
        region: None,
        status: ResourceStatus::Live,
        mode: Mode::Managed,
        url: Some("https://existing.pages.dev".into()),
        group: None,
    };
    store
        .put_resource(&existing)
        .await
        .expect("existing resource");
    let report = engine
        .accounts()
        .discover(&Actor::Person, "host")
        .await
        .expect("partial scan");
    assert!(!report.complete);
    assert_eq!(report.warnings[0].resource_type, "pages");
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].mode, Mode::Watching);
    assert_eq!(
        store.resource("pages").await.expect("resource"),
        Some(existing)
    );
    assert_eq!(
        store.resources(None).await.expect("resources").items.len(),
        2
    );
    let mut job = report.items[0].clone();
    job.kind = ResourceKind::Job;
    job.mode = Mode::Managed;
    store.put_resource(&job).await.expect("local job role");
    let refreshed = engine
        .accounts()
        .discover(&Actor::Person, "host")
        .await
        .expect("refresh job");
    assert_eq!(refreshed.items[0].id, job.id);
    assert_eq!(refreshed.items[0].kind, ResourceKind::Job);
    assert_eq!(refreshed.items[0].mode, Mode::Managed);
    assert_eq!(
        store.resource(&job.id).await.unwrap().unwrap().kind,
        ResourceKind::Job
    );
    let source = Source::Folder {
        path: "/fixture".into(),
    };
    let project = engine
        .projects()
        .create_project(&Actor::Person, "new".into(), source.clone(), Mode::Managed)
        .await
        .unwrap();
    engine
        .projects()
        .group(&Actor::Person, &project.id, std::slice::from_ref(&job.id))
        .await
        .unwrap();
    engine
        .projects()
        .set_mode(&project.id, Mode::Managed)
        .await
        .unwrap();
    let plan = engine
        .deploys()
        .plan(&Actor::Person, source, "new", None, None)
        .await
        .unwrap();
    assert_eq!(
        plan.resources[0].kind,
        ResourceKind::Web,
        "source inference alone has no existing role"
    );
    let queued = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .unwrap();
    let snapshot = store.plan(&queued.plan_id).await.unwrap().unwrap();
    assert_eq!(snapshot.resources[0].kind, ResourceKind::Job);
    assert_eq!(
        snapshot.resources[0].provider_ref.as_deref(),
        Some("worker:new")
    );

    store.close().await;
}

#[tokio::test]
async fn deploy_permission_cannot_create_projects_domains_or_add_resources() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault),
            Arc::new(Inspector),
            vec![provider.clone()],
            Arc::new(Builder::default()),
        )
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
        )),
    );
    store
        .put_account(&Account {
            last_discovery: None,
            id: "host".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    store
        .put_agent(&Agent {
            id: "deployer".into(),
            name: "Deployer".into(),
            transport: AgentTransport::Stdio,
            scopes: vec!["apps:read".into(), "apps:write".into()],
            last_seen: chrono::Utc::now().to_rfc3339(),
            revoked: false,
            grant: Default::default(),
        })
        .await
        .expect("agent");
    let actor = Actor::Agent {
        id: "deployer".into(),
    };
    let source = Source::Folder {
        path: "/fixture".into(),
    };
    assert!(
        engine
            .projects()
            .create_project(&actor, "blocked".into(), source.clone(), Mode::Managed)
            .await
            .is_err()
    );
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            source.clone(),
            Mode::Managed,
        )
        .await
        .expect("owner project");
    let mut plan = engine
        .deploys()
        .plan(&actor, source.clone(), "project", None, None)
        .await
        .expect("planning is allowed");
    assert!(
        engine
            .deploys()
            .enqueue_deploy(&actor, &project.id, &plan.id)
            .await
            .is_err(),
        "first allocation requires creation"
    );
    let first = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("owner queue");
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let first = finished(&store, &first.id).await;
    assert_eq!(first.status, DeployStatus::Live);
    let redeploy = engine
        .deploys()
        .enqueue_deploy(&actor, &project.id, &plan.id)
        .await
        .expect("existing resource redeploy");
    assert_eq!(
        finished(&store, &redeploy.id).await.status,
        DeployStatus::Live
    );
    assert!(
        engine
            .environments()
            .add_domain(
                &actor,
                &project.id,
                &first.releases[0].resource_id,
                "app.example.com",
                "example.com",
                false
            )
            .await
            .is_err()
    );
    assert_eq!(provider.domains.load(Ordering::SeqCst), 0);
    let mut extra = plan.resources[0].clone();
    extra.name = "extra".into();
    plan.resources.push(extra);
    engine
        .deploys()
        .edit_plan(&actor, plan.clone())
        .await
        .expect("proposal");
    assert!(
        engine
            .deploys()
            .enqueue_deploy(&actor, &project.id, &plan.id)
            .await
            .is_err(),
        "edited plans cannot bypass allocation permissions"
    );
    engine
        .agents()
        .set_agent_scopes(
            &Actor::Person,
            "deployer",
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
        )
        .await
        .expect("owner expands scopes");
    let expanded = engine
        .deploys()
        .enqueue_deploy(&actor, &project.id, &plan.id)
        .await
        .expect("creation permitted");
    assert_eq!(
        finished(&store, &expanded.id).await.status,
        DeployStatus::Live
    );
    engine
        .environments()
        .add_domain(
            &actor,
            &project.id,
            &first.releases[0].resource_id,
            "app.example.com",
            "example.com",
            false,
        )
        .await
        .expect("domain permitted");
    engine
        .projects()
        .create_project(&actor, "new-project".into(), source, Mode::Managed)
        .await
        .expect("new project permitted");
    stop.cancel();
    worker.await.expect("worker").expect("shutdown");
    store.close().await;
}

#[derive(Default)]
struct Exporter {
    fails: AtomicBool,
    exported: AtomicUsize,
    change_inventory: std::sync::Mutex<Option<Arc<TestProvider>>>,
}

#[async_trait::async_trait]
impl hoista_server::application::ports::database_export::DatabaseExporter for Exporter {
    fn available(&self) -> bool {
        true
    }

    async fn archives(
        &self,
        _cursor: Option<&str>,
    ) -> Result<
        hoista_server::application::ports::catalog::Page<
            hoista_server::domain::database_export::DatabaseArchive,
        >,
        EngineError,
    > {
        Ok(hoista_server::application::ports::catalog::Page {
            items: vec![],
            next_cursor: None,
        })
    }

    async fn export(
        &self,
        resource_id: &str,
        source: &hoista_server::application::ports::database_export::DatabaseExportSource,
    ) -> Result<hoista_server::domain::database_export::DatabaseArchive, EngineError> {
        if self.fails.load(Ordering::SeqCst) {
            return Err(EngineError::Conflict("Fixture export failed."));
        }
        self.exported.fetch_add(1, Ordering::SeqCst);
        if let Some(provider) = self.change_inventory.lock().expect("fixture").take() {
            provider.extra_database.store(true, Ordering::SeqCst);
        }
        Ok(hoista_server::domain::database_export::DatabaseArchive {
            id: uuid::Uuid::new_v4().to_string(),
            resource_id: resource_id.into(),
            branch_id: source.branch_id.clone(),
            database_name: source.database_name.clone(),
            path: "/fixture/archive.dump".into(),
            bytes: 100,
            sha256: "a".repeat(64),
            completed_at: hoista_server::application::engine::now(),
        })
    }
}

#[tokio::test]
async fn deletion_validates_the_whole_map_and_removes_consumers_before_dependencies() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let exporter = Arc::new(Exporter::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    )
    .with_owner_check(Arc::new(
        hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
    ))
    .with_database_exporter(exporter.clone());
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    for cyclic in [true, false] {
        provider.deleted.lock().unwrap().clear();
        exporter.exported.store(0, Ordering::SeqCst);
        let project = engine
            .projects()
            .create_project(
                &Actor::Person,
                "fixture".into(),
                Source::Folder {
                    path: "/fixture".into(),
                },
                Mode::Managed,
            )
            .await
            .unwrap();
        let ids: Vec<_> = ["a-storage", "b-service", "c-web"]
            .into_iter()
            .map(|name| format!("{}-{name}", project.id))
            .collect();
        for (i, id) in ids.iter().enumerate() {
            store
                .put_resource(&Resource {
                    id: id.clone(),
                    project_id: Some(project.id.clone()),
                    kind: if i == 0 {
                        ResourceKind::Database
                    } else {
                        ResourceKind::Web
                    },
                    provider: Provider::Cloudflare,
                    account_id: "account".into(),
                    provider_ref: id.clone(),
                    name: id.clone(),
                    region: None,
                    status: ResourceStatus::Live,
                    mode: Mode::Managed,
                    url: None,
                    group: None,
                })
                .await
                .unwrap();
        }
        for (from, to) in if cyclic {
            vec![(2, 1), (1, 0), (0, 2)]
        } else {
            vec![(2, 1), (1, 0)]
        } {
            store
                .put_link(&Link {
                    id: format!("{}-{from}-{to}", project.id),
                    from_resource: ids[from].clone(),
                    to_resource: ids[to].clone(),
                    via: "binding".into(),
                    derived: true,
                    missing: false,
                })
                .await
                .unwrap();
        }
        assert!(matches!(
            engine
                .projects()
                .delete_resource(&Actor::Person, &ids[0], None, None)
                .await,
            Err(EngineError::Conflict(_))
        ));
        assert!(provider.deleted.lock().unwrap().is_empty());
        let preview = engine
            .projects()
            .preview_deletion(&Actor::Person, GatedAction::DeleteProject, &project.id)
            .await
            .unwrap();
        assert_eq!(preview.resources.len(), 3);
        assert!(preview.blockers.is_empty());
        let preview = if !cyclic {
            assert_eq!(preview.resources[0].id, ids[2]);
            assert_eq!(preview.resources[2].kind, ResourceKind::Database);
            let mut observed = store.resource(&ids[2]).await.unwrap().unwrap();
            observed.status = ResourceStatus::Sleeping;
            store.put_resource(&observed).await.unwrap();
            assert_eq!(
                engine
                    .projects()
                    .preview_deletion(&Actor::Person, GatedAction::DeleteProject, &project.id)
                    .await
                    .unwrap()
                    .fingerprint,
                preview.fingerprint
            );
            engine
                .environments()
                .add_domain(
                    &Actor::Person,
                    &project.id,
                    &ids[2],
                    "remove.example.com",
                    "example.com",
                    false,
                )
                .await
                .unwrap();
            let stale = hoista_server::domain::deletion::DeletionConfirmation {
                backup_databases: true,
                confirmation_name: preview.confirmation_name.clone(),
                fingerprint: preview.fingerprint.clone(),
            };
            assert!(
                engine
                    .deploys()
                    .delete_project(&Actor::Person, &project.id, None, Some(&stale))
                    .await
                    .is_err()
            );
            assert!(provider.deleted.lock().unwrap().is_empty());
            assert_eq!(provider.detached.load(Ordering::SeqCst), 0);
            let refreshed = engine
                .projects()
                .preview_deletion(&Actor::Person, GatedAction::DeleteProject, &project.id)
                .await
                .unwrap();
            assert_ne!(refreshed.fingerprint, preview.fingerprint);
            assert_eq!(refreshed.domains.len(), 1);
            assert_eq!(refreshed.domains[0].hostname, "remove.example.com");
            refreshed
        } else {
            preview
        };
        let confirmation = hoista_server::domain::deletion::DeletionConfirmation {
            backup_databases: true,
            confirmation_name: preview.confirmation_name,
            fingerprint: preview.fingerprint,
        };
        let confirmation = if !cyclic {
            assert_eq!(preview.databases.len(), 2);
            exporter.fails.store(true, Ordering::SeqCst);
            assert!(
                engine
                    .deploys()
                    .delete_project(&Actor::Person, &project.id, None, Some(&confirmation))
                    .await
                    .is_err()
            );
            assert!(provider.deleted.lock().unwrap().is_empty());
            assert_eq!(provider.detached.load(Ordering::SeqCst), 0);
            exporter.fails.store(false, Ordering::SeqCst);
            *exporter.change_inventory.lock().unwrap() = Some(provider.clone());
            assert!(
                engine
                    .deploys()
                    .delete_project(&Actor::Person, &project.id, None, Some(&confirmation))
                    .await
                    .is_err()
            );
            assert!(provider.deleted.lock().unwrap().is_empty());
            assert_eq!(provider.detached.load(Ordering::SeqCst), 0);
            let current = engine
                .projects()
                .preview_deletion(&Actor::Person, GatedAction::DeleteProject, &project.id)
                .await
                .unwrap();
            assert_eq!(current.databases.len(), 3);
            hoista_server::domain::deletion::DeletionConfirmation {
                backup_databases: true,
                confirmation_name: current.confirmation_name,
                fingerprint: current.fingerprint,
            }
        } else {
            confirmation
        };
        let result = if cyclic {
            engine
                .deploys()
                .delete_project(&Actor::Person, &project.id, None, Some(&confirmation))
                .await
        } else {
            let (agent, _) = engine
                .agents()
                .register_agent(
                    "Delete fixture".into(),
                    vec!["apps:read".into(), "apps:write".into()],
                    AgentTransport::Stdio,
                )
                .await
                .unwrap();
            let actor = Actor::Agent { id: agent.id };
            let approval = engine
                .request_approval(
                    &actor,
                    GatedAction::DeleteProject,
                    project.id.clone(),
                    "Remove fixture".into(),
                )
                .await
                .unwrap();
            engine
                .projects()
                .decide_approval(&approval.id, true, Some(&confirmation))
                .await
                .unwrap();
            exporter.fails.store(true, Ordering::SeqCst);
            assert!(
                engine
                    .deploys()
                    .delete_project(&actor, &project.id, Some(&approval.id), None)
                    .await
                    .is_err()
            );
            assert_eq!(
                store
                    .approval(&approval.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .decision,
                ApprovalDecision::Allowed
            );
            assert!(provider.deleted.lock().unwrap().is_empty());
            exporter.fails.store(false, Ordering::SeqCst);
            let result = engine
                .deploys()
                .delete_project(&actor, &project.id, Some(&approval.id), None)
                .await;
            assert_eq!(
                store
                    .approval(&approval.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .decision,
                ApprovalDecision::Consumed
            );
            result
        };
        if cyclic {
            assert_eq!(result.unwrap().len(), 2);
            assert_eq!(
                *provider.deleted.lock().unwrap(),
                vec![ids[2].clone(), ids[1].clone(), ids[0].clone()]
            );
            assert!(store.project(&project.id).await.unwrap().is_none());
        } else {
            assert_eq!(result.unwrap().len(), 3);
            assert_eq!(exporter.exported.load(Ordering::SeqCst), 5);
            assert_eq!(
                *provider.deleted.lock().unwrap(),
                vec![ids[2].clone(), ids[1].clone(), ids[0].clone()]
            );
            assert!(store.project(&project.id).await.unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn individual_database_deletion_reviews_transitive_disruption_and_preserves_consumers() {
    use hoista_server::domain::deletion::DeletionConfirmation;
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let exporter = Arc::new(Exporter::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    )
    .with_database_exporter(exporter.clone());
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "fixture".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .unwrap();
    for (id, kind) in [
        ("db", ResourceKind::Database),
        ("api", ResourceKind::Service),
        ("web", ResourceKind::Web),
    ] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: Some(project.id.clone()),
                kind,
                provider: Provider::Cloudflare,
                account_id: "account".into(),
                provider_ref: id.into(),
                name: id.into(),
                region: None,
                status: ResourceStatus::Live,
                mode: Mode::Managed,
                url: None,
                group: None,
            })
            .await
            .unwrap();
    }
    engine
        .projects()
        .save_link(&Link {
            id: "api-db".into(),
            from_resource: "api".into(),
            to_resource: "db".into(),
            via: "DATABASE_URL".into(),
            derived: true,
            missing: false,
        })
        .await
        .unwrap();
    let first = engine
        .projects()
        .preview_deletion(&Actor::Person, GatedAction::DeleteResource, "db")
        .await
        .unwrap();
    assert!(first.blockers.is_empty());
    assert_eq!(first.resources.len(), 1);
    assert_eq!(
        first
            .affected_resources
            .iter()
            .map(|resource| resource.id.as_str())
            .collect::<Vec<_>>(),
        vec!["api"]
    );
    let stale = DeletionConfirmation {
        backup_databases: true,
        confirmation_name: first.confirmation_name,
        fingerprint: first.fingerprint,
    };
    engine
        .projects()
        .save_link(&Link {
            id: "web-api".into(),
            from_resource: "web".into(),
            to_resource: "api".into(),
            via: "API_URL".into(),
            derived: false,
            missing: false,
        })
        .await
        .unwrap();
    assert!(
        engine
            .projects()
            .delete_resource(&Actor::Person, "db", None, Some(&stale))
            .await
            .is_err()
    );
    assert_eq!(exporter.exported.load(Ordering::SeqCst), 0);
    assert!(provider.deleted.lock().unwrap().is_empty());
    engine
        .environments()
        .add_domain(
            &Actor::Person,
            &project.id,
            "api",
            "keep.example.com",
            "example.com",
            false,
        )
        .await
        .unwrap();
    let current = engine
        .projects()
        .preview_deletion(&Actor::Person, GatedAction::DeleteResource, "db")
        .await
        .unwrap();
    assert_eq!(
        current
            .affected_resources
            .iter()
            .map(|resource| resource.id.as_str())
            .collect::<Vec<_>>(),
        vec!["api", "web"]
    );
    assert!(current.domains.is_empty());
    let mut confirmation = DeletionConfirmation {
        backup_databases: false,
        confirmation_name: current.confirmation_name,
        fingerprint: current.fingerprint,
    };
    assert!(
        engine
            .projects()
            .delete_resource(&Actor::Person, "db", None, Some(&confirmation))
            .await
            .is_err()
    );
    confirmation.backup_databases = true;
    exporter.fails.store(true, Ordering::SeqCst);
    assert!(
        engine
            .projects()
            .delete_resource(&Actor::Person, "db", None, Some(&confirmation))
            .await
            .is_err()
    );
    assert!(provider.deleted.lock().unwrap().is_empty());
    assert_eq!(
        store.resource("api").await.unwrap().unwrap().status,
        ResourceStatus::Live
    );
    exporter.fails.store(false, Ordering::SeqCst);
    let archives = engine
        .projects()
        .delete_resource(&Actor::Person, "db", None, Some(&confirmation))
        .await
        .unwrap();
    assert_eq!(archives.len(), 2);
    assert_eq!(*provider.deleted.lock().unwrap(), vec!["db"]);
    assert!(store.resource("db").await.unwrap().is_none());
    for id in ["api", "web"] {
        assert_eq!(
            store.resource(id).await.unwrap().unwrap().status,
            ResourceStatus::Unknown
        );
    }
    assert_eq!(store.links(&project.id).await.unwrap().len(), 1);
    assert_eq!(
        store
            .custom_domains(Some(&project.id), None)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(provider.detached.load(Ordering::SeqCst), 0);
    assert!(store.project(&project.id).await.unwrap().is_some());
    assert_eq!(
        engine
            .projects()
            .project_overview(&project.id)
            .await
            .unwrap()
            .status,
        ResourceStatus::Unknown
    );
}

struct FixtureTail {
    closed: Arc<AtomicUsize>,
    flood: bool,
}

#[async_trait::async_trait]
impl hoista_server::application::ports::providers::LogSession for FixtureTail {
    async fn next(&mut self) -> Result<Option<hoista_server::domain::logs::LogEntry>, EngineError> {
        if !self.flood {
            std::future::pending::<()>().await;
        }
        tokio::task::yield_now().await;
        Ok(Some(hoista_server::domain::logs::LogEntry {
            timestamp: None,
            observed_at: chrono::Utc::now().to_rfc3339(),
            level: hoista_server::domain::logs::LogLevel::Info,
            source: "fixture".into(),
            message: "hello".into(),
        }))
    }

    async fn close(&mut self) -> Result<(), EngineError> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn live_tails_bound_queues_and_close_on_drop_pause_and_shutdown() {
    use hoista_server::application::engine::LogFrame;
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    );
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "logs".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .unwrap();
    store
        .put_resource(&Resource {
            id: "worker".into(),
            project_id: Some(project.id.clone()),
            kind: ResourceKind::Web,
            provider: Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:fixture".into(),
            name: "worker".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Managed,
            url: None,
            group: None,
        })
        .await
        .unwrap();
    assert!(
        engine
            .observe()
            .live_logs(&Actor::Person, "other", "worker", "", None)
            .await
            .is_err()
    );
    let mut streams = vec![];
    for _ in 0..8 {
        let mut stream = engine
            .observe()
            .live_logs(&Actor::Person, &project.id, "worker", "", None)
            .await
            .unwrap();
        assert!(matches!(
            stream.recv().await,
            Some(LogFrame::Ready {
                historical: false,
                ..
            })
        ));
        streams.push(stream);
    }
    assert!(
        engine
            .observe()
            .live_logs(&Actor::Person, &project.id, "worker", "", None)
            .await
            .is_err()
    );
    drop(streams);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while provider.log_closed.load(Ordering::SeqCst) < 8 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    provider.log_flood.store(true, Ordering::SeqCst);
    let mut flood = engine
        .observe()
        .live_logs(&Actor::Person, &project.id, "worker", "", None)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while provider.log_closed.load(Ordering::SeqCst) < 9 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut frames = 0;
    let mut ended = false;
    while let Some(frame) = flood.recv().await {
        frames += 1;
        if let LogFrame::End {
            reason,
            cleanup_complete,
        } = frame
        {
            assert_eq!(reason, "logs.slow_consumer");
            assert!(cleanup_complete);
            ended = true;
        }
    }
    assert!(ended && frames <= 65);
    let capture = engine
        .observe()
        .logs(
            &Actor::Person,
            &project.id,
            "worker",
            "2000-01-01T00:00:00Z",
            "",
        )
        .await
        .unwrap();
    assert!(!capture.historical);
    assert!(capture.started_at.is_some());
    assert!(capture.cleanup_complete);
    assert!(capture.items.len() <= 100);
    assert!(matches!(
        capture.end_reason.as_str(),
        "logs.capture_limit" | "logs.slow_consumer"
    ));
    provider.log_flood.store(false, Ordering::SeqCst);
    store
        .put_agent(&Agent {
            id: "reader".into(),
            name: "reader".into(),
            transport: AgentTransport::Stdio,
            scopes: vec!["apps:read".into()],
            last_seen: chrono::Utc::now().to_rfc3339(),
            revoked: false,
            grant: Default::default(),
        })
        .await
        .unwrap();
    let mut agent = engine
        .observe()
        .live_logs(
            &Actor::Agent {
                id: "reader".into(),
            },
            &project.id,
            "worker",
            "",
            None,
        )
        .await
        .unwrap();
    assert!(matches!(agent.recv().await, Some(LogFrame::Ready { .. })));
    let mut settings = store.settings().await.unwrap();
    settings.agents_enabled = false;
    store.put_settings(&settings).await.unwrap();
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(7), agent.recv())
        .await
        .unwrap();
    assert!(
        matches!(terminal,Some(LogFrame::End { reason,cleanup_complete:true }) if reason == "agent.disabled")
    );
    let mut owner = engine
        .observe()
        .live_logs(&Actor::Person, &project.id, "worker", "", None)
        .await
        .unwrap();
    assert!(matches!(owner.recv().await, Some(LogFrame::Ready { .. })));
    engine.observe().stop_logs().await;
    assert!(
        matches!(owner.recv().await,Some(LogFrame::End { reason,cleanup_complete:true }) if reason == "logs.cancelled")
    );
    assert_eq!(provider.log_closed.load(Ordering::SeqCst), 12);
    assert!(
        engine
            .observe()
            .live_logs(&Actor::Person, &project.id, "worker", "", None)
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn history_checks_resource_and_rechecks_revocation_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "logs".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .unwrap();
    store
        .put_resource(&Resource {
            id: "worker".into(),
            project_id: Some(project.id.clone()),
            kind: ResourceKind::Web,
            provider: Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:fixture".into(),
            name: "worker".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Managed,
            url: None,
            group: None,
        })
        .await
        .unwrap();

    use axum::{body::Body, http::Request};
    use hoista_server::domain::logs::LogHistoryQuery;
    use tower::ServiceExt;
    let query = LogHistoryQuery {
        from: "2026-01-01T00:00:00Z".into(),
        to: "2026-01-02T00:00:00Z".into(),
        query: String::new(),
        cursor: None,
    };
    assert!(
        engine
            .observe()
            .log_history(&Actor::Person, "other", "worker", &query)
            .await
            .is_err()
    );
    let router = hoista_server::catalog::router(engine.clone());
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/projects/{}/logs/history?resource=worker&from=2026-01-01T00:00:00Z&to=2026-01-02T00:00:00Z",
                    project.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["coverage_complete"], false);
    let (agent, _) = engine
        .agents()
        .register_agent(
            "History reader".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let reply = hoista_server::mcp::dispatch(
        &engine,
        &actor,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "get_log_history",
                "arguments": {
                    "project_id": project.id,
                    "resource_id": "worker",
                    "from": query.from,
                    "to": query.to,
                },
            },
        }),
    )
    .await
    .unwrap();
    assert_eq!(reply["result"]["isError"], false);
    provider.hold.store(true, Ordering::SeqCst);
    let task = tokio::spawn({
        let engine = engine.clone();
        let project = project.clone();
        async move {
            engine
                .observe()
                .log_history(&actor, &project.id, "worker", &query)
                .await
        }
    });
    provider.entered.notified().await;
    engine
        .agents()
        .revoke_agent(&Actor::Person, &agent.id)
        .await
        .unwrap();
    provider.continue_write.notify_one();
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn database_observation_supports_watching_and_rechecks_access() {
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "logs".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Watching,
        )
        .await
        .unwrap();
    store
        .put_resource(&Resource {
            id: "worker".into(),
            project_id: Some(project.id.clone()),
            kind: ResourceKind::Database,
            provider: Provider::Cloudflare,
            account_id: "account".into(),
            provider_ref: "worker:fixture".into(),
            name: "worker".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Watching,
            url: None,
            group: None,
        })
        .await
        .unwrap();

    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let result = engine
        .projects()
        .database_observation(&Actor::Person, "worker")
        .await
        .unwrap();
    assert!(result.wake_count.is_none());
    let router = hoista_server::catalog::router(engine.clone());
    let response = router
        .oneshot(
            Request::builder()
                .uri("/resources/worker/database")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Database reader".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let result = hoista_server::mcp::dispatch(
        &engine,
        &actor,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "get_database", "arguments": {"id": "worker"}},
        }),
    )
    .await
    .unwrap();
    assert_eq!(result["result"]["isError"], false);
    provider.hold.store(true, Ordering::SeqCst);
    let task = tokio::spawn({
        let engine = engine.clone();
        async move {
            engine
                .projects()
                .database_observation(&actor, "worker")
                .await
        }
    });
    provider.entered.notified().await;
    engine
        .agents()
        .revoke_agent(&Actor::Person, &agent.id)
        .await
        .unwrap();
    provider.continue_write.notify_one();
    assert!(task.await.unwrap().is_err());
    let mut reads = vec![];
    for _ in 0..8 {
        reads.push(tokio::spawn({
            let engine = engine.clone();
            async move {
                engine
                    .projects()
                    .database_observation(&Actor::Person, "worker")
                    .await
            }
        }));
        provider.entered.notified().await;
    }
    assert!(matches!(
        engine
            .projects()
            .database_observation(&Actor::Person, "worker")
            .await,
        Err(EngineError::Conflict(_))
    ));
    let mut account = store.account("account").await.unwrap().unwrap();
    account.status = AccountStatus::Disconnected;
    store.put_account(&account).await.unwrap();
    provider.continue_write.notify_waiters();
    for read in reads {
        assert!(read.await.unwrap().is_err());
    }
    account.status = AccountStatus::Connected;
    store.put_account(&account).await.unwrap();
    provider.hold.store(false, Ordering::SeqCst);
    assert!(
        engine
            .projects()
            .database_observation(&Actor::Person, "worker")
            .await
            .is_ok()
    );
    let mut resource = store.resource("worker").await.unwrap().unwrap();
    resource.kind = ResourceKind::Web;
    store.put_resource(&resource).await.unwrap();
    assert!(
        engine
            .projects()
            .database_observation(&Actor::Person, "worker")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn shared_resources_keep_one_owner_and_block_external_disruption() {
    use hoista_server::domain::deletion::DeletionConfirmation;
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    for project in ["owner", "consumer", "indirect"] {
        store
            .put_project(&Project {
                id: project.into(),
                name: project.into(),
                source: Source::Unlinked,
                mode: Mode::Managed,
                created_at: "2026-11-02T00:00:00Z".into(),
                environment_id: "env_default".into(),
            })
            .await
            .unwrap();
        store
            .put_resource(&Resource {
                id: project.into(),
                project_id: Some(project.into()),
                kind: ResourceKind::Web,
                provider: Provider::Cloudflare,
                account_id: "account".into(),
                provider_ref: project.into(),
                name: project.into(),
                region: None,
                status: ResourceStatus::Live,
                mode: Mode::Managed,
                url: None,
                group: None,
            })
            .await
            .unwrap();
    }
    let original = engine
        .projects()
        .preview_deletion(&Actor::Person, GatedAction::DeleteProject, "owner")
        .await
        .unwrap();
    for (id, from, to) in [
        ("direct", "consumer", "owner"),
        ("transitive", "indirect", "consumer"),
    ] {
        engine
            .projects()
            .save_link(&Link {
                id: id.into(),
                from_resource: from.into(),
                to_resource: to.into(),
                via: "SHARED_RESOURCE".into(),
                derived: false,
                missing: true,
            })
            .await
            .unwrap();
    }
    assert!(
        engine
            .projects()
            .group(&Actor::Person, "consumer", &["owner".into()])
            .await
            .is_err()
    );
    assert_eq!(
        store
            .resource("owner")
            .await
            .unwrap()
            .unwrap()
            .project_id
            .as_deref(),
        Some("owner")
    );
    let detail = engine
        .projects()
        .resource_detail(&Actor::Person, "owner")
        .await
        .unwrap();
    assert_eq!(detail.used_by[0].project.as_ref().unwrap().id, "consumer");
    let (nodes, links) = engine
        .projects()
        .project_map_resources("owner")
        .await
        .unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(links.len(), 1);
    let projected = serde_json::to_value(
        engine
            .projects()
            .project_map(&Actor::Person, "owner")
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(projected["graph"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(projected["graph"]["edges"][0]["kind"], "dependency");
    assert_eq!(projected["graph"]["edges"][0]["origin"], "manual");
    assert_eq!(projected["graph"]["edges"][0]["missing"], true);
    assert!(
        projected["graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["type"] == "resource")
    );

    let preview = engine
        .projects()
        .preview_deletion(&Actor::Person, GatedAction::DeleteProject, "owner")
        .await
        .unwrap();
    assert_ne!(preview.fingerprint, original.fingerprint);
    assert!(!preview.blockers.is_empty());
    assert_eq!(
        preview
            .affected_resources
            .iter()
            .map(|resource| resource.project_id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        vec!["consumer", "indirect"]
    );
    assert!(
        engine
            .deploys()
            .delete_project(
                &Actor::Person,
                "owner",
                None,
                Some(&DeletionConfirmation {
                    backup_databases: false,
                    confirmation_name: original.confirmation_name,
                    fingerprint: original.fingerprint
                })
            )
            .await
            .is_err()
    );
    assert!(
        engine
            .projects()
            .delete_resource(&Actor::Person, "owner", None, None)
            .await
            .is_err()
    );
    assert!(
        engine
            .deploys()
            .control_resource(&Actor::Person, "owner", false)
            .await
            .is_err()
    );
    assert_eq!(
        store.resource("owner").await.unwrap().unwrap().status,
        ResourceStatus::Live
    );
    assert!(provider.deleted.lock().unwrap().is_empty());
    // Shared edges survive metadata backup with only one owner and no restored management.
    let backup = engine.data().export().await.unwrap();
    assert_eq!(backup.links.len(), 2);
    let restored_dir = tempfile::tempdir().unwrap();
    let restored_paths = StatePaths::new(Some(restored_dir.path().join("state"))).unwrap();
    let restored_store = SqliteStore::open(&restored_paths).await.unwrap();
    let restored = Engine::new(
        Arc::new(restored_store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![],
        Arc::new(Builder::default()),
    );
    restored.data().restore(backup).await.unwrap();
    assert_eq!(
        restored
            .projects()
            .resource_detail(&Actor::Person, "owner")
            .await
            .unwrap()
            .used_by[0]
            .project
            .as_ref()
            .unwrap()
            .id,
        "consumer"
    );
    assert_eq!(
        restored_store
            .resource("owner")
            .await
            .unwrap()
            .unwrap()
            .mode,
        Mode::Watching
    );
    // Delete the outer consumer, then its dependency project; shared owner is retained.
    for id in ["indirect", "consumer"] {
        let preview = engine
            .projects()
            .preview_deletion(&Actor::Person, GatedAction::DeleteProject, id)
            .await
            .unwrap();
        assert!(preview.blockers.is_empty());
        engine
            .deploys()
            .delete_project(
                &Actor::Person,
                id,
                None,
                Some(&DeletionConfirmation {
                    backup_databases: false,
                    confirmation_name: preview.confirmation_name,
                    fingerprint: preview.fingerprint,
                }),
            )
            .await
            .unwrap();
        assert!(store.resource("owner").await.unwrap().is_some());
    }
    assert_eq!(
        *provider.deleted.lock().unwrap(),
        vec!["indirect", "consumer"]
    );
    assert!(
        engine
            .projects()
            .preview_deletion(&Actor::Person, GatedAction::DeleteProject, "owner")
            .await
            .unwrap()
            .blockers
            .is_empty()
    );
    assert!(
        engine
            .deploys()
            .control_resource(&Actor::Person, "owner", false)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn monitor_persists_idle_activity_and_clears_stale_evidence_without_provider_writes() {
    async fn cycle(engine: &Arc<Engine>, store: &SqliteStore, expected: ResourceStatus) {
        let stop = tokio_util::sync::CancellationToken::new();
        let handle = tokio::spawn(engine.clone().run_monitor(stop.clone()));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if store.resource("web").await.unwrap().unwrap().status == expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        handle.await.unwrap().unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    ));
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "activity".into(),
            Source::Unlinked,
            Mode::Watching,
        )
        .await
        .unwrap();
    store
        .put_account(&Account {
            last_discovery: None,
            id: "a".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    let mut settings = store.settings().await.unwrap();
    settings.idle_days = 2;
    store.put_settings(&settings).await.unwrap();
    let today = chrono::Utc::now().date_naive();
    let row = |resource: &str, offset, requests| Usage {
        resource_id: resource.into(),
        day: (today - chrono::Duration::days(offset)).to_string(),
        requests,
        errors: None,
        compute_seconds: None,
        storage_bytes: None,
        egress_bytes: None,
    };
    for (id, status, kind) in [
        ("web", ResourceStatus::Live, ResourceKind::Web),
        ("paused", ResourceStatus::Paused, ResourceKind::Web),
        ("sleeping", ResourceStatus::Sleeping, ResourceKind::Service),
        ("failing", ResourceStatus::Failing, ResourceKind::Web),
        ("unknown", ResourceStatus::Unknown, ResourceKind::Web),
        ("job", ResourceStatus::Live, ResourceKind::Job),
    ] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: Some(project.id.clone()),
                kind,
                provider: Provider::Cloudflare,
                account_id: "a".into(),
                provider_ref: format!("worker:{id}"),
                name: id.into(),
                region: None,
                status,
                mode: Mode::Watching,
                url: None,
                group: None,
            })
            .await
            .unwrap();
        for offset in 0..=2 {
            store.put_usage(&row(id, offset, Some(0))).await.unwrap();
        }
    }
    cycle(&engine, &store, ResourceStatus::Idle).await;
    assert!(engine.projects().attention(Default::default()).await.unwrap().notices.iter().any(|n| matches!(n, hoista_server::application::engine::AttentionNotice::IdleResource { resource_id, .. } if resource_id == "web")));
    store.put_usage(&row("web", 0, Some(1))).await.unwrap();
    cycle(&engine, &store, ResourceStatus::Live).await;
    assert!(!engine.projects().attention(Default::default()).await.unwrap().notices.iter().any(|n| matches!(n, hoista_server::application::engine::AttentionNotice::IdleResource { resource_id, .. } if resource_id == "web")));
    store.put_usage(&row("web", 0, Some(0))).await.unwrap();
    cycle(&engine, &store, ResourceStatus::Idle).await;
    store.put_usage(&row("web", 1, None)).await.unwrap();
    cycle(&engine, &store, ResourceStatus::Unknown).await;
    for (id, status) in [
        ("paused", ResourceStatus::Paused),
        ("sleeping", ResourceStatus::Sleeping),
        ("failing", ResourceStatus::Failing),
        ("unknown", ResourceStatus::Unknown),
        ("job", ResourceStatus::Live),
    ] {
        assert_eq!(store.resource(id).await.unwrap().unwrap().status, status);
    }
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    assert_eq!(provider.rolled_back.load(Ordering::SeqCst), 0);
    assert!(provider.deleted.lock().unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn deployment_metadata_preserves_authority_and_rollback_attribution() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    let (mut agent, _) = engine
        .agents()
        .register_agent(
            "First agent".into(),
            vec!["apps:write".into(), "apps:create".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let first = engine
        .deploys()
        .enqueue_deploy_with_options(
            &actor,
            &project.id,
            &plan.id,
            DeployOptions {
                request_key: Some("550e8400-e29b-41d4-a716-446655440000".into()),
                expected_plan_revision: Some(plan.revision),
                title: Some("  Fix upload validation  ".into()),
                channel: Some(DeployChannel::Cli),
            },
        )
        .await
        .unwrap();
    assert_eq!(first.title.as_deref(), Some("Fix upload validation"));
    assert_eq!(first.channel, DeployChannel::Mcp);
    assert_eq!(first.started_by, agent.id);
    assert_eq!(first.started_by_name.as_deref(), Some("First agent"));
    let retry_options = DeployOptions {
        request_key: Some("550e8400-e29b-41d4-a716-446655440000".into()),
        expected_plan_revision: Some(plan.revision),
        title: Some("Fix upload validation".into()),
        channel: Some(DeployChannel::Cli),
    };
    let key = retry_options.request_key.as_deref().unwrap();
    assert!(
        engine
            .deploys()
            .deployment_request(&actor, key, None)
            .await
            .is_err(),
        "write-only agent cannot inspect receipts"
    );
    assert_eq!(
        engine
            .deploys()
            .deployment_request(&Actor::Person, key, Some(&agent.id))
            .await
            .unwrap()
            .deploy_id,
        first.id
    );
    agent.scopes.push("apps:read".into());
    store.put_agent(&agent).await.unwrap();
    assert!(
        engine
            .accounts()
            .environment_key_recovery(&actor)
            .await
            .is_err()
    );
    assert!(
        engine
            .accounts()
            .retry_environment_key_setup(&actor, &"0".repeat(64))
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .deploys()
            .deployment_request(&actor, key, None)
            .await
            .unwrap()
            .deploy_id,
        first.id
    );
    assert!(
        engine
            .deploys()
            .deployment_request(&actor, key, Some("person"))
            .await
            .is_err()
    );
    assert!(
        engine
            .deploys()
            .deployment_request(&Actor::Person, key, None)
            .await
            .is_err(),
        "owner namespace does not alias agent namespace"
    );
    let mut changed = retry_options.clone();
    changed.title = Some("Different operation".into());
    assert!(
        engine
            .deploys()
            .enqueue_deploy_with_options(&actor, &project.id, &plan.id, changed)
            .await
            .is_err()
    );
    let mut changed_plan = plan.clone();
    changed_plan.revision += 1;
    store.put_plan(&changed_plan).await.unwrap();
    assert_eq!(
        engine
            .deploys()
            .enqueue_deploy_with_options(&actor, &project.id, &plan.id, retry_options.clone())
            .await
            .unwrap()
            .id,
        first.id
    );
    let mut unkeyed = retry_options.clone();
    unkeyed.request_key = None;
    assert!(
        engine
            .deploys()
            .enqueue_deploy_with_options(&actor, &project.id, &plan.id, unkeyed)
            .await
            .is_err()
    );
    let mut no_revision = retry_options.clone();
    no_revision.expected_plan_revision = None;
    assert!(
        engine
            .deploys()
            .enqueue_deploy_with_options(&actor, &project.id, &plan.id, no_revision)
            .await
            .is_err()
    );
    assert!(
        engine
            .environments()
            .replace_environment(
                &actor,
                &project.id,
                &plan.id,
                vec![("PRIVATE".into(), Secret::new("private-value".into()))],
                retry_options.clone()
            )
            .await
            .is_err()
    );
    agent.name = "Renamed agent".into();
    store.put_agent(&agent).await.unwrap();
    assert_eq!(
        store
            .deploy(&first.id)
            .await
            .unwrap()
            .unwrap()
            .started_by_name
            .as_deref(),
        Some("First agent")
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let first = finished(&store, &first.id).await;
    assert_eq!(first.status, DeployStatus::Live);
    let rollback = engine
        .deploys()
        .enqueue_rollback_with_options(
            &Actor::Person,
            &first.id,
            DeployOptions {
                request_key: Some("550e8400-e29b-41d4-a716-446655440001".into()),
                expected_plan_revision: None,
                title: Some("Restore upload behavior".into()),
                channel: Some(DeployChannel::Desktop),
            },
        )
        .await
        .unwrap();
    assert_eq!(rollback.started_by, "person");
    assert_eq!(rollback.started_by_name, None);
    assert_eq!(rollback.channel, DeployChannel::Desktop);
    assert_eq!(rollback.title.as_deref(), Some("Restore upload behavior"));
    assert_eq!(rollback.provenance, first.provenance);
    let rollback = finished(&store, &rollback.id).await;
    assert_eq!(rollback.release_version, Some(2));
    let replay = engine
        .deploys()
        .enqueue_rollback_with_options(
            &Actor::Person,
            &first.id,
            DeployOptions {
                request_key: Some("550e8400-e29b-41d4-a716-446655440001".into()),
                expected_plan_revision: None,
                title: Some("Restore upload behavior".into()),
                channel: Some(DeployChannel::Desktop),
            },
        )
        .await
        .unwrap();
    assert_eq!(replay.id, rollback.id);
    assert_eq!(replay.status, rollback.status);
    assert!(
        engine
            .deploys()
            .enqueue_rollback_with_options(&actor, &first.id, retry_options.clone())
            .await
            .is_err()
    );
    for title in [
        "".to_owned(),
        "  ".into(),
        "a\nb".into(),
        "valid\n".into(),
        "x".repeat(161),
        format!("sk_live_{}", "a".repeat(24)),
    ] {
        assert!(
            engine
                .deploys()
                .enqueue_deploy_with_options(
                    &Actor::Person,
                    &project.id,
                    &plan.id,
                    DeployOptions {
                        request_key: None,
                        expected_plan_revision: None,
                        title: Some(title),
                        channel: Some(DeployChannel::Cli),
                    }
                )
                .await
                .is_err()
        );
    }
    assert!(
        engine
            .deploys()
            .enqueue_deploy_with_options(
                &Actor::Person,
                &project.id,
                &plan.id,
                DeployOptions {
                    request_key: None,
                    expected_plan_revision: None,
                    title: None,
                    channel: Some(DeployChannel::Mcp),
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .deploys()
            .enqueue_deploy_with_options(&actor, &project.id, &plan.id, retry_options.clone())
            .await
            .unwrap()
            .id,
        first.id
    );
    agent.revoked = true;
    store.put_agent(&agent).await.unwrap();
    assert!(
        engine
            .deploys()
            .deployment_request(&actor, retry_options.request_key.as_deref().unwrap(), None)
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .deploys()
            .deployment_request(
                &Actor::Person,
                retry_options.request_key.as_deref().unwrap(),
                Some(&agent.id)
            )
            .await
            .unwrap()
            .deploy_id,
        first.id
    );
    assert!(
        engine
            .deploys()
            .enqueue_deploy_with_options(&actor, &project.id, &plan.id, DeployOptions::default())
            .await
            .is_err()
    );
    assert!(
        engine
            .deploys()
            .enqueue_deploy_with_options(&actor, &project.id, &plan.id, retry_options)
            .await
            .is_err()
    );
    assert_eq!(
        store.deploys(&project.id, None).await.unwrap().items.len(),
        2
    );
    stop.cancel();
    worker.await.unwrap().unwrap();
    store.close().await;
}

#[tokio::test]
async fn deployment_metadata_is_exposed_through_http_and_agent_tools() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use serde_json::json;
    use tower::ServiceExt;
    let router = hoista_server::catalog::router(engine.clone());
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/deploys", project.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"plan_id":plan.id,"title":"Desktop change"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let id = response["id"].as_str().unwrap();
    let queued = store.deploy(id).await.unwrap().unwrap();
    assert_eq!(queued.channel, DeployChannel::Desktop);
    assert_eq!(queued.title.as_deref(), Some("Desktop change"));
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let first = finished(&store, id).await;
    stop.cancel();
    worker.await.unwrap().unwrap();
    for (payload, expected_channel, expected_title) in [
        (None, DeployChannel::Desktop, None),
        (
            Some(json!({"channel":"cli","title":"CLI restore"})),
            DeployChannel::Cli,
            Some("CLI restore"),
        ),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/deploys/{}/rollback", first.id));
        let body = if let Some(payload) = payload {
            request = request.header("content-type", "application/json");
            Body::from(payload.to_string())
        } else {
            Body::empty()
        };
        let response = router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let value: Deploy =
            serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
        assert_eq!(value.channel, expected_channel);
        assert_eq!(value.title.as_deref(), expected_title);
    }
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Tool caller".into(),
            vec!["apps:write".into(), "apps:create".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent { id: agent.id };
    for (name, args) in [
        (
            "deploy",
            json!({"project_id":project.id,"plan_id":plan.id,"title":"Agent change"}),
        ),
        ("rollback", json!({"id":first.id,"title":"Agent restore"})),
    ] {
        let result = hoista_server::mcp::dispatch(
            &engine,
            &actor,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": name, "arguments": args},
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert_eq!(result["result"]["structuredContent"]["channel"], "mcp");
        assert_eq!(
            result["result"]["structuredContent"]["title"],
            args["title"]
        );
        assert_eq!(
            result["result"]["structuredContent"]["started_by_name"],
            "Tool caller"
        );
    }
    store.close().await;
}

// Local builds run in isolation, which Windows does not have yet: there the build is refused
// before a snapshot is taken.
#[cfg(unix)]
#[tokio::test]
async fn failed_local_build_keeps_snapshot_identity_in_durable_history() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(hoista_server::infrastructure::build::LocalBuilder::new(
        paths.root(),
    ));
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("index.html"), "safe source").unwrap();
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: source.path().to_str().unwrap().into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    let queued = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let failed = finished(&store, &queued.id).await;
    assert_eq!(failed.status, DeployStatus::Failed);
    assert_eq!(failed.release_version, None);
    let provenance = failed
        .provenance
        .as_ref()
        .unwrap_or_else(|| panic!("completed snapshot retained after output failure: {failed:?}"));
    assert_eq!(provenance.snapshot_files, 1);
    assert_eq!(provenance.snapshot_sha256.len(), 64);
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    stop.cancel();
    worker.await.unwrap().unwrap();
    assert!(!paths.root().join("builds").join(&queued.id).exists());
    drop(engine);
    store.close().await;
    drop(store);
    let reopened = SqliteStore::open(&paths).await.unwrap();
    assert_eq!(
        reopened
            .deploy(&queued.id)
            .await
            .unwrap()
            .unwrap()
            .provenance,
        failed.provenance
    );
    reopened.close().await;
}

#[tokio::test]
async fn environment_metadata_tracks_public_inputs_and_protects_managed_bindings() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    builder.supplied.lock().unwrap().extend([
        (
            "VITE_PUBLIC_ANALYTICS_KEY".into(),
            "public-fixture-key".into(),
        ),
        ("SERVER_KEY".into(), "opaque-private-value".into()),
    ]);
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let queued = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .unwrap();
    let deployed = finished(&store, &queued.id).await;
    assert_eq!(deployed.status, DeployStatus::Live);
    stop.cancel();
    worker.await.unwrap().unwrap();
    let resource_id = &deployed.releases[0].resource_id;
    let rows = store.secrets(&project.id).await.unwrap();
    assert_eq!(
        rows.len(),
        2,
        "public build inputs survive omission from runtime secrets"
    );
    for row in rows {
        assert_eq!(row.set_by.as_deref(), Some("person"));
        let value = EnvironmentVariable::from(row);
        assert_eq!(value.browser_visible, value.name.starts_with("VITE_"));
        let json = serde_json::to_string(&value).unwrap();
        assert!(!json.contains("fingerprint"));
        assert!(!json.contains("opaque-private-value"));
        assert!(!json.contains("public-fixture-key"));
    }
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Environment editor".into(),
            vec!["apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let changed = engine
        .environments()
        .set_secret(
            &Actor::Agent {
                id: agent.id.clone(),
            },
            &project.id,
            resource_id,
            "SERVER_KEY",
            Secret::new("new-private-value".into()),
        )
        .await
        .unwrap();
    assert_eq!(changed.set_by.as_deref(), Some(agent.id.as_str()));
    assert_eq!(provider.secret_writes.load(Ordering::SeqCst), 1);
    assert!(
        engine
            .environments()
            .set_secret(
                &Actor::Person,
                &project.id,
                resource_id,
                "VITE_PRIVATE_SECRET",
                Secret::new("credential".into())
            )
            .await
            .is_err()
    );
    let mut managed = changed.clone();
    managed.name = "DATABASE_URL".into();
    managed.set_by = Some("hoista".into());
    managed.managed_by = Some(Provider::Neon);
    store.put_secret(&managed).await.unwrap();
    assert!(
        engine
            .environments()
            .set_secret(
                &Actor::Person,
                &project.id,
                resource_id,
                "DATABASE_URL",
                Secret::new("replacement".into())
            )
            .await
            .is_err()
    );
    assert_eq!(provider.secret_writes.load(Ordering::SeqCst), 1);
    assert!(
        engine
            .environments()
            .replace_environment(
                &Actor::Person,
                &project.id,
                &plan.id,
                vec![("DATABASE_URL".into(), Secret::new("replacement".into()))],
                DeployOptions::default()
            )
            .await
            .is_err()
    );
    // Older metadata still respects graph-derived binding protection.
    managed.managed_by = None;
    store.put_secret(&managed).await.unwrap();
    let mut dependency = store.resource(resource_id).await.unwrap().unwrap();
    dependency.id = "managed-dependency".into();
    dependency.name = "database".into();
    dependency.provider_ref = "database-fixture".into();
    dependency.kind = ResourceKind::Database;
    store.put_resource(&dependency).await.unwrap();
    store
        .put_link(&Link {
            id: "managed-link".into(),
            from_resource: resource_id.clone(),
            to_resource: dependency.id,
            via: "DATABASE_URL".into(),
            derived: true,
            missing: true,
        })
        .await
        .unwrap();
    assert!(
        engine
            .environments()
            .set_secret(
                &Actor::Person,
                &project.id,
                resource_id,
                "DATABASE_URL",
                Secret::new("replacement".into())
            )
            .await
            .is_err()
    );
    assert_eq!(provider.secret_writes.load(Ordering::SeqCst), 1);
    let mut legacy = serde_json::to_value(changed).unwrap();
    for field in ["set_by", "managed_by"] {
        legacy.as_object_mut().unwrap().remove(field);
    }
    let legacy: SecretMetadata = serde_json::from_value(legacy).unwrap();
    assert!(legacy.set_by.is_none());
    assert!(legacy.managed_by.is_none());
    store.close().await;
}

#[tokio::test]
async fn environment_replacement_deploys_new_inputs_and_blocks_stale_source_overwrites() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use serde_json::json;
    use tower::ServiceExt;
    builder.supplied.lock().unwrap().extend([
        ("VITE_ANALYTICS_KEY".into(), "old-public".into()),
        ("SERVER_KEY".into(), "old-private".into()),
    ]);
    let response = hoista_server::catalog::router(engine.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/projects/{}/environment/deploy", project.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "plan_id": plan.id,
                        "changes": [
                            {"name": "SERVER_KEY", "value": "new-private-fixture"},
                            {
                                "name": "VITE_ANALYTICS_KEY",
                                "value": "new-public-fixture",
                            },
                        ],
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = to_bytes(response.into_body(), 32768).await.unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("new-private-fixture"));
    let queued: Deploy = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        queued.environment_keys,
        vec!["SERVER_KEY", "VITE_ANALYTICS_KEY"]
    );
    assert_eq!(
        provider.released.load(Ordering::SeqCst),
        0,
        "admission does not write to providers"
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let success = finished(&store, &queued.id).await;
    assert_eq!(success.status, DeployStatus::Live);
    let built = builder.built.lock().unwrap()[0].clone();
    assert!(built.contains(&("SERVER_KEY".into(), "new-private-fixture".into())));
    assert!(built.contains(&("VITE_ANALYTICS_KEY".into(), "new-public-fixture".into())));
    assert!(
        store
            .secrets(&project.id)
            .await
            .unwrap()
            .iter()
            .all(|row| row.requires_source_sync)
    );
    let stale = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .unwrap();
    let failed = finished(&store, &stale.id).await;
    assert_eq!(failed.status, DeployStatus::Failed);
    assert!(failed.steps.iter().any(|step| {
        step.message
            .as_ref()
            .is_some_and(|message| message.contains("last explicit replacement"))
    }));
    assert_eq!(builder.built.lock().unwrap().len(), 1);
    *builder.supplied.lock().unwrap() = built;
    let synced = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .unwrap();
    assert_eq!(
        finished(&store, &synced.id).await.status,
        DeployStatus::Live
    );
    assert!(
        store
            .secrets(&project.id)
            .await
            .unwrap()
            .iter()
            .all(|row| !row.requires_source_sync)
    );
    stop.cancel();
    worker.await.unwrap().unwrap();
    let cancelled = engine
        .environments()
        .replace_environment(
            &Actor::Person,
            &project.id,
            &plan.id,
            vec![("SERVER_KEY".into(), Secret::new("cancelled-value".into()))],
            DeployOptions::default(),
        )
        .await
        .unwrap();
    engine
        .deploys()
        .cancel_deploy(&Actor::Person, &cancelled.id)
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    assert_eq!(
        finished(&store, &cancelled.id).await.status,
        DeployStatus::Failed
    );
    assert_eq!(builder.built.lock().unwrap().len(), 2);
    stop.cancel();
    worker.await.unwrap().unwrap();
    let rollback = engine
        .deploys()
        .enqueue_rollback(&Actor::Person, &success.id)
        .await
        .unwrap();
    assert!(
        rollback.environment_keys.is_empty(),
        "code rollback never requests prior secret values"
    );
    store.close().await;
}

#[tokio::test]
async fn environment_replacement_lost_on_restart_fails_without_using_old_values() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    builder
        .supplied
        .lock()
        .unwrap()
        .push(("SERVER_KEY".into(), "old-value".into()));
    let queued = engine
        .environments()
        .replace_environment(
            &Actor::Person,
            &project.id,
            &plan.id,
            vec![(
                "SERVER_KEY".into(),
                Secret::new("memory-only-fixture-value".into()),
            )],
            DeployOptions::default(),
        )
        .await
        .unwrap();
    let encoded = serde_json::to_string(&store.deploy(&queued.id).await.unwrap()).unwrap();
    assert!(!encoded.contains("memory-only-fixture-value"));
    drop(engine);
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let failed = finished(&store, &queued.id).await;
    assert_eq!(failed.status, DeployStatus::Failed);
    assert!(failed.steps.iter().any(|step| {
        step.message
            .as_ref()
            .is_some_and(|message| message.contains("Submit the replacement again"))
    }));
    assert!(builder.built.lock().unwrap().is_empty());
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    stop.cancel();
    worker.await.unwrap().unwrap();
    for values in [
        vec![],
        vec![("VITE_SECRET".into(), Secret::new("bad".into()))],
        vec![
            ("KEY".into(), Secret::new("a".into())),
            ("KEY".into(), Secret::new("b".into())),
        ],
    ] {
        assert!(
            engine
                .environments()
                .replace_environment(
                    &Actor::Person,
                    &project.id,
                    &plan.id,
                    values,
                    DeployOptions::default()
                )
                .await
                .is_err()
        );
    }
    assert_eq!(
        store.deploys(&project.id, None).await.unwrap().items.len(),
        1
    );
    store.close().await;
}

#[tokio::test]
async fn agent_environment_request_rechecks_revocation_before_consuming_values() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    let (mut agent, _) = engine
        .agents()
        .register_agent(
            "Environment agent".into(),
            vec!["apps:write".into(), "apps:create".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let response = hoista_server::mcp::dispatch(
        &engine,
        &actor,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "replace_environment",
                "arguments": {
                    "project_id": project.id,
                    "plan_id": plan.id,
                    "changes": [
                        {
                            "name": "SERVER_KEY",
                            "value": "private-agent-input",
                        },
                    ],
                },
            },
        }),
    )
    .await
    .unwrap();
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(!response.to_string().contains("private-agent-input"));
    let queued: Deploy =
        serde_json::from_value(response["result"]["structuredContent"].clone()).unwrap();
    assert_eq!(queued.channel, DeployChannel::Mcp);
    agent.revoked = true;
    store.put_agent(&agent).await.unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    assert_eq!(
        finished(&store, &queued.id).await.status,
        DeployStatus::Failed
    );
    assert!(builder.built.lock().unwrap().is_empty());
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    stop.cancel();
    worker.await.unwrap().unwrap();
    store.close().await;
}

#[tokio::test]
async fn connection_scans_inventory_and_keeps_connection_when_discovery_is_incomplete() {
    let directory = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(directory.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    );
    provider.partial_discovery.store(true, Ordering::SeqCst);
    let account = engine
        .accounts()
        .connect_account(
            Provider::Cloudflare,
            "Connected account".into(),
            "remote".into(),
            Secret::new("fixture-token".into()),
        )
        .await
        .unwrap();
    assert_eq!(account.status, AccountStatus::Connected);
    let summary = account.last_discovery.as_ref().unwrap();
    assert!(!summary.complete);
    assert_eq!(summary.observed_resources, Some(1));
    assert_eq!(summary.warnings[0].resource_type, "pages");
    let resources = engine.projects().all_resources().await.unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].mode, Mode::Watching);
    assert!(resources[0].project_id.is_none());
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "imported".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .unwrap();
    let mut managed = resources[0].clone();
    managed.project_id = Some(project.id);
    managed.mode = Mode::Managed;
    store.put_resource(&managed).await.unwrap();
    let again = engine
        .accounts()
        .connect_account(
            Provider::Cloudflare,
            "Reconnected".into(),
            "remote".into(),
            Secret::new("replacement-token".into()),
        )
        .await
        .unwrap();
    assert_eq!(again.id, account.id);
    assert_eq!(
        engine.projects().all_resources().await.unwrap()[0].id,
        managed.id
    );
    assert_eq!(
        engine.projects().all_resources().await.unwrap()[0].mode,
        Mode::Managed
    );
    provider.discovery_fails.store(true, Ordering::SeqCst);
    let failed = engine
        .accounts()
        .connect_account(
            Provider::Cloudflare,
            "Still connected".into(),
            "remote".into(),
            Secret::new("another-token".into()),
        )
        .await
        .unwrap();
    assert_eq!(failed.status, AccountStatus::Connected);
    let summary = failed.last_discovery.as_ref().unwrap();
    assert!(!summary.complete);
    assert_eq!(summary.observed_resources, None);
    assert_eq!(summary.warnings[0].code, "provider.unavailable");
    assert!(
        !serde_json::to_string(&failed)
            .unwrap()
            .contains("private-provider-response")
    );
    assert_eq!(engine.projects().all_resources().await.unwrap().len(), 1);
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    provider.discovery_fails.store(false, Ordering::SeqCst);
    provider.partial_discovery.store(false, Ordering::SeqCst);
    let report = engine
        .accounts()
        .discover(&Actor::Person, &account.id)
        .await
        .unwrap();
    assert!(report.complete);
    assert!(report.items.is_empty());
    let saved = store.account(&account.id).await.unwrap().unwrap();
    assert_eq!(saved.last_discovery.unwrap().observed_resources, Some(0));
    assert_eq!(
        engine.projects().all_resources().await.unwrap().len(),
        1,
        "an empty inventory does not silently delete history"
    );
    assert_eq!(provider.discovery_calls.load(Ordering::SeqCst), 4);
    provider.verification.store(1, Ordering::SeqCst);
    assert!(
        engine
            .accounts()
            .connect_account(
                Provider::Cloudflare,
                "Rejected".into(),
                "remote".into(),
                Secret::new("expired-token".into())
            )
            .await
            .is_err()
    );
    assert_eq!(provider.discovery_calls.load(Ordering::SeqCst), 4);
    provider.verification.store(0, Ordering::SeqCst);
    let fault = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(paths.database()),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_discovery_summary BEFORE UPDATE ON accounts WHEN json_type(NEW.data, '$.last_discovery') = 'object' BEGIN SELECT RAISE(ABORT, 'fixture summary failure'); END")
        .execute(&fault)
        .await
        .unwrap();
    assert!(
        engine
            .accounts()
            .connect_account(
                Provider::Cloudflare,
                "Saved but scan status unavailable".into(),
                "remote".into(),
                Secret::new("fixture-token".into())
            )
            .await
            .is_err()
    );
    let saved = store.account(&account.id).await.unwrap().unwrap();
    assert_eq!(saved.status, AccountStatus::Connected);
    assert!(saved.last_discovery.is_none());
    sqlx::query("DROP TRIGGER reject_discovery_summary")
        .execute(&fault)
        .await
        .unwrap();
    fault.close().await;
    let mut legacy = serde_json::to_value(account).unwrap();
    legacy.as_object_mut().unwrap().remove("last_discovery");
    assert!(
        serde_json::from_value::<Account>(legacy)
            .unwrap()
            .last_discovery
            .is_none()
    );
    store.close().await;
}

#[tokio::test]
async fn account_diagnostics_redact_errors_preserve_state_and_recheck_context() {
    use hoista_server::application::engine::AccountDiagnosticStatus as Status;
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    ));
    let account = Account {
        last_discovery: None,
        id: "account".into(),
        provider: Provider::Cloudflare,
        label: "fixture".into(),
        external_id: "remote".into(),
        scopes: vec!["old".into()],
        status: AccountStatus::Connected,
        connected_at: "2026-11-02T00:00:00Z".into(),
        credential: None,
        in_1password: false,
    };
    store.put_account(&account).await.unwrap();
    for (code, expected) in [
        (0, Status::Verified),
        (1, Status::Expired),
        (2, Status::MissingScopes),
        (3, Status::Unavailable),
        (4, Status::Unavailable),
    ] {
        provider.verification.store(code, Ordering::SeqCst);
        let page = engine
            .observe()
            .diagnose_accounts(&Actor::Person, None)
            .await
            .unwrap();
        assert_eq!(page.items[0].status, expected);
        assert!(page.items[0].checked_at.is_some());
        assert_eq!(page.items[0].verified_scopes.is_empty(), code != 0);
        let serialized = serde_json::to_string(&page).unwrap();
        assert!(
            !serialized.contains("fixture-token")
                && !serialized.contains("Fixture verification failure")
        );
        assert_eq!(store.account("account").await.unwrap().unwrap(), account);
    }
    provider.verification.store(0, Ordering::SeqCst);
    provider.verification_wait.store(true, Ordering::SeqCst);
    let running_engine = engine.clone();
    let task = tokio::spawn(async move {
        running_engine
            .observe()
            .diagnose_accounts(&Actor::Person, None)
            .await
    });
    provider.verification_entered.notified().await;
    assert!(matches!(
        engine
            .observe()
            .diagnose_accounts(&Actor::Person, None)
            .await,
        Err(EngineError::Conflict(_))
    ));
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        engine.accounts().disconnect_account("account"),
    )
    .await
    .unwrap()
    .unwrap();
    provider.verification_continue.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap().items[0].status,
        Status::NotChecked
    );
    assert_eq!(
        engine
            .observe()
            .diagnose_accounts(&Actor::Person, None)
            .await
            .unwrap()
            .items[0]
            .status,
        Status::Disconnected
    );
    store.put_account(&account).await.unwrap();
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Diagnostic reader".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .unwrap();
    let actor = Actor::Agent {
        id: agent.id.clone(),
    };
    let running_engine = engine.clone();
    let task = tokio::spawn(async move {
        running_engine
            .observe()
            .diagnose_accounts(&actor, None)
            .await
    });
    provider.verification_entered.notified().await;
    engine
        .agents()
        .revoke_agent(&Actor::Person, &agent.id)
        .await
        .unwrap();
    provider.verification_continue.notify_one();
    assert!(task.await.unwrap().is_err());
    let timed = tokio::time::timeout(
        std::time::Duration::from_secs(12),
        engine.observe().diagnose_accounts(&Actor::Person, None),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(timed.items[0].status, Status::TimedOut);
    assert!(timed.items[0].verified_scopes.is_empty());
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    store.close().await;
}

#[tokio::test]
async fn graceful_restart_drains_active_and_queued_memory_only_environment() {
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            last_discovery: None,
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    let mut deployments = Vec::new();
    for name in ["one", "two"] {
        let project = engine
            .projects()
            .create_project(
                &Actor::Person,
                name.into(),
                Source::Folder {
                    path: "/fixture".into(),
                },
                Mode::Managed,
            )
            .await
            .unwrap();
        let plan = engine
            .deploys()
            .plan(&Actor::Person, project.source.clone(), name, None, None)
            .await
            .unwrap();
        deployments.push(
            engine
                .environments()
                .replace_environment(
                    &Actor::Person,
                    &project.id,
                    &plan.id,
                    vec![(
                        "SERVER_KEY".into(),
                        Secret::new(format!("memory-only-{name}")),
                    )],
                    DeployOptions::default(),
                )
                .await
                .unwrap(),
        );
    }
    provider.hold.store(true, Ordering::SeqCst);
    let stop = tokio_util::sync::CancellationToken::new();
    let drain = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(
        engine
            .clone()
            .run_deployments_until_idle(stop.clone(), drain.clone()),
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .unwrap();
    drain.cancel();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!worker.is_finished());
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    provider.hold.store(false, Ordering::SeqCst);
    provider.continue_write.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!stop.is_cancelled());
    assert_eq!(provider.released.load(Ordering::SeqCst), 2);
    for deploy in deployments {
        let saved = store.deploy(&deploy.id).await.unwrap().unwrap();
        assert_eq!(saved.status, DeployStatus::Live);
        assert!(!saved.cancel_requested);
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("memory-only-")
        );
    }
    {
        let built = builder.built.lock().unwrap();
        for name in ["one", "two"] {
            assert!(built.iter().any(|values| {
                values.contains(&("SERVER_KEY".into(), format!("memory-only-{name}")))
            }));
        }
    }
    store.close().await;
}

#[tokio::test]
async fn graceful_restart_does_not_drop_an_accepted_owner_provider_request() {
    use hoista_server::{client::LocalClient, lifecycle::DaemonLifecycle, server};
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let provider = Arc::new(TestProvider::default());
    provider.verification_wait.store(true, Ordering::SeqCst);
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    ));
    let stop = tokio_util::sync::CancellationToken::new();
    let lifecycle = Arc::new(DaemonLifecycle::new(stop.clone(), false));
    let serving = {
        let paths = paths.clone();
        let store = store.clone();
        let stop = stop.clone();
        let lifecycle = lifecycle.clone();
        tokio::spawn(async move {
            server::serve_with_lifecycle(&paths, store, stop, engine, Some(lifecycle)).await
        })
    };
    let client = LocalClient::new(&paths).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while client.health().await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let connecting = {
        let client = LocalClient::new(&paths).unwrap();
        tokio::spawn(async move {
            client
                .post_json(
                    "/accounts",
                    &serde_json::json!({
                        "provider": "cloudflare",
                        "label": "Fixture",
                        "external_id": "remote",
                        "token": "test-only-token",
                    }),
                )
                .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.verification_entered.notified(),
    )
    .await
    .unwrap();
    let accepted = client
        .post_json(
            "/daemon/restart",
            &serde_json::json!({"instance_id":lifecycle.status().instance_id}),
        )
        .await
        .unwrap();
    assert_eq!(accepted["restart_requested"], true);
    // Exceed the signal-shutdown listener deadline; restart must still own this write.
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert!(!serving.is_finished());
    assert!(!connecting.is_finished());
    provider.verification_wait.store(false, Ordering::SeqCst);
    provider.verification_continue.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), connecting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result["status"], "connected");
    tokio::time::timeout(std::time::Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(store.accounts(None).await.unwrap().items.len(), 1);
    store.close().await;
}

#[tokio::test]
async fn detection_failure_retains_early_git_observation_without_claiming_a_snapshot() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(hoista_server::infrastructure::build::LocalBuilder::new(
        paths.root(),
    ));
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("index.html"), "safe source").unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(source.path())
                .status()
                .unwrap()
                .success()
        );
    }
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(hoista_server::infrastructure::inspection::LocalInspector),
        vec![provider.clone()],
        builder.clone(),
    ));
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: source.path().to_str().unwrap().into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");

    let queued = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .unwrap();
    std::fs::write(
        source.path().join("package.json"),
        "invalid-json-private-marker",
    )
    .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    let failed = finished(&store, &queued.id).await;
    assert_eq!(failed.status, DeployStatus::Failed);
    assert_eq!(failed.release_version, None);
    assert!(failed.provenance.is_none());
    let observed = failed.source_observation.as_ref().unwrap();
    assert!(
        matches!(observed, hoista_server::domain::provenance::SourceObservation::LocalGit { commit: Some(commit), dirty: true } if commit.len() == 40 || commit.len() == 64)
    );
    let encoded = serde_json::to_string(&failed).unwrap();
    assert!(!encoded.contains("invalid-json-private-marker"));
    assert!(
        failed
            .failure
            .as_ref()
            .unwrap()
            .message
            .contains("package.json")
    );
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    // Broken Git metadata must not hide the primary manifest failure.
    std::fs::remove_dir_all(source.path().join(".git")).unwrap();
    std::fs::write(source.path().join(".git"), "invalid git metadata").unwrap();
    let retry = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .unwrap();
    let second = finished(&store, &retry.id).await;
    assert_eq!(
        second.source_observation,
        Some(hoista_server::domain::provenance::SourceObservation::Unavailable)
    );
    assert!(second.provenance.is_none());
    assert_eq!(
        second.failure.as_ref().unwrap().message,
        failed.failure.as_ref().unwrap().message
    );
    assert_eq!(provider.released.load(Ordering::SeqCst), 0);
    stop.cancel();
    worker.await.unwrap().unwrap();
    assert!(!paths.root().join("builds").join(&queued.id).exists());
    drop(engine);
    store.close().await;
    drop(store);
    let reopened = SqliteStore::open(&paths).await.unwrap();
    assert_eq!(
        reopened
            .deploy(&queued.id)
            .await
            .unwrap()
            .unwrap()
            .source_observation,
        failed.source_observation
    );
    reopened.close().await;
}

#[tokio::test]
async fn project_totals_count_status_and_providers() {
    let dir = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(dir.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![],
        Arc::new(Builder::default()),
    );
    let empty = engine.accounts().project_totals().await.unwrap();
    assert_eq!(
        (empty.projects, empty.live, empty.failing, empty.providers),
        (0, 0, 0, 0)
    );
    for (provider, account) in [(Provider::Cloudflare, "cf"), (Provider::Fly, "fly")] {
        store
            .put_account(&Account {
                last_discovery: None,
                id: account.into(),
                provider,
                label: account.into(),
                external_id: account.into(),
                scopes: vec![],
                status: AccountStatus::Connected,
                connected_at: "2026-11-02T00:00:00Z".into(),
                credential: None,
                in_1password: false,
            })
            .await
            .unwrap();
    }
    for (project, account, provider, status) in [
        ("site", "cf", Provider::Cloudflare, ResourceStatus::Live),
        ("api", "fly", Provider::Fly, ResourceStatus::Failing),
    ] {
        store
            .put_project(&Project {
                id: project.into(),
                name: project.into(),
                source: Source::Unlinked,
                mode: Mode::Managed,
                created_at: "2026-11-02T00:00:00Z".into(),
                environment_id: "env_default".into(),
            })
            .await
            .unwrap();
        store
            .put_resource(&Resource {
                id: project.into(),
                project_id: Some(project.into()),
                kind: ResourceKind::Web,
                provider,
                account_id: account.into(),
                provider_ref: project.into(),
                name: project.into(),
                region: None,
                status,
                mode: Mode::Managed,
                url: None,
                group: None,
            })
            .await
            .unwrap();
    }
    let totals = engine.accounts().project_totals().await.unwrap();
    assert_eq!(
        (
            totals.projects,
            totals.live,
            totals.failing,
            totals.providers
        ),
        (2, 1, 1, 2)
    );
}

#[tokio::test]
async fn watched_projects_merge_into_one_without_touching_the_provider() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault),
        Arc::new(Inspector),
        vec![Arc::new(TestProvider::default())],
        Arc::new(Builder::default()),
    );
    let source = Source::Folder {
        path: "/fixture".into(),
    };
    let api = engine
        .projects()
        .create_project(&Actor::Person, "api".into(), source.clone(), Mode::Watching)
        .await
        .unwrap();
    let db = engine
        .projects()
        .create_project(
            &Actor::Person,
            "postgres".into(),
            source.clone(),
            Mode::Watching,
        )
        .await
        .unwrap();
    let managed = engine
        .projects()
        .create_project(&Actor::Person, "site".into(), source, Mode::Managed)
        .await
        .unwrap();
    store
        .put_account(&Account {
            last_discovery: None,
            id: "rw".into(),
            provider: Provider::Railway,
            label: "fixture".into(),
            external_id: "workspace".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    for (id, project, kind) in [
        ("svc", &api.id, ResourceKind::Service),
        ("pg", &db.id, ResourceKind::Database),
    ] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: Some(project.clone()),
                kind,
                provider: Provider::Railway,
                account_id: "rw".into(),
                provider_ref: format!("project/{id}"),
                name: id.into(),
                region: None,
                status: ResourceStatus::Live,
                mode: Mode::Watching,
                url: None,
                group: None,
            })
            .await
            .unwrap();
    }
    assert!(
        engine
            .projects()
            .merge_projects(&Actor::Person, &api.id, &api.id)
            .await
            .is_err()
    );
    assert!(
        engine
            .projects()
            .merge_projects(&Actor::Person, &db.id, &managed.id)
            .await
            .is_err(),
        "managed projects keep their own resources"
    );
    let merged = engine
        .projects()
        .merge_projects(&Actor::Person, &db.id, &api.id)
        .await
        .expect("merge");
    assert_eq!(merged.id, api.id);
    assert_eq!(
        store
            .resource("pg")
            .await
            .unwrap()
            .unwrap()
            .project_id
            .as_deref(),
        Some(api.id.as_str())
    );
    assert_eq!(store.project(&db.id).await.unwrap(), None);
    // Renaming changes only Hoista's name and refuses a taken one.
    assert!(
        engine
            .projects()
            .rename_project(&Actor::Person, &api.id, "site")
            .await
            .is_err()
    );
    assert!(
        engine
            .projects()
            .rename_project(&Actor::Person, &api.id, "Bad Name")
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .projects()
            .rename_project(&Actor::Person, &api.id, "backend")
            .await
            .unwrap()
            .name,
        "backend"
    );
    // Splitting moves one resource into a new watched project; the old project keeps the rest.
    let alone = engine
        .projects()
        .split_resource(&Actor::Person, "svc", "service")
        .await
        .expect("split");
    assert_eq!(alone.mode, Mode::Watching);
    assert_eq!(
        store
            .resource("svc")
            .await
            .unwrap()
            .unwrap()
            .project_id
            .as_deref(),
        Some(alone.id.as_str())
    );
    assert!(store.project(&api.id).await.unwrap().is_some());
    // Merging back removes the emptied project.
    engine
        .projects()
        .merge_projects(&Actor::Person, &api.id, &alone.id)
        .await
        .expect("merge back");
    assert_eq!(store.project(&api.id).await.unwrap(), None);
    assert!(
        engine
            .projects()
            .stop_watching_resource(&Actor::Agent { id: "agent".into() }, "pg")
            .await
            .is_err()
    );
    engine
        .projects()
        .stop_watching_resource(&Actor::Person, "pg")
        .await
        .expect("take one out");
    assert_eq!(
        store.resource("pg").await.unwrap().unwrap().project_id,
        None
    );
    assert!(store.project(&alone.id).await.unwrap().is_some());
    assert!(
        engine
            .projects()
            .stop_watching_resource(&Actor::Person, "pg")
            .await
            .is_err()
    );
    // A provider project with only a database becomes a project like any other.
    let data = engine
        .projects()
        .import_project(
            &Actor::Person,
            "just-a-database".into(),
            None,
            Mode::Watching,
            &["pg".into()],
        )
        .await
        .expect("a database-only project");
    assert_eq!(data.resources.len(), 1);
    // Stopping watching forgets the project; its resource goes back to found.
    assert!(
        engine
            .projects()
            .stop_watching(&Actor::Agent { id: "agent".into() }, &alone.id)
            .await
            .is_err()
    );
    engine
        .projects()
        .stop_watching(&Actor::Person, &alone.id)
        .await
        .expect("stop watching");
    assert_eq!(store.project(&alone.id).await.unwrap(), None);
    assert_eq!(
        store.resource("svc").await.unwrap().unwrap().project_id,
        None
    );
    store.close().await;
}

/// A writable in-memory keychain, for flows that store and renew credentials.
#[derive(Default)]
struct MemoryVault(std::sync::Mutex<std::collections::HashMap<String, String>>);
#[async_trait::async_trait]
impl TokenVault for MemoryVault {
    async fn read(&self, reference: &str) -> Result<Secret, EngineError> {
        self.0
            .lock()
            .expect("test step")
            .get(reference)
            .map(|value| Secret::new(value.clone()))
            .ok_or(EngineError::NotFound)
    }

    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError> {
        self.0
            .lock()
            .expect("test step")
            .insert(reference.into(), value.expose().into());
        Ok(())
    }

    async fn delete(&self, reference: &str) -> Result<(), EngineError> {
        self.0.lock().expect("test step").remove(reference);
        Ok(())
    }
}

/// A provider with a one-click connection: code exchange, then rotating renewals.
#[derive(Default)]
struct SignInProvider {
    renewals: AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderAdapter for SignInProvider {
    fn provider(&self) -> Provider {
        Provider::Railway
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            deploy: false,
            logs: false,
            metrics: false,
            usage: false,
            rollback: false,
            pause: false,
            domains: false,
            secrets: false,
            discover: true,
            server: false,
            ..Capabilities::default()
        }
    }

    fn oauth(&self) -> Option<hoista_server::application::ports::providers::ProviderOAuth> {
        Some(
            hoista_server::application::ports::providers::ProviderOAuth {
                authorize_url: "https://backboard.railway.com/oauth/auth",
                client_id: "client",
                scopes: "openid offline_access",
                ports: &[47_191, 47_192],
                extra: &[("prompt", "consent")],
                redirect_host: "127.0.0.1",
                logo: "<svg></svg>",
            },
        )
    }

    async fn oauth_token(
        &self,
        grant: hoista_server::application::ports::providers::OAuthGrant<'_>,
    ) -> Result<hoista_server::application::ports::providers::OAuthTokens, EngineError> {
        use hoista_server::application::ports::providers::{OAuthGrant, OAuthTokens};
        let (access, refresh) = match grant {
            OAuthGrant::Code {
                code, redirect_uri, ..
            } => {
                assert_eq!(code, "the-code");
                assert!(redirect_uri.starts_with("http://127.0.0.1:4719"));
                ("access-1".to_owned(), "refresh-1".to_owned())
            }
            OAuthGrant::Refresh(refresh) => {
                let round = self.renewals.fetch_add(1, Ordering::SeqCst) + 1;
                assert_eq!(
                    refresh,
                    format!("refresh-{round}"),
                    "the newest refresh token is used"
                );
                (
                    format!("access-{}", round + 1),
                    format!("refresh-{}", round + 1),
                )
            }
        };
        Ok(OAuthTokens {
            access: Secret::new(access),
            refresh: Some(Secret::new(refresh)),
            // Shorter than the renewal margin, so every read renews.
            expires_in: 60,
            granted: vec![],
        })
    }

    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        assert_eq!(token.expose(), "access-1");
        Ok(vec![
            AccountChoice {
                external_id: "workspace".into(),
                name: "My workspace".into(),
            },
            AccountChoice {
                external_id: "team".into(),
                name: "Team workspace".into(),
            },
        ])
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec!["workspace:member".into()])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn signing_in_on_the_provider_page_connects_and_renews_without_a_secret() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let vault = Arc::new(MemoryVault::default());
    let provider = Arc::new(SignInProvider::default());
    let engine = Engine::new(
        Arc::new(store.clone()),
        vault.clone(),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    );
    // Agents can't start a sign-in; only the person, in the project.
    assert!(
        engine
            .accounts()
            .start_sign_in(
                &Actor::Agent { id: "agent".into() },
                Provider::Railway,
                false
            )
            .await
            .is_err()
    );
    // Opening the page again replaces the sign-in still waiting.
    let replaced = engine
        .accounts()
        .start_sign_in(&Actor::Person, Provider::Railway, false)
        .await
        .expect("start");
    let started = engine
        .accounts()
        .start_sign_in(&Actor::Person, Provider::Railway, false)
        .await
        .expect("start again");
    assert!(
        engine
            .accounts()
            .finish_sign_in(&Actor::Person, &replaced.id)
            .await
            .is_err()
    );
    let url = url::Url::parse(&started.url).expect("url");
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["prompt"], "consent");
    assert!(!query.contains_key("client_secret"));
    let redirect = query["redirect_uri"].clone();
    // A stale state is ignored; the browser then arrives with the right one.
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("test step");
    let stale = client
        .get(format!("{redirect}?code=x&state=wrong"))
        .send()
        .await
        .expect("test step")
        .text()
        .await
        .expect("test step");
    assert!(stale.contains("out of date"));
    let page = client
        .get(format!("{redirect}?code=the-code&state={}", query["state"]))
        .send()
        .await
        .expect("test step")
        .text()
        .await
        .expect("test step");
    assert!(page.contains("is connected"));
    let account = engine
        .accounts()
        .finish_sign_in(&Actor::Person, &started.id)
        .await
        .expect("finish");
    assert_eq!(account.label, "My workspace");
    assert_eq!(account.external_id, "workspace");
    // The keychain holds the tokens, not a bare token; each read renews here.
    let first = engine
        .accounts()
        .account_token(&account.id)
        .await
        .expect("token");
    assert_eq!(first.expose(), "access-2");
    let second = engine
        .accounts()
        .account_token(&account.id)
        .await
        .expect("token");
    assert_eq!(second.expose(), "access-3");
    assert_eq!(provider.renewals.load(Ordering::SeqCst), 2);
    // Every shared workspace is connected, on the same sign-in: renewing
    // through one moves both on, since a refresh token works only once.
    let accounts = engine.accounts().all_accounts().await.expect("accounts");
    let team = accounts
        .iter()
        .find(|other| other.external_id == "team")
        .expect("second workspace connected");
    assert_eq!(team.label, "Team workspace");
    assert_eq!(team.credential.as_deref(), Some(account.id.as_str()));
    let third = engine
        .accounts()
        .account_token(&team.id)
        .await
        .expect("token");
    assert_eq!(third.expose(), "access-4");
    // Removing one workspace keeps the sign-in for the other.
    engine
        .accounts()
        .disconnect_account(&team.id)
        .await
        .expect("disconnect");
    let fourth = engine
        .accounts()
        .account_token(&account.id)
        .await
        .expect("token");
    assert_eq!(fourth.expose(), "access-5");
    // A finished sign-in can't be finished twice.
    assert!(
        engine
            .accounts()
            .finish_sign_in(&Actor::Person, &started.id)
            .await
            .is_err()
    );
    // On a server the desktop app relays: no listener here, the app forwards the answer.
    let relayed = engine
        .accounts()
        .start_sign_in(&Actor::Person, Provider::Railway, true)
        .await
        .expect("start relayed");
    let port = relayed.relay_port.expect("relay port");
    let url = url::Url::parse(&relayed.url).expect("url");
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert!(query["redirect_uri"].ends_with(&format!(":{port}/callback")));
    assert!(
        engine
            .accounts()
            .relay_callback(&Actor::Person, &relayed.id, "wrong", Some("the-code"))
            .is_err()
    );
    assert!(
        engine
            .accounts()
            .relay_callback(
                &Actor::Agent { id: "agent".into() },
                &relayed.id,
                &query["state"],
                Some("the-code")
            )
            .is_err()
    );
    engine
        .accounts()
        .relay_callback(
            &Actor::Person,
            &relayed.id,
            &query["state"],
            Some("the-code"),
        )
        .expect("relay");
    let again = engine
        .accounts()
        .finish_sign_in(&Actor::Person, &relayed.id)
        .await
        .expect("finish relayed");
    assert_eq!(again.external_id, "workspace");
    store.close().await;
}

#[test]
fn removing_things_at_a_provider_needs_the_persons_approval() {
    use hoista_server::application::ports::providers::ProviderAction;
    for action in [
        ProviderAction::RemoveService,
        ProviderAction::RemoveVolume { volume_id: "v1" },
        ProviderAction::RemoveTcpProxy { proxy_id: "x1" },
        ProviderAction::RemoveDomain {
            domain: "api.example.com",
        },
    ] {
        assert!(
            Engine::provider_action_gate("resource", &action).is_some(),
            "{action:?} needs approval"
        );
    }
    for action in [
        ProviderAction::Redeploy,
        ProviderAction::AddVolume {
            mount_path: "/data",
        },
        ProviderAction::AddTcpProxy { port: 5432 },
    ] {
        assert!(Engine::provider_action_gate("resource", &action).is_none());
    }
}

/// A provider with a two-operation catalog: one read, one destructive change.
#[derive(Default)]
struct OpsProvider {
    ran: AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderAdapter for OpsProvider {
    fn provider(&self) -> Provider {
        Provider::Railway
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            deploy: false,
            logs: false,
            metrics: false,
            usage: false,
            rollback: false,
            pause: false,
            domains: false,
            secrets: false,
            discover: true,
            server: false,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }

    fn provider_operations(
        &self,
    ) -> Vec<hoista_server::application::ports::providers::ProviderOperationInfo> {
        use hoista_server::application::ports::providers::{ProviderOperationInfo, ProviderRisk};
        [
            ("backupList", ProviderRisk::Read),
            ("backupRestore", ProviderRisk::Destroy),
        ]
        .into_iter()
        .map(|(name, risk)| ProviderOperationInfo {
            name: name.into(),
            area: "Storage".into(),
            risk,
            request: "Query".into(),
            description: String::new(),
            args: vec![],
        })
        .collect()
    }

    async fn provider_operation(
        &self,
        _: &Secret,
        _: &str,
        reference: Option<&str>,
        _: Option<&std::path::Path>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        self.ran.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"ran": name, "on": reference, "args": args}))
    }
}

#[tokio::test]
async fn provider_operations_read_freely_but_destroy_only_with_the_persons_approval() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let vault = Arc::new(MemoryVault::default());
    let provider = Arc::new(OpsProvider::default());
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            vault.clone(),
            Arc::new(Inspector),
            vec![provider.clone()],
            Arc::new(Builder::default()),
        )
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
        )),
    );
    let account = engine
        .accounts()
        .connect_account(
            Provider::Railway,
            "Workspace".into(),
            "ws".into(),
            Secret::new("tok".into()),
        )
        .await
        .expect("account");
    store
        .put_resource(&Resource {
            id: "svc".into(),
            project_id: None,
            kind: ResourceKind::Service,
            provider: Provider::Railway,
            account_id: account.id.clone(),
            provider_ref: "p1/s1".into(),
            name: "api".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Watching,
            url: None,
            group: None,
        })
        .await
        .expect("resource");
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Ops".into(),
            vec!["apps:read".into(), "apps:write".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("agent");
    let actor = Actor::Agent { id: agent.id };
    let call = |name: &str, arguments: serde_json::Value| {
        let engine = engine.clone();
        let actor = actor.clone();
        let name = name.to_owned();
        async move {
            hoista_server::mcp::dispatch(
                &engine,
                &actor,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {"name": name, "arguments": arguments},
                }),
            )
            .await
            .expect("reply")["result"]
                .clone()
        }
    };
    let listed = call(
        "list_provider_operations",
        serde_json::json!({"resource_id":"svc"}),
    )
    .await;
    assert_eq!(
        listed["structuredContent"]["items"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let read = call(
        "run_provider_operation",
        serde_json::json!({"resource_id":"svc","name":"backupList"}),
    )
    .await;
    assert_eq!(read["structuredContent"]["result"]["on"], "p1/s1");
    // The destructive one waits for the person, and nothing runs meanwhile.
    let args = serde_json::json!({"backupId":"b1"});
    let pending = call(
        "run_provider_operation",
        serde_json::json!({"resource_id":"svc","name":"backupRestore","args":args}),
    )
    .await;
    assert_eq!(pending["structuredContent"]["status"], "pending");
    let approval = pending["structuredContent"]["approval_id"]
        .as_str()
        .expect("approval")
        .to_owned();
    assert_eq!(provider.ran.load(Ordering::SeqCst), 1);
    let early = call("run_provider_operation", serde_json::json!({"resource_id":"svc","name":"backupRestore","args":args,"approval_id":approval})).await;
    // Retrying before the person answers says it's still waiting, and runs nothing.
    assert_eq!(early["structuredContent"]["status"], "pending");
    assert_eq!(early["structuredContent"]["approval_id"], approval.as_str());
    engine
        .projects()
        .decide_approval(&approval, true, None)
        .await
        .expect("allow");
    // The approval covers these exact arguments only.
    let other = call(
        "run_provider_operation",
        serde_json::json!({
            "resource_id": "svc",
            "name": "backupRestore",
            "args": {"backupId": "b2"},
            "approval_id": approval,
        }),
    )
    .await;
    // Other arguments: nothing runs, and the person is asked about those instead.
    assert_eq!(other["structuredContent"]["status"], "pending");
    assert_ne!(other["structuredContent"]["approval_id"], approval.as_str());
    let done = call("run_provider_operation", serde_json::json!({"resource_id":"svc","name":"backupRestore","args":args,"approval_id":approval})).await;
    assert_eq!(done["structuredContent"]["result"]["ran"], "backupRestore");
    // Once only.
    let again = call("run_provider_operation", serde_json::json!({"resource_id":"svc","name":"backupRestore","args":args,"approval_id":approval})).await;
    // Once only: a used approval means asking again, not running again.
    assert_eq!(again["structuredContent"]["status"], "pending");
    assert_ne!(again["structuredContent"]["approval_id"], approval.as_str());
    assert_eq!(provider.ran.load(Ordering::SeqCst), 2);
    store.close().await;
}

#[derive(Default)]
struct ServerProvider {
    made: std::sync::Mutex<Vec<(String, std::collections::BTreeMap<String, String>)>>,
}

#[async_trait::async_trait]
impl ProviderAdapter for ServerProvider {
    fn provider(&self) -> Provider {
        Provider::Railway
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            deploy: false,
            logs: false,
            metrics: false,
            usage: false,
            rollback: false,
            pause: false,
            domains: false,
            secrets: false,
            discover: true,
            server: true,
            ..Capabilities::default()
        }
    }

    async fn verify(&self, _: &Secret, _: &str) -> Result<Vec<String>, EngineError> {
        Ok(vec![])
    }

    async fn discover(&self, _: &Secret, _: &str) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(vec![])
    }

    fn server_key_steps(&self, reference: &str) -> Option<(String, Option<String>)> {
        Some((
            "Seal HOISTA_SEALED_KEY at the provider.".into(),
            reference
                .split_once('/')
                .map(|(project, _)| format!("https://railway.com/project/{project}")),
        ))
    }

    async fn provider_new_hoista_server(
        &self,
        _: &Secret,
        _: &str,
        source: &hoista_server::application::ports::providers::ServerSource,
        variables: &std::collections::BTreeMap<String, Secret>,
    ) -> Result<(String, String), EngineError> {
        use hoista_server::application::ports::providers::ServerSource;
        self.made.lock().expect("server fixture").push((
            match source {
                ServerSource::Image(image) => format!("image {image}"),
                ServerSource::Repo { repo, branch } => {
                    format!("repo {repo} {}", branch.as_deref().unwrap_or("-"))
                }
            },
            variables
                .iter()
                .map(|(name, value)| (name.clone(), value.expose().to_owned()))
                .collect(),
        ));
        Ok(("p1/s1".into(), "hoista-production.up.railway.app".into()))
    }
}

#[tokio::test]
async fn hoista_is_put_on_a_server_with_a_claim_link_only_the_person_gets() {
    use base64::Engine as _;
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(ServerProvider::default());
    let image = format!("ghcr.io/hoista/hoistd@sha256:{}", "a".repeat(64));
    let build = |yes: bool| {
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(MemoryVault::default()),
            Arc::new(Inspector),
            vec![provider.clone()],
            Arc::new(Builder::default()),
        )
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::FixedOwnerCheck(yes),
        ))
        .with_server_image(Some(image.clone()))
    };
    let engine = build(true);
    let account = engine
        .accounts()
        .connect_account(
            Provider::Railway,
            "Workspace".into(),
            "ws".into(),
            Secret::new("tok".into()),
        )
        .await
        .expect("account");
    // Agents can't, and nothing happens without the owner check.
    let agent = Actor::Agent { id: "a".into() };
    assert!(
        engine
            .accounts()
            .set_up_server(&agent, &account.id, None, None)
            .await
            .is_err()
    );
    let refusing = Engine::new(
        Arc::new(store.clone()),
        engine.vault.clone(),
        Arc::new(Inspector),
        vec![provider.clone()],
        Arc::new(Builder::default()),
    )
    .with_owner_check(Arc::new(
        hoista_server::application::ports::owner_check::FixedOwnerCheck(false),
    ));
    assert!(
        refusing
            .accounts()
            .set_up_server(&Actor::Person, &account.id, None, None)
            .await
            .is_err()
    );
    assert!(provider.made.lock().expect("server fixture").is_empty());

    // By default the server runs the published image, pinned by digest.
    let made = engine
        .accounts()
        .set_up_server(&Actor::Person, &account.id, None, None)
        .await
        .expect("set up");
    assert_eq!(made.url, "https://hoista-production.up.railway.app");
    let code = made
        .claim_link
        .strip_prefix("https://hoista-production.up.railway.app/#code=")
        .expect("the code rides after #");
    assert!(hoista_server::application::agents::passkey::is_claim_code(
        code
    ));
    // A provider that couldn't hide the key: the person is told how, and where.
    assert!(!made.key_hidden);
    assert!(
        made.seal_help
            .as_deref()
            .is_some_and(|help| help.contains("Seal HOISTA_SEALED_KEY"))
    );
    assert_eq!(
        made.seal_page.as_deref(),
        Some("https://railway.com/project/p1")
    );
    // The key isn't kept here.
    assert!(engine.vault.read("server/key").await.is_err());
    {
        let calls = provider.made.lock().expect("server fixture");
        let (source, variables) = &calls[0];
        assert_eq!(source, &format!("image {image}"));
        assert_eq!(variables["HOISTA_IMAGE"], image);
        assert_eq!(variables["HOISTA_CLAIM_CODE"], code);
        // The provider adds the address it gives; the engine doesn't guess it.
        assert!(!variables.contains_key("HOISTA_PUBLIC_URL"));
        let key = base64::engine::general_purpose::STANDARD
            .decode(&variables["HOISTA_SEALED_KEY"])
            .expect("base64 key");
        assert_eq!(key.len(), 32);
    }
    let settings = engine.repository.settings().await.expect("settings");
    assert_eq!(
        settings.server.and_then(|server| server.image).as_deref(),
        Some(image.as_str())
    );

    // Someone who wants to build it themselves names their copy of the repository.
    engine
        .accounts()
        .set_up_server(&Actor::Person, &account.id, Some("me/hoista"), Some("main"))
        .await
        .expect("from a repository");
    {
        let calls = provider.made.lock().expect("server fixture");
        assert_eq!(calls[1].0, "repo me/hoista main");
        assert!(!calls[1].1.contains_key("HOISTA_IMAGE"));
    }
    // A build without a published image can only build from a repository.
    let unpublished = build(true).with_server_image(None);
    assert!(
        unpublished
            .accounts()
            .set_up_server(&Actor::Person, &account.id, None, None)
            .await
            .is_err()
    );
    // Only an image pinned by digest counts.
    assert!(hoista_server::is_pinned_image(&image));
    assert!(!hoista_server::is_pinned_image(
        "ghcr.io/hoista/hoistd:latest"
    ));
    assert!(!hoista_server::is_pinned_image(
        "ghcr.io/hoista/hoistd@sha256:abc"
    ));
    assert!(!hoista_server::is_pinned_image(&format!(
        "hoistd@sha256:{}",
        "a".repeat(64)
    )));
    store.close().await;
}

/// A computer with no way to isolate builds.
struct Unfenced(Arc<Builder>);
#[async_trait::async_trait]
impl BuildService for Unfenced {
    fn isolation(&self) -> Result<&'static str, &'static str> {
        Err("Builds can't be isolated in this test.")
    }

    async fn secrets(&self, plan: &Plan) -> Result<Vec<(String, Secret)>, EngineError> {
        self.0.secrets(plan).await
    }

    async fn build(
        &self,
        plan: &Plan,
        id: &str,
        secrets: Vec<(String, Secret)>,
    ) -> Result<Artifact, EngineError> {
        self.0.build(plan, id, secrets).await
    }

    async fn migrate(&self, plan: &Plan, id: &str, direct: &Secret) -> Result<(), EngineError> {
        self.0.migrate(plan, id, direct).await
    }

    async fn cleanup(&self, id: &str) -> Result<(), EngineError> {
        self.0.cleanup(id).await
    }
}

#[tokio::test]
async fn build_code_never_runs_unisolated_unless_the_person_chose_it() {
    let dir = tempfile::tempdir().expect("directory");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let provider = Arc::new(TestProvider::default());
    let builder = Arc::new(Builder::default());
    let source = tempfile::tempdir().expect("source");
    std::fs::write(source.path().join("index.html"), "safe source").expect("source");
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault),
            Arc::new(Inspector),
            vec![provider.clone()],
            Arc::new(Unfenced(builder.clone())),
        )
        .with_owner_check(Arc::new(
            hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
        )),
    );
    assert!(engine.settings().build_isolation().is_err());
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "project".into(),
            Source::Folder {
                path: source.path().to_str().expect("path").into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            "project",
            None,
            None,
        )
        .await
        .expect("plan");
    let stop = tokio_util::sync::CancellationToken::new();
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));

    let queued = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queued");
    let stopped = finished(&store, &queued.id).await;
    assert_eq!(stopped.status, DeployStatus::Failed);
    let failure = stopped.failure.expect("why");
    assert_eq!(failure.code, "build.not_isolated");
    assert!(
        failure.message.contains("Settings → Safety"),
        "{}",
        failure.message
    );
    assert!(
        builder.built.lock().expect("built").is_empty(),
        "no build code ran"
    );

    // An agent can't turn it on; the person can, after the owner check.
    assert!(
        engine
            .settings()
            .update_settings(
                &Actor::Agent { id: "a".into() },
                SettingsPatch {
                    unisolated_builds: Some(true),
                    ..SettingsPatch::default()
                },
            )
            .await
            .is_err()
    );
    engine
        .settings()
        .update_settings(
            &Actor::Person,
            SettingsPatch {
                unisolated_builds: Some(true),
                ..SettingsPatch::default()
            },
        )
        .await
        .expect("turned on");
    let queued = engine
        .deploys()
        .enqueue_deploy(&Actor::Person, &project.id, &plan.id)
        .await
        .expect("queued");
    let built = finished(&store, &queued.id).await;
    assert_ne!(
        built.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("build.not_isolated")
    );
    assert_eq!(builder.built.lock().expect("built").len(), 1);
    stop.cancel();
    worker.await.expect("worker").expect("worker");
    store.close().await;
}
