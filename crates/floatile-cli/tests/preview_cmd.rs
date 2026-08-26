//! `preview` 参数/诊断契约与真实宿主会话替换证据。

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn preview_argument_errors_use_shared_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["preview", "--duration-ms", "0", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["code"], "FPREVIEW_ARGUMENT");
    assert_eq!(value["severity"], "error");
}

#[test]
#[ignore = "requires Xvfb and FLOATTILE_PREVIEW_HOST"]
fn real_preview_replacement_reaches_running() {
    let project = workspace_root().join("plugins/clock-wasm");
    let first = floatile_cli::PreviewSession::start(&project, Duration::from_secs(5))
        .expect("start first preview");
    std::thread::sleep(Duration::from_millis(100));
    let second = floatile_cli::PreviewSession::start(&project, Duration::from_millis(800))
        .expect("start replacement preview");
    drop(first);
    let report = second.wait().expect("wait for replacement");
    assert!(report.running, "replacement failed: {report:?}");
    assert_eq!(report.code, "ok");
}
