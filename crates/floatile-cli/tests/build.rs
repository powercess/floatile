//! 集成测试：对真实作者项目（plugins/clock-wasm）执行 build_project 全链路。
//!
//! 覆盖：cargo metadata → wasm 编译 → build_ftui 生成 widget.ftui → manifest
//! 生成 → zip 打包 → 自校验。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use floatile_cli::package::PackageLimits;
use floatile_cli::{BuildError, build_project, validate_package};

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
fn build_fails_cleanly_on_missing_project() {
    let result = build_project(
        &std::env::temp_dir().join("does-not-exist-floatile"),
        &PathBuf::from("/tmp/x.floatile"),
    );
    assert!(matches!(result, Err(BuildError::CargoMetadata(_))));
}
