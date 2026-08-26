//! PP-M4：从 Cargo 包快照而非仓库私有 path 构建干净目录中的 Rust 作者项目。

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use floatile_cli::generate_template;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn run(command: &mut Command, label: &str) {
    let output = command.output().expect(label);
    assert!(
        output.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn cargo_package(root: &Path, package: &str, sdk_dependencies: bool) {
    let mut command = Command::new("cargo");
    command.current_dir(root).env("RUSTC_WRAPPER", "").args([
        "package",
        "-p",
        package,
        "--no-verify",
        "--allow-dirty",
        "--locked",
    ]);
    if sdk_dependencies {
        command.args([
            "--config",
            "patch.crates-io.floatile-sdk-macros.path=\"crates/floatile-sdk-macros\"",
            "--config",
            "patch.crates-io.floatile-ui-schema.path=\"crates/floatile-ui-schema\"",
        ]);
    }
    run(&mut command, &format!("package {package}"));
}

fn unpack(root: &Path, temp: &Path, package: &str) -> PathBuf {
    let source = root.join(format!("target/package/{package}-0.1.0.crate"));
    run(
        Command::new("tar")
            .arg("-xzf")
            .arg(source)
            .arg("-C")
            .arg(temp),
        &format!("unpack {package}"),
    );
    temp.join(format!("{package}-0.1.0"))
}

#[test]
fn generated_project_resolves_only_packaged_sdk_sources() {
    let root = workspace_root();
    cargo_package(&root, "floatile-ui-schema", false);
    cargo_package(&root, "floatile-sdk-macros", false);
    cargo_package(&root, "floatile-sdk", true);

    let temp =
        std::env::temp_dir().join(format!("floatile-sdk-package-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir(&temp).expect("create temp root");

    let ui_schema = unpack(&root, &temp, "floatile-ui-schema");
    let macros = unpack(&root, &temp, "floatile-sdk-macros");
    let sdk = unpack(&root, &temp, "floatile-sdk");
    let project = temp.join("author-project");
    generate_template(&project, "dev.example.clean", "Clean Project")
        .expect("generate author project");

    let patch = |name: &str, path: &Path| {
        format!(
            "patch.crates-io.{name}.path={:?}",
            path.display().to_string()
        )
    };
    let mut check = Command::new("cargo");
    check
        .env("RUSTC_WRAPPER", "")
        .arg("check")
        .arg("--manifest-path")
        .arg(project.join("Cargo.toml"))
        .args(["--target", "wasm32-wasip2"])
        .args(["--config", &patch("floatile-sdk", &sdk)])
        .args(["--config", &patch("floatile-sdk-macros", &macros)])
        .args(["--config", &patch("floatile-ui-schema", &ui_schema)])
        .env("CARGO_TARGET_DIR", temp.join("target"));
    run(&mut check, "check generated author project");

    let manifest = std::fs::read_to_string(project.join("Cargo.toml")).unwrap();
    assert!(!manifest.contains(root.to_string_lossy().as_ref()));
    assert!(sdk.join("wit/floatile-widget.wit").is_file());

    std::fs::remove_dir_all(temp).expect("clean temp root");
}
