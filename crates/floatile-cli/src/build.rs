//! `build`：从作者项目自动构建 `.floatile` 包。
//!
//! 编排：`cargo metadata` 定位产物 → `cargo build --target wasm32-wasip2`（组件）
//! → `cargo run --bin build_ftui --features build-host`（宿主生成 widget.ftui）
//! → `floatile.toml` 生成 manifest → zip 打包 → 自校验。

use std::borrow::Cow;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use floatile_core::manifest::Manifest;

use crate::package::{PackageError, PackageLimits, validate_package};
use crate::project;

/// 构建错误（稳定 code `FBUILD_*`）。
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("项目目录不可用: {0}")]
    ProjectDirectory(String),
    #[error("项目配置失败: {0}")]
    Project(#[from] project::ProjectError),
    #[error("cargo 元数据失败: {0}")]
    CargoMetadata(String),
    #[error("wasm 构建失败（需要 wasm32-wasip2 target）: {0}")]
    WasmBuild(String),
    #[error("build_ftui 运行失败: {0}")]
    BuildFtui(String),
    #[error("打包失败: {0}")]
    Package(#[from] PackageError),
    #[error("I/O 失败: {0}")]
    Io(String),
}

impl BuildError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProjectDirectory(_) => "FBUILD_PROJECT_DIRECTORY",
            Self::Project(_) => "FBUILD_PROJECT",
            Self::CargoMetadata(_) => "FBUILD_CARGO_METADATA",
            Self::WasmBuild(_) => "FBUILD_WASM_BUILD",
            Self::BuildFtui(_) => "FBUILD_FTUI",
            Self::Package(_) => "FBUILD_PACKAGE",
            Self::Io(_) => "FBUILD_IO",
        }
    }

    /// Agent/CI 可见的有界描述，不包含 cargo stderr 或宿主路径。
    pub fn public_detail(&self) -> Cow<'static, str> {
        match self {
            Self::ProjectDirectory(_) => Cow::Borrowed("项目目录不存在或不可访问"),
            Self::Project(_) => Cow::Borrowed("项目配置无效"),
            Self::CargoMetadata(_) => Cow::Borrowed("Cargo 项目元数据检查失败"),
            Self::WasmBuild(_) => Cow::Borrowed("WASM Component 构建失败"),
            Self::BuildFtui(_) => Cow::Borrowed("Floatile UI 生成失败"),
            Self::Package(_) => Cow::Borrowed("生成包未通过安全校验"),
            Self::Io(_) => Cow::Borrowed("项目输入或构建产物 I/O 失败"),
        }
    }
}

/// cargo metadata 的关键字段。
struct CargoMeta {
    package_name: String,
    target_dir: PathBuf,
}

fn cargo_metadata(manifest: &Path) -> Result<CargoMeta, BuildError> {
    let project_dir = manifest.parent().unwrap_or_else(|| Path::new("."));
    let output = Command::new("cargo")
        .current_dir(project_dir)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .map_err(|e| BuildError::CargoMetadata(e.to_string()))?;
    if !output.status.success() {
        return Err(BuildError::CargoMetadata(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| BuildError::CargoMetadata(format!("解析 metadata JSON: {e}")))?;
    let target_dir = json
        .get("target_directory")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| BuildError::CargoMetadata("缺少 target_directory".to_owned()))?;
    // workspace 的 metadata 会列出全部成员；按 manifest_path 精确匹配目标包。
    let manifest_abs = manifest
        .canonicalize()
        .unwrap_or_else(|_| manifest.to_path_buf());
    let package_name = json
        .get("packages")
        .and_then(|v| v.as_array())
        .and_then(|packages| {
            packages.iter().find_map(|p| {
                let p_manifest = p
                    .get("manifest_path")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from);
                let matched = p_manifest
                    .as_ref()
                    .and_then(|m| m.canonicalize().ok())
                    .map(|m| m == manifest_abs)
                    .unwrap_or(false);
                matched.then(|| {
                    p.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned()
                })
            })
        })
        .ok_or_else(|| BuildError::CargoMetadata("找不到目标 package".to_owned()))?;
    Ok(CargoMeta {
        package_name,
        target_dir,
    })
}

/// 构建项目目录中的插件，输出 `.floatile` 到 `out`。
pub fn build_project(project_dir: &Path, out: &Path) -> Result<Manifest, BuildError> {
    let project_dir = project_dir.canonicalize().map_err(|error| {
        BuildError::ProjectDirectory(format!(
            "无法解析项目目录 {}: {error}",
            project_dir.display()
        ))
    })?;
    let manifest_path = project_dir.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(BuildError::CargoMetadata(format!(
            "{} 不存在",
            manifest_path.display()
        )));
    }
    let meta = cargo_metadata(&manifest_path)?;

    // 1. 编译 wasm 组件。
    let wasm_build = Command::new("cargo")
        .current_dir(&project_dir)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .arg("--manifest-path")
        .arg(&manifest_path)
        .output()
        .map_err(|e| BuildError::WasmBuild(e.to_string()))?;
    if !wasm_build.status.success() {
        return Err(BuildError::WasmBuild(
            String::from_utf8_lossy(&wasm_build.stderr).to_string(),
        ));
    }
    let wasm_path = meta
        .target_dir
        .join("wasm32-wasip2/release")
        .join(format!("{}.wasm", meta.package_name.replace('-', "_")));
    let wasm = std::fs::read(&wasm_path)
        .map_err(|e| BuildError::WasmBuild(format!("读取 {} 失败: {e}", wasm_path.display())))?;

    // 2. 宿主运行 build_ftui 生成 widget.ftui。
    let ftui_output = Command::new("cargo")
        .current_dir(&project_dir)
        // Author projects conventionally use the same `build_ftui` binary name. Cargo places
        // host binaries directly under the target profile directory, so concurrent builds of
        // different plugins can otherwise overwrite/run each other's executable on Windows.
        .env(
            "CARGO_TARGET_DIR",
            meta.target_dir
                .join("floatile-host")
                .join(&meta.package_name),
        )
        .args([
            "run",
            "--quiet",
            "--bin",
            "build_ftui",
            "--features",
            "build-host",
        ])
        .arg("--manifest-path")
        .arg(&manifest_path)
        .output()
        .map_err(|e| BuildError::BuildFtui(e.to_string()))?;
    if !ftui_output.status.success() {
        return Err(BuildError::BuildFtui(
            String::from_utf8_lossy(&ftui_output.stderr).to_string(),
        ));
    }
    let ftui_json = String::from_utf8(ftui_output.stdout)
        .map_err(|e| BuildError::BuildFtui(format!("build_ftui 输出非 UTF-8: {e}")))?;
    // 校验 ftui 是合法 JSON。
    serde_json::from_str::<serde_json::Value>(&ftui_json)
        .map_err(|e| BuildError::BuildFtui(format!("build_ftui 输出非 JSON: {e}")))?;

    // 3. floatile.toml → manifest。
    let toml_text = std::fs::read_to_string(project_dir.join("floatile.toml"))
        .map_err(|e| BuildError::Io(format!("读取 floatile.toml: {e}")))?;
    let config = project::parse_floatile_toml(&toml_text)?;
    let manifest = project::generate_manifest(&config)?;

    // 4. 打包 + 自校验。
    package(&manifest, &ftui_json, &wasm, out)?;
    Ok(manifest)
}

/// 打包 `.floatile` zip 并自校验。`out` 为输出文件路径（`.floatile`）。
pub fn package(
    manifest: &Manifest,
    ftui_json: &str,
    wasm: &[u8],
    out: &Path,
) -> Result<(), PackageError> {
    let manifest_json =
        serde_json::to_string(manifest).map_err(|e| PackageError::InvalidJson(e.to_string()))?;

    let mut bytes = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("manifest.json", options)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .write_all(manifest_json.as_bytes())
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .start_file("ui/widget.ftui", options)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .write_all(ftui_json.as_bytes())
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .start_file("logic/plugin.wasm", options)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .write_all(wasm)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    }

    // 自校验：产物必须通过包校验。
    validate_package(&bytes, &PackageLimits::default())?;

    let mut file =
        std::fs::File::create(out).map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    file.write_all(&bytes)
        .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    Ok(())
}
