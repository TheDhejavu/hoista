//! Admission keys use real SQLite and an isolated credential fixture; no native secrets.

use hoista_server::{
    application::{
        environments::admission_keys,
        ports::{
            catalog::{CatalogRepository, EngineError},
            vault::TokenVault,
        },
    },
    domain::Secret,
    infrastructure::{paths::StatePaths, store::SqliteStore},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct Vault {
    values: Arc<Mutex<HashMap<String, String>>>,
    fail: AtomicBool,
    read_delay: AtomicBool,
    delay: AtomicBool,
    entered: tokio::sync::Notify,
    release: Arc<tokio::sync::Notify>,
    completed: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl TokenVault for Vault {
    async fn read(&self, reference: &str) -> Result<Secret, EngineError> {
        if self.read_delay.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(EngineError::Store);
        }
        self.values
            .lock()
            .expect("admission key fixture")
            .get(reference)
            .cloned()
            .map(Secret::new)
            .ok_or(EngineError::NotFound)
    }

    async fn write(&self, reference: &str, value: Secret) -> Result<(), EngineError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(EngineError::Store);
        }
        if self.delay.load(Ordering::SeqCst) {
            let values = self.values.clone();
            let release = self.release.clone();
            let completed = self.completed.clone();
            let reference = reference.to_owned();
            let task = tokio::spawn(async move {
                release.notified().await;
                values
                    .lock()
                    .expect("admission key fixture")
                    .insert(reference, value.expose().into());
                completed.notify_one();
            });
            self.entered.notify_one();
            task.await.map_err(|_| EngineError::Store)?;
        } else {
            self.values
                .lock()
                .expect("admission key fixture")
                .insert(reference.into(), value.expose().into());
        }
        Ok(())
    }

    async fn delete(&self, _: &str) -> Result<(), EngineError> {
        panic!("key lifecycle never deletes credentials implicitly")
    }
}

async fn setup() -> (tempfile::TempDir, StatePaths, SqliteStore) {
    let dir = tempfile::tempdir().expect("admission key fixture");
    let paths = StatePaths::new(Some(dir.path().join("state"))).expect("admission key fixture");
    let store = SqliteStore::open(&paths)
        .await
        .expect("admission key fixture");
    (dir, paths, store)
}

#[tokio::test]
async fn original_key_survives_restart_and_missing_or_changed_material_is_never_replaced() {
    let (_dir, paths, store) = setup().await;
    let vault = Vault::default();
    let first = admission_keys::load(&store, &vault)
        .await
        .expect("admission key fixture");
    let record = store
        .environment_admission_key()
        .await
        .expect("admission key fixture")
        .expect("admission key fixture");
    let values = vec![(
        "PASSWORD".into(),
        Secret::new("low-entropy-test-value".into()),
    )];
    let fingerprint =
        admission_keys::fingerprint(&first, b"operation", &values).expect("admission key fixture");
    store.close().await;
    drop(store);
    let store = SqliteStore::open(&paths)
        .await
        .expect("admission key fixture");
    let again = admission_keys::load(&store, &vault)
        .await
        .expect("admission key fixture");
    assert_eq!(
        fingerprint,
        admission_keys::fingerprint(&again, b"operation", &values).expect("admission key fixture")
    );
    vault
        .values
        .lock()
        .expect("admission key fixture")
        .insert(record.reference.clone(), "a".repeat(96));
    assert_eq!(
        admission_keys::load(&store, &vault)
            .await
            .unwrap_err()
            .code(),
        "keychain.admission_unavailable"
    );
    vault.values.lock().expect("admission key fixture").clear();
    assert!(admission_keys::load(&store, &vault).await.is_err());
    assert!(
        vault
            .values
            .lock()
            .expect("admission key fixture")
            .is_empty()
    );
    assert_eq!(
        store
            .environment_admission_key()
            .await
            .expect("admission key fixture"),
        Some(record)
    );
    store.close().await;
    let database = std::fs::read(paths.database()).expect("admission key fixture");
    assert!(
        !database
            .windows(first.expose().len())
            .any(|window| window == first.expose().as_bytes())
    );
}

#[tokio::test]
async fn cancelled_initialization_cannot_overwrite_the_next_active_key() {
    let (_dir, _paths, store) = setup().await;
    let vault = Arc::new(Vault::default());
    vault.delay.store(true, Ordering::SeqCst);
    let pending = {
        let store = store.clone();
        let vault = vault.clone();
        tokio::spawn(async move { admission_keys::load(&store, vault.as_ref()).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), vault.entered.notified())
        .await
        .expect("admission key fixture");
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    vault.delay.store(false, Ordering::SeqCst);
    let active = admission_keys::load(&store, vault.as_ref())
        .await
        .expect("admission key fixture");
    vault.release.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        vault.completed.notified(),
    )
    .await
    .expect("admission key fixture");
    assert_eq!(vault.values.lock().expect("admission key fixture").len(), 2);
    assert_eq!(
        active.expose(),
        admission_keys::load(&store, vault.as_ref())
            .await
            .expect("admission key fixture")
            .expose()
    );
    store.close().await;
}

#[tokio::test]
async fn initialization_failures_are_bounded_and_concurrent_activation_keeps_one_winner() {
    let (_dir, _paths, store) = setup().await;
    let vault = Vault::default();
    vault.fail.store(true, Ordering::SeqCst);
    for _ in 0..16 {
        assert!(admission_keys::load(&store, &vault).await.is_err());
    }
    assert_eq!(
        admission_keys::load(&store, &vault)
            .await
            .unwrap_err()
            .code(),
        "state.conflict"
    );
    assert!(
        store
            .environment_admission_key()
            .await
            .expect("admission key fixture")
            .is_none()
    );
    assert!(
        vault
            .values
            .lock()
            .expect("admission key fixture")
            .is_empty()
    );
    store.close().await;
    let (_dir, _paths, store) = setup().await;
    let vault = Arc::new(Vault::default());
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let store = store.clone();
        let vault = vault.clone();
        tasks.spawn(async move { admission_keys::load(&store, vault.as_ref()).await });
    }
    let mut keys = std::collections::BTreeSet::new();
    while let Some(result) = tasks.join_next().await {
        keys.insert(
            result
                .expect("admission key fixture")
                .expect("admission key fixture")
                .expose()
                .to_owned(),
        );
    }
    assert_eq!(keys.len(), 1);
    store.close().await;
}

#[test]
fn secret_fingerprints_are_order_independent_but_bind_values_boundaries_metadata_and_key() {
    let key = Secret::new("a".repeat(96));
    let values = vec![
        ("B".into(), Secret::new("".into())),
        ("A".into(), Secret::new("one\ntwo".into())),
    ];
    let first = admission_keys::fingerprint(&key, b"actor/app/plan/key", &values)
        .expect("admission key fixture");
    let mut reverse = values;
    reverse.reverse();
    assert_eq!(
        first,
        admission_keys::fingerprint(&key, b"actor/app/plan/key", &reverse)
            .expect("admission key fixture")
    );
    assert_ne!(
        first,
        admission_keys::fingerprint(&key, b"other request", &reverse)
            .expect("admission key fixture")
    );
    assert_ne!(
        first,
        admission_keys::fingerprint(
            &Secret::new("b".repeat(96)),
            b"actor/app/plan/key",
            &reverse
        )
        .expect("admission key fixture")
    );
    reverse[0].1 = Secret::new("one\ntwo\n".into());
    assert_ne!(
        first,
        admission_keys::fingerprint(&key, b"actor/app/plan/key", &reverse)
            .expect("admission key fixture")
    );
    reverse.push(("A".into(), Secret::new("duplicate".into())));
    assert!(admission_keys::fingerprint(&key, b"metadata", &reverse).is_err());
    assert!(
        admission_keys::fingerprint(
            &key,
            b"metadata",
            &[("A".into(), Secret::new("x".repeat(16385)))]
        )
        .is_err()
    );
}

#[tokio::test]
async fn recovery_reports_distinguish_original_missing_changed_and_unavailable_keys() {
    use admission_keys::EnvironmentKeyState as State;
    let (_dir, _paths, store) = setup().await;
    let vault = Vault::default();
    assert_eq!(
        admission_keys::inspect(&store, &vault).await.unwrap().state,
        State::NotInitialized
    );
    let key = admission_keys::load(&store, &vault).await.unwrap();
    let report = admission_keys::inspect(&store, &vault).await.unwrap();
    assert_eq!(report.state, State::Available);
    assert!(!report.retry_setup_allowed);
    let reference = report.active_reference.as_ref().unwrap();
    let encoded = serde_json::to_string(&report).unwrap();
    assert!(!encoded.contains(key.expose()));
    assert!(!encoded.contains("commitment"));
    vault
        .values
        .lock()
        .unwrap()
        .insert(reference.clone(), "a".repeat(96));
    assert_eq!(
        admission_keys::inspect(&store, &vault).await.unwrap().state,
        State::Changed
    );
    vault.values.lock().unwrap().clear();
    assert_eq!(
        admission_keys::inspect(&store, &vault).await.unwrap().state,
        State::Missing
    );
    vault
        .values
        .lock()
        .unwrap()
        .insert(reference.clone(), key.expose().into());
    vault.read_delay.store(true, Ordering::SeqCst);
    let timed = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        admission_keys::inspect(&store, &vault),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(timed.state, State::Unavailable);
    vault.read_delay.store(false, Ordering::SeqCst);
    vault.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        admission_keys::inspect(&store, &vault).await.unwrap().state,
        State::Unavailable
    );
    assert!(
        store
            .retire_environment_key_setup(&report.review_token)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .environment_admission_key()
            .await
            .unwrap()
            .unwrap()
            .reference,
        *reference
    );
    store.close().await;
}

#[tokio::test]
async fn reviewed_setup_retirement_is_atomic_and_restores_capacity_without_deleting_keys() {
    let (_dir, paths, store) = setup().await;
    let vault = Vault::default();
    vault.fail.store(true, Ordering::SeqCst);
    assert!(admission_keys::load(&store, &vault).await.is_err());
    let stale = store
        .environment_key_snapshot()
        .await
        .unwrap()
        .review_token()
        .unwrap();
    for _ in 1..16 {
        assert!(admission_keys::load(&store, &vault).await.is_err());
    }
    assert!(store.retire_environment_key_setup(&stale).await.is_err());
    let before = store.environment_key_snapshot().await.unwrap();
    assert_eq!(before.pending.len(), 16);
    let fault = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(paths.database()),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_key_repair BEFORE INSERT ON events WHEN NEW.kind = 'key.admission.setup_retired' BEGIN SELECT RAISE(ABORT, 'fixture'); END;")
        .execute(&fault)
        .await
        .unwrap();
    let token = before.review_token().unwrap();
    assert!(store.retire_environment_key_setup(&token).await.is_err());
    assert_eq!(
        store
            .environment_key_snapshot()
            .await
            .unwrap()
            .review_token()
            .unwrap(),
        token
    );
    sqlx::query("DROP TRIGGER reject_key_repair")
        .execute(&fault)
        .await
        .unwrap();
    store.retire_environment_key_setup(&token).await.unwrap();
    let after = store.environment_key_snapshot().await.unwrap();
    assert_eq!(after.retired, 16);
    assert!(after.pending.is_empty());
    assert!(after.active.is_none());
    assert!(store.retire_environment_key_setup(&token).await.is_err());
    assert!(
        sqlx::query("UPDATE environment_admission_keys SET retired = 0")
            .execute(&fault)
            .await
            .is_err()
    );
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE kind = 'key.admission.setup_retired'",
    )
    .fetch_one(&fault)
    .await
    .unwrap();
    assert_eq!(events, 1);
    vault.fail.store(false, Ordering::SeqCst);
    admission_keys::load(&store, &vault).await.unwrap();
    let ready = store.environment_key_snapshot().await.unwrap();
    assert!(ready.active.is_some());
    assert!(ready.pending.is_empty());
    assert_eq!(ready.retired, 16);
    assert!(
        store
            .retire_environment_key_setup(&ready.review_token().unwrap())
            .await
            .is_err()
    );
    fault.close().await;
    store.close().await;
}

#[tokio::test]
async fn delayed_setup_cannot_activate_after_owner_retirement() {
    let (_dir, _paths, store) = setup().await;
    let vault = Arc::new(Vault::default());
    vault.delay.store(true, Ordering::SeqCst);
    let pending = {
        let store = store.clone();
        let vault = vault.clone();
        tokio::spawn(async move { admission_keys::load(&store, vault.as_ref()).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), vault.entered.notified())
        .await
        .unwrap();
    let review = store
        .environment_key_snapshot()
        .await
        .unwrap()
        .review_token()
        .unwrap();
    store.retire_environment_key_setup(&review).await.unwrap();
    vault.release.notify_one();
    assert!(pending.await.unwrap().is_err());
    assert!(store.environment_admission_key().await.unwrap().is_none());
    assert_eq!(
        vault.values.lock().unwrap().len(),
        1,
        "retirement must not delete the delayed credential"
    );
    vault.delay.store(false, Ordering::SeqCst);
    admission_keys::load(&store, vault.as_ref()).await.unwrap();
    let current = store.environment_key_snapshot().await.unwrap();
    assert!(current.active.is_some());
    assert!(current.pending.is_empty());
    assert_eq!(current.retired, 1);
    assert_eq!(vault.values.lock().unwrap().len(), 2);
    store.close().await;
}

#[tokio::test]
async fn admission_key_records_keep_their_rules() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../store/migrations/0001_initial.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO environment_admission_keys (reference, commitment, active) VALUES ('original-active', ?, 1)")
        .bind("a".repeat(64))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO environment_admission_keys (reference) VALUES ('pending')")
        .execute(&pool)
        .await
        .unwrap();
    let active: (String, i64, i64) =
        sqlx::query_as("SELECT commitment, active, retired FROM environment_admission_keys WHERE reference = 'original-active'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active, ("a".repeat(64), 1, 0));
    assert!(
        sqlx::query("UPDATE environment_admission_keys SET retired = 1 WHERE active = 1")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("UPDATE environment_admission_keys SET retired = 1 WHERE reference = 'pending'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE environment_admission_keys SET active = 1, commitment = ? WHERE reference = 'pending'")
            .bind("b".repeat(64))
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM environment_admission_keys")
            .execute(&pool)
            .await
            .is_err()
    );
    pool.close().await;
}

#[tokio::test]
async fn key_access_deadline_preserves_the_original_record_and_can_recover_after_unlock() {
    let (_dir, _paths, store) = setup().await;
    let vault = Vault::default();
    let original = admission_keys::load(&store, &vault).await.unwrap();
    let record = store.environment_admission_key().await.unwrap();
    vault.read_delay.store(true, Ordering::SeqCst);
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        admission_keys::load(&store, &vault),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(error.code(), "keychain.admission_unavailable");
    assert_eq!(store.environment_admission_key().await.unwrap(), record);
    assert_eq!(vault.values.lock().unwrap().len(), 1);
    vault.read_delay.store(false, Ordering::SeqCst);
    assert_eq!(
        admission_keys::load(&store, &vault).await.unwrap().expose(),
        original.expose()
    );
    store.close().await;
}
