//! `new/build/test/install` 共享自动化参数、JSON 与退出码契约。

#![allow(clippy::unwrap_used)]

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(args)
        .output()
        .unwrap()
}

fn error(args: &[&str], code: &str, exit: i32) -> serde_json::Value {
    let output = run(args);
    assert_eq!(output.status.code(), Some(exit));
    assert!(output.stdout.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["status"], "error");
    assert_eq!(value["severity"], "error");
    assert_eq!(value["code"], code);
    assert_eq!(value["warnings"], serde_json::json!([]));
    value
}

#[test]
fn automation_flags_are_not_misparsed_as_paths() {
    let temp = std::env::temp_dir().join(format!("floatile-new-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let output = run(&[
        "new",
        "--json",
        "--no-interactive",
        "--deny-warnings",
        temp.to_str().unwrap(),
        "dev.example.output",
        "Output Test",
    ]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["status"], "ok");
    assert_eq!(value["project"]["id"], "dev.example.output");
    assert!(temp.join("Cargo.toml").is_file());
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn author_command_argument_errors_share_exit_and_json_contract() {
    error(&["new", "--surprise", "--json"], "FNEW_ARGUMENT", 2);
    error(&["build", "--surprise", "--json"], "FBUILD_ARGUMENT", 2);
    error(&["test", "--timeout", "--json"], "FTEST_ARGUMENT", 2);
    error(&["install", "--json"], "FINST_ARGUMENT", 2);
}

#[test]
fn behavior_failures_are_redacted_and_use_exit_one() {
    let private = "/tmp/private-author-project-name";
    let build = error(&["build", private, "--json"], "FBUILD_CARGO_METADATA", 1);
    assert!(!build["detail"].as_str().unwrap().contains(private));
    let install = error(
        &["install", private, "--store", "/tmp", "--json"],
        "FINST_IO",
        1,
    );
    assert!(!install["detail"].as_str().unwrap().contains(private));
}
