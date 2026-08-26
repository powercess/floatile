//! `run` 参数契约与持久 Installation/Instance/真实宿主 E2E。

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn run_argument_errors_use_shared_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["run", "--duration-ms", "0", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["code"], "FRUN_ARGUMENT");
}

#[test]
#[ignore = "requires Xvfb and FLOATTILE_PREVIEW_HOST"]
fn run_persists_exact_instance_and_reuses_identical_installation() {
    let temp = std::env::temp_dir().join(format!("floatile-run-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let store = temp.join("plugins");
    let database = temp.join("layout.db");
    std::fs::create_dir_all(&store).unwrap();
    let project = workspace_root().join("plugins/clock-wasm");

    let first = floatile_cli::run_project(
        &project,
        &database,
        &store,
        std::time::Duration::from_millis(700),
    )
    .expect("first run");
    let second = floatile_cli::run_project(
        &project,
        &database,
        &store,
        std::time::Duration::from_millis(700),
    )
    .expect("second run");
    assert!(first.running && second.running);
    assert_ne!(first.instance_id, second.instance_id);
    let instances = floatile_cli::list_instances(&database).unwrap();
    assert_eq!(instances.len(), 2);
    assert!(instances.iter().all(|instance| {
        instance.desired_state == floatile_core::InstanceDesiredState::Running
            && instance.generation == 1
            && instance.digest == instances[0].digest
    }));
    std::fs::remove_dir_all(temp).unwrap();
}
