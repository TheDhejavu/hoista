//! CLI smoke tests using isolated state directories and no native credentials.

use std::process::Command;

#[test]
fn profile_setup_round_trip_and_validation_work_without_a_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state");
    let output = Command::new(env!("CARGO_BIN_EXE_hoist"))
        .args([
            "profile",
            "set",
            "--name",
            "Ada Lovelace",
            "--initials",
            "AL",
            "--state-dir",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_hoist"))
        .args(["profile", "--json", "--state-dir"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let profile: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(profile["name"], "Ada Lovelace");
    let invalid = directory.path().join("invalid");
    let output = Command::new(env!("CARGO_BIN_EXE_hoist"))
        .args([
            "profile",
            "set",
            "--name",
            "",
            "--initials",
            "A",
            "--state-dir",
        ])
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!invalid.exists());
}
