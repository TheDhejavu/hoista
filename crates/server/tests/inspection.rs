//! Detected authentication requirements expose names without generating secret values.

use hoista_server::infrastructure::inspection::detect;
#[test]
fn better_auth_is_generated_but_unrelated_api_keys_remain_required() {
    let source = tempfile::tempdir().expect("source");
    std::fs::write(
        source.path().join("package.json"),
        r#"{"dependencies":{"express":"5","better-auth":"1"}}"#,
    )
    .expect("manifest");
    std::fs::write(
        source.path().join("server.ts"),
        "const key = process.env.STRIPE_SECRET_KEY;",
    )
    .expect("source");
    for generated in [
        ".next",
        ".open-next",
        ".svelte-kit",
        ".wrangler",
        ".hoista-worker",
    ] {
        std::fs::create_dir(source.path().join(generated)).unwrap();
        std::fs::write(
            source.path().join(generated).join("old.js"),
            "process.env.OLD_GENERATED_SECRET",
        )
        .unwrap();
    }
    let detected = detect(source.path()).expect("detection");
    assert_eq!(detected.generated_secret_names, ["BETTER_AUTH_SECRET"]);
    assert_eq!(
        detected.env_names,
        ["BETTER_AUTH_SECRET", "STRIPE_SECRET_KEY"]
    );
    std::fs::write(
        source.path().join("package.json"),
        r#"{"dependencies":{"express":"5"}}"#,
    )
    .expect("manifest");
    assert!(
        detect(source.path())
            .expect("detection")
            .generated_secret_names
            .is_empty()
    );
}

#[test]
fn literal_python_and_indexed_javascript_environment_names_are_detected_without_values() {
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("requirements.txt"),
        "fastapi==0.115.12\n",
    )
    .unwrap();
    std::fs::write(
        source.path().join("main.py"),
        r#"
import os
first = os.getenv("APP_LABEL", "default")
second = os.environ['API_TOKEN']
third = os.environ.get( "OPTIONAL_FLAG" )
fourth = os.getenv('OTHER_TOKEN')
dynamic = os.getenv("DYNAMIC" + suffix)
also_dynamic = os.environ[name]
"#,
    )
    .unwrap();
    std::fs::write(
        source.path().join("client.ts"),
        r#"
const a = process.env['SERVER_NAME'];
const b = import.meta.env["VITE_PUBLIC_LABEL"];
const c = process.env["DYNAMIC" + suffix];
"#,
    )
    .unwrap();
    std::fs::write(
        source.path().join(".env"),
        "DO_NOT_INSPECT_VALUE=private-value\n",
    )
    .unwrap();
    let detected = detect(source.path()).unwrap();
    assert_eq!(detected.framework, "python");
    assert_eq!(
        detected.env_names,
        [
            "API_TOKEN",
            "APP_LABEL",
            "OPTIONAL_FLAG",
            "OTHER_TOKEN",
            "SERVER_NAME",
            "VITE_PUBLIC_LABEL"
        ]
    );
    assert!(!format!("{detected:?}").contains("private-value"));
}
