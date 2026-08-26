//! `floatile check` 的阶段、JSON、失败脱敏与临时产物清理契约。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn command_with_temp(temp: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_floatile"));
    command
        .env("TMPDIR", temp)
        .env("TMP", temp)
        .env("TEMP", temp);
    command
}

#[test]
fn check_validates_real_author_project_without_retaining_package() {
    let temp = std::env::temp_dir().join(format!("floatile-check-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir(&temp).unwrap();
    let output = command_with_temp(&temp)
        .args([
            "check",
            workspace_root()
                .join("plugins/clock-wasm")
                .to_str()
                .unwrap(),
            "--json",
            "--no-interactive",
            "--deny-warnings",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["status"], "ok");
    assert_eq!(report["code"], "ok");
    assert_eq!(report["inspection"]["package"]["id"], "dev.floatile.clock");
    for phase in ["metadata", "wasm", "ui", "manifest", "package"] {
        assert_eq!(report["phases"][phase], true, "phase {phase}");
    }
    assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
fn check_failure_is_stable_redacted_and_cleans_temporary_directory() {
    let temp = std::env::temp_dir().join(format!(
        "floatile-check-failure-test-{}",
        std::process::id()
    ));
    let missing = temp.join("private-project-name");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir(&temp).unwrap();
    let output = command_with_temp(&temp)
        .args(["check", missing.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["status"], "error");
    assert_eq!(report["severity"], "error");
    assert_eq!(report["code"], "FBUILD_CARGO_METADATA");
    assert_eq!(report["phases"]["metadata"], false);
    assert!(
        !report["detail"]
            .as_str()
            .unwrap()
            .contains("private-project-name")
    );
    assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
fn check_unknown_option_uses_json_mode_regardless_of_option_order() {
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["check", "--surprise", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["code"], "FCHECK_ARGUMENT");
    assert_eq!(report["severity"], "error");
}
