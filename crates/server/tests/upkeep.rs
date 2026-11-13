//! Upkeep: automatic backups (saved whole, only the newest kept, restored
//! only into an empty Hoista, never holding a sign-in), and cutting off
//! every agent when something looks wrong.

use hoista_server::{
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::{CatalogRepository as _, EngineError},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        backups::LocalBackups, build::LocalBuilder, inspection::LocalInspector, paths::StatePaths,
        store::SqliteStore,
    },
};
use std::sync::Arc;

struct Vault;
#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, _: &str) -> Result<Secret, EngineError> {
        Ok(Secret::new("keychain-only-token".into()))
    }

    async fn write(&self, _: &str, _: Secret) -> Result<(), EngineError> {
        Ok(())
    }

    async fn delete(&self, _: &str) -> Result<(), EngineError> {
        Ok(())
    }
}

async fn hoista(paths: &StatePaths) -> (Arc<Engine>, SqliteStore) {
    let store = SqliteStore::open(paths).await.expect("store");
    let engine = Arc::new(
        Engine::new(
            Arc::new(store.clone()),
            Arc::new(Vault),
            Arc::new(LocalInspector),
            vec![],
            Arc::new(LocalBuilder::new(paths.root())),
        )
        .with_backup_folder(Arc::new(LocalBackups::new(paths))),
    );
    (engine, store)
}

#[tokio::test]
async fn backups_keep_the_newest_and_restore_only_into_an_empty_hoista() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let (engine, store) = hoista(&paths).await;
    store
        .put_account(&Account {
            last_discovery: None,
            id: "acct-1".into(),
            provider: hoista_server::Provider::Railway,
            label: "Workspace".into(),
            external_id: "ws".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("account");
    let source = tempfile::tempdir().expect("source");
    engine
        .projects()
        .create_project(
            &Actor::Person,
            "shop".into(),
            Source::Folder {
                path: source.path().to_str().expect("path").into(),
            },
            Mode::Managed,
        )
        .await
        .expect("project");
    engine
        .settings()
        .update_settings(
            &Actor::Person,
            SettingsPatch {
                backup_keep: Some(2),
                ..SettingsPatch::default()
            },
        )
        .await
        .expect("keep two");
    assert!(
        engine
            .settings()
            .update_settings(
                &Actor::Person,
                SettingsPatch {
                    backup_every_hours: Some(5),
                    ..SettingsPatch::default()
                },
            )
            .await
            .is_err(),
        "only 6 hours, a day, a week or never"
    );

    let mut saved = Vec::new();
    for _ in 0..3 {
        saved.push(engine.data().save_backup().await.expect("saved"));
        // Names carry the second they were saved in.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }
    let listed = engine.data().backups().await.expect("list");
    assert_eq!(listed.len(), 2, "only the newest two are kept");
    assert_eq!(listed[0].name, saved[2].name);
    assert_eq!(listed[1].name, saved[1].name);
    let file = paths.root().join("backups").join(&listed[0].name);
    let text = std::fs::read_to_string(&file).expect("file");
    assert!(text.contains("shop") && text.contains("Workspace"));
    assert!(
        !text.contains("keychain-only-token"),
        "no sign-in is in a backup"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&file).expect("meta").permissions().mode();
        assert_eq!(mode & 0o077, 0, "readable only by this account");
    }

    // Restoring needs an empty Hoista; names outside the folder are refused.
    assert!(engine.data().restore_backup(&listed[0].name).await.is_err());
    assert!(engine.data().restore_backup("../hoista.db").await.is_err());
    let fresh_paths = StatePaths::new(Some(temp.path().join("fresh"))).expect("paths");
    let (fresh, fresh_store) = hoista(&fresh_paths).await;
    std::fs::create_dir_all(fresh_paths.root().join("backups")).expect("folder");
    std::fs::copy(
        &file,
        fresh_paths.root().join("backups").join(&listed[0].name),
    )
    .expect("copy");
    fresh
        .data()
        .restore_backup(&listed[0].name)
        .await
        .expect("restored");
    let projects = fresh_store.projects(None).await.expect("projects");
    assert_eq!(projects.items[0].name, "shop");
    assert_eq!(
        projects.items[0].mode,
        Mode::Watching,
        "restored projects start watching"
    );
    assert_eq!(
        fresh_store.accounts(None).await.expect("accounts").items[0].id,
        "acct-1"
    );
    store.close().await;
    fresh_store.close().await;
}

#[tokio::test]
async fn cutting_off_every_agent_revokes_them_all_and_pauses_access() {
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let (engine, store) = hoista(&paths).await;
    for (id, transport) in [("a1", AgentTransport::Stdio), ("a2", AgentTransport::Link)] {
        store
            .put_agent(&Agent {
                id: id.into(),
                name: format!("Agent {id}"),
                transport,
                scopes: vec!["apps:read".into()],
                last_seen: "2026-11-04T00:00:00Z".into(),
                revoked: false,
                grant: AgentGrant::default(),
            })
            .await
            .expect("agent");
    }
    assert!(
        engine
            .agents()
            .stop_all_agents(&Actor::Agent { id: "a1".into() })
            .await
            .is_err(),
        "an agent can't use it"
    );
    assert_eq!(
        engine
            .agents()
            .stop_all_agents(&Actor::Person)
            .await
            .expect("stopped"),
        2
    );
    let agents = store.agents(None).await.expect("agents");
    assert!(agents.items.iter().all(|agent| agent.revoked));
    assert!(!store.settings().await.expect("settings").agents_enabled);
    // Running it again finds nobody left to cut off.
    assert_eq!(
        engine
            .agents()
            .stop_all_agents(&Actor::Person)
            .await
            .expect("again"),
        0
    );
    store.close().await;
}
