#![allow(clippy::unwrap_used)]

use std::process::Command;

#[test]
fn conformance_command_exposes_the_versioned_lifecycle_suite() {
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args([
            "conformance",
            "--json",
            "--no-interactive",
            "--deny-warnings",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["status"], "ok");
    assert_eq!(report["suite"], "sdk-lifecycle-v1");
    assert_eq!(report["contract"]["engineApiVersion"], "1.2.0");
    assert_eq!(report["contract"]["vectors"].as_array().unwrap().len(), 3);
    assert_eq!(report["warnings"], serde_json::json!([]));
}

#[test]
fn conformance_command_rejects_unknown_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["conformance", "unexpected", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["code"], "FCONF_ARGUMENT");
}
