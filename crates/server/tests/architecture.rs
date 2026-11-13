//! Enforces the documented dependency direction for business rules and use cases.

use std::path::Path;

fn check_layer(root: &Path, prohibited: &[&str]) {
    let pattern =
        regex::Regex::new(&format!(r"\b({})\s*::", prohibited.join("|"))).expect("pattern");
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry.expect("source directory");
        if entry.path().extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).expect("source file");
        // Paths in prose are not dependencies. Paths in use trees and expressions are.
        let code = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !pattern.is_match(&code),
            "Forbidden dependency in {}",
            entry.path().display()
        );
    }
}

#[test]
fn domain_and_application_do_not_depend_on_transport_or_storage_implementations() {
    // The domain is its own crate, so Cargo already stops it using the engine's
    // layers. This also catches I/O reached through the standard library.
    check_layer(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../domain/src"),
        &[
            "application",
            "infrastructure",
            "delivery",
            "sqlx",
            "reqwest",
            "axum",
            "hyper",
            "tokio",
            "std\\s*::\\s*fs",
        ],
    );
    // The app is its own crate too; this also catches libraries that would reach
    // transport or storage directly if they were ever added to its manifest.
    check_layer(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/src"),
        &[
            "infrastructure",
            "delivery",
            "sqlx",
            "reqwest",
            "axum",
            "hyper",
            "tokio_tungstenite",
        ],
    );
}

#[test]
fn only_the_wiring_names_storage_implementations() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut folders = vec![src];
    let mut files = Vec::new();
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).expect("server sources") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                folders.push(path);
            } else {
                files.push(path);
            }
        }
    }
    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.extension().is_none_or(|e| e != "rs") || matches!(name, "wiring.rs" | "lib.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source");
        // Unit tests may set up a real store.
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        if code.contains("infrastructure::") || code.contains("hoista_store") {
            offenders.push(name.to_owned());
        }
    }
    assert!(
        offenders.is_empty(),
        "these server files reach storage directly instead of through ports or wiring.rs: {offenders:?}"
    );
}
