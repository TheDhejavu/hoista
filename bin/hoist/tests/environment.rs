//! Real CLI/IPC environment deployments with fixture providers and no native credentials.

use hoista_server::{
    Provider,
    application::{
        StateStore,
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
    server,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
#[derive(Default)]
struct Vault(Mutex<std::collections::HashMap<String, String>>);
#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, reference: &str) -> Result<Secret, EngineError> {
        if reference.starts_with("environment-admission:") {
            return self
                .0
                .lock()
                .expect("fixture vault")
                .get(reference)
                .cloned()
                .map(Secret::new)
                .ok_or(EngineError::NotFound);
        }
        Ok(Secret::new("fixture-provider-token".into()))
    }

    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError> {
        assert!(reference.starts_with("environment-admission:"));
        self.0
            .lock()
            .expect("fixture vault")
            .insert(reference.into(), value.expose().into());
        Ok(())
    }

    async fn delete(&self, _: &str) -> Result<(), EngineError> {
        panic!("not disconnecting")
    }
}

struct Inspector;
#[async_trait::async_trait]
impl SourceInspector for Inspector {
    async fn detect(&self, _: &Source) -> Result<Detection, EngineError> {
        Ok(Detection {
            framework: "fixture".into(),
            shape: "edge".into(),
            package_manager: None,
            build_command: vec![],
            output_directory: "dist".into(),
            env_names: vec!["SERVER_KEY".into()],
            postgres: false,
            warnings: vec![],
            prisma_migration: None,
            generated_secret_names: vec![],
            schedules: None,
        })
    }
}

struct Builder;
#[async_trait::async_trait]
impl BuildService for Builder {
    async fn secrets(&self, _: &Plan) -> Result<Vec<(String, Secret)>, EngineError> {
        Ok(vec![])
    }

    async fn build(
        &self,
        _: &Plan,
        _: &str,
        secrets: Vec<(String, Secret)>,
    ) -> Result<Artifact, EngineError> {
        Ok(Artifact {
            provenance: None,
            assets: vec![],
            worker: Some(vec![]),
            worker_name: "index.mjs".into(),
            worker_modules: vec![],
            image: None,
            secrets,
        })
    }

    async fn cleanup(&self, _: &str) -> Result<(), EngineError> {
        Ok(())
    }
}

#[derive(Default)]
struct TestProvider(Mutex<Vec<Vec<(String, String)>>>);
#[async_trait::async_trait]
impl ProviderAdapter for TestProvider {
    fn provider(&self) -> Provider {
        Provider::Cloudflare
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            deploy: true,
            logs: false,
            metrics: false,
            usage: false,
            rollback: false,
            pause: false,
            domains: false,
            secrets: true,
            discover: true,
            server: false,
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
        Ok(vec![])
    }

    async fn deploy(
        &self,
        _: &Secret,
        _: &str,
        _: &ResourcePlan,
        artifact: &Artifact,
    ) -> Result<Release, EngineError> {
        self.0.lock().expect("environment CLI fixture").push(
            artifact
                .secrets
                .iter()
                .map(|(name, value)| (name.clone(), value.expose().into()))
                .collect(),
        );
        Ok(Release {
            reference: "worker:env-fixture".into(),
            version: "fixture-version".into(),
            url: Some("https://fixture.workers.dev".into()),
            schedules: None,
        })
    }

    async fn health(&self, _: &Secret, _: &str, _: &Release) -> Result<(), EngineError> {
        Ok(())
    }
}

async fn cli(paths: &StatePaths, args: &[&str], input: &str, success: bool) -> Value {
    cli_output(paths, args, input, success).await.0
}

async fn cli_output(
    paths: &StatePaths,
    args: &[&str],
    input: &str,
    success: bool,
) -> (Value, String) {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_hoist"))
        .args(args)
        .args(["--json", "--no-input", "--state-dir"])
        .arg(paths.root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("environment CLI fixture");
    let _ = child
        .stdin
        .take()
        .expect("environment CLI fixture")
        .write_all(input.as_bytes())
        .await;
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
        .await
        .expect("environment CLI fixture")
        .expect("environment CLI fixture");
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for output in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(output).contains("private-fixture-value"));
    }
    let value = if success {
        serde_json::from_slice(&output.stdout).expect("environment CLI fixture")
    } else {
        Value::Null
    };
    (
        value,
        String::from_utf8(output.stderr).expect("CLI stderr is UTF-8"),
    )
}

#[tokio::test]
async fn cli_environment_changes_reach_deployments_without_disclosing_values() {
    let directory = tempfile::tempdir().unwrap();
    let paths = StatePaths::new(Some(directory.path().join("state"))).unwrap();
    let store = SqliteStore::open(&paths).await.unwrap();
    let host = Arc::new(TestProvider::default());
    let vault = Arc::new(Vault::default());
    let engine = Arc::new(Engine::new(
        Arc::new(store.clone()),
        vault.clone(),
        Arc::new(Inspector),
        vec![host.clone()],
        Arc::new(Builder),
    ));
    store
        .put_account(&Account {
            id: "account".into(),
            provider: Provider::Cloudflare,
            label: "Fixture".into(),
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
    let project = engine
        .projects()
        .create_project(
            &Actor::Person,
            "env-fixture".into(),
            Source::Folder {
                path: "/fixture".into(),
            },
            Mode::Managed,
        )
        .await
        .unwrap();
    let plan = engine
        .deploys()
        .plan(
            &Actor::Person,
            project.source.clone(),
            &project.name,
            None,
            None,
        )
        .await
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let serving = {
        let paths = paths.clone();
        let store = store.clone();
        let engine = engine.clone();
        let stop = stop.clone();
        tokio::spawn(async move { server::serve(&paths, store, stop, engine).await })
    };
    let owner = hoista_server::client::LocalClient::new(&paths).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while owner.health().await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let initial = cli(&paths, &["doctor", "environment-key"], "", true).await;
    assert_eq!(initial["state"], "not_initialized");
    assert_eq!(initial["retry_setup_allowed"], false);
    let pending_reference = format!("environment-admission:{}", uuid::Uuid::new_v4());
    store
        .reserve_environment_admission_key(&pending_reference)
        .await
        .unwrap();
    let review = cli(&paths, &["doctor", "environment-key"], "", true).await;
    assert_eq!(review["retry_setup_allowed"], true);
    assert_eq!(review["pending_references"], json!([pending_reference]));
    let token = review["review_token"].as_str().unwrap().to_owned();
    let retry_setup = ["doctor", "environment-key", "--retry-setup", token.as_str()];
    let repaired = cli(&paths, &retry_setup, "", true).await;
    assert_eq!(repaired["retired_attempts"], 1);
    assert_eq!(repaired["pending_references"], json!([]));
    assert!(vault.0.lock().unwrap().is_empty());
    cli(&paths, &retry_setup, "", false).await;

    let shown = cli(&paths, &["plan", "show", &plan.id], "", true).await;
    assert_eq!(shown["id"], plan.id);
    let resource_name = &plan.resources[0].name;
    let original_account = &plan.resources[0].account_id;
    let mut second_account = store.account(original_account).await.unwrap().unwrap();
    second_account.id = "second-account".into();
    second_account.label = "Second".into();
    second_account.external_id = "second-remote".into();
    store.put_account(&second_account).await.unwrap();
    let set = |extra: &'static [&'static str]| {
        let mut args = vec!["plan", "set", plan.id.as_str(), resource_name.as_str()];
        args.extend_from_slice(extra);
        args
    };
    let changed = cli(&paths, &set(&["--account", "second-account"]), "", true).await;
    assert_eq!(changed["resources"][0]["account_id"], "second-account");
    let mut back = set(&[]);
    back.extend(["--account", original_account.as_str()]);
    cli(&paths, &back, "", true).await;
    let changed = cli(&paths, &set(&["--region", "fixture-region"]), "", true).await;
    assert_eq!(changed["resources"][0]["region"], "fixture-region");
    let cleared = cli(&paths, &set(&["--region", "-"]), "", true).await;
    assert!(cleared["resources"][0]["region"].is_null());
    cli(
        &paths,
        &[
            "plan",
            "set",
            &plan.id,
            "no-such-resource",
            "--account",
            original_account,
        ],
        "",
        false,
    )
    .await;
    cli(&paths, &set(&["--account", "missing-account"]), "", false).await;
    cli(&paths, &set(&[]), "", false).await;
    let edit = ["plan", "edit", plan.id.as_str()];
    let mut invalid = shown.clone();
    invalid["framework"] = json!("different-framework");
    cli(&paths, &edit, &invalid.to_string(), false).await;
    invalid = shown.clone();
    invalid["unexpected_secret"] = json!("private-fixture-value");
    cli(&paths, &edit, &invalid.to_string(), false).await;
    cli(&paths, &edit, &shown.to_string(), false).await;
    let fresh = cli(&paths, &["plan", "show", &plan.id], "", true).await;
    assert!(fresh["revision"].as_i64().unwrap() > shown["revision"].as_i64().unwrap());
    let edited = cli(&paths, &edit, &fresh.to_string(), true).await;
    assert_eq!(
        edited["revision"].as_i64().unwrap(),
        fresh["revision"].as_i64().unwrap() + 1
    );
    let reviewed_revision = edited["revision"].as_i64().unwrap().to_string();
    let reviewed = |extra: &[&'static str]| {
        let mut args = vec![
            "deploy",
            "--project",
            project.id.as_str(),
            "--plan",
            plan.id.as_str(),
        ];
        args.extend_from_slice(extra);
        args
    };
    cli(&paths, &reviewed(&["--plan-revision", "0"]), "", false).await;
    cli(
        &paths,
        &[
            "env",
            "set",
            &project.id,
            "SERVER_KEY",
            "--value-stdin",
            "--plan",
            &plan.id,
            "--plan-revision",
            "0",
        ],
        "private-fixture-value",
        false,
    )
    .await;
    cli(&paths, &reviewed(&["--plan-revision", "-1"]), "", false).await;
    for (tool, args) in [
        (
            "deploy",
            json!({"project_id":project.id,"plan_id":plan.id,"expected_plan_revision":0}),
        ),
        (
            "replace_environment",
            json!({
                "project_id": project.id,
                "plan_id": plan.id,
                "expected_plan_revision": 0,
                "changes": [{"name": "SERVER_KEY", "value": "private-fixture-value"}],
            }),
        ),
    ] {
        let result = hoista_server::mcp::dispatch(
            &engine,
            &Actor::Person,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": tool, "arguments": args},
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["result"]["isError"], true);
        assert!(!result.to_string().contains("private-fixture-value"));
        assert!(
            result.to_string().contains("reviewed plan changed"),
            "{result}"
        );
    }
    for revision in [-1_i64, 9_007_199_254_740_992] {
        assert!(
            owner
                .post_json(
                    &format!("/projects/{}/deploys", project.id),
                    &json!({"plan_id":plan.id,"expected_plan_revision":revision})
                )
                .await
                .is_err()
        );
    }
    assert!(
        store
            .deploys(&project.id, None)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        host.0.lock().unwrap().is_empty(),
        "Plan edits must not deploy"
    );
    let automatic = cli(&paths, &reviewed(&[]), "", true).await;
    let auto_key = automatic["request_key"].as_str().unwrap().to_owned();
    assert!(uuid::Uuid::parse_str(&auto_key).is_ok());
    assert_eq!(automatic["plan_revision"].to_string(), reviewed_revision);
    let mut again = reviewed(&["--plan-revision"]);
    again.extend([
        reviewed_revision.as_str(),
        "--request-key",
        auto_key.as_str(),
    ]);
    let retried = cli(&paths, &again, "", true).await;
    assert_eq!(automatic["id"], retried["id"]);
    let key = "550e8400-e29b-41d4-a716-446655440002";
    let mut keyed = reviewed(&["--request-key", key, "--plan-revision"]);
    keyed.push(reviewed_revision.as_str());
    let first_keyed = cli(&paths, &keyed, "", true).await;
    let second_keyed = cli(&paths, &keyed, "", true).await;
    assert_eq!(first_keyed["id"], second_keyed["id"]);
    // The same key with the same inputs is the same deploy, revision given or not.
    let unrevised = cli(&paths, &reviewed(&["--request-key", key]), "", true).await;
    assert_eq!(first_keyed["id"], unrevised["id"]);
    let mut changed_intent = keyed.clone();
    changed_intent.extend(["--message", "Changed intent"]);
    cli(&paths, &changed_intent, "", false).await;
    let mcp_key = "550e8400-e29b-41d4-a716-446655440003";
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"deploy","arguments":{"project_id":project.id,"plan_id":plan.id,"expected_plan_revision":edited["revision"],"request_key":mcp_key}}});
    let one = hoista_server::mcp::dispatch(&engine, &Actor::Person, request.clone())
        .await
        .unwrap();
    let two = hoista_server::mcp::dispatch(&engine, &Actor::Person, request)
        .await
        .unwrap();
    assert_ne!(one["result"]["isError"], true, "{one}");
    assert_eq!(one, two);
    let inspected = hoista_server::mcp::dispatch(
        &engine,
        &Actor::Person,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "get_deployment_request", "arguments": {"request_key": mcp_key}},
        }),
    )
    .await
    .unwrap();
    assert_ne!(inspected["result"]["isError"], true, "{inspected}");
    let receipt: Value =
        serde_json::from_str(inspected["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(receipt["request_key"], mcp_key);
    assert!(!receipt.to_string().contains("fingerprint"));
    cli(
        &paths,
        &["deploys", "request", "550e8400-e29b-41d4-a716-446655440099"],
        "",
        false,
    )
    .await;
    assert_eq!(
        store.deploys(&project.id, None).await.unwrap().items.len(),
        3
    );
    let worker = tokio::spawn(engine.clone().run_deployments(stop.clone()));
    assert_eq!(
        cli(&paths, &["env", "list", &project.name], "", true).await,
        json!([])
    );
    let env_file = directory.path().join("fixture.env");
    let env_path = env_file.to_str().unwrap().to_owned();
    let bulk = "SERVER_KEY=\"private-fixture-value-bulk\\nsecond line\"\nOPTIONAL=\n";
    for one in [true, false] {
        let (args, input): (Vec<&str>, String) = if one {
            (
                vec![
                    "env",
                    "set",
                    &project.name,
                    "SERVER_KEY",
                    "--value-stdin",
                    "--plan",
                    &plan.id,
                    "--plan-revision",
                    &reviewed_revision,
                    "--message",
                    "Rotate fixture",
                ],
                "private-fixture-value\n".to_owned(),
            )
        } else {
            std::fs::write(&env_file, bulk).unwrap();
            (
                vec![
                    "env",
                    "set",
                    &project.id,
                    "--file",
                    &env_path,
                    "--plan",
                    &plan.id,
                ],
                String::new(),
            )
        };
        let queued = cli(&paths, &args, &input, true).await;
        let retry_key = queued["request_key"].as_str().unwrap().to_owned();
        assert_eq!(queued["channel"], "cli");
        let snapshot_id = queued["plan_id"].as_str().unwrap();
        let snapshot = cli(&paths, &["plan", "show", snapshot_id], "", true).await;
        assert_eq!(snapshot["revision"], edited["revision"]);
        cli(
            &paths,
            &["plan", "edit", snapshot_id],
            &snapshot.to_string(),
            false,
        )
        .await;
        let id = queued["id"].as_str().unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let deploy = store.deploy(id).await.unwrap().unwrap();
                if !matches!(deploy.status, DeployStatus::Queued | DeployStatus::Running) {
                    break deploy;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(result.status, DeployStatus::Live, "{:?}", result.steps);
        let mut retry_args = args.clone();
        retry_args.extend(["--request-key", retry_key.as_str()]);
        if !retry_args.contains(&"--plan-revision") {
            retry_args.extend(["--plan-revision", &reviewed_revision]);
        }
        let replay = cli(&paths, &retry_args, &input, true).await;
        assert_eq!(replay["id"], queued["id"]);
        assert_eq!(replay["status"], "live");
        let retained_keys = std::mem::take(&mut *vault.0.lock().unwrap());
        let missing = cli(&paths, &["doctor", "environment-key"], "", true).await;
        assert_eq!(missing["state"], "missing");
        assert_eq!(missing["retry_setup_allowed"], false);
        cli(
            &paths,
            &[
                "doctor",
                "environment-key",
                "--retry-setup",
                missing["review_token"].as_str().unwrap(),
            ],
            "",
            false,
        )
        .await;
        let inspected = cli(&paths, &["deploys", "request", &retry_key], "", true).await;
        assert_eq!(inspected["deploy_id"], queued["id"]);
        assert_eq!(inspected["deployment"]["status"], "live");
        assert!(!inspected.to_string().contains("fingerprint"));
        assert!(!inspected.to_string().contains("commitment"));
        assert!(
            vault.0.lock().unwrap().is_empty(),
            "Inspection must not initialize credentials"
        );
        *vault.0.lock().unwrap() = retained_keys;
        // The same key with other values is refused, never a second deploy.
        let changed = if one {
            format!("{input}changed")
        } else {
            std::fs::write(&env_file, bulk.replace("bulk", "bulk-changed")).unwrap();
            String::new()
        };
        cli(&paths, &retry_args, &changed, false).await;
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("private-fixture-value")
        );
    }
    {
        let seen = host.0.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].contains(&("SERVER_KEY".into(), "private-fixture-value\n".into())));
        assert!(seen[1].contains(&(
            "SERVER_KEY".into(),
            "private-fixture-value-bulk\nsecond line".into()
        )));
        assert!(seen[1].contains(&("OPTIONAL".into(), String::new())));
    }
    assert!(!paths.root().join("diagnostics").exists());
    let report = cli(&paths, &["diagnostics"], "", true).await;
    assert!(
        !paths.root().join("diagnostics").exists(),
        "Preview must not write a report"
    );
    assert_eq!(report["format_version"], 1);
    assert_eq!(
        report["projects"][0]["deployments"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert!(!report.to_string().contains(&project.id));
    let receipt = cli(&paths, &["diagnostics", "--save"], "", true).await;
    let path = std::path::Path::new(receipt["path"].as_str().unwrap());
    assert!(path.starts_with(paths.root().join("diagnostics")));
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len() as u64, receipt["bytes"].as_u64().unwrap());
    assert!(!String::from_utf8_lossy(&bytes).contains("private-fixture-value"));
    assert!(
        owner
            .post_json("/diagnostics", &json!({"secret":"private-fixture-value"}))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_dir(paths.root().join("diagnostics"))
            .unwrap()
            .count(),
        1
    );
    let listed = cli(&paths, &["env", "list", &project.id], "", true).await;
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "SERVER_KEY")
    );
    // A bad .env file is named by line, never by value; a name it lacks is named.
    std::fs::write(&env_file, "SERVER_KEY=ok\nprivate-fixture-value\n").unwrap();
    let (_, said) = cli_output(
        &paths,
        &["env", "set", &project.name, "--file", &env_path],
        "",
        false,
    )
    .await;
    assert!(said.contains("Line 2"), "{said}");
    std::fs::write(&env_file, "SERVER_KEY=private-fixture-value\n").unwrap();
    let (_, said) = cli_output(
        &paths,
        &[
            "env",
            "set",
            &project.name,
            "MISSING_NAME",
            "--file",
            &env_path,
        ],
        "",
        false,
    )
    .await;
    assert!(said.contains("MISSING_NAME"), "{said}");
    engine
        .projects()
        .set_mode(&project.id, Mode::Watching)
        .await
        .unwrap();
    cli(
        &paths,
        &["env", "set", &project.id, "SERVER_KEY", "--value-stdin"],
        "private-fixture-value",
        false,
    )
    .await;
    assert_eq!(host.0.lock().unwrap().len(), 2);
    assert!(
        !format!("{:?}", store.events_after(0).await.unwrap()).contains("private-fixture-value")
    );
    stop.cancel();
    worker.await.unwrap().unwrap();
    serving.await.unwrap().unwrap();
    let mut persisted = vec![std::fs::read(paths.database()).expect("read live fixture database")];
    match std::fs::read(paths.root().join("hoista.db-wal")) {
        Ok(bytes) => persisted.push(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("Cannot inspect fixture WAL: {error}"),
    }
    for bytes in persisted {
        assert!(!String::from_utf8_lossy(&bytes).contains("private-fixture-value"));
        for key in vault.0.lock().unwrap().values() {
            assert!(
                !bytes
                    .windows(key.len())
                    .any(|window| window == key.as_bytes())
            );
        }
    }
    store.close().await;
}
