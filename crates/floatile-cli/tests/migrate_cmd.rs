#![allow(clippy::unwrap_used)]

use std::process::Command;

fn legacy_project(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "floatile-migrate-cmd-{}-{label}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("floatile.toml"),
        "[plugin]\nid = \"dev.floatile.legacy\"\nname = \"Legacy\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    dir
}

#[test]
fn migrate_defaults_to_read_only_dry_run() {
    let dir = legacy_project("dry-run");
    let before = std::fs::read_to_string(dir.join("floatile.toml")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args([
            "migrate",
            dir.to_str().unwrap(),
            "--json",
            "--no-interactive",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["mode"], "dry-run");
    assert_eq!(report["changed"], false);
    assert_eq!(report["changes"][0]["code"], "FMIGRATE_SDK_EXPLICIT");
    assert_eq!(
        std::fs::read_to_string(dir.join("floatile.toml")).unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn migrate_write_is_explicit_and_idempotent() {
    let dir = legacy_project("write");
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args([
            "migrate",
            dir.to_str().unwrap(),
            "--write",
            "--json",
            "--no-interactive",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["mode"], "write");
    assert_eq!(report["changed"], true);
    let migrated = std::fs::read_to_string(dir.join("floatile.toml")).unwrap();
    assert_eq!(migrated.matches("[sdk]").count(), 1);

    let second = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["migrate", dir.to_str().unwrap(), "--write", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(report["changed"], false);
    assert_eq!(report["changes"], serde_json::json!([]));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn migrate_rejects_unknown_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["migrate", "--apply", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["code"], "FMIGRATE_ARGUMENT");
}
