//! PostgreSQL exports: `pg_dump` custom-format archives in private files,
//! read back in full by `pg_restore` before they're kept. The password goes
//! in the environment, never on the command line. Unix only: Windows would
//! need owner-only folders and a way to stop the whole process tree.

use super::{paths, process, tool_paths};
use crate::{
    application::{
        engine::now,
        ports::{
            catalog::{EngineError, Page},
            database_export::{DatabaseExportSource, DatabaseExporter},
        },
    },
    domain::database_export::DatabaseArchive,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    process::Command,
};

const MAX_ARCHIVE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// How long `pg_dump`, and then `pg_restore`, may each take.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// Seconds `pg_dump` waits to connect.
const CONNECT_TIMEOUT_SECONDS: &str = "15";
const DEFAULT_PORT: u16 = 5432;
/// Every custom-format archive starts with this.
const ARCHIVE_MAGIC: &[u8; 5] = b"PGDMP";
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const ARCHIVES_PER_PAGE: usize = 100;
/// The most folder entries looked at when listing archives.
const MAX_SCANNED_ENTRIES: usize = 30_000;
const MAX_ID_BYTES: usize = 256;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Exports with the `pg_dump` and `pg_restore` found on this computer.
pub struct PostgresExporter {
    directory: PathBuf,
    tools: Option<(PathBuf, PathBuf)>,
    max_bytes: u64,
    deadline: Duration,
}

impl PostgresExporter {
    /// Archives go in `database-exports` under the state folder. Only looks
    /// for the tools; installs nothing and connects to nothing.
    pub fn new(state_root: &Path) -> Self {
        Self {
            directory: state_root.join("database-exports"),
            tools: tool_paths::executable("pg_dump").zip(tool_paths::executable("pg_restore")),
            max_bytes: MAX_ARCHIVE_BYTES,
            deadline: EXPORT_TIMEOUT,
        }
    }
}

fn failure(message: &'static str) -> EngineError {
    EngineError::Conflict(message)
}

/// A PostgreSQL tool with an empty environment: none of the person's `PG*`
/// settings, service files, home folder or credentials reach it.
fn command(program: &Path, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PGCONNECT_TIMEOUT", CONNECT_TIMEOUT_SECONDS)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::null());
    command
}

/// Whether `id` is a UUID written the way Hoista writes them.
fn is_archive_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}

/// Decodes one part of the connection address.
fn decode(value: &str) -> Result<String, EngineError> {
    let decoded = percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| failure("The database connection contains invalid text."))?
        .into_owned();
    if decoded.is_empty() || decoded.chars().any(char::is_control) {
        return Err(failure("The database connection contains invalid fields."));
    }
    Ok(decoded)
}

/// Points `command` at the database in the connection address, as `PG*`
/// variables. Only TLS connections, and only the `sslmode` and
/// `channel_binding` options; anything else in the address is refused.
fn connection(command: &mut Command, source: &DatabaseExportSource) -> Result<(), EngineError> {
    let uri = url::Url::parse(source.connection.expose())
        .map_err(|_| failure("The database connection is invalid."))?;
    if !matches!(uri.scheme(), "postgres" | "postgresql") || uri.fragment().is_some() {
        return Err(failure(
            "Use a direct PostgreSQL connection for database export.",
        ));
    }
    let host = uri
        .host()
        .ok_or_else(|| failure("The database connection has no host."))?;
    let host = match host {
        url::Host::Domain(domain) => domain.to_owned(),
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => address.to_string(),
    };
    let database = decode(uri.path().strip_prefix('/').unwrap_or(uri.path()))?;
    if database != source.database_name {
        return Err(failure(
            "The export connection targets a different database.",
        ));
    }
    command
        .env("PGHOST", host)
        .env("PGPORT", uri.port().unwrap_or(DEFAULT_PORT).to_string())
        .env("PGUSER", decode(uri.username())?)
        .env("PGDATABASE", database)
        .env(
            "PGPASSWORD",
            decode(
                uri.password()
                    .ok_or_else(|| failure("The database connection has no password."))?,
            )?,
        )
        .env("PGSSLMODE", "require");
    let mut seen = BTreeSet::new();
    for (key, value) in uri.query_pairs() {
        if !seen.insert(key.to_string()) {
            return Err(failure("The database connection has duplicate options."));
        }
        match key.as_ref() {
            "sslmode" if matches!(value.as_ref(), "require" | "verify-full" | "verify-ca") => {
                command.env("PGSSLMODE", value.as_ref());
            }
            "channel_binding" if matches!(value.as_ref(), "prefer" | "require" | "disable") => {
                command.env("PGCHANNELBINDING", value.as_ref());
            }
            _ => {
                return Err(failure(
                    "The database connection includes unsupported export options.",
                ));
            }
        }
    }
    Ok(())
}

#[async_trait::async_trait]
impl DatabaseExporter for PostgresExporter {
    fn available(&self) -> bool {
        cfg!(unix) && self.tools.is_some()
    }

    async fn archives(&self, cursor: Option<&str>) -> Result<Page<DatabaseArchive>, EngineError> {
        if cursor.is_some_and(|cursor| !is_archive_id(cursor)) {
            return Err(failure("Invalid database archive cursor."));
        }
        if !self.directory.exists() {
            return Ok(Page {
                items: vec![],
                next_cursor: None,
            });
        }
        paths::reject_symlink(&self.directory)
            .map_err(|_| failure("Database export storage must not be a symbolic link."))?;
        let directory = self
            .directory
            .canonicalize()
            .map_err(|_| failure("Cannot read database export storage."))?;
        let mut entries = tokio::fs::read_dir(&directory)
            .await
            .map_err(|_| failure("Cannot list database archives."))?;
        let mut names = Vec::new();
        let mut inspected = 0;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|_| failure("Cannot list database archives."))?
        {
            inspected += 1;
            if inspected > MAX_SCANNED_ENTRIES {
                return Err(failure(
                    "Database archive storage exceeds its safe scan limit.",
                ));
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if let Some(id) = name.strip_suffix(".json") {
                if !is_archive_id(id) {
                    return Err(failure("Database archive receipt has an invalid identity."));
                }
                if cursor.is_none_or(|cursor| id > cursor) {
                    names.push(id.to_owned());
                }
            }
        }
        names.sort();
        let more = names.len() > ARCHIVES_PER_PAGE;
        names.truncate(ARCHIVES_PER_PAGE);
        let mut items = Vec::new();
        for id in &names {
            let manifest = directory.join(format!("{id}.json"));
            if !tokio::fs::symlink_metadata(&manifest)
                .await
                .map_err(|_| failure("Cannot inspect archive receipt."))?
                .file_type()
                .is_file()
            {
                return Err(failure("Database archive receipt must be a regular file."));
            }
            let file = tokio::fs::File::open(&manifest)
                .await
                .map_err(|_| failure("Cannot open archive receipt."))?;
            let mut bytes = Vec::new();
            file.take(MAX_RECEIPT_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| failure("Cannot read archive receipt."))?;
            if bytes.len() as u64 > MAX_RECEIPT_BYTES {
                return Err(failure("Database archive receipt exceeds its size limit."));
            }
            let receipt: DatabaseArchive = serde_json::from_slice(&bytes)
                .map_err(|_| failure("Database archive receipt is invalid."))?;
            let path = directory.join(format!("{id}.dump"));
            if receipt.id != *id
                || Path::new(&receipt.path) != path
                || receipt.sha256.len() != 64
                || !receipt.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(failure(
                    "Database archive receipt does not match its local archive.",
                ));
            }
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|_| failure("The completed database archive is missing."))?;
            if !metadata.file_type().is_file() || metadata.len() != receipt.bytes {
                return Err(failure(
                    "The completed database archive changed; verify it before use.",
                ));
            }
            items.push(receipt);
        }
        Ok(Page {
            items,
            next_cursor: if more { names.last().cloned() } else { None },
        })
    }

    async fn export(
        &self,
        resource_id: &str,
        source: &DatabaseExportSource,
    ) -> Result<DatabaseArchive, EngineError> {
        if !self.available() {
            return Err(failure(
                "Install pg_dump and pg_restore; private database exports currently require Unix.",
            ));
        }
        let (dump, restore) = self
            .tools
            .as_ref()
            .ok_or_else(|| failure("PostgreSQL export tools are unavailable. Check hoist tools; after installing pg_dump and pg_restore, refresh hoist tools configure and restart the daemon."))?;
        if resource_id.is_empty()
            || resource_id.len() > MAX_ID_BYTES
            || source.branch_id.is_empty()
            || source.branch_id.len() > MAX_ID_BYTES
            || source.database_name.len() > MAX_ID_BYTES
        {
            return Err(failure("Use a valid database export identity."));
        }
        paths::private_directory(&self.directory)
            .map_err(|_| failure("Cannot prepare private database export storage."))?;
        let directory = self
            .directory
            .canonicalize()
            .map_err(|_| failure("Cannot locate database export storage."))?;
        let temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|_| failure("Cannot create private database export."))?;
        let mut output = tokio::fs::File::from_std(
            temporary
                .reopen()
                .map_err(|_| failure("Cannot open private database export."))?,
        );
        let mut dump_command = command(dump, &directory);
        dump_command.args([
            "--format=custom",
            "--no-password",
            "--lock-wait-timeout=30s",
        ]);
        connection(&mut dump_command, source)?;
        let status = process::run_to_file(
            &mut dump_command,
            &mut output,
            self.max_bytes,
            self.deadline,
        )
        .await
        .map_err(|_| failure("Database export failed or exceeded its time/size limit. No deletion was authorized by this export."))?;
        if !status.success() {
            return Err(failure(
                "pg_dump failed. Check database access and client/server version compatibility.",
            ));
        }
        output
            .sync_all()
            .await
            .map_err(|_| failure("Cannot flush the completed database archive."))?;
        let bytes = output
            .metadata()
            .await
            .map_err(|_| failure("Cannot inspect database archive."))?
            .len();
        if bytes < ARCHIVE_MAGIC.len() as u64 {
            return Err(failure("PostgreSQL returned an empty archive."));
        }
        output
            .rewind()
            .await
            .map_err(|_| failure("Cannot inspect database archive."))?;
        let mut header = [0; ARCHIVE_MAGIC.len()];
        output
            .read_exact(&mut header)
            .await
            .map_err(|_| failure("Cannot inspect database archive."))?;
        if &header != ARCHIVE_MAGIC {
            return Err(failure(
                "PostgreSQL did not return a custom-format archive.",
            ));
        }
        // Turning the whole archive into SQL, thrown away, reads every table's
        // data as well as the contents list. Nothing runs and nothing connects.
        let mut verify = command(restore, &directory);
        verify.args(["--file", "/dev/null"]).arg(temporary.path());
        let status = process::run(&mut verify, None, self.deadline)
            .await
            .map_err(|_| failure("Database archive verification failed or timed out."))?;
        if !status.success() {
            return Err(failure(
                "pg_restore could not read the complete database archive.",
            ));
        }
        output
            .rewind()
            .await
            .map_err(|_| failure("Cannot hash database archive."))?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
        loop {
            let count = output
                .read(&mut buffer)
                .await
                .map_err(|_| failure("Cannot hash database archive."))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        drop(output);
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(format!("{id}.dump"));
        let receipt = DatabaseArchive {
            id,
            resource_id: resource_id.into(),
            branch_id: source.branch_id.clone(),
            database_name: source.database_name.clone(),
            path: path.to_string_lossy().into_owned(),
            bytes,
            sha256: format!("{:x}", digest.finalize()),
            completed_at: now(),
        };
        // A fresh name, so no earlier archive is ever replaced.
        temporary
            .persist_noclobber(&path)
            .map_err(|_| failure("Cannot publish completed database archive."))?;
        let metadata = serde_json::to_vec_pretty(&receipt).map_err(|_| EngineError::Store)?;
        let manifest = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|_| failure("Archive saved, but its receipt could not be created."))?;
        let mut manifest_file = tokio::fs::File::from_std(
            manifest
                .reopen()
                .map_err(|_| failure("Archive saved, but its receipt could not be opened."))?,
        );
        manifest_file
            .write_all(&metadata)
            .await
            .map_err(|_| failure("Archive saved, but its receipt could not be written."))?;
        manifest_file
            .sync_all()
            .await
            .map_err(|_| failure("Archive saved, but its receipt could not be flushed."))?;
        drop(manifest_file);
        manifest
            .persist_noclobber(directory.join(format!("{}.json", receipt.id)))
            .map_err(|_| failure("Archive saved, but its receipt could not be published."))?;
        std::fs::File::open(&directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| failure("Cannot confirm durable archive storage."))?;
        Ok(receipt)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(directory: &Path, name: &str, script: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("database export fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("database export fixture");
        path
    }

    fn source() -> DatabaseExportSource {
        DatabaseExportSource {
            branch_id: "br-fixture".into(),
            database_name: "app db".into(),
            connection: crate::domain::Secret::new(
                "postgresql://owner:p%40ss%24word@db.example:5433/app%20db?sslmode=require&channel_binding=require".into(),
            ),
        }
    }

    fn exporter(root: &Path, dump: &str, restore: &str) -> PostgresExporter {
        PostgresExporter {
            directory: root.join("exports"),
            tools: Some((
                executable(root, "dump", dump),
                executable(root, "restore", restore),
            )),
            max_bytes: 1024,
            deadline: Duration::from_secs(2),
        }
    }

    #[tokio::test]
    async fn private_archive_is_validated_hashed_and_published_without_connection_values() {
        let root = tempfile::tempdir().expect("database export fixture");
        let exporter = exporter(
            root.path(),
            r#"
[ "$#" = 3 ] || exit 1
[ "$1" = '--format=custom' ] || exit 2
[ "$2" = '--no-password' ] || exit 3
[ "$PGPASSWORD" = 'p@ss$word' ] || exit 4
[ "$PGDATABASE" = 'app db' ] || exit 5
[ "$PGHOST" = 'db.example' ] || exit 6
[ "$PGPORT" = '5433' ] || exit 7
[ "$PGUSER" = 'owner' ] || exit 8
[ "$PGSSLMODE" = 'require' ] || exit 9
[ "$PGCHANNELBINDING" = 'require' ] || exit 10
[ -z "$HOME" ] || exit 11
printf 'PGDMPfixture-data'
"#,
            r#"
[ "$#" = 3 ] || exit 1
[ "$1" = '--file' ] || exit 2
[ "$2" = '/dev/null' ] || exit 3
[ -z "$PGPASSWORD" ] || exit 4
[ -z "$PGDATABASE" ] || exit 5
[ -s "$3" ] || exit 6
/bin/cat "$3" > /dev/null
"#,
        );
        let receipt = exporter
            .export("resource", &source())
            .await
            .expect("database export fixture");
        assert_eq!(receipt.bytes, 17);
        assert_eq!(
            receipt.sha256,
            format!("{:x}", Sha256::digest(b"PGDMPfixture-data"))
        );
        assert_eq!(
            std::fs::metadata(&receipt.path)
                .expect("database export fixture")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&exporter.directory)
                .expect("database export fixture")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let manifest = exporter.directory.join(format!("{}.json", receipt.id));
        assert_eq!(
            std::fs::metadata(&manifest)
                .expect("database export fixture")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let json = std::fs::read_to_string(manifest).expect("database export fixture");
        assert_eq!(
            serde_json::from_str::<DatabaseArchive>(&json).expect("database export fixture"),
            receipt
        );
        assert!(!json.contains("p@ss") && !json.contains("postgresql://"));
        assert_eq!(
            std::fs::read_dir(&exporter.directory)
                .expect("database export fixture")
                .count(),
            2
        );
        let page = exporter.archives(None).await.expect("archive catalog");
        assert_eq!(page.items, vec![receipt.clone()]);
        assert!(page.next_cursor.is_none());
        assert!(exporter.archives(Some("../../outside")).await.is_err());
        assert!(
            exporter
                .archives(Some(&receipt.id))
                .await
                .expect("cursor")
                .items
                .is_empty()
        );
        std::fs::write(&receipt.path, b"changed").expect("change archive");
        assert!(exporter.archives(None).await.is_err());
    }

    #[tokio::test]
    async fn failed_oversized_invalid_or_timed_out_exports_never_publish_a_receipt() {
        for (dump, restore, timeout) in [
            ("printf PGDMPpartial; exit 4", "exit 0", false),
            ("printf not-an-archive", "exit 0", false),
            ("printf PGDMPvalid", "exit 4", false),
            ("printf PGDMP; /usr/bin/yes x", "exit 0", false),
            ("printf PGDMP; sleep 30", "exit 0", true),
        ] {
            let root = tempfile::tempdir().expect("database export fixture");
            let mut exporter = exporter(root.path(), dump, restore);
            if timeout {
                exporter.deadline = Duration::from_millis(30);
            }
            assert!(exporter.export("resource", &source()).await.is_err());
            assert_eq!(
                std::fs::read_dir(&exporter.directory)
                    .expect("database export fixture")
                    .count(),
                0
            );
        }
    }

    #[tokio::test]
    async fn connections_cannot_override_destinations_tls_or_local_files() {
        let root = tempfile::tempdir().expect("database export fixture");
        let exporter = exporter(root.path(), "exit 99", "exit 99");
        for uri in [
            "postgresql://owner:password@db.example/other",
            "postgresql://owner:password@db.example/app%20db?sslmode=disable",
            "postgresql://owner:password@db.example/app%20db?passfile=/tmp/leak",
            "postgresql://owner:password@db.example/app%20db?sslmode=require&sslmode=disable",
            "postgresql://owner:password@db.example/app%20db?host=evil.example",
        ] {
            let mut source = source();
            source.connection = crate::domain::Secret::new(uri.into());
            assert!(exporter.export("resource", &source).await.is_err());
            assert_eq!(
                std::fs::read_dir(&exporter.directory)
                    .expect("database export fixture")
                    .count(),
                0
            );
        }
    }

    #[tokio::test]
    async fn export_directory_symlinks_are_rejected() {
        let root = tempfile::tempdir().expect("database export fixture");
        let elsewhere = tempfile::tempdir().expect("database export fixture");
        let exporter = exporter(root.path(), "printf PGDMPvalid", "exit 0");
        std::os::unix::fs::symlink(elsewhere.path(), &exporter.directory)
            .expect("database export fixture");
        assert!(exporter.export("resource", &source()).await.is_err());
        assert_eq!(
            std::fs::read_dir(elsewhere.path())
                .expect("database export fixture")
                .count(),
            0
        );
    }
}
