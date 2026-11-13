//! Opt-in real build where the inherited `PATH` has no Node, as when a login
//! manager starts the daemon, so tools come from the saved tool paths. Makes
//! no provider calls.

use hoista_server::{
    Provider,
    application::{deploys::planning::plan, ports::build::BuildService},
    domain::{Secret, model::*},
    infrastructure::{build::LocalBuilder, inspection::detect, paths::StatePaths, tool_paths},
};

#[tokio::test]
#[ignore = "requires explicit service fixture state and Node/npm; runs a real Vite build using saved tool paths"]
async fn service_tools_build_from_saved_path() {
    let state =
        std::env::var_os("HOISTA_SERVICE_TEST_STATE").expect("Set isolated service fixture state");
    let paths = StatePaths::new(Some(state.into())).unwrap();
    let inherited = std::env::var_os("PATH").unwrap();
    let expected = std::env::var_os("HOISTA_SERVICE_TEST_INHERITED_PATH")
        .expect("Set explicit minimal inherited PATH");
    assert_eq!(inherited, expected);
    assert!(
        std::process::Command::new("node")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err(),
        "The inherited environment must not already find Node"
    );
    let saved = tool_paths::read(&paths)
        .unwrap()
        .expect("Configure tools before launching fixture");
    assert!(saved.find("node").is_some() && saved.find("npm").is_some());
    tool_paths::initialize(&paths).unwrap();
    assert!(tool_paths::initialize(&paths).is_err());
    // Initialization must not mutate the global environment; only owned children use saved PATH.
    assert_eq!(std::env::var_os("PATH").unwrap(), inherited);
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/apps/vite-spa");
    let account = Account {
        last_discovery: None,
        id: "service-fixture".into(),
        provider: Provider::Cloudflare,
        label: "No provider calls".into(),
        external_id: "unused".into(),
        status: AccountStatus::Connected,
        scopes: vec![],
        connected_at: "2026-11-02T00:00:00Z".into(),
        credential: None,
        in_1password: false,
    };
    let planned = plan(
        Source::Folder {
            path: source.to_str().unwrap().into(),
        },
        "service-fixture",
        detect(&source).unwrap(),
        &[account],
        &providers(),
        None,
        None,
    )
    .unwrap();
    let builder = LocalBuilder::new(paths.root());
    let id = uuid::Uuid::new_v4().to_string();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        builder.build(
            &planned,
            &id,
            vec![(
                "VITE_PUBLIC_LABEL".into(),
                Secret::new("https://service-fixture.example".into()),
            )],
        ),
    )
    .await;
    builder.cleanup(&id).await.unwrap();
    let artifact = result
        .expect("Bounded service build")
        .expect("Actual Vite build with configured executable lookup");
    assert!(
        artifact
            .assets
            .iter()
            .any(|asset| asset.path.ends_with("index.html"))
    );
    assert!(
        artifact
            .assets
            .iter()
            .any(|asset| asset.path.ends_with(".js")
                && String::from_utf8_lossy(&asset.bytes)
                    .contains("https://service-fixture.example"))
    );
    assert!(artifact.provenance.is_some());
    assert!(artifact.worker.is_none());
    let receipt = serde_json::json!({
        "built": true,
        "framework": "vite",
        "asset_count": artifact.assets.len(),
        "inherited_path_unchanged": true,
    });
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.root().join("service-build-receipt.json"))
        .unwrap();
    file.write_all(serde_json::to_string(&receipt).unwrap().as_bytes())
        .unwrap();
    file.sync_all().unwrap();
}

/// Every provider Hoista connects to, as plans are made against them.
fn providers() -> hoista_server::application::deploys::planning::Providers {
    hoista_server::application::deploys::planning::Providers::new(
        hoista_server::infrastructure::providers::provider_capabilities(),
    )
}
