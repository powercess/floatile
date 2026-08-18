//! 集成测试：用真实 clock-wasm 构建 `.floatile` 并自校验。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use floatile_cli::package::PackageLimits;
use floatile_cli::{generate_manifest, package, parse_floatile_toml, validate_package};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn clock_wasm() -> Vec<u8> {
    let wasm_path = workspace_root().join("target/wasm32-wasip2/debug/floatile_clock_wasm.wasm");
    if !wasm_path.exists() {
        let status = Command::new("cargo")
            .current_dir(workspace_root())
            .args([
                "build",
                "-p",
                "floatile-clock-wasm",
                "--target",
                "wasm32-wasip2",
            ])
            .status()
            .expect("failed to build clock-wasm");
        assert!(status.success(), "clock-wasm 构建失败");
    }
    std::fs::read(&wasm_path).unwrap()
}

fn sample_toml() -> &'static str {
    r#"[plugin]
id = "dev.floatile.clock"
name = "World Clock"
version = "0.1.0"
[widget]
default_size = [240, 120]
[permissions.timer]
max_per_minute = 60
max_active = 2
"#
}

fn clock_ftui() -> String {
    // 与 clock 的 State 结构一致的最小 widget.ftui。
    serde_json::json!({
        "uiApiVersion": "1.0.0",
        "state": {
            "initial": { "time": "--:--:--", "running": false },
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["time", "running"],
                "properties": {
                    "time": { "type": "string", "maxLength": 32 },
                    "running": { "type": "boolean" }
                }
            }
        },
        "events": {},
        "root": { "type": "Column", "props": {}, "children": [
            { "type": "Text", "props": { "text": { "bind": "$.time" } }, "children": [] }
        ] }
    })
    .to_string()
}

#[test]
fn builds_and_validates_package() {
    let config = parse_floatile_toml(sample_toml()).unwrap();
    let manifest = generate_manifest(&config).unwrap();
    let wasm = clock_wasm();
    assert!(!wasm.is_empty(), "需要先构建 clock-wasm");

    let out = std::env::temp_dir().join(format!(
        "floatile-build-test-{}.floatile",
        std::process::id()
    ));
    package(&manifest, &clock_ftui(), &wasm, &out).unwrap();

    // 产物可被 validate 接受。
    let bytes = std::fs::read(&out).unwrap();
    let validated = validate_package(&bytes, &PackageLimits::default()).unwrap();
    assert_eq!(validated.manifest.id.0, "dev.floatile.clock");
    assert_eq!(validated.entry_names.len(), 3);

    let _ = std::fs::remove_file(&out);
}
