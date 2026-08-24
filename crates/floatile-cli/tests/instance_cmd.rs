//! `floatile instance` 二进制合同：安装后的同包多实例可独立 CRUD。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use floatile_cli::build_project;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(args)
        .output()
        .expect("floatile CLI 应可执行")
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("测试路径应为 UTF-8")
}

#[test]
fn installed_package_supports_two_independent_instance_lifecycles() {
    let root = std::env::temp_dir().join(format!(
        "floatile-instance-command-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let package = root.join("clock.floatile");
    let plugin_store = root.join("plugins");
    let database = root.join("layout.db");
    build_project(&workspace_root().join("plugins/clock-wasm"), &package).unwrap();

    let installed = run(&[
        "install",
        path_text(&package),
        "--store",
        path_text(&plugin_store),
        "--json",
    ]);
    assert!(installed.status.success(), "{:?}", installed.stderr);

    let create_args = [
        "instance",
        "create",
        "dev.floatile.clock",
        "--version",
        "0.1.0",
        "--db",
        path_text(&database),
        "--store",
        path_text(&plugin_store),
        "--json",
    ];
    let first = run(&create_args);
    let second = run(&create_args);
    assert!(first.status.success(), "{:?}", first.stderr);
    assert!(second.status.success(), "{:?}", second.stderr);
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(first["schemaVersion"], 1);
    let first_id = first["instance"]["instanceId"].as_u64().unwrap();
    let second_id = second["instance"]["instanceId"].as_u64().unwrap();
    assert_ne!(first_id, second_id);

    let second_id_text = second_id.to_string();
    for (action, extra) in [("get", None), ("configure", Some("{}"))] {
        let mut args = vec![
            "instance",
            action,
            &second_id_text,
            "--db",
            path_text(&database),
            "--store",
            path_text(&plugin_store),
            "--json",
        ];
        if let Some(config) = extra {
            args.extend(["--config", config]);
        }
        let output = run(&args);
        assert!(output.status.success(), "{:?}", output.stderr);
    }

    let started = run(&[
        "instance",
        "start",
        &first_id.to_string(),
        "--db",
        path_text(&database),
        "--json",
    ]);
    assert!(started.status.success(), "{:?}", started.stderr);

    let rejected = run(&[
        "instance",
        "delete",
        &first_id.to_string(),
        "--db",
        path_text(&database),
        "--json",
    ]);
    assert!(!rejected.status.success());
    let rejected: serde_json::Value = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(rejected["schemaVersion"], 1);
    assert_eq!(rejected["code"], "FINSTANCE_MUST_BE_STOPPED");

    for action in ["stop", "delete"] {
        let output = run(&[
            "instance",
            action,
            &first_id.to_string(),
            "--db",
            path_text(&database),
            "--json",
        ]);
        assert!(output.status.success(), "{:?}", output.stderr);
    }

    let listed = run(&["instance", "list", "--db", path_text(&database), "--json"]);
    assert!(listed.status.success(), "{:?}", listed.stderr);
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let instances = listed["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0]["instanceId"], second_id);
    assert!(plugin_store.join("dev.floatile.clock/0.1.0").is_dir());

    let _ = std::fs::remove_dir_all(root);
}
