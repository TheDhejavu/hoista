//! Catalog store tests against real SQLite: invariants, approvals and
//! pagination.

use hoista_server::{
    Provider,
    application::ports::catalog::CatalogRepository,
    domain::model::*,
    infrastructure::{paths::StatePaths, store::SqliteStore},
};

async fn setup() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("test directory");
    let paths = StatePaths::new(Some(directory.path().join("state"))).expect("test path");
    let store = SqliteStore::open(&paths).await.expect("migrated store");
    (directory, store)
}

fn project(id: &str) -> Project {
    Project {
        id: id.into(),
        name: "fixture".into(),
        source: Source::Folder {
            path: "/fixture".into(),
        },
        mode: Mode::Watching,
        created_at: "2026-11-02T00:00:00.000Z".into(),
        environment_id: "env_default".into(),
    }
}

fn deployment(id: &str, project_id: &str) -> Deploy {
    Deploy {
        title: None,
        channel: DeployChannel::Desktop,
        started_by_name: None,
        environment_keys: vec![],
        failure: None,
        release_version: None,
        provenance: None,
        source_observation: None,
        cancel_requested: false,
        plan_id: "fixture-plan".into(),
        releases: vec![],
        id: id.into(),
        project_id: project_id.into(),
        version: 999,
        source: project("x").source,
        started_by: "person".into(),
        status: DeployStatus::Queued,
        steps: vec![],
        url: None,
        started_at: "2026-11-02T00:00:00.000Z".into(),
        finished_at: None,
        duration_ms: None,
        rollback_of: None,
    }
}

#[tokio::test]
async fn approvals_are_bound_to_action_target_agent_expiry_and_consumed_once() {
    use hoista_server::application::StateStore;
    let (_directory, store) = setup().await;
    let agent = Agent {
        id: "agent".into(),
        name: "Fixture".into(),
        transport: AgentTransport::Stdio,
        scopes: vec!["apps:write".into()],
        last_seen: "2026-11-02T00:00:00.000Z".into(),
        revoked: false,
        grant: Default::default(),
    };
    store.put_agent(&agent).await.unwrap();
    let approval = Approval {
        deletion: None,
        id: "approval".into(),
        agent_id: agent.id.clone(),
        action: GatedAction::DeleteProject,
        target: "project-a".into(),
        reason: "Delete test project".into(),
        created_at: "2026-11-02T00:00:00.000Z".into(),
        expires_at: "2026-11-02T00:15:00.000Z".into(),
        decision: ApprovalDecision::Pending,
    };
    store.put_approval(&approval).await.unwrap();
    let now = "2026-11-02T00:01:00.000Z";
    assert!(
        store
            .consume_approval(
                "approval",
                "agent",
                GatedAction::DeleteProject,
                "project-a",
                now
            )
            .await
            .is_err()
    );
    store.decide_approval("approval", true, now).await.unwrap();
    assert!(
        store
            .consume_approval(
                "approval",
                "other-agent",
                GatedAction::DeleteProject,
                "project-a",
                now
            )
            .await
            .is_err()
    );
    assert!(
        store
            .consume_approval(
                "approval",
                "agent",
                GatedAction::DeleteResource,
                "project-a",
                now
            )
            .await
            .is_err()
    );
    assert!(
        store
            .consume_approval(
                "approval",
                "agent",
                GatedAction::DeleteProject,
                "project-b",
                now
            )
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        store.consume_approval(
            "approval",
            "agent",
            GatedAction::DeleteProject,
            "project-a",
            now
        ),
        store.consume_approval(
            "approval",
            "agent",
            GatedAction::DeleteProject,
            "project-a",
            now
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let subjects: Vec<(String, Option<String>)> = store
        .events_after(0)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.kind.starts_with("approval.") || event.kind.starts_with("agent."))
        .map(|event| (event.kind, event.subject_id))
        .collect();
    assert_eq!(
        subjects,
        [
            ("agent.updated".into(), Some("agent".into())),
            ("approval.created".into(), Some("approval".into())),
            ("approval.resolved".into(), Some("approval".into())),
            ("approval.consumed".into(), Some("approval".into())),
        ],
        "events name the record they are about"
    );
    let mut expired = approval.clone();
    expired.id = "expired".into();
    store.put_approval(&expired).await.unwrap();
    assert!(
        store
            .decide_approval("expired", true, "2026-11-02T00:15:00.000Z")
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn account_resource_and_graph_constraints_are_enforced() {
    let (_directory, store) = setup().await;
    let account = Account {
        last_discovery: None,
        id: "cloud".into(),
        provider: Provider::Cloudflare,
        label: "Cloudflare".into(),
        external_id: "external".into(),
        scopes: vec![],
        status: AccountStatus::Connected,
        connected_at: "2026-11-02T00:00:00.000Z".into(),
        credential: None,
        in_1password: false,
    };
    store.put_account(&account).await.unwrap();
    store.put_project(&project("one")).await.unwrap();
    store.put_project(&project("two")).await.unwrap();
    let left = Resource {
        id: "left".into(),
        project_id: Some("one".into()),
        kind: ResourceKind::Web,
        provider: Provider::Cloudflare,
        account_id: account.id.clone(),
        provider_ref: "worker:left".into(),
        name: "left".into(),
        region: None,
        status: ResourceStatus::Unknown,
        mode: Mode::Watching,
        url: None,
        group: None,
    };
    let mut right = left.clone();
    right.id = "right".into();
    right.provider_ref = "worker:right".into();
    right.project_id = Some("two".into());
    store.put_resource(&left).await.unwrap();
    store.put_resource(&right).await.unwrap();
    assert!(store.remove_account(&account.id).await.is_err());
    let link = Link {
        id: "edge".into(),
        from_resource: "left".into(),
        to_resource: "right".into(),
        via: "DATABASE_URL".into(),
        derived: false,
        missing: false,
    };
    store.put_link(&link).await.unwrap();
    right.project_id = left.project_id.clone();
    store.put_resource(&right).await.unwrap();
    store.put_link(&link).await.unwrap();
    assert_eq!(store.links("one").await.unwrap().len(), 1);
    let mut wrong = right.clone();
    wrong.provider = Provider::Neon;
    assert!(store.put_resource(&wrong).await.is_err());
    assert!(store.remove_project("one").await.is_err());
    let mut third = right.clone();
    third.id = "third".into();
    third.provider_ref = "worker:third".into();
    store.put_resource(&third).await.unwrap();
    let mut moved_link = link;
    moved_link.to_resource = third.id.clone();
    store.put_link(&moved_link).await.unwrap();
    store.remove_resource("right").await.unwrap();
    assert_eq!(store.links("one").await.unwrap(), vec![moved_link]);
    let mut duplicate = third.clone();
    duplicate.provider_ref = left.provider_ref.clone();
    assert!(store.put_resource(&duplicate).await.is_err());
    assert_eq!(
        store.resource("third").await.unwrap().unwrap().provider_ref,
        third.provider_ref
    );
    third.provider_ref = "worker:renamed".into();
    store.put_resource(&third).await.unwrap();
    // Resource uniqueness follows updated metadata, so another id cannot alias it.
    let mut alias = third.clone();
    alias.id = "alias".into();
    assert!(store.put_resource(&alias).await.is_err());
    store.remove_resource("third").await.unwrap();
    assert!(store.links("one").await.unwrap().is_empty());
    store.close().await;
}

#[tokio::test]
async fn catalog_pagination_is_stable_and_deploy_versions_are_monotonic() {
    use hoista_server::application::StateStore;
    let (_directory, store) = setup().await;
    for i in 0..105 {
        store
            .put_project(&project(&format!("project-{i:03}")))
            .await
            .unwrap();
    }
    let first = store.projects(None).await.unwrap();
    assert_eq!(first.items.len(), 100);
    let next = store.projects(first.next_cursor.as_deref()).await.unwrap();
    assert_eq!(next.items.len(), 5);
    assert!(next.next_cursor.is_none());
    let deploy = deployment("first", "project-000");
    let mut first = store.enqueue(&deploy).await.unwrap();
    assert_eq!(first.version, 1);
    let mut second = deploy;
    second.id = "second".into();
    assert_eq!(store.enqueue(&second).await.unwrap().version, 2);
    let mut other = deployment("other-project-deploy", "project-001");
    other.id = "third".into();
    store.enqueue(&other).await.unwrap();
    let recent: Vec<String> = store
        .recent_deploys()
        .await
        .unwrap()
        .into_iter()
        .map(|deploy| deploy.id)
        .collect();
    assert_eq!(
        recent,
        ["third", "second", "first"],
        "newest first across projects"
    );
    store.cancel_deploy("person", &first.id).await.unwrap();
    store.cancel_deploy("person", &first.id).await.unwrap();
    // A stale worker snapshot cannot erase a cancellation request.
    first.status = DeployStatus::Running;
    store.put_deploy(&first).await.unwrap();
    let mut all_events = Vec::new();
    loop {
        let cursor = all_events
            .last()
            .map_or(0, |event: &hoista_server::application::Event| event.id);
        let page = store.events_after(cursor).await.unwrap();
        if page.is_empty() {
            break;
        }
        all_events.extend(page);
    }
    let deploy_events: Vec<(String, Option<String>)> = all_events
        .into_iter()
        .filter(|event| event.kind.starts_with("deploy."))
        .map(|event| (event.kind, event.subject_id))
        .collect();
    assert_eq!(
        deploy_events,
        [
            ("deploy.queued".into(), Some("first".into())),
            ("deploy.queued".into(), Some("second".into())),
            ("deploy.queued".into(), Some("third".into())),
            ("deploy.cancel_requested".into(), Some("first".into())),
            ("deploy.step".into(), Some("first".into())),
        ]
    );
    assert!(
        store
            .deploy(&first.id)
            .await
            .unwrap()
            .unwrap()
            .cancel_requested
    );
    assert!(
        store
            .deploys("project-000", None)
            .await
            .unwrap()
            .items
            .iter()
            .find(|d| d.id == first.id)
            .unwrap()
            .cancel_requested
    );
    store.recover_deploys().await.unwrap();
    assert_eq!(
        store.deploy("first").await.unwrap().unwrap().status,
        DeployStatus::Failed
    );
    assert!(
        store
            .deploy("first")
            .await
            .unwrap()
            .unwrap()
            .failure
            .is_none(),
        "Crash recovery must not invent observed failure evidence"
    );
    assert!(store.put_deploy(&first).await.is_err());
    assert!(store.cancel_deploy("person", &first.id).await.is_err());
    assert_eq!(
        store.deploys("project-000", None).await.unwrap().items[0].version,
        2
    );
    store.close().await;
}

#[tokio::test]
async fn import_and_management_changes_commit_with_events_or_not_at_all() {
    let (directory, store) = setup().await;
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
    store.put_project(&project("project")).await.unwrap();
    for id in ["a", "b"] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: None,
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
            .unwrap();
    }
    assert!(
        store
            .group_resources("person", "project", &["a".into(), "missing".into()])
            .await
            .is_err()
    );
    assert_eq!(store.resource("a").await.unwrap().unwrap().project_id, None);
    assert!(
        store
            .group_resources("person", "project", &["a".into(), "a".into()])
            .await
            .is_err()
    );
    assert_eq!(store.resource("a").await.unwrap().unwrap().project_id, None);
    store
        .group_resources("person", "project", &["a".into(), "b".into()])
        .await
        .unwrap();
    let fault = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(directory.path().join("state/hoista.db")),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_mode_event BEFORE INSERT ON events WHEN NEW.kind = 'app.mode' BEGIN SELECT RAISE(ABORT, 'injected failure'); END")
        .execute(&fault)
        .await
        .unwrap();
    assert!(
        store
            .set_project_mode("project", Mode::Managed)
            .await
            .is_err()
    );
    assert_eq!(
        store.project("project").await.unwrap().unwrap().mode,
        Mode::Watching
    );
    for id in ["a", "b"] {
        assert_eq!(
            store.resource(id).await.unwrap().unwrap().mode,
            Mode::Watching
        );
    }
    sqlx::query("DROP TRIGGER reject_mode_event")
        .execute(&fault)
        .await
        .unwrap();
    store
        .set_project_mode("project", Mode::Managed)
        .await
        .unwrap();
    for id in ["a", "b"] {
        assert_eq!(
            store.resource(id).await.unwrap().unwrap().mode,
            Mode::Managed
        );
    }
    // A retried import retains the user's subsequent management choice.
    let regrouped = store
        .group_resources("person", "project", &["a".into()])
        .await
        .unwrap();
    assert_eq!(regrouped[0].mode, Mode::Managed);
    fault.close().await;
    store.close().await;
}

#[tokio::test]
async fn concurrent_queue_and_recovery_writes_preserve_every_command() {
    let (_directory, store) = setup().await;
    store
        .put_project(&project("shared"))
        .await
        .expect("project");
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(21));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..20 {
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            store
                .enqueue(&Deploy {
                    title: None,
                    channel: DeployChannel::Desktop,
                    started_by_name: None,
                    environment_keys: vec![],
                    failure: None,
                    release_version: None,
                    provenance: None,
                    source_observation: None,
                    id: format!("concurrent-{index}"),
                    project_id: "shared".into(),
                    version: 0,
                    plan_id: "fixture".into(),
                    source: project("source").source,
                    releases: vec![],
                    started_by: "person".into(),
                    status: DeployStatus::Queued,
                    cancel_requested: false,
                    steps: vec![],
                    url: None,
                    started_at: "2026-11-02T00:00:00Z".into(),
                    finished_at: None,
                    duration_ms: None,
                    rollback_of: None,
                })
                .await
        });
    }
    barrier.wait().await;
    store.recover_deploys().await.expect("recovery");
    let mut versions = Vec::new();
    while let Some(result) = tasks.join_next().await {
        versions.push(result.expect("task").expect("concurrent enqueue").version);
    }
    versions.sort_unstable();
    assert_eq!(versions, (1..=20).collect::<Vec<_>>());
    assert_eq!(
        store
            .deploys("shared", None)
            .await
            .expect("queue")
            .items
            .len(),
        20
    );
    store.close().await;
}

#[tokio::test]
async fn new_import_rolls_back_project_assignments_and_events_together() {
    use hoista_server::application::StateStore;
    let (directory, store) = setup().await;
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
    for id in ["a", "b"] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: None,
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
            .unwrap();
    }
    let mut value = project("imported");
    value.source = Source::Unlinked;
    value.mode = Mode::Managed;
    let before = store.events_after(0).await.unwrap().len();
    for ids in [
        vec!["a".into(), "missing".into()],
        vec!["a".into(), "a".into()],
        vec![],
    ] {
        assert!(store.import_project("person", &value, &ids).await.is_err());
        assert!(store.project(&value.id).await.unwrap().is_none());
        assert!(
            store
                .resource("a")
                .await
                .unwrap()
                .unwrap()
                .project_id
                .is_none()
        );
        assert_eq!(store.events_after(0).await.unwrap().len(), before);
    }
    let fault = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(directory.path().join("state/hoista.db")),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_import BEFORE INSERT ON events WHEN NEW.kind = 'import.grouped' BEGIN SELECT RAISE(ABORT, 'fixture'); END")
        .execute(&fault)
        .await
        .unwrap();
    assert!(
        store
            .import_project("person", &value, &["a".into(), "b".into()])
            .await
            .is_err()
    );
    assert!(store.project(&value.id).await.unwrap().is_none());
    assert!(
        store
            .resource("a")
            .await
            .unwrap()
            .unwrap()
            .project_id
            .is_none()
    );
    assert_eq!(store.events_after(0).await.unwrap().len(), before);
    sqlx::query("DROP TRIGGER reject_import")
        .execute(&fault)
        .await
        .unwrap();
    let resources = store
        .import_project("person", &value, &["a".into(), "b".into()])
        .await
        .unwrap();
    assert!(
        resources
            .iter()
            .all(|resource| resource.mode == Mode::Managed
                && resource.project_id.as_deref() == Some("imported"))
    );
    assert_eq!(
        store.project("imported").await.unwrap().unwrap().source,
        Source::Unlinked
    );
    let other = project("other");
    assert!(
        store
            .import_project("person", &other, &["a".into()])
            .await
            .is_err()
    );
    assert!(store.project("other").await.unwrap().is_none());
    assert!(
        hoista_server::infrastructure::source::resolve(&Source::Unlinked)
            .await
            .is_err()
    );
    let source = Source::Folder {
        path: "/fixture/source".into(),
    };
    sqlx::query("CREATE TRIGGER reject_source BEFORE INSERT ON events WHEN NEW.kind = 'app.source_linked' BEGIN SELECT RAISE(ABORT, 'fixture'); END")
        .execute(&fault)
        .await
        .unwrap();
    assert!(
        store
            .link_source("person", "imported", &source)
            .await
            .is_err()
    );
    assert_eq!(
        store.project("imported").await.unwrap().unwrap().source,
        Source::Unlinked
    );
    sqlx::query("DROP TRIGGER reject_source")
        .execute(&fault)
        .await
        .unwrap();
    let linked = store
        .link_source("person", "imported", &source)
        .await
        .unwrap();
    assert_eq!(linked.source, source);
    assert_eq!(linked.mode, Mode::Managed);
    assert!(
        store
            .link_source(
                "person",
                "imported",
                &Source::Folder {
                    path: "/other".into()
                }
            )
            .await
            .is_err()
    );
    // A managed project keeps its source; there's nothing to disconnect.
    assert!(
        store
            .link_source("person", "imported", &Source::Unlinked)
            .await
            .is_err()
    );
    fault.close().await;
    store.close().await;
}

#[tokio::test]
async fn secret_and_usage_pages_cover_composite_keys_and_reject_other_filters() {
    let (directory, store) = setup().await;
    store
        .put_account(&Account {
            last_discovery: None,
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Disconnected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .unwrap();
    store.put_project(&project("paged")).await.unwrap();
    for id in ["a", "b"] {
        store
            .put_resource(&Resource {
                id: id.into(),
                project_id: Some("paged".into()),
                kind: ResourceKind::Web,
                provider: Provider::Cloudflare,
                account_id: "account".into(),
                provider_ref: format!("worker:{id}"),
                name: id.into(),
                region: None,
                status: ResourceStatus::Unknown,
                mode: Mode::Watching,
                url: None,
                group: None,
            })
            .await
            .unwrap();
    }
    let fixture = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(directory.path().join("state/hoista.db")),
    )
    .await
    .unwrap();
    let mut tx = fixture.begin().await.unwrap();
    for index in 0..1105 {
        let value = SecretMetadata {
            requires_source_sync: false,
            set_by: None,
            managed_by: None,
            project_id: "paged".into(),
            resource_id: if index % 2 == 0 { "a" } else { "b" }.into(),
            name: format!("KEY_{index:04}"),
            fingerprint: "never-public".into(),
            set_at: "2026-11-02T00:00:00Z".into(),
        };
        sqlx::query("INSERT INTO secret_metadata(resource_id,name,data) VALUES(?,?,?)")
            .bind(&value.resource_id)
            .bind(&value.name)
            .bind(serde_json::to_string(&value).unwrap())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    for index in 0..105 {
        let day = (chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(index))
            .to_string();
        for resource in ["a", "b"] {
            let value = Usage {
                resource_id: resource.into(),
                day: day.clone(),
                requests: Some(1),
                errors: None,
                compute_seconds: None,
                storage_bytes: None,
                egress_bytes: None,
            };
            sqlx::query("INSERT INTO usage(resource_id,day,data) VALUES(?,?,?)")
                .bind(resource)
                .bind(&day)
                .bind(serde_json::to_string(&value).unwrap())
                .execute(&mut *tx)
                .await
                .unwrap();
        }
    }
    tx.commit().await.unwrap();
    let first = store.secrets_page("paged", None).await.unwrap();
    assert_eq!(first.items.len(), 100);
    let key = first.next_cursor.clone().unwrap();
    assert!(store.secrets_page("another", Some(&key)).await.is_err());
    assert!(
        store
            .usage_page("paged", "2026-01-01", Some(&key))
            .await
            .is_err()
    );
    assert!(
        store
            .secrets_page("paged", Some("not-a-cursor"))
            .await
            .is_err()
    );
    let mut cursor = None;
    let mut names = std::collections::BTreeSet::new();
    loop {
        let page = store
            .secrets_page("paged", cursor.as_deref())
            .await
            .unwrap();
        assert!(page.items.len() <= 100);
        for item in page.items {
            assert!(names.insert((item.resource_id, item.name)));
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(names.len(), 1105);
    assert_eq!(store.secrets("paged").await.unwrap().len(), 1105);
    let first = store.usage_page("paged", "2026-01-01", None).await.unwrap();
    assert_eq!(first.items.len(), 100);
    let key = first.next_cursor.clone().unwrap();
    assert!(
        store
            .usage_page("paged", "2026-02-01", Some(&key))
            .await
            .is_err()
    );
    assert!(
        store
            .usage_page("another", "2026-01-01", Some(&key))
            .await
            .is_err()
    );
    let mut cursor = None;
    let mut observations = std::collections::BTreeSet::new();
    loop {
        let page = store
            .usage_page("paged", "2026-01-01", cursor.as_deref())
            .await
            .unwrap();
        for item in page.items {
            assert!(observations.insert((item.day, item.resource_id)));
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(observations.len(), 210);
    // A cursor remains usable even if its boundary row is removed after the first page.
    let boundary = first.items.last().unwrap();
    sqlx::query("DELETE FROM usage WHERE resource_id=? AND day=?")
        .bind(&boundary.resource_id)
        .bind(&boundary.day)
        .execute(&fixture)
        .await
        .unwrap();
    assert_eq!(
        store
            .usage_page("paged", "2026-01-01", Some(&key))
            .await
            .unwrap()
            .items
            .len(),
        100
    );
    fixture.close().await;
    store.close().await;
}

#[tokio::test]
async fn successful_versions_skip_failures_and_commit_atomically_with_events() {
    let (directory, store) = setup().await;
    store.put_project(&project("releases")).await.unwrap();
    store.put_project(&project("other")).await.unwrap();
    let fault = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(directory.path().join("state/hoista.db")),
    )
    .await
    .unwrap();
    for (id, status, expected) in [
        ("failure", DeployStatus::Failed, None),
        ("first-release", DeployStatus::Live, Some(1)),
        ("another-failure", DeployStatus::Failed, None),
        ("rollback", DeployStatus::RolledBack, Some(2)),
    ] {
        let mut input = deployment(id, "releases");
        input.release_version = Some(999);
        let mut queued = store.enqueue(&input).await.unwrap();
        assert_eq!(queued.release_version, None);
        queued.status = DeployStatus::Running;
        queued.release_version = Some(999);
        store.put_deploy(&queued).await.unwrap();
        assert_eq!(
            store.deploy(id).await.unwrap().unwrap().release_version,
            None
        );
        queued.status = status;
        sqlx::query("CREATE TRIGGER reject_finish BEFORE INSERT ON events WHEN NEW.kind = 'deploy.finished' BEGIN SELECT RAISE(ABORT, 'injected failure'); END")
            .execute(&fault)
            .await
            .unwrap();
        assert!(store.put_deploy(&queued).await.is_err());
        let unchanged = store.deploy(id).await.unwrap().unwrap();
        assert_eq!(unchanged.status, DeployStatus::Running);
        assert_eq!(unchanged.release_version, None);
        sqlx::query("DROP TRIGGER reject_finish")
            .execute(&fault)
            .await
            .unwrap();
        store.put_deploy(&queued).await.unwrap();
        assert_eq!(
            store.deploy(id).await.unwrap().unwrap().release_version,
            expected
        );
        assert!(store.put_deploy(&queued).await.is_err());
    }
    let mut other = store
        .enqueue(&deployment("other-release", "other"))
        .await
        .unwrap();
    other.status = DeployStatus::Live;
    store.put_deploy(&other).await.unwrap();
    assert_eq!(
        store
            .deploy(&other.id)
            .await
            .unwrap()
            .unwrap()
            .release_version,
        Some(1)
    );
    let mut cancelled = store
        .enqueue(&deployment("cancelled", "releases"))
        .await
        .unwrap();
    store.cancel_deploy("person", &cancelled.id).await.unwrap();
    cancelled.status = DeployStatus::Running;
    store.put_deploy(&cancelled).await.unwrap();
    store.recover_deploys().await.unwrap();
    let recovered = store.deploy(&cancelled.id).await.unwrap().unwrap();
    assert_eq!(recovered.status, DeployStatus::Failed);
    assert_eq!(recovered.release_version, None);
    fault.close().await;
    store.close().await;
}

#[tokio::test]
async fn concurrent_successes_receive_unique_contiguous_release_versions() {
    let (_directory, store) = setup().await;
    store
        .put_project(&project("concurrent-releases"))
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(21));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..20 {
        let mut queued = store
            .enqueue(&deployment(
                &format!("release-{index}"),
                "concurrent-releases",
            ))
            .await
            .unwrap();
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            queued.status = DeployStatus::Live;
            store.put_deploy(&queued).await.unwrap();
        });
    }
    barrier.wait().await;
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    let mut numbers = store
        .deploys("concurrent-releases", None)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|value| value.release_version.unwrap())
        .collect::<Vec<_>>();
    numbers.sort_unstable();
    assert_eq!(numbers, (1..=20).collect::<Vec<_>>());
    store.close().await;
}

#[tokio::test]
async fn concurrent_plan_edits_commit_only_one_revision_and_one_event() {
    use hoista_server::application::StateStore;
    let (_directory, store) = setup().await;
    let plan = Plan {
        revision: 0,
        id: "concurrent-plan".into(),
        source: Source::Unlinked,
        framework: "static".into(),
        package_manager: None,
        build_command: vec![],
        output_directory: ".".into(),
        env_names: vec![],
        generated_secret_names: vec![],
        prisma_migration: None,
        resources: vec![],
        warnings: vec![],
    };
    let mut legacy = serde_json::to_value(&plan).unwrap();
    legacy.as_object_mut().unwrap().remove("revision");
    assert_eq!(serde_json::from_value::<Plan>(legacy).unwrap().revision, 0);
    store.put_plan(&plan).await.unwrap();
    let mut first = plan.clone();
    first.revision = 1;
    first.output_directory = "first".into();
    let mut second = first.clone();
    second.output_directory = "second".into();
    let (a, b) = tokio::join!(store.update_plan(&first, 0), store.update_plan(&second, 0));
    assert_ne!(a.is_ok(), b.is_ok(), "Exactly one writer may win");
    let saved = store.plan(&plan.id).await.unwrap().unwrap();
    assert_eq!(saved, if a.is_ok() { first.clone() } else { second });
    assert!(store.update_plan(&first, 0).await.is_err());
    assert!(
        store.update_plan(&first, 1).await.is_err(),
        "Must advance exactly once"
    );
    assert_eq!(
        store
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .filter(|event| event.kind == "plan.updated")
            .count(),
        2
    );
    let mut next = saved.clone();
    next.revision = 2;
    store.update_plan(&next, 1).await.unwrap();
    assert_eq!(store.plan(&plan.id).await.unwrap().unwrap().revision, 2);
    next.id = "queued:immutable".into();
    store.put_plan(&next).await.unwrap();
    next.revision = 3;
    assert!(store.update_plan(&next, 2).await.is_err());
    store.close().await;
}

#[tokio::test]
async fn diagnostic_reports_exclude_free_text_bound_history_and_publish_atomically() {
    use hoista_server::{
        application::observe::diagnostic_report, infrastructure::diagnostic_report as files,
    };
    use sha2::{Digest, Sha256};
    let (directory, store) = setup().await;
    store
        .put_project(&project("private-identity"))
        .await
        .unwrap();
    for index in 0..11 {
        let mut deploy = deployment(&format!("private-attempt-{index}"), "private-identity");
        deploy.title = Some("private-content".into());
        deploy.started_by_name = Some("private-content".into());
        deploy.started_at = "private-content".into();
        deploy.steps = (0..17)
            .map(|_| DeployStep {
                name: "private-content".into(),
                status: StepStatus::Failed,
                message: Some("private-content".into()),
            })
            .collect();
        deploy.failure = Some(DeployFailure {
            provider_response: None,
            missing_secrets: vec!["private-content".into()],
            code: "private-content".into(),
            message: "private-content".into(),
            steps: vec!["private-content".into()],
            release_started: true,
            compensation_failed: true,
        });
        store.enqueue(&deploy).await.unwrap();
    }
    let report = diagnostic_report::collect(&store).await;
    let serialized = serde_json::to_string(&report).unwrap();
    for forbidden in [
        "private-identity",
        "private-attempt",
        "private-content",
        "/fixture",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "Raw metadata must not enter reports"
        );
    }
    assert_eq!(report.projects.len(), 1);
    assert_eq!(report.projects[0].deployments.len(), 10);
    assert!(report.projects[0].older_deployments_omitted);
    let attempt = &report.projects[0].deployments[0];
    assert_eq!(attempt.steps.len(), 16);
    assert!(attempt.steps_truncated);
    assert_eq!(attempt.steps[0].name, "other");
    assert_eq!(attempt.failure_code.as_deref(), Some("unclassified"));
    assert!(attempt.started_at.is_none());
    let another = diagnostic_report::collect(&store).await;
    assert_ne!(report.projects[0].reference, another.projects[0].reference);
    let paths = StatePaths::new(Some(directory.path().join("reports-state"))).unwrap();
    let receipt = files::save(&paths, &report).unwrap();
    let bytes = std::fs::read(&receipt.path).unwrap();
    assert_eq!(bytes.len(), receipt.bytes as usize);
    assert_eq!(receipt.sha256, format!("{:x}", Sha256::digest(&bytes)));
    let restored: diagnostic_report::DiagnosticReport = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored.format_version, 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&receipt.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let linked = directory.path().join("linked-state");
        std::fs::create_dir(&linked).unwrap();
        std::os::unix::fs::symlink(paths.root().join("diagnostics"), linked.join("diagnostics"))
            .unwrap();
        assert!(files::save(&StatePaths::new(Some(linked)).unwrap(), &report).is_err());
    }
    for index in 0..49 {
        std::fs::write(
            paths
                .root()
                .join("diagnostics")
                .join(format!("old-{index}")),
            b"old",
        )
        .unwrap();
    }
    assert!(files::save(&paths, &report).is_err());
    assert_eq!(std::fs::read(&receipt.path).unwrap(), bytes);
    assert_eq!(
        std::fs::read_dir(paths.root().join("diagnostics"))
            .unwrap()
            .count(),
        50
    );
    for index in 0..101 {
        store
            .put_project(&project(&format!("extra-{index}")))
            .await
            .unwrap();
    }
    let bounded = diagnostic_report::collect(&store).await;
    assert_eq!(bounded.projects.len(), 100);
    assert!(bounded.projects_truncated);
    let mut oversized = report;
    oversized.limitations.push("x".repeat(2 * 1024 * 1024));
    assert!(files::save(&paths, &oversized).is_err());
    store.close().await;
}

#[tokio::test]
async fn diagnostics_preserve_available_sections_when_saved_catalog_data_is_unreadable() {
    use hoista_server::{
        application::observe::diagnostic_report, infrastructure::diagnostic_report as files,
    };
    let (directory, store) = setup().await;
    store.put_project(&project("broken-project")).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        directory.path().join("state/hoista.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE projects SET data = ? WHERE id = ?")
        .bind(r#"{"id":"broken-project","name":"private-unreadable-database-payload"}"#)
        .bind("broken-project")
        .execute(&pool)
        .await
        .unwrap();
    let report = diagnostic_report::collect(&store).await;
    assert_eq!(report.collection_errors.len(), 1);
    assert_eq!(report.collection_errors[0].section, "projects");
    assert_eq!(report.collection_errors[0].code, "state.unavailable");
    assert!(report.projects_truncated);
    assert!(!report.accounts_truncated);
    assert!(!report.resources_truncated);
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains("private-unreadable"));
    let paths = StatePaths::new(Some(directory.path().join("report-state"))).unwrap();
    assert!(files::save(&paths, &report).is_ok());
    pool.close().await;
    store.close().await;
}

#[tokio::test]
async fn deployment_admission_is_atomic_under_concurrent_retries_and_survives_deletion() {
    use hoista_server::application::ports::catalog::DeploymentAdmission;
    let (directory, store) = setup().await;
    store.put_project(&project("admission")).await.unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let receipt = DeploymentAdmission::new("person", &key, &"a".repeat(64)).unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(13));
    let mut tasks = tokio::task::JoinSet::new();
    for n in 0..12 {
        let store = store.clone();
        let receipt = receipt.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            store
                .enqueue_with_admission(
                    &deployment(&format!("retry-{n}"), "admission"),
                    Some(&receipt),
                )
                .await
        });
    }
    barrier.wait().await;
    let mut ids = std::collections::BTreeSet::new();
    while let Some(result) = tasks.join_next().await {
        let value = result.unwrap().unwrap();
        assert_eq!(value.version, 1);
        ids.insert(value.id);
    }
    assert_eq!(ids.len(), 1);
    let accepted = ids.into_iter().next().unwrap();
    assert_eq!(
        store.deploys("admission", None).await.unwrap().items.len(),
        1
    );
    let different = DeploymentAdmission::new("person", &key, &"b".repeat(64)).unwrap();
    assert!(store.admitted_deploy(&different).await.is_err());
    let other_actor = DeploymentAdmission::new("another-agent", &key, &"a".repeat(64)).unwrap();
    assert!(store.admitted_deploy(&other_actor).await.unwrap().is_none());
    assert!(
        store
            .enqueue_with_admission(&deployment("wrong-actor", "admission"), Some(&other_actor))
            .await
            .is_err()
    );
    // A full queue must still allow retrieval of an already accepted request.
    for n in 0..99 {
        store
            .enqueue(&deployment(&format!("fill-{n}"), "admission"))
            .await
            .unwrap();
    }
    assert!(
        store
            .enqueue(&deployment("overflow", "admission"))
            .await
            .is_err()
    );
    assert_eq!(
        store
            .enqueue_with_admission(&deployment("retry-full", "admission"), Some(&receipt))
            .await
            .unwrap()
            .id,
        accepted
    );
    store.cancel_deploy("person", &accepted).await.unwrap();
    assert!(
        store
            .admitted_deploy(&receipt)
            .await
            .unwrap()
            .unwrap()
            .cancel_requested
    );
    store.close().await;
    drop(store);
    let paths = StatePaths::new(Some(directory.path().join("state"))).unwrap();
    let reopened = SqliteStore::open(&paths).await.unwrap();
    assert_eq!(
        reopened
            .admitted_deploy(&receipt)
            .await
            .unwrap()
            .unwrap()
            .id,
        accepted
    );
    let inspected = reopened
        .deployment_request("person", &key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inspected.deploy_id, accepted);
    assert!(inspected.deployment.unwrap().cancel_requested);
    assert!(
        reopened
            .deployment_request("another-agent", &key)
            .await
            .unwrap()
            .is_none()
    );
    reopened.remove_project("admission").await.unwrap();
    reopened.put_project(&project("admission")).await.unwrap();
    let removed = reopened
        .deployment_request("person", &key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(removed.deploy_id, accepted);
    assert!(removed.deployment.is_none());
    assert_eq!(removed.project_id, "admission");
    assert!(reopened.admitted_deploy(&receipt).await.is_err());
    assert!(
        reopened
            .enqueue_with_admission(&deployment("after-delete", "admission"), Some(&receipt))
            .await
            .is_err()
    );
    assert!(
        reopened
            .deploys("admission", None)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    reopened.close().await;
}

#[tokio::test]
async fn failed_admission_receipt_rolls_back_queue_version_and_event() {
    use hoista_server::application::ports::catalog::DeploymentAdmission;
    let (directory, store) = setup().await;
    store
        .put_project(&project("atomic-admission"))
        .await
        .unwrap();
    let paths = StatePaths::new(Some(directory.path().join("state"))).unwrap();
    let fault = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(paths.database()),
    )
    .await
    .unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let receipt = DeploymentAdmission::new("person", &key, &"a".repeat(64)).unwrap();
    sqlx::query("CREATE TRIGGER reject_admission BEFORE INSERT ON deployment_admissions BEGIN SELECT RAISE(ABORT, 'fixture'); END;")
        .execute(&fault)
        .await
        .unwrap();
    assert!(
        store
            .enqueue_with_admission(&deployment("atomic", "atomic-admission"), Some(&receipt))
            .await
            .is_err()
    );
    assert!(store.admitted_deploy(&receipt).await.unwrap().is_none());
    assert!(
        store
            .deploys("atomic-admission", None)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE kind = 'deploy.queued'")
            .fetch_one(&fault)
            .await
            .unwrap();
    assert_eq!(events, 0);
    sqlx::query("DROP TRIGGER reject_admission")
        .execute(&fault)
        .await
        .unwrap();
    let value = store
        .enqueue_with_admission(&deployment("atomic", "atomic-admission"), Some(&receipt))
        .await
        .unwrap();
    assert_eq!(value.version, 1);
    store
        .enqueue_with_admission(
            &deployment("same-again", "atomic-admission"),
            Some(&receipt),
        )
        .await
        .unwrap();
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE kind = 'deploy.queued'")
            .fetch_one(&fault)
            .await
            .unwrap();
    assert_eq!(events, 1);
    assert!(
        sqlx::query("DELETE FROM deployment_admissions")
            .execute(&fault)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE deployment_admissions SET fingerprint = fingerprint")
            .execute(&fault)
            .await
            .is_err()
    );
    assert!(DeploymentAdmission::new("person", "private-invalid-key", &"a".repeat(64)).is_err());
    assert!(DeploymentAdmission::new("person", &key, "private-invalid-fingerprint").is_err());
    fault.close().await;
    store.close().await;
}
