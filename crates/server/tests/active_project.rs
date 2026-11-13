//! The project of the folder an agent works in, the checks before making
//! another, plans inside a project, and agents answering unclear changes.

use hoista_server::{
    application::{
        engine::{Actor, Engine},
        ports::{
            catalog::{CatalogRepository, EngineError},
            vault::TokenVault,
        },
    },
    domain::{Secret, model::*},
    infrastructure::{
        build::LocalBuilder, inspection::LocalInspector, paths::StatePaths, store::SqliteStore,
    },
    mcp,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);
#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, key: &str) -> Result<Secret, EngineError> {
        self.0
            .lock()
            .expect("vault")
            .get(key)
            .map(|s| Secret::new(s.clone()))
            .ok_or(EngineError::NotFound)
    }

    async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
        self.0
            .lock()
            .expect("vault")
            .insert(key.into(), value.expose().into());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), EngineError> {
        self.0.lock().expect("vault").remove(key);
        Ok(())
    }
}

async fn setup() -> (tempfile::TempDir, SqliteStore, Engine) {
    let http =
        hoista_server::infrastructure::providers::http::ProviderHttp::new().expect("fixture");
    let temp = tempfile::tempdir().expect("temp");
    let paths = StatePaths::new(Some(temp.path().join("state"))).expect("paths");
    let store = SqliteStore::open(&paths).await.expect("store");
    let engine = Engine::new(
        Arc::new(store.clone()),
        Arc::new(Vault::default()),
        Arc::new(LocalInspector),
        hoista_server::infrastructure::providers::adapters(&http),
        Arc::new(LocalBuilder::new(paths.root())),
    )
    .with_owner_check(Arc::new(
        hoista_server::application::ports::owner_check::FixedOwnerCheck(true),
    ));
    (temp, store, engine)
}

async fn agent(engine: &Engine) -> Actor {
    let (agent, _) = engine
        .agents()
        .register_agent(
            "Claude".into(),
            vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
            AgentTransport::Stdio,
        )
        .await
        .expect("agent");
    Actor::Agent { id: agent.id }
}

/// A tool call from a coding tool working in `folder`.
async fn call_in(
    engine: &Engine,
    actor: &Actor,
    folder: Option<&Path>,
    name: &str,
    args: Value,
) -> Value {
    let reply = mcp::dispatch_in(
        engine,
        actor,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}),
        folder.map(Path::to_path_buf),
    )
    .await
    .expect("reply");
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("null")
        .to_owned();
    json!({
        "error": reply["result"]["isError"] == true,
        "body": serde_json::from_str::<Value>(&text).unwrap_or(json!(text)),
    })
}

fn away_check(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let away = temp.path().join("nowhere");
    std::fs::create_dir_all(&away).expect("fixture");
    away
}

fn folder(path: &Path) -> Source {
    Source::Folder {
        path: path.display().to_string(),
    }
}

fn site(dir: &Path) {
    std::fs::create_dir_all(dir).expect("fixture");
    std::fs::write(dir.join("index.html"), "hello").expect("fixture");
}

#[tokio::test]
async fn a_folder_inside_a_project_is_that_project() {
    let (temp, store, engine) = setup().await;
    let actor = agent(&engine).await;
    let root = temp.path().join("sentinel");
    let api = root.join("apps/api");
    site(&api);
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel".into(),
            folder(&root),
            Mode::Managed,
        )
        .await
        .expect("fixture");

    // The agent works in a subfolder: get_my_access names the project, and
    // tools about one project use it when project_id is left out.
    let access = call_in(&engine, &actor, Some(&api), "get_my_access", json!({})).await;
    assert_eq!(
        access["body"]["active_project"]["project_id"],
        json!(project.id)
    );
    assert_eq!(
        access["body"]["active_project"]["found_by"],
        "source_folder"
    );
    let read = call_in(&engine, &actor, Some(&api), "list_domains", json!({})).await;
    assert!(!read["error"].as_bool().expect("fixture"), "{read}");
    let read = call_in(
        &engine,
        &actor,
        Some(&away_check(&temp)),
        "list_domains",
        json!({}),
    )
    .await;
    assert!(read["error"].as_bool().expect("fixture"), "{read}");
    // Elsewhere there's none, and nothing is guessed.
    let away = temp.path().join("elsewhere");
    std::fs::create_dir_all(&away).expect("fixture");
    let access = call_in(&engine, &actor, Some(&away), "get_my_access", json!({})).await;
    assert_eq!(access["body"]["active_project"], Value::Null);
    let access = call_in(&engine, &actor, None, "get_my_access", json!({})).await;
    assert_eq!(access["body"]["active_project"], Value::Null);

    // Making another project here is refused, unless the person said so.
    let again = json!({"name": "sentinel-edge"});
    let refused = call_in(&engine, &actor, Some(&api), "create_project", again.clone()).await;
    assert!(refused["error"].as_bool().expect("fixture"));
    assert!(
        refused["body"].to_string().contains(&project.id),
        "{refused}"
    );
    // Even without a folder, a source inside the project is refused.
    let inside = json!({"name": "sentinel-api", "source": {"kind": "folder", "path": api.display().to_string()}});
    let refused = call_in(&engine, &actor, None, "create_project", inside).await;
    assert!(refused["error"].as_bool().expect("fixture"), "{refused}");
    // A reason alone isn't enough; confirm_new with a reason is.
    let half = json!({"name": "sentinel-edge", "reason": "the person asked for it"});
    assert!(
        call_in(&engine, &actor, Some(&api), "create_project", half).await["error"]
            .as_bool()
            .expect("fixture")
    );
    let sure = json!({
        "name": "sentinel-edge",
        "confirm_new": true,
        "reason": "The person said the edge worker is a separate project.",
    });
    let made = call_in(&engine, &actor, Some(&api), "create_project", sure).await;
    assert!(!made["error"].as_bool().expect("fixture"), "{made}");
    assert_eq!(store.projects(None).await.expect("fixture").items.len(), 2);

    // The tool list lets project_id be left out.
    let tools = mcp::tools();
    for name in ["use_project", "resolve_operation"] {
        assert!(tools.iter().any(|tool| tool["name"] == name), "{name}");
    }
    let deploy = tools
        .iter()
        .find(|tool| tool["name"] == "deploy")
        .expect("fixture");
    assert!(
        !deploy["inputSchema"]["required"]
            .as_array()
            .expect("fixture")
            .contains(&json!("project_id"))
    );
    store.close().await;
}

#[tokio::test]
async fn use_project_names_the_project_in_the_folder() {
    let (temp, store, engine) = setup().await;
    let actor = agent(&engine).await;
    let repo = temp.path().join("checkout");
    let deep = repo.join("services/worker");
    std::fs::create_dir_all(&deep).expect("fixture");
    std::fs::create_dir_all(repo.join(".git")).expect("fixture");
    // A project with no source: nothing ties the folder to it yet.
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "pieces".into(),
            Source::Unlinked,
            Mode::Managed,
        )
        .await
        .expect("fixture");
    let access = call_in(&engine, &actor, Some(&deep), "get_my_access", json!({})).await;
    assert_eq!(access["body"]["active_project"], Value::Null);

    // Only a coding tool with a folder can name it.
    let none = call_in(
        &engine,
        &actor,
        None,
        "use_project",
        json!({"project_id": project.id}),
    )
    .await;
    assert!(none["error"].as_bool().expect("fixture"));
    let used = call_in(
        &engine,
        &actor,
        Some(&deep),
        "use_project",
        json!({"project_id": project.id}),
    )
    .await;
    assert!(!used["error"].as_bool().expect("fixture"), "{used}");
    // Written at the repository's root, kept out of git.
    let file = std::fs::read_to_string(repo.join(".hoista/project.json")).expect("fixture");
    assert!(file.contains(&project.id));
    assert_eq!(
        std::fs::read_to_string(repo.join(".hoista/.gitignore")).expect("fixture"),
        "*\n"
    );
    let access = call_in(&engine, &actor, Some(&deep), "get_my_access", json!({})).await;
    assert_eq!(
        access["body"]["active_project"]["project_id"],
        json!(project.id)
    );
    assert_eq!(access["body"]["active_project"]["found_by"], "project_file");
    // A file naming a project that's gone names nothing.
    std::fs::write(
        repo.join(".hoista/project.json"),
        r#"{"project_id":"gone"}"#,
    )
    .expect("fixture");
    let access = call_in(&engine, &actor, Some(&deep), "get_my_access", json!({})).await;
    assert_eq!(access["body"]["active_project"], Value::Null);
    store.close().await;
}

#[tokio::test]
async fn a_clone_of_the_projects_repository_is_that_project() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let (temp, store, engine) = setup().await;
    let actor = agent(&engine).await;
    let clone = temp.path().join("clone");
    std::fs::create_dir_all(&clone).expect("fixture");
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&clone)
                .output()
                .expect("fixture")
                .status
                .success()
        );
    };
    git(&["init", "-q"]);
    git(&["remote", "add", "origin", "git@github.com:sam/sentinel.git"]);
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel".into(),
            Source::Github {
                repository: "sam/sentinel".into(),
                commit: "a".repeat(40),
            },
            Mode::Watching,
        )
        .await
        .expect("fixture");
    let access = call_in(&engine, &actor, Some(&clone), "get_my_access", json!({})).await;
    assert_eq!(
        access["body"]["active_project"]["project_id"],
        json!(project.id)
    );
    assert_eq!(access["body"]["active_project"]["found_by"], "repository");
    store.close().await;
}

#[tokio::test]
async fn plans_inside_a_project_come_from_its_folder_or_one_inside_it() {
    let (temp, store, engine) = setup().await;
    let root = temp.path().join("mono");
    let web = root.join("apps/web");
    site(&web);
    site(&root);
    let outside = temp.path().join("other");
    site(&outside);
    store
        .put_account(&Account {
            last_discovery: None,
            id: "cf".into(),
            provider: hoista_server::Provider::Cloudflare,
            label: "fixture".into(),
            external_id: "remote".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: chrono::Utc::now().to_rfc3339(),
            credential: None,
            in_1password: false,
        })
        .await
        .expect("fixture");
    let project = engine
        .projects()
        .create_project(&Actor::Person, "mono".into(), folder(&root), Mode::Managed)
        .await
        .expect("fixture");
    // One app of the monorepo plans on its own.
    let plan = engine
        .projects()
        .plan_in_project(&Actor::Person, &project.id, folder(&web), "mono-web", None)
        .await
        .expect("fixture");
    assert_eq!(plan.source, folder(&web));
    assert!(plan.source.within(&project.source));
    // A folder outside the project is refused before anything is planned.
    assert!(
        engine
            .projects()
            .plan_in_project(&Actor::Person, &project.id, folder(&outside), "x", None)
            .await
            .is_err()
    );
    // The same through MCP, with the folder's project filled in.
    let actor = agent(&engine).await;
    let planned = call_in(
        &engine,
        &actor,
        Some(&web),
        "plan_deploy",
        json!({"source": {"kind": "folder", "path": web.display().to_string()}, "name": "mono-web"}),
    )
    .await;
    assert!(!planned["error"].as_bool().expect("fixture"), "{planned}");
    store.close().await;
}

#[test]
fn a_subfolder_is_within_its_projects_folder() {
    let at = |path: &str| Source::Folder { path: path.into() };
    assert!(at("/code/mono/apps/api").within(&at("/code/mono")));
    assert!(at("/code/mono").within(&at("/code/mono/")));
    assert!(!at("/code/mono-other").within(&at("/code/mono")));
    assert!(!at("/code").within(&at("/code/mono")));
    // macOS shows /tmp as /private/tmp.
    assert!(at("/private/tmp/sentinel/apps/edge").within(&at("/tmp/sentinel")));
    assert!(!at("/code/mono").within(&Source::Unlinked));
}

#[tokio::test]
async fn an_agent_answers_an_unclear_change_with_evidence() {
    let (_temp, store, engine) = setup().await;
    let actor = agent(&engine).await;
    let operation = Operation {
        id: "op1".into(),
        provider: hoista_server::Provider::Vercel,
        account: "acct".into(),
        kind: "command".into(),
        target: "vercel dns add".into(),
        state: OperationState::Unclear,
        repeats: true,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        message: None,
        key: "k1".into(),
    };
    store.put_operation(&operation).await.expect("fixture");
    let short = call_in(
        &engine,
        &actor,
        None,
        "resolve_operation",
        json!({"operation_id": "op1", "happened": false, "evidence": "no"}),
    )
    .await;
    assert!(short["error"].as_bool().expect("fixture"));
    let answered = call_in(
        &engine,
        &actor,
        None,
        "resolve_operation",
        json!({
            "operation_id": "op1",
            "happened": false,
            "evidence": "vercel dns ls example.com: no record for mail",
        }),
    )
    .await;
    assert!(!answered["error"].as_bool().expect("fixture"), "{answered}");
    assert_eq!(answered["body"]["state"], "did_not_happen");
    assert!(
        answered["body"]["message"]
            .as_str()
            .expect("fixture")
            .contains("no record for mail")
    );
    // Answered once; it stays answered.
    let again = call_in(
        &engine,
        &actor,
        None,
        "resolve_operation",
        json!({
            "operation_id": "op1",
            "happened": true,
            "evidence": "vercel dns ls example.com: it is there",
        }),
    )
    .await;
    assert!(again["error"].as_bool().expect("fixture"));
    // A read-only connection can't answer.
    let (reader, _) = engine
        .agents()
        .register_agent(
            "Reader".into(),
            vec!["apps:read".into()],
            AgentTransport::Stdio,
        )
        .await
        .expect("fixture");
    store
        .put_operation(&Operation {
            id: "op2".into(),
            key: "k2".into(),
            ..operation
        })
        .await
        .expect("fixture");
    let refused = call_in(
        &engine,
        &Actor::Agent { id: reader.id },
        None,
        "resolve_operation",
        json!({
            "operation_id": "op2",
            "happened": false,
            "evidence": "vercel dns ls example.com: nothing",
        }),
    )
    .await;
    assert!(refused["error"].as_bool().expect("fixture"));
    store.close().await;
}

#[tokio::test]
async fn a_plan_uses_the_projects_database_instead_of_a_new_one() {
    let (temp, store, engine) = setup().await;
    let root = temp.path().join("uptime");
    let api = root.join("apps/api");
    std::fs::create_dir_all(&api).expect("fixture");
    std::fs::write(
        api.join("package.json"),
        r#"{"name":"api","scripts":{"start":"node server.js"},"dependencies":{"express":"4.19.0","pg":"8.11.0"}}"#,
    )
    .expect("fixture");
    std::fs::write(
        api.join("package-lock.json"),
        r#"{"lockfileVersion":3,"packages":{}}"#,
    )
    .expect("fixture");
    std::fs::write(
        api.join("server.js"),
        "require('http').createServer().listen(process.env.PORT)",
    )
    .expect("fixture");
    for (id, provider) in [
        ("fly", hoista_server::Provider::Fly),
        ("neon", hoista_server::Provider::Neon),
    ] {
        store
            .put_account(&Account {
                last_discovery: None,
                id: id.into(),
                provider,
                label: "fixture".into(),
                external_id: "remote".into(),
                scopes: vec![],
                status: AccountStatus::Connected,
                connected_at: chrono::Utc::now().to_rfc3339(),
                credential: None,
                in_1password: false,
            })
            .await
            .expect("fixture");
    }
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "sentinel".into(),
            folder(&root),
            Mode::Managed,
        )
        .await
        .expect("fixture");
    // Without the project, the plan proposes a database named after the app.
    let plain = engine
        .deploys()
        .plan(
            &Actor::Person,
            folder(&api),
            "sentinel-uptime-api",
            None,
            None,
        )
        .await
        .expect("fixture");
    assert!(
        plain
            .resources
            .iter()
            .any(|r| r.name == "sentinel-uptime-api-db"),
        "{plain:?}"
    );
    // The project already has a database it watches.
    store
        .put_resource(&Resource {
            id: "db".into(),
            project_id: Some(project.id.clone()),
            account_id: "neon".into(),
            provider: hoista_server::Provider::Neon,
            provider_ref: "neon-1".into(),
            name: "sentinel-db".into(),
            kind: ResourceKind::Database,
            mode: Mode::Watching,
            region: None,
            status: ResourceStatus::Live,
            url: None,
            group: None,
        })
        .await
        .expect("fixture");
    let plan = engine
        .projects()
        .plan_in_project(
            &Actor::Person,
            &project.id,
            folder(&api),
            "sentinel-uptime-api",
            None,
        )
        .await
        .expect("fixture");
    assert!(
        plan.resources
            .iter()
            .all(|r| r.kind != ResourceKind::Database),
        "{plan:?}"
    );
    assert!(plan.resources.iter().all(|r| r.depends_on.is_empty()));
    assert!(plan.warnings.iter().any(|w| w.contains("sentinel-db")));
    store.close().await;
}
