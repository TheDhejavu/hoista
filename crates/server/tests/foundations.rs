//! Offline workflow tests through real SQLite and the HTTP boundary.

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use hoista_server::{
    application::StateStore,
    domain::Profile,
    infrastructure::{paths::StatePaths, store::SqliteStore},
    {contracts, http::router},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

async fn setup() -> (tempfile::TempDir, StatePaths, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary test directory");
    let paths = StatePaths::new(Some(directory.path().join("state"))).expect("explicit test path");
    let store = SqliteStore::open(&paths).await.expect("fresh test store");
    (directory, paths, store)
}

async fn request(
    api: Router,
    method: &str,
    path: &str,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let response = api
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("valid test request and JSON response"),
        )
        .await
        .expect("valid test request and JSON response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 8192)
        .await
        .expect("valid test request and JSON response");
    (
        status,
        serde_json::from_slice(&bytes).expect("valid test request and JSON response"),
    )
}

#[tokio::test]
async fn first_run_profile_is_validated_persisted_and_replayed_after_restart() {
    let (_directory, paths, store) = setup().await;
    let api = router(store.clone(), CancellationToken::new());
    let (status, body) = request(api.clone(), "GET", "/profile", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "profile.not_found");
    let (status, body) = request(
        api.clone(),
        "PUT",
        "/profile",
        r#"{"name":" Ayodeji ","initials":" AY "}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Ayodeji");
    for invalid in [
        r#"{"name":"","initials":"A"}"#,
        r#"{"name":"A","initials":"ABCDE"}"#,
        r#"{"name":"A","initials":"A","token":"never-log-this"}"#,
        "not-json",
    ] {
        let (status, body) = request(api.clone(), "PUT", "/profile", invalid).await;
        assert!(status.is_client_error());
        assert!(!body.to_string().contains("never-log-this"));
    }
    let (status, _) = request(api.clone(), "PUT", "/profile", &"x".repeat(4097)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let events = store.events_after(0).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "profile.updated");
    assert!(!events[0].summary.contains("Ayodeji"));
    chrono::DateTime::parse_from_rfc3339(&events[0].at).unwrap();
    let cursor = events[0].id;
    drop(api);
    store.close().await;
    drop(store);
    let reopened = SqliteStore::open(&paths).await.unwrap();
    assert_eq!(reopened.profile().await.unwrap().unwrap().name(), "Ayodeji");
    assert_eq!(reopened.events_after(0).await.unwrap()[0].id, cursor);
    assert!(reopened.events_after(cursor).await.unwrap().is_empty());
    reopened.close().await;
}

#[tokio::test]
async fn a_failed_event_insert_rolls_back_the_profile_change() {
    let (_directory, paths, store) = setup().await;
    store
        .save_profile(&Profile::new("Original".into(), "O".into()).unwrap())
        .await
        .unwrap();
    let fault = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(paths.database()),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'injected failure'); END")
        .execute(&fault)
        .await
        .unwrap();
    assert!(
        store
            .save_profile(&Profile::new("Changed".into(), "C".into()).unwrap())
            .await
            .is_err()
    );
    assert_eq!(store.profile().await.unwrap().unwrap().name(), "Original");
    assert_eq!(store.events_after(0).await.unwrap().len(), 1);
    assert!(
        sqlx::query("DELETE FROM events")
            .execute(&fault)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE events SET summary = 'changed'")
            .execute(&fault)
            .await
            .is_err()
    );
    fault.close().await;
    store.close().await;
}

#[tokio::test]
async fn exclusive_ownership_lasts_until_the_final_store_handle_is_dropped() {
    let (_directory, paths, store) = setup().await;
    let clone = store.clone();
    store.close().await;
    drop(store);
    assert!(SqliteStore::open(&paths).await.is_err());
    drop(clone);
    let reopened = SqliteStore::open(&paths).await.unwrap();
    reopened.close().await;
}

#[tokio::test]
async fn event_stream_replays_from_cursor_and_shuts_down_without_losing_commits() {
    let (_directory, _paths, store) = setup().await;
    let profile = Profile::new("Ada".into(), "A".into()).unwrap();
    store.save_profile(&profile).await.unwrap();
    store.save_profile(&profile).await.unwrap();
    let cancel = CancellationToken::new();
    let api = router(store.clone(), cancel.clone());
    let response = api
        .clone()
        .oneshot(
            Request::builder()
                .uri("/events")
                .header("last-event-id", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // Poll the first frame before cancellation so the replay is actually consumed.
    let mut body = response.into_body();
    use http_body_util::BodyExt;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
    assert!(text.contains("id: 2"));
    assert!(text.contains("event: profile.updated"));
    assert!(!text.contains("id: 1"));
    cancel.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(2), body.collect())
        .await
        .unwrap()
        .unwrap();
    let response = api
        .oneshot(
            Request::builder()
                .uri("/events")
                .header("last-event-id", "-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    store.close().await;
}

#[tokio::test]
async fn event_stream_after_latest_skips_history_and_follows_new_events() {
    use http_body_util::BodyExt;
    let (_directory, _paths, store) = setup().await;
    let profile = Profile::new("Ada".into(), "A".into()).unwrap();
    store.save_profile(&profile).await.unwrap();
    store.save_profile(&profile).await.unwrap();
    assert_eq!(store.latest_event_id().await.unwrap(), 2);
    let cancel = CancellationToken::new();
    let api = router(store.clone(), cancel.clone());
    let response = api
        .clone()
        .oneshot(
            Request::builder()
                .uri("/events?after=latest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    store.save_profile(&profile).await.unwrap();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
    assert!(text.contains("id: 3"), "{text}");
    assert!(text.contains("\"subject_id\":null"), "{text}");
    cancel.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(2), body.collect())
        .await
        .unwrap()
        .unwrap();
    let (status, body) = request(api, "GET", "/events?after=now", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "events.invalid_start");
    store.close().await;
}

#[tokio::test]
async fn slow_streams_are_bounded_and_event_pages_have_no_gaps() {
    let (_directory, _paths, store) = setup().await;
    let profile = Profile::new("Ada".into(), "A".into()).unwrap();
    for _ in 0..105 {
        store.save_profile(&profile).await.unwrap();
    }
    let first = store.events_after(0).await.unwrap();
    assert_eq!(first.len(), 100);
    let next = store.events_after(first.last().unwrap().id).await.unwrap();
    assert_eq!(next.len(), 5);
    let api = router(store.clone(), CancellationToken::new());
    let mut bodies = Vec::new();
    for _ in 0..32 {
        let response = api
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        bodies.push(response.into_body());
    }
    let (status, body) = request(api.clone(), "GET", "/events", "").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["code"], "events.limit");
    bodies.pop();
    let response = api
        .oneshot(
            Request::builder()
                .uri("/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    drop(bodies);
    store.close().await;
}

#[tokio::test]
async fn unavailable_storage_returns_safe_json_errors() {
    let (_directory, _paths, store) = setup().await;
    let api = router(store.clone(), CancellationToken::new());
    store.close().await;
    for path in ["/profile", "/events"] {
        let (status, body) = request(api.clone(), "GET", path, "").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "state.unavailable");
    }
}

#[test]
fn checked_in_contracts_match_the_rust_api_types() {
    // The desktop app uses these endpoint types directly. Comparing generated output
    // alone can't catch a type removed from the registry.
    let declarations = contracts::typescript();
    for name in [
        "ResourceDetail",
        "ResourceRelationship",
        "RecordedResourceRelease",
        "LogHistory",
        "LogHistoryQuery",
        "Page",
        "SourceProvenance",
        "DaemonStatus",
        "RestartRequest",
        "AccountChoice",
        "AccountDiagnostic",
        "AccountDiagnosticStatus",
        "ProjectMap",
        "MapGraph",
        "MapNode",
        "MapEdge",
        "MapEdgeKind",
        "MapEdgeOrigin",
        "ImportProposal",
        "ImportEvidence",
        "ImportEvidenceKind",
    ] {
        assert!(
            declarations.contains(&format!("export type {name}")),
            "Missing desktop contract: {name}"
        );
    }

    assert_eq!(
        include_str!("../../../docs/backend/api/types.ts"),
        contracts::typescript()
    );
    let saved: serde_json::Value =
        serde_json::from_str(include_str!("../../../docs/backend/api/schema.json")).unwrap();
    assert_eq!(
        saved,
        serde_json::to_value(contracts::json_schema()).unwrap()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn state_is_private_and_symlinked_paths_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (directory, paths, store) = setup().await;
    assert_eq!(
        std::fs::metadata(paths.root())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(paths.database())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let alias = directory.path().join("alias");
    symlink(paths.root(), &alias).unwrap();
    assert!(
        SqliteStore::open(&StatePaths::new(Some(alias)).unwrap())
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn daemon_restart_requires_current_identity_and_preserves_supervisor_policy() {
    use hoista_server::{http::router_with_lifecycle, lifecycle::DaemonLifecycle};
    use std::sync::Arc;
    let (_directory, _paths, store) = setup().await;
    for supervised in [false, true] {
        let stop = CancellationToken::new();
        let control = Arc::new(DaemonLifecycle::new(stop.clone(), supervised));
        let api = router_with_lifecycle(store.clone(), stop.clone(), None, Some(control.clone()));
        let (_, status) = request(api.clone(), "GET", "/daemon", "").await;
        assert_eq!(status["supervised"], supervised);
        assert_eq!(status["restart_requested"], false);
        for bad in [
            r#"{"instance_id":"stale"}"#,
            r#"{"instance_id":"stale","force":true}"#,
            "not-json",
        ] {
            assert!(
                request(api.clone(), "POST", "/daemon/restart", bad)
                    .await
                    .0
                    .is_client_error()
            );
            assert!(!stop.is_cancelled());
        }
        let input = serde_json::json!({"instance_id":status["instance_id"]}).to_string();
        let (code, accepted) = request(api.clone(), "POST", "/daemon/restart", &input).await;
        assert_eq!(code, StatusCode::ACCEPTED);
        assert_eq!(accepted["restart_requested"], true);
        assert!(stop.is_cancelled());
        assert_eq!(control.handoff(), supervised);
        assert_eq!(
            request(api.clone(), "POST", "/daemon/restart", &input)
                .await
                .0,
            StatusCode::ACCEPTED
        );
        if !supervised {
            let conflicting =
                serde_json::json!({"instance_id":status["instance_id"],"handoff":true}).to_string();
            assert_eq!(
                request(api.clone(), "POST", "/daemon/restart", &conflicting)
                    .await
                    .0,
                StatusCode::CONFLICT
            );
            assert!(!control.handoff());
        }
        let next = DaemonLifecycle::new(CancellationToken::new(), supervised);
        assert_ne!(control.status().instance_id, next.status().instance_id);
        assert!(
            next.request_restart(&contracts::RestartRequest {
                instance_id: control.status().instance_id,
                handoff: false
            })
            .is_err()
        );
    }
    let stop = CancellationToken::new();
    let control = DaemonLifecycle::new(stop.clone(), false);
    control
        .request_restart(&contracts::RestartRequest {
            instance_id: control.status().instance_id,
            handoff: true,
        })
        .unwrap();
    assert!(control.handoff() && stop.is_cancelled());
    store.close().await;
}

/// Sets one recorded migration checksum, as a changed migration would leave it.
async fn record_checksum(paths: &StatePaths, version: i64, hex: &str) {
    let fixture = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(paths.database()),
    )
    .await
    .expect("test database opens");
    sqlx::query("UPDATE _sqlx_migrations SET checksum = unhex(?) WHERE version = ?")
        .bind(hex)
        .bind(version)
        .execute(&fixture)
        .await
        .expect("checksum fixture is written");
    fixture.close().await;
}

#[tokio::test]
async fn an_unknown_migration_checksum_is_still_rejected() {
    let (_directory, paths, store) = setup().await;
    store.close().await;
    drop(store);
    record_checksum(&paths, 1, &"00".repeat(48)).await;
    assert!(matches!(
        SqliteStore::open(&paths).await,
        Err(hoista_server::infrastructure::store::StoreError::Migration(
            _
        ))
    ));
}

#[tokio::test]
async fn activity_lists_newest_first_with_filters_and_pages() {
    let (_directory, _paths, store) = setup().await;
    let profile = Profile::new("Ada".into(), "A".into()).unwrap();
    for _ in 0..3 {
        store.save_profile(&profile).await.unwrap();
    }
    let api = router(store.clone(), CancellationToken::new());
    let (status, body) = request(api.clone(), "GET", "/activity", "").await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["3", "2", "1"]);
    assert!(body["next_cursor"].is_null());
    let (_, older) = request(api.clone(), "GET", "/activity?cursor=3", "").await;
    assert_eq!(older["items"].as_array().unwrap().len(), 2);
    let (_, other) = request(api.clone(), "GET", "/activity?actor=agent-1", "").await;
    assert!(other["items"].as_array().unwrap().is_empty());
    let (status, _) = request(api, "GET", "/activity?cursor=-4", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    store.close().await;
}
