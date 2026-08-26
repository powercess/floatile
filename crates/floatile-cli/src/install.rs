//! 原子安装：校验 → staging → digest → 原子 rename 进插件存储 + install.json。
//!
//! 对应 manifest-v1 §6 的安全校验顺序第 9/10 步：为全部允许文件计算 digest，全部
//! 检查通过后原子移动进插件存储；任何失败清理暂存目录，绝不留下半安装状态。
//!
//! 安装目录约定：`<store>/<id>/<version>/`，其中 `<store>` 由调用方指定（CLI 默认
//! 取 `--store`，否则 `$FLOATTILE_PLUGIN_DIR`）；`install.json` 记录 id/version、
//! 每文件 SHA-256 与覆盖全部规范文件集合的聚合 digest，供宿主 PluginManager 在
//! 加载前做完整性校验。

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use floatile_core::install::{InstallMeta, content_digest, file_digest, hex_encode};
use floatile_core::manifest::Manifest;
use thiserror::Error;

use crate::package::{PackageError, PackageLimits, ValidatedPackage, validate_package};

/// 安装错误（稳定 code `FINST_*`）。
#[derive(Debug, Error)]
pub enum InstallError {
    #[error("包校验失败: {0}")]
    Package(#[from] PackageError),
    #[error("插件存储根目录不可用: {0}")]
    StoreUnavailable(String),
    #[error("插件 {id} {version} 已安装")]
    AlreadyInstalled { id: String, version: String },
    #[error("写入暂存目录失败: {0}")]
    StagingWrite(String),
    #[error("提交安装失败: {0}")]
    Commit(String),
    #[error("I/O 失败: {0}")]
    Io(String),
}

impl InstallError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Package(_) => "FINST_PACKAGE",
            Self::StoreUnavailable(_) => "FINST_STORE_UNAVAILABLE",
            Self::AlreadyInstalled { .. } => "FINST_ALREADY_INSTALLED",
            Self::StagingWrite(_) => "FINST_STAGING_WRITE",
            Self::Commit(_) => "FINST_COMMIT",
            Self::Io(_) => "FINST_IO",
        }
    }

    pub fn public_detail(&self) -> Cow<'static, str> {
        match self {
            Self::Package(_) => Cow::Borrowed("插件包未通过安全校验"),
            Self::StoreUnavailable(_) => Cow::Borrowed("插件存储不可用"),
            Self::AlreadyInstalled { id, version } => {
                Cow::Owned(format!("插件 {id} {version} 已安装"))
            }
            Self::StagingWrite(_) => Cow::Borrowed("无法写入安装暂存目录"),
            Self::Commit(_) => Cow::Borrowed("无法原子提交安装"),
            Self::Io(_) => Cow::Borrowed("插件包读取失败"),
        }
    }
}

/// 安装结果（供 CLI 输出与宿主后续读取）。
#[derive(Debug)]
pub struct InstalledPackage {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub meta: InstallMeta,
}

/// 插件存储根目录下某插件的安装目录：`<store>/<id>/<version>`。
///
/// `id` 与 `version` 都经过 manifest 校验（反向域名 + 严格 semver），因此路径安全；
/// 但仍由本函数集中定义布局，CLI 写入与宿主读取共享同一约定。
pub fn install_dir(store: &Path, id: &str, version: &str) -> PathBuf {
    store.join(id).join(version)
}

/// 原子安装一个已通过完整校验的 `.floatile` 包。
///
/// 语义：
/// 1. 校验（复用 `validate_package`，含路径/预算/zip-bomb/manifest/UI/WASM 拒绝）；
/// 2. 在 `<store>` 下创建同文件系统的 `.staging-*` 兄弟目录并写入全部校验通过的条目
///    与 `install.json`（含每文件 digest 与聚合 digest），逐文件 fsync；
/// 3. `rename(staging, final)` 原子提交；任何失败删除 staging，绝不产生半安装目录。
pub fn install_package(
    bytes: &[u8],
    store: &Path,
    source: &str,
    limits: &PackageLimits,
) -> Result<InstalledPackage, InstallError> {
    let validated = validate_package(bytes, limits)?;
    let manifest = validated.manifest.clone();
    let id = manifest.id.0.clone();
    let version = manifest.version.clone();
    let final_dir = install_dir(store, &id, &version);

    if final_dir.exists() {
        return Err(InstallError::AlreadyInstalled { id, version });
    }
    // 父目录 `<store>/<id>` 必须在 rename 前存在，否则 rename 会失败。
    let parent = final_dir.parent().unwrap_or(store); // install_dir 的产物至少含一层 `id`，parent 恒存在
    fs::create_dir_all(parent).map_err(|e| InstallError::StoreUnavailable(e.to_string()))?;

    let staging = store.join(format!(".staging-{}", nonce()));

    let meta = match write_staging(&staging, &validated, &manifest, source) {
        Ok(meta) => meta,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    if let Err(e) = fs::rename(&staging, &final_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(InstallError::Commit(e.to_string()));
    }

    Ok(InstalledPackage {
        dir: final_dir,
        manifest,
        meta,
    })
}

/// 向暂存目录写入全部校验通过的文件 + install.json，并返回安装元数据。
fn write_staging(
    staging: &Path,
    validated: &ValidatedPackage,
    manifest: &Manifest,
    source: &str,
) -> Result<InstallMeta, InstallError> {
    let installed_at = now_secs();

    let mut file_digests = BTreeMap::new();
    for (name, bytes) in &validated.files {
        file_digests.insert(name.clone(), hex_encode(&file_digest(bytes)));
    }
    let aggregate = hex_encode(&content_digest(&validated.files));

    let meta = InstallMeta {
        manifest_version: manifest.manifest_version,
        id: manifest.id.0.clone(),
        version: manifest.version.clone(),
        engine_api_version: manifest.engine_api_version.clone(),
        ui_api_version: manifest.ui_api_version.clone(),
        installed_at,
        source: source.to_owned(),
        files: file_digests,
        digest: aggregate,
    };

    for (name, bytes) in &validated.files {
        let path = staging.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", parent.display())))?;
        }
        let mut file = fs::File::create(&path)
            .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", path.display())))?;
        file.write_all(bytes)
            .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", path.display())))?;
        file.sync_all()
            .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", path.display())))?;
    }

    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| InstallError::StagingWrite(format!("install.json: {e}")))?;
    let meta_path = staging.join("install.json");
    let mut meta_file = fs::File::create(&meta_path)
        .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", meta_path.display())))?;
    meta_file
        .write_all(&meta_json)
        .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", meta_path.display())))?;
    meta_file
        .sync_all()
        .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", meta_path.display())))?;

    // fsync 目录本身（unix 上使 rename 更持久；Windows 不支持打开目录，尽力而为）。
    #[cfg(unix)]
    {
        if let Ok(dir) = fs::File::open(staging) {
            let _ = dir.sync_all();
        }
    }

    Ok(meta)
}

/// 当前 UNIX 秒（用于 install.json 的时间戳）。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 暂存目录去重：进程 id + 纳秒时间戳（同文件系统内 rename 需唯一且在同一目录树下）。
fn nonce() -> String {
    let pid = std::process::id();
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{pid}-{ns}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Cursor;

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

    /// 与 package.rs 测试一致的合法 manifest/UI；wasm 用真实 clock-wasm。
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

    fn real_wasm() -> Vec<u8> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let path = root.join("target/wasm32-wasip2/debug/floatile_clock_wasm.wasm");
        if !path.exists() {
            let status = std::process::Command::new("cargo")
                .current_dir(&root)
                .args([
                    "build",
                    "-p",
                    "floatile-clock-wasm",
                    "--target",
                    "wasm32-wasip2",
                ])
                .status()
                .expect("failed to run cargo build for clock-wasm");
            assert!(status.success(), "clock-wasm 构建失败");
        }
        fs::read(&path).unwrap_or_default()
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

    fn temp_store(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("floatile-install-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn installs_valid_package_atomically() {
        let store = temp_store("valid");
        let installed = install_package(
            &valid_pkg_bytes(),
            &store,
            "clock.floatile",
            &Default::default(),
        )
        .unwrap();
        let version_dir = install_dir(&store, "dev.floatile.clock", "0.1.0");
        assert_eq!(installed.dir, version_dir);

        // 所有校验通过的条目都落盘。
        for name in ["manifest.json", "ui/widget.ftui", "logic/plugin.wasm"] {
            assert!(version_dir.join(name).exists(), "缺失 {name}");
        }
        // install.json 存在且可解析，digest 与按实际文件重新计算的 digest 一致。
        let meta: InstallMeta =
            serde_json::from_slice(&fs::read(version_dir.join("install.json")).unwrap()).unwrap();
        assert_eq!(meta.id, "dev.floatile.clock");
        assert_eq!(meta.version, "0.1.0");
        let actual: BTreeMap<String, Vec<u8>> = meta
            .files
            .keys()
            .map(|k| (k.clone(), fs::read(version_dir.join(k)).unwrap()))
            .collect();
        assert_eq!(meta.digest, hex_encode(&content_digest(&actual)));
        assert_eq!(meta.digest.len(), 64);
        // 无残留暂存目录。
        let leftovers: Vec<_> = fs::read_dir(&store)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
            .collect();
        assert!(leftovers.is_empty(), "存在未清理的暂存目录");
    }

    #[test]
    fn refuses_reinstall_same_version() {
        let store = temp_store("reinstall");
        install_package(
            &valid_pkg_bytes(),
            &store,
            "a.floatile",
            &Default::default(),
        )
        .unwrap();
        let err = install_package(
            &valid_pkg_bytes(),
            &store,
            "b.floatile",
            &Default::default(),
        )
        .unwrap_err();
        assert!(matches!(err, InstallError::AlreadyInstalled { .. }));
        assert_eq!(err.code(), "FINST_ALREADY_INSTALLED");
    }

    #[test]
    fn installs_different_versions_side_by_side() {
        let store = temp_store("versions");
        install_package(
            &valid_pkg_bytes(),
            &store,
            "a.floatile",
            &Default::default(),
        )
        .unwrap();
        // 构造 0.2.0 版本的合法包（仅版本不同）。
        let wasm = real_wasm();
        let mut manifest: serde_json::Value = serde_json::from_str(&valid_manifest_json()).unwrap();
        manifest["version"] = serde_json::json!("0.2.0");
        let bytes = build_zip(&[
            ("manifest.json", manifest.to_string().as_bytes()),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        install_package(&bytes, &store, "v2.floatile", &Default::default()).unwrap();
        assert!(install_dir(&store, "dev.floatile.clock", "0.1.0").exists());
        assert!(install_dir(&store, "dev.floatile.clock", "0.2.0").exists());
    }

    #[test]
    fn rejects_evil_package_with_no_partial_install() {
        let store = temp_store("evil");
        let wasm = real_wasm();

        // 路径穿越条目 → 校验拒绝，且不留下任何安装产物/半安装目录。
        let traversal = build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("../evil", b"x"),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        let err =
            install_package(&traversal, &store, "evil.floatile", &Default::default()).unwrap_err();
        assert!(matches!(
            err,
            InstallError::Package(PackageError::InvalidPath(_))
        ));
        assert!(!install_dir(&store, "dev.floatile.clock", "0.1.0").exists());
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0, "安装失败应无残留");

        // 解压超预算（zip bomb 类）→ 校验拒绝，同样无残留。
        let mut blast: Vec<u8> = vec![];
        {
            // 用低上限模拟：单条目超过 max_single_entry。
            let big = vec![b'x'; 1024 * 1024 * 20];
            blast = build_zip(&[
                ("manifest.json", valid_manifest_json().as_bytes()),
                ("ui/widget.ftui", valid_ui_ir().as_bytes()),
                ("logic/plugin.wasm", wasm.as_slice()),
                ("assets/big", big.as_slice()),
            ]);
        }
        let limits = PackageLimits {
            max_single_entry: 1024 * 1024,
            ..PackageLimits::default()
        };
        let err2 = install_package(&blast, &store, "evil2.floatile", &limits).unwrap_err();
        assert!(matches!(
            err2,
            InstallError::Package(PackageError::EntryTooLarge { .. })
        ));
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0, "安装失败应无残留");
    }
}
