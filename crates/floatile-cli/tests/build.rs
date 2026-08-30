//! 集成测试：对真实作者项目（plugins/clock-wasm）执行 build_project 全链路。
//!
//! 覆盖：cargo metadata → wasm 编译 → build_ftui 生成 widget.ftui → manifest
//! 生成 → zip 打包 → 自校验。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use floatile_cli::package::PackageLimits;
use floatile_cli::{BuildError, build_project, validate_package};
use floatile_core::{manifest_json_schema, validate_manifest_json_with_schema};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
fn builds_clock_wasm_project_end_to_end() {
    let project_dir = workspace_root().join("plugins/clock-wasm");
    let out = std::env::temp_dir().join(format!(
        "floatile-build-e2e-{}.floatile",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out);

    let manifest = build_project(&project_dir, &out).expect("build_project 应成功");
    assert_eq!(manifest.id.0, "dev.floatile.clock");

    // 产物可被 validate 接受，且包含 manifest/ui/logic 三入口。
    let bytes = std::fs::read(&out).unwrap();
    let validated = validate_package(&bytes, &PackageLimits::default()).unwrap();
    assert_eq!(validated.manifest.id.0, "dev.floatile.clock");
    assert_eq!(validated.entry_names.len(), 3);

    let _ = std::fs::remove_file(&out);
}

#[test]
fn build_project_accepts_relative_project_directory() {
    let workspace = workspace_root();
    let current = std::env::current_dir().unwrap().canonicalize().unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let relative = if current == workspace {
        PathBuf::from("plugins/clock-wasm")
    } else if current == workspace.join("crates/floatile-cli") {
        PathBuf::from("../../plugins/clock-wasm")
    } else {
        panic!("非预期测试工作目录: {}", current.display());
    };
    let out = std::env::temp_dir().join(format!(
        "floatile-build-relative-{}.floatile",
        std::process::id()
    ));
    let manifest = build_project(&relative, &out).expect("相对项目目录应从当前目录解析");
    assert_eq!(manifest.id.0, "dev.floatile.clock");
    let _ = std::fs::remove_file(out);
}

#[test]
fn build_fails_cleanly_on_missing_project() {
    let result = build_project(
        &std::env::temp_dir().join("does-not-exist-floatile"),
        &PathBuf::from("/tmp/x.floatile"),
    );
    assert!(matches!(result, Err(BuildError::CargoMetadata(_))));
}

#[test]
fn manifest_json_schema_artifact_validates_same_as_serde() {
    // 独立 schema 产物（CLI schema 命令输出面）必须与 Manifest serde 序列化一致：
    // 对 clock-wasm 构建出的 manifest 做 self-consistency 校验，证明单源无 drift。
    let project_dir = workspace_root().join("plugins/clock-wasm");
    let out = std::env::temp_dir().join(format!(
        "floatile-schema-e2e-{}.floatile",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out);

    let manifest = build_project(&project_dir, &out).expect("build_project 应成功");
    let manifest_json = serde_json::to_value(&manifest).unwrap();

    // 生成的独立 JSON Schema 可序列化且为 draft-07 object。
    let schema = manifest_json_schema();
    assert_eq!(
        schema["$schema"],
        serde_json::json!("http://json-schema.org/draft-07/schema#")
    );
    assert!(schema["type"] == serde_json::json!("object"));

    // schema 能通过自身校验构建出的 manifest（结构一致性，无 drift）。
    assert!(
        validate_manifest_json_with_schema(&manifest_json).is_ok(),
        "独立 manifest schema 应接受由 serde 序列化的 manifest"
    );

    let _ = std::fs::remove_file(&out);
}
