//! `floatile inspect` 的成功、失败与版本化 JSON 契约。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;
use std::{io::Read, io::Write};

use floatile_cli::build_project;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn package_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "floatile-inspect-{tag}-{}.floatile",
        std::process::id()
    ))
}

fn replace_manifest(package: &[u8], manifest: &[u8]) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(package)).unwrap();
    let mut files = Vec::new();
    for index in 0..input.len() {
        let mut entry = input.by_index(index).unwrap();
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        files.push((name, bytes));
    }
    let mut output = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        for (name, bytes) in files {
            zip.start_file(name.clone(), zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(if name == "manifest.json" {
                manifest
            } else {
                &bytes
            })
            .unwrap();
        }
        zip.finish().unwrap();
    }
    output.into_inner()
}

#[test]
fn inspect_reports_validated_package_contract_and_digests() {
    let package = package_path("valid");
    let _ = std::fs::remove_file(&package);
    build_project(&workspace_root().join("plugins/clock-wasm"), &package)
        .expect("clock package should build");

    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args([
            "inspect",
            package.to_str().unwrap(),
            "--json",
            "--no-interactive",
            "--deny-warnings",
        ])
        .output()
        .expect("inspect should run");
    assert!(
        output.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["status"], "ok");
    assert_eq!(report["code"], "ok");
    assert_eq!(report["warnings"], serde_json::json!([]));
    assert_eq!(report["package"]["id"], "dev.floatile.clock");
    assert_eq!(report["compatibility"]["status"], "compatible");
    assert_eq!(report["permissions"][0]["capability"], "timer:schedule");
    assert_eq!(report["digest"].as_str().unwrap().len(), 64);
    let entries = report["entries"].as_array().unwrap();
    assert_eq!(entries.len(), report["budget"]["entryCount"]);
    assert!(entries.iter().all(|entry| {
        entry["sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64)
            && entry["path"].as_str().is_some()
            && entry["bytes"].as_u64().is_some()
    }));
    assert!(
        entries
            .windows(2)
            .all(|pair| pair[0]["path"].as_str() < pair[1]["path"].as_str())
    );

    let human = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["inspect", package.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human_stdout = String::from_utf8(human.stdout).unwrap();
    assert!(human_stdout.contains("dev.floatile.clock"));
    assert!(human_stdout.contains("permission timer:schedule"));

    let _ = std::fs::remove_file(&package);
}

#[test]
fn inspect_rejects_corrupt_packages_with_stable_redacted_json() {
    let package = package_path("corrupt");
    std::fs::write(&package, b"not a zip").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["inspect", package.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["schemaVersion"], 1);
    assert_eq!(error["status"], "error");
    assert_eq!(error["severity"], "error");
    assert_eq!(error["warnings"], serde_json::json!([]));
    assert_eq!(error["code"], "FPAK_CORRUPT_ZIP");
    assert!(
        !error["detail"]
            .as_str()
            .unwrap()
            .contains(package.to_str().unwrap())
    );
    let _ = std::fs::remove_file(&package);
}

#[test]
fn inspect_rejects_unknown_capability_before_reporting_metadata() {
    let valid = package_path("capability-source");
    let invalid = package_path("capability-invalid");
    build_project(&workspace_root().join("plugins/clock-wasm"), &valid).unwrap();
    let valid_bytes = std::fs::read(&valid).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&valid_bytes)).unwrap();
    let mut manifest_bytes = Vec::new();
    archive
        .by_name("manifest.json")
        .unwrap()
        .read_to_end(&mut manifest_bytes)
        .unwrap();
    drop(archive);
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    manifest["permissions"][0]["capability"] = serde_json::json!("network:ambient");
    let invalid_bytes = replace_manifest(&valid_bytes, &serde_json::to_vec(&manifest).unwrap());
    std::fs::write(&invalid, invalid_bytes).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["inspect", invalid.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "非法包不得输出部分元数据");
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "FPAK_INVALID_MANIFEST");

    let _ = std::fs::remove_file(valid);
    let _ = std::fs::remove_file(invalid);
}

#[test]
fn inspect_rejects_unknown_options_with_usage_exit_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["inspect", "plugin.floatile", "--surprise", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "FINSPECT_ARGUMENT");
}
