//! Real PostgreSQL acceptance, driven by scripts/tests/postgres_export.py's isolated TLS cluster.

use hoista_server::{
    application::ports::database_export::{DatabaseExportSource, DatabaseExporter},
    domain::Secret,
    infrastructure::database_export::PostgresExporter,
};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires the disposable TLS PostgreSQL fixture in scripts/tests/postgres_export.py"]
async fn native_postgres_archive_restores_schema_rows_sequences_and_large_objects() {
    let uri = std::env::var("HOISTA_TEST_DATABASE_URL").expect("fixture connection");
    let binary = PathBuf::from(std::env::var("HOISTA_TEST_POSTGRES_BIN").expect("fixture tools"));
    let directory = tempfile::tempdir().expect("archive state");
    let exporter = PostgresExporter::new(directory.path());
    let archive = exporter
        .export(
            "fixture-resource",
            &DatabaseExportSource {
                branch_id: "fixture-branch".into(),
                database_name: "hoista_source_fixture".into(),
                connection: Secret::new(uri.clone()),
            },
        )
        .await
        .expect("real pg_dump archive verified by pg_restore");
    assert!(archive.bytes > 100);
    let listed = exporter.archives(None).await.expect("durable receipt");
    assert_eq!(listed.items, vec![archive.clone()]);
    let url = url::Url::parse(&uri).expect("fixture URL");
    let prepare = |program: &str| {
        let mut command = Command::new(binary.join(program));
        command
            .env_clear()
            .env("PGHOST", url.host_str().expect("host"))
            .env("PGPORT", url.port().expect("port").to_string())
            .env("PGUSER", "fixture")
            .env("PGPASSWORD", "fixture-password")
            .env("PGSSLMODE", "require")
            .env("PGDATABASE", "hoista_restore_fixture")
            .env("PGCONNECT_TIMEOUT", "5")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        command
    };
    let mut restore = prepare("pg_restore");
    restore
        .args([
            "--exit-on-error",
            "--no-owner",
            "--no-acl",
            "--dbname",
            "hoista_restore_fixture",
        ])
        .arg(&archive.path);
    let restored = tokio::time::timeout(Duration::from_secs(30), restore.output())
        .await
        .expect("restore deadline")
        .expect("restore process");
    assert!(
        restored.status.success(),
        "archive restores into a separate empty database"
    );
    let mut query = prepare("psql");
    query.args([
        "-X",
        "-A",
        "-t",
        "--no-password",
        "-c",
        "SELECT count(*),string_agg(body,'|' ORDER BY id),(SELECT encode(payload,'hex') FROM notes WHERE id=2) FROM notes; SELECT encode(lo_get(oid),'hex') FROM objects; INSERT INTO notes(body,payload) VALUES ('third',decode('00','hex')) RETURNING id; SELECT count FROM note_count;",
    ]);
    let result = tokio::time::timeout(Duration::from_secs(30), query.output())
        .await
        .expect("query deadline")
        .expect("query process");
    assert!(
        result.status.success(),
        "restored tables, views, sequences and large objects are readable"
    );
    let output = String::from_utf8(result.stdout).expect("fixture output");
    assert!(output.contains("2|Hello, Lagos|Unicode: àṣẹ|010203ff"));
    assert!(output.contains("aabbccdd"));
    assert!(output.contains("\n3\nINSERT 0 1\n3\n"));
}
