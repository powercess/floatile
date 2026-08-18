//! `.floatile` 包安全校验（manifest-v1 §6 的安装前校验顺序）。
//!
//! 有界读取 zip：预算（条目数/大小/压缩比）、规范化路径（绝对/`..`/`.`/反斜杠/
//! NUL/前导点）、大小写碰撞与 symlink/设备条目拒绝；随后校验 manifest（core）、
//! UI IR（ui-schema）与 WASM Component（world/import 白名单）。任何失败都返回稳定
//! code 并拒绝安装，不泄漏宿主内部结构。

use std::collections::BTreeSet;
use std::io::{Cursor, Read};

use floatile_core::ManifestError;
use floatile_core::manifest::{Manifest, PackagePath, validate_manifest};
use floatile_ui_schema::UiDocument;
use floatile_ui_schema::UiSchemaError;
use thiserror::Error;
use wasmtime::component::Component;

/// P0 包预算（manifest-v1 §6；evil corpus 数据后可冻结）。
#[derive(Debug, Clone, Copy)]
pub struct PackageLimits {
    /// 压缩包最大字节数。
    pub max_archive_bytes: usize,
    /// 最大条目数。
    pub max_entries: usize,
    /// 解压后总字节上限。
    pub max_uncompressed_total: usize,
    /// 单条目解压字节上限。
    pub max_single_entry: usize,
    /// 压缩比阈值（uncompressed/compressed，防 zip bomb）。
    pub max_compression_ratio: u32,
}

impl Default for PackageLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 8 * 1024 * 1024,
            max_entries: 256,
            max_uncompressed_total: 64 * 1024 * 1024,
            max_single_entry: 16 * 1024 * 1024,
            max_compression_ratio: 200,
        }
    }
}

/// 校验通过后的包内容（供加载/安装消费）。
#[derive(Debug, Clone)]
pub struct ValidatedPackage {
    pub manifest: Manifest,
    pub ui_document: UiDocument,
    pub wasm: Vec<u8>,
    pub entry_names: Vec<String>,
}

/// 包校验错误（稳定 code `FPAK_*`）。
#[derive(Debug, Error)]
pub enum PackageError {
    #[error("包大小 {actual} 超过上限 {limit}")]
    ArchiveTooLarge { actual: usize, limit: usize },
    #[error("条目数 {actual} 超过上限 {limit}")]
    TooManyEntries { actual: usize, limit: usize },
    #[error("解压总大小超限: {detail}")]
    UncompressedTooLarge { detail: String },
    #[error("单条目大小超限: {detail}")]
    EntryTooLarge { detail: String },
    #[error("压缩比异常（疑似 zip bomb）: {detail}")]
    ZipBomb { detail: String },
    #[error("zip 损坏: {0}")]
    CorruptZip(String),
    #[error("非法包路径 `{0}`")]
    InvalidPath(String),
    #[error("重复规范化路径 `{0}`")]
    DuplicatePath(String),
    #[error("大小写碰撞路径 `{0}`")]
    CaseCollision(String),
    #[error("不允许的特殊条目 `{0}`")]
    SpecialEntry(String),
    #[error("缺失 manifest.json")]
    MissingManifest,
    #[error("manifest 非法: {0}")]
    InvalidManifest(#[from] ManifestError),
    #[error("缺失入口 `{0}`")]
    MissingEntrypoint(String),
    #[error("UI IR 非法: {0}")]
    InvalidUiIr(#[from] UiSchemaError),
    #[error("WASM 组件非法: {0}")]
    InvalidWasm(String),
    #[error("WASM 包含未允许的 import `{0}`")]
    DisallowedImport(String),
    #[error("WASM 未导出 widget-contract world")]
    MissingWorldExport,
    #[error("JSON 解析失败: {0}")]
    InvalidJson(String),
}

impl PackageError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ArchiveTooLarge { .. } => "FPAK_ARCHIVE_TOO_LARGE",
            Self::TooManyEntries { .. } => "FPAK_TOO_MANY_ENTRIES",
            Self::UncompressedTooLarge { .. } => "FPAK_UNCOMPRESSED_TOO_LARGE",
            Self::EntryTooLarge { .. } => "FPAK_ENTRY_TOO_LARGE",
            Self::ZipBomb { .. } => "FPAK_ZIP_BOMB",
            Self::CorruptZip(_) => "FPAK_CORRUPT_ZIP",
            Self::InvalidPath(_) => "FPAK_INVALID_PATH",
            Self::DuplicatePath(_) => "FPAK_DUPLICATE_PATH",
            Self::CaseCollision(_) => "FPAK_CASE_COLLISION",
            Self::SpecialEntry(_) => "FPAK_SPECIAL_ENTRY",
            Self::MissingManifest => "FPAK_MISSING_MANIFEST",
            Self::InvalidManifest(_) => "FPAK_INVALID_MANIFEST",
            Self::MissingEntrypoint(_) => "FPAK_MISSING_ENTRYPOINT",
            Self::InvalidUiIr(_) => "FPAK_INVALID_UI_IR",
            Self::InvalidWasm(_) => "FPAK_INVALID_WASM",
            Self::DisallowedImport(_) => "FPAK_DISALLOWED_IMPORT",
            Self::MissingWorldExport => "FPAK_MISSING_WORLD_EXPORT",
            Self::InvalidJson(_) => "FPAK_INVALID_JSON",
        }
    }
}

/// 校验一个 `.floatile` 包字节流。
pub fn validate_package(
    bytes: &[u8],
    limits: &PackageLimits,
) -> Result<ValidatedPackage, PackageError> {
    if bytes.len() > limits.max_archive_bytes {
        return Err(PackageError::ArchiveTooLarge {
            actual: bytes.len(),
            limit: limits.max_archive_bytes,
        });
    }

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    if archive.len() > limits.max_entries {
        return Err(PackageError::TooManyEntries {
            actual: archive.len(),
            limit: limits.max_entries,
        });
    }

    // 第一遍：路径安全、预算、特殊条目、去重/大小写碰撞。
    let mut normalized = BTreeSet::new();
    let mut lowercase = BTreeSet::new();
    let mut entries: Vec<(String, u64, bool)> = Vec::new(); // (name, size, is_dir)
    let mut total_uncompressed: u64 = 0;

    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        let raw_name = file.name().to_owned();
        PackagePath::parse(&raw_name).map_err(|e| PackageError::InvalidPath(e.to_string()))?;
        if raw_name.chars().count() > 256 {
            return Err(PackageError::InvalidPath(format!("{raw_name}: 路径过长")));
        }
        if !normalized.insert(raw_name.clone()) {
            return Err(PackageError::DuplicatePath(raw_name.clone()));
        }
        let lower = raw_name.to_lowercase();
        if !lowercase.insert(lower.clone()) {
            return Err(PackageError::CaseCollision(raw_name.clone()));
        }

        let mode = file.unix_mode();
        if let Some(mode) = mode {
            // 拒绝 symlink / 设备 / FIFO / socket（S_IFMT 分类）。
            const S_IFMT: u32 = 0o170000;
            const S_IFLNK: u32 = 0o120000;
            const S_IFCHR: u32 = 0o020000;
            const S_IFBLK: u32 = 0o060000;
            const S_IFIFO: u32 = 0o010000;
            const S_IFSOCK: u32 = 0o140000;
            let kind = mode & S_IFMT;
            if matches!(kind, S_IFLNK | S_IFCHR | S_IFBLK | S_IFIFO | S_IFSOCK) {
                return Err(PackageError::SpecialEntry(raw_name.clone()));
            }
        }

        let size = file.size();
        if size > limits.max_single_entry as u64 {
            return Err(PackageError::EntryTooLarge {
                detail: format!("{raw_name}: {size} > {}", limits.max_single_entry),
            });
        }
        total_uncompressed += size;
        if total_uncompressed > limits.max_uncompressed_total as u64 {
            return Err(PackageError::UncompressedTooLarge {
                detail: format!("{total_uncompressed} > {}", limits.max_uncompressed_total),
            });
        }
        // zip bomb：压缩比异常。
        let compressed = file.compressed_size();
        if let Some(ratio) = size.checked_div(compressed)
            && compressed > 0
            && ratio > limits.max_compression_ratio as u64
        {
            return Err(PackageError::ZipBomb {
                detail: format!("{raw_name}: ratio {ratio}"),
            });
        }
        entries.push((raw_name, size, file.is_dir()));
    }

    // 读取并校验 manifest.json。
    let manifest_bytes = match read_entry(&mut archive, "manifest.json") {
        Err(PackageError::MissingEntrypoint(_)) => return Err(PackageError::MissingManifest),
        other => other?,
    };
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| PackageError::InvalidJson(e.to_string()))?;
    validate_manifest(&manifest)?;

    // 入口必须存在且为普通文件。
    let ui_path = manifest.entrypoints.ui.as_str();
    let logic_path = manifest.entrypoints.logic.as_str();
    let ui_bytes = read_entry(&mut archive, ui_path)?;
    let wasm_bytes = read_entry(&mut archive, logic_path)?;

    // config.schema 若声明则必须存在。
    if let Some(config) = &manifest.config {
        read_entry(&mut archive, config.schema.as_str())?;
    }

    // UI IR 校验。
    let ui_document: UiDocument = serde_json::from_slice(&ui_bytes)
        .map_err(|e| PackageError::InvalidJson(format!("{ui_path}: {e}")))?;
    floatile_ui_schema::validate_document(&ui_document)?;

    // WASM 组件校验：合法组件 + import 白名单 + 导出 widget-contract world。
    let wasm = validate_wasm(&wasm_bytes)?;

    let entry_names = entries.iter().map(|(n, _, _)| n.clone()).collect();
    Ok(ValidatedPackage {
        manifest,
        ui_document,
        wasm,
        entry_names,
    })
}

/// 读取包内条目字节；要求条目存在且是普通文件。
fn read_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    path: &str,
) -> Result<Vec<u8>, PackageError> {
    let mut file = archive
        .by_name(path)
        .map_err(|_| PackageError::MissingEntrypoint(path.to_owned()))?;
    if file.is_dir() {
        return Err(PackageError::MissingEntrypoint(format!(
            "{path}: 是目录而非文件"
        )));
    }
    let mut buf = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buf)
        .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    Ok(buf)
}

/// 校验 WASM Component：可解析、import 只允许 floatile:widget 与 wasi、导出
/// widget-contract world。
fn validate_wasm(bytes: &[u8]) -> Result<Vec<u8>, PackageError> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    let engine =
        wasmtime::Engine::new(&config).map_err(|e| PackageError::InvalidWasm(e.to_string()))?;
    let component = Component::from_binary(&engine, bytes)
        .map_err(|e| PackageError::InvalidWasm(e.to_string()))?;
    let ty = component.component_type();

    for (name, _extern) in ty.imports(&engine) {
        // 只允许 floatile:widget/* 与 wasi:*/*（零 ambient 之外的一律拒绝）。
        let namespace = name.split(':').next().unwrap_or("");
        if namespace != "floatile" && namespace != "wasi" {
            return Err(PackageError::DisallowedImport(name.to_owned()));
        }
    }
    let exports_contract = ty
        .exports(&engine)
        .any(|(name, _)| name.starts_with("floatile:widget/widget-contract"));
    if !exports_contract {
        return Err(PackageError::MissingWorldExport);
    }
    Ok(bytes.to_vec())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 构建一个内存 zip。
    fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, data) in files {
                w.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    fn valid_manifest_json() -> String {
        serde_json::json!({
            "manifestVersion": 1,
            "id": "dev.floatile.clock",
            "name": "World Clock",
            "version": "0.1.0",
            "publisher": { "id": "dev.floatile", "name": "Floatile Labs" },
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": [ { "capability": "timer:schedule", "params": { "maxPerMinute": 60, "maxActive": 2 } } ]
        })
        .to_string()
    }

    fn valid_ui_ir() -> String {
        serde_json::json!({
            "uiApiVersion": "1.0.0",
            "state": { "initial": {}, "schema": { "type": "object", "additionalProperties": false, "properties": {} } },
            "events": {},
            "root": { "type": "Column", "props": {}, "children": [] }
        })
        .to_string()
    }

    /// 真实 clock-wasm 组件字节（读自 target；构建则由 runtime 测试负责）。
    fn real_wasm() -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target/wasm32-wasip2/debug/floatile_clock_wasm.wasm");
        std::fs::read(&path).unwrap_or_default()
    }

    fn valid_pkg_bytes() -> Vec<u8> {
        let wasm = real_wasm();
        assert!(!wasm.is_empty(), "需要先构建 clock-wasm");
        build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ])
    }

    #[test]
    fn accepts_valid_package() {
        let pkg = validate_package(&valid_pkg_bytes(), &PackageLimits::default()).unwrap();
        assert_eq!(pkg.manifest.id.0, "dev.floatile.clock");
        assert_eq!(pkg.entry_names.len(), 3);
    }

    #[test]
    fn rejects_missing_manifest() {
        let wasm = real_wasm();
        let bytes = build_zip(&[
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        assert!(matches!(
            validate_package(&bytes, &PackageLimits::default()),
            Err(PackageError::MissingManifest)
        ));
    }

    #[test]
    fn rejects_path_traversal_entry() {
        let wasm = real_wasm();
        let bytes = build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("../evil", b"x"),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        assert!(matches!(
            validate_package(&bytes, &PackageLimits::default()),
            Err(PackageError::InvalidPath(_))
        ));
    }

    #[test]
    fn rejects_absolute_path_entry() {
        let wasm = real_wasm();
        let bytes = build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("/etc/passwd", b"x"),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        assert!(matches!(
            validate_package(&bytes, &PackageLimits::default()),
            Err(PackageError::InvalidPath(_))
        ));
    }

    #[test]
    fn rejects_case_collision() {
        let wasm = real_wasm();
        let bytes = build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("Assets/a", b"x"),
            ("assets/A", b"y"),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        assert!(matches!(
            validate_package(&bytes, &PackageLimits::default()),
            Err(PackageError::CaseCollision(_))
        ));
    }

    #[test]
    fn rejects_bad_manifest() {
        let wasm = real_wasm();
        let mut manifest =
            serde_json::from_str::<serde_json::Value>(&valid_manifest_json()).unwrap();
        manifest["engineApiVersion"] = serde_json::json!("2.0.0");
        let bytes = build_zip(&[
            ("manifest.json", manifest.to_string().as_bytes()),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        assert!(matches!(
            validate_package(&bytes, &PackageLimits::default()),
            Err(PackageError::InvalidManifest(_))
        ));
    }

    #[test]
    fn rejects_bad_ui_ir() {
        let wasm = real_wasm();
        let bytes = build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("ui/widget.ftui", r#"{"uiApiVersion":"1.0.0","state":{"initial":{},"schema":{"type":"object","additionalProperties":false,"properties":{}}},"events":{},"root":{"type":"Canvas","props":{}}}"#.as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        assert!(matches!(
            validate_package(&bytes, &PackageLimits::default()),
            Err(PackageError::InvalidUiIr(_))
        ));
    }

    #[test]
    fn rejects_missing_entrypoint() {
        let wasm = real_wasm();
        let bytes = build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            // 缺 logic/plugin.wasm
        ]);
        // wasm 用 real；这里缺 wasm 入口 → MissingEntrypoint。
        let _ = wasm;
        assert!(matches!(
            validate_package(&bytes, &PackageLimits::default()),
            Err(PackageError::MissingEntrypoint(_))
        ));
    }

    #[test]
    fn rejects_too_many_entries() {
        let mut files: Vec<(String, Vec<u8>)> = vec![
            ("manifest.json".into(), valid_manifest_json().into_bytes()),
            ("ui/widget.ftui".into(), valid_ui_ir().into_bytes()),
        ];
        for i in 0..1000 {
            files.push((format!("assets/x{i}"), b"x".to_vec()));
        }
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let bytes = build_zip(&refs);
        let limits = PackageLimits {
            max_entries: 10,
            ..PackageLimits::default()
        };
        assert!(matches!(
            validate_package(&bytes, &limits),
            Err(PackageError::TooManyEntries { .. })
        ));
    }
}
