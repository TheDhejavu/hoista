//! Cron source inference and plan validation run without provider credentials.

use hoista_server::{
    Provider,
    application::deploys::planning::{plan, release_order},
    domain::model::*,
    infrastructure::inspection::detect,
};

#[test]
fn cron_configuration_preserves_omission_and_explicit_empty_lists() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("package.json"),
        r#"{"dependencies":{"hono":"4"}}"#,
    )
    .unwrap();
    assert!(detect(root.path()).unwrap().schedules.is_none());
    std::fs::write(
        root.path().join("wrangler.toml"),
        "[triggers]\ncrons = []\n",
    )
    .unwrap();
    assert_eq!(detect(root.path()).unwrap().schedules, Some(vec![]));
    std::fs::write(
        root.path().join("wrangler.toml"),
        "[triggers]\ncrons = ['*/5 * * * *']\n",
    )
    .unwrap();
    let detected = detect(root.path()).unwrap();
    assert_eq!(detected.schedules, Some(vec!["*/5 * * * *".into()]));
    std::fs::write(
        root.path().join("wrangler.toml"),
        "[triggers]\ncrons = ['60 * * * *']\n",
    )
    .unwrap();
    assert!(detect(root.path()).is_err());
    std::fs::remove_file(root.path().join("wrangler.toml")).unwrap();
    std::fs::write(
        root.path().join("wrangler.json"),
        r#"{"triggers":{"crons":["0 2 * * 1"]}}"#,
    )
    .unwrap();
    assert_eq!(
        detect(root.path()).unwrap().schedules,
        Some(vec!["0 2 * * 1".into()])
    );
    std::fs::write(
        root.path().join("wrangler.json"),
        r#"{"triggers":{"crons":null}}"#,
    )
    .unwrap();
    assert!(detect(root.path()).is_err());
}

#[test]
fn jobs_plan_on_cloudflare_and_unsupported_cron_targets_fail() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("wrangler.toml"),
        "[triggers]\ncrons = ['0 2 * * *']\n",
    )
    .unwrap();
    let account = Account {
        last_discovery: None,
        id: "cf".into(),
        provider: Provider::Cloudflare,
        external_id: "external".into(),
        label: "Cloudflare".into(),
        status: AccountStatus::Connected,
        scopes: vec![],
        connected_at: "2026-11-02T00:00:00Z".into(),
        credential: None,
        in_1password: false,
    };
    let source = Source::Folder {
        path: root.path().to_string_lossy().into(),
    };
    let mut result = plan(
        source.clone(),
        "nightly",
        detect(root.path()).unwrap(),
        std::slice::from_ref(&account),
        &providers(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(result.framework, "worker");
    std::fs::write(
        root.path().join("wrangler.toml"),
        "[triggers]\ncrons = []\n",
    )
    .unwrap();
    assert_eq!(detect(root.path()).unwrap().shape, "job");
    assert_eq!(result.resources[0].kind, ResourceKind::Job);
    assert_eq!(
        result.resources[0].schedules,
        Some(vec!["0 2 * * *".into()])
    );
    assert!(release_order(&result.resources, &providers()).is_ok());
    assert!(
        plan(
            source,
            "nightly",
            detect(root.path()).unwrap(),
            &[account],
            &providers(),
            Some(Provider::Fly),
            None
        )
        .is_err()
    );
    result.resources[0].provider = Provider::Fly;
    assert!(release_order(&result.resources, &providers()).is_err());
    result.resources[0].provider = Provider::Cloudflare;
    result.resources[0].kind = ResourceKind::Database;
    assert!(release_order(&result.resources, &providers()).is_err());
}

/// Every provider Hoista connects to, as plans are made against them.
fn providers() -> hoista_server::application::deploys::planning::Providers {
    hoista_server::application::deploys::planning::Providers::new(
        hoista_server::infrastructure::providers::provider_capabilities(),
    )
}
