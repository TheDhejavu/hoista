//! Prisma: picking production migrations and running its CLI in isolation,
//! with no external accounts.

use hoista_server::infrastructure::inspection::detect;
// Only the migration test uses these, and it runs on Unix only.
#[cfg(unix)]
use hoista_server::{
    application::ports::build::BuildService,
    domain::{Secret, model::*},
    infrastructure::build::LocalBuilder,
};
use std::path::Path;
fn source(root: &Path) {
    std::fs::create_dir_all(root.join("prisma/migrations/20260101_initial")).expect("history");
    std::fs::write(
        root.join("package.json"),
        r#"{"dependencies":{"express":"1","@prisma/client":"7"},"devDependencies":{"prisma":"7"}}"#,
    )
    .expect("package");
    std::fs::write(
        root.join("prisma/schema.prisma"),
        "datasource db {\n provider = \"postgresql\"\n url = env(\"DATABASE_URL\")\n}\n",
    )
    .expect("schema");
    std::fs::write(
        root.join("prisma/migrations/20260101_initial/migration.sql"),
        "CREATE TABLE example (id INTEGER PRIMARY KEY);",
    )
    .expect("migration");
}

#[test]
fn inspection_requires_checked_in_postgres_history_and_accepts_custom_schema() {
    let dir = tempfile::tempdir().expect("directory");
    source(dir.path());
    assert!(
        detect(dir.path())
            .expect("detected")
            .prisma_migration
            .is_some()
    );
    std::fs::write(
        dir.path().join("prisma/schema.prisma"),
        "// provider = \"postgresql\"\ndatasource db { provider = \"sqlite\" }",
    )
    .expect("schema");
    assert!(detect(dir.path()).is_err());
    std::fs::write(
        dir.path().join("prisma/schema.prisma"),
        "datasource db { provider = \"postgresql\"\nurl = \"postgresql://other/db\" }",
    )
    .expect("schema");
    assert!(detect(dir.path()).is_err());
    source(dir.path());
    std::fs::rename(dir.path().join("prisma"), dir.path().join("database")).expect("move schema");
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"dependencies":{"express":"1","@prisma/client":"7"},"devDependencies":{"prisma":"7"},"prisma":{"schema":"database/schema.prisma"}}"#,
    )
    .expect("package");
    assert_eq!(
        detect(dir.path())
            .expect("custom schema")
            .prisma_migration
            .expect("migration")
            .migrations,
        "database/migrations"
    );
    std::fs::remove_file(
        dir.path()
            .join("database/migrations/20260101_initial/migration.sql"),
    )
    .expect("remove history");
    assert!(detect(dir.path()).is_err());
}

#[cfg(unix)]
#[test]
fn inspection_rejects_schema_parent_symlinks() {
    let dir = tempfile::tempdir().expect("directory");
    let outside = tempfile::tempdir().expect("outside");
    source(dir.path());
    std::fs::rename(dir.path().join("prisma"), outside.path().join("prisma")).expect("move");
    std::os::unix::fs::symlink(outside.path().join("prisma"), dir.path().join("prisma"))
        .expect("link");
    assert!(detect(dir.path()).is_err());
}

// Local builds and migrations run in isolation, which Windows does not have yet.
#[cfg(unix)]
#[tokio::test]
async fn migration_cli_uses_direct_environment_and_never_reset_or_seed() {
    let dir = tempfile::tempdir().expect("directory");
    let builder = LocalBuilder::new(dir.path());
    let id = uuid::Uuid::new_v4().to_string();
    let workspace = dir.path().join("builds").join(&id);
    source(&workspace);
    std::fs::create_dir_all(workspace.join("node_modules/prisma/build")).expect("cli directory");
    std::fs::write(
        workspace.join("node_modules/prisma/package.json"),
        r#"{"version":"7.0.0"}"#,
    )
    .expect("version");
    std::fs::write(workspace.join("node_modules/prisma/build/index.js"), r#"
const fs = require('node:fs');
if (process.env.DATABASE_URL !== 'fixture-direct' || process.env.DIRECT_URL !== 'fixture-direct') process.exit(2);
if (JSON.stringify(process.argv.slice(2)) !== JSON.stringify(['migrate','deploy','--schema','prisma/schema.prisma','--config','.hoista-prisma.config.ts'])) process.exit(3);
if (fs.existsSync('fail')) { console.error('fixture-direct'); process.exit(1); }
fs.writeFileSync('applied', 'deploy');
"#).expect("cli");
    let plan = Plan {
        revision: 0,
        id: "plan".into(),
        source: Source::Folder {
            path: workspace.to_string_lossy().into(),
        },
        framework: "node".into(),
        package_manager: Some("npm".into()),
        build_command: vec![],
        output_directory: ".".into(),
        env_names: vec![],
        generated_secret_names: vec![],
        prisma_migration: Some(PrismaMigration {
            schema: "prisma/schema.prisma".into(),
            migrations: "prisma/migrations".into(),
        }),
        resources: vec![],
        warnings: vec![],
    };
    builder
        .migrate(&plan, &id, &Secret::new("fixture-direct".into()))
        .await
        .expect("migration");
    assert_eq!(
        std::fs::read_to_string(workspace.join("applied")).expect("marker"),
        "deploy"
    );
    assert!(
        !std::fs::read_to_string(workspace.join(".hoista-prisma.config.ts"))
            .expect("config")
            .contains("fixture-direct")
    );
    std::fs::write(workspace.join("fail"), "").expect("failure switch");
    let error = builder
        .migrate(&plan, &id, &Secret::new("fixture-direct".into()))
        .await
        .expect_err("failure");
    assert!(!error.to_string().contains("fixture-direct"));
    builder.cleanup(&id).await.expect("cleanup");
    assert!(!workspace.exists());
}
