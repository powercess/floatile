//! 已安装插件目录的只读完整性校验与精确 Installation 解析。
//!
//! CLI 与 shell 共用本模块，避免一边按 `install.json` 建立实例引用、另一边用不同
//! 规则加载运行内容。安装写入仍由 `floatile-cli` 负责；本模块只读取和复核。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use floatile_core::install::{InstallMeta, content_digest, file_digest, hex_encode};
use floatile_core::instance::{InstallationRef, InstanceConfig, InstanceModelError};
use floatile_core::manifest::{Manifest, ManifestError, validate_manifest};
use semver::Version;

/// 一份已按 `install.json` 复核的不可变安装内容。
#[derive(Debug)]
pub struct InstalledInstallation {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub meta: InstallMeta,
    files: BTreeMap<String, Vec<u8>>,
}

impl InstalledInstallation {
    /// 读取 manifest 已声明的包内文件。
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// 构造精确、内容寻址的 Installation 引用。
    pub fn reference(&self) -> Result<InstallationRef, InstallationCatalogError> {
        InstallationRef::from_install_meta(&self.meta)
            .map_err(InstallationCatalogError::InvalidIdentity)
    }

    /// 按 manifest 声明的随包 schema 校验实例 Config。这是 CLI 创建与 shell
    /// 恢复共用的单一语义，防止绕过 CLI 写入的数据在运行时逃逸校验。
    pub fn validate_config(&self, config: &InstanceConfig) -> Result<(), ConfigValidationError> {
        let Some(config_ref) = &self.manifest.config else {
            return if config.as_object().is_empty() {
                Ok(())
            } else {
                Err(ConfigValidationError::NotDeclared)
            };
        };
        let bytes = self
            .file(config_ref.schema.as_str())
            .ok_or(ConfigValidationError::SchemaMissing)?;
        let schema =
            serde_json::from_slice(bytes).map_err(|_| ConfigValidationError::SchemaInvalid)?;
        config.validate_schema(&schema)?;
        Ok(())
    }
}

/// Installation 的 Config 契约错误（稳定 code `FCONFIG_*`）。
#[derive(Debug, thiserror::Error)]
pub enum ConfigValidationError {
    #[error("插件未声明 config schema，只允许空配置")]
    NotDeclared,
    #[error("配置 schema 缺失")]
    SchemaMissing,
    #[error("配置 schema JSON 无效")]
    SchemaInvalid,
    #[error("实例配置无效: {0}")]
    Config(#[from] InstanceModelError),
}

impl ConfigValidationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotDeclared => "FCONFIG_NOT_DECLARED",
            Self::SchemaMissing => "FCONFIG_SCHEMA_MISSING",
            Self::SchemaInvalid => "FCONFIG_SCHEMA_INVALID",
            Self::Config(_) => "FCONFIG_VALUE_INVALID",
        }
    }
}

/// 安装目录读取错误（稳定 code `FCAT_*`）。
#[derive(Debug, thiserror::Error)]
pub enum InstallationCatalogError {
    #[error("读取安装目录失败: {0}")]
    Read(String),
    #[error("install.json 缺失或损坏: {0}")]
    InvalidMeta(String),
    #[error("插件 {id} 文件 `{file}` digest 不匹配")]
    DigestMismatch { id: String, file: String },
    #[error("安装元数据与 manifest 身份不一致")]
    MetadataMismatch,
    #[error("安装内容身份无效: {0}")]
    InvalidIdentity(InstanceModelError),
    #[error("manifest 非法: {0}")]
    InvalidManifest(#[from] ManifestError),
}

impl InstallationCatalogError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Read(_) => "FCAT_READ",
            Self::InvalidMeta(_) => "FCAT_INVALID_META",
            Self::DigestMismatch { .. } => "FCAT_DIGEST_MISMATCH",
            Self::MetadataMismatch => "FCAT_METADATA_MISMATCH",
            Self::InvalidIdentity(_) => "FCAT_INVALID_IDENTITY",
            Self::InvalidManifest(_) => "FCAT_INVALID_MANIFEST",
        }
    }
}

/// 加载精确 id/version；目录不存在返回 `None`。
pub fn load_exact(
    root: &Path,
    id: &str,
    version: &str,
) -> Result<Option<InstalledInstallation>, InstallationCatalogError> {
    let dir = root.join(id).join(version);
    if !dir.is_dir() {
        return Ok(None);
    }
    load_from_dir(&dir).map(Some)
}

/// 加载并复核一个持久化 InstallationRef，不允许静默升级或 digest 漂移。
pub fn load_reference(
    root: &Path,
    reference: &InstallationRef,
) -> Result<Option<InstalledInstallation>, InstallationCatalogError> {
    let Some(installation) = load_exact(root, &reference.plugin().0, reference.version())? else {
        return Ok(None);
    };
    if installation.reference()? != *reference {
        return Err(InstallationCatalogError::MetadataMismatch);
    }
    Ok(Some(installation))
}

/// 加载某插件的最高语义版本；只用于未固定 Installation 的内建兼容路径。
pub fn load_highest(
    root: &Path,
    id: &str,
) -> Result<Option<InstalledInstallation>, InstallationCatalogError> {
    let id_dir = root.join(id);
    if !id_dir.is_dir() {
        return Ok(None);
    }
    let mut best: Option<(Version, PathBuf)> = None;
    let entries = std::fs::read_dir(&id_dir)
        .map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(version) = Version::parse(&name) else {
            continue;
        };
        let dir = entry.path();
        if dir.is_dir() && best.as_ref().is_none_or(|(current, _)| version > *current) {
            best = Some((version, dir));
        }
    }
    match best {
        Some((_, dir)) => load_from_dir(&dir).map(Some),
        None => Ok(None),
    }
}

/// 枚举插件存储中每个合法插件 id 的最高语义版本，并逐一复核完整性。
///
/// 根目录不存在时返回空集合；任一规范插件目录损坏时返回错误，不把篡改静默伪装为
/// “未安装”。结果按插件 id 稳定排序，供 shell 控制面与加载器共享。
pub fn list_highest(root: &Path) -> Result<Vec<InstalledInstallation>, InstallationCatalogError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(root)
        .map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
    let mut installations = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !entry.path().is_dir() || !is_valid_plugin_id_dir(&id) {
            continue;
        }
        if let Some(installation) = load_highest(root, &id)? {
            installations.push(installation);
        }
    }
    installations.sort_by(|left, right| left.meta.id.cmp(&right.meta.id));
    Ok(installations)
}

/// 枚举插件存储中的全部合法安装版本，并逐一复核完整性。
///
/// 结果先按插件 id、再按语义版本降序排列。管理面使用此函数展示并存版本；运行时仍必须
/// 通过精确 [`InstallationRef`] 加载，不允许把该枚举当作静默升级策略。
pub fn list_all(root: &Path) -> Result<Vec<InstalledInstallation>, InstallationCatalogError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(root)
        .map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
    let mut installations = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !entry.path().is_dir() || !is_valid_plugin_id_dir(&id) {
            continue;
        }
        let versions = std::fs::read_dir(entry.path())
            .map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
        for version_entry in versions {
            let version_entry =
                version_entry.map_err(|error| InstallationCatalogError::Read(error.to_string()))?;
            let version_text = version_entry.file_name().to_string_lossy().into_owned();
            let Ok(version) = Version::parse(&version_text) else {
                continue;
            };
            if version_entry.path().is_dir() {
                installations.push((id.clone(), version, load_from_dir(&version_entry.path())?));
            }
        }
    }
    installations.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
    Ok(installations
        .into_iter()
        .map(|(_, _, installation)| installation)
        .collect())
}

fn is_valid_plugin_id_dir(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '.'
                || character == '-'
        })
        && name.contains('.')
        && !name.starts_with('.')
        && !name.ends_with('.')
}

fn load_from_dir(dir: &Path) -> Result<InstalledInstallation, InstallationCatalogError> {
    let meta_bytes = std::fs::read(dir.join("install.json"))
        .map_err(|error| InstallationCatalogError::InvalidMeta(error.to_string()))?;
    let meta: InstallMeta = serde_json::from_slice(&meta_bytes)
        .map_err(|error| InstallationCatalogError::InvalidMeta(error.to_string()))?;

    let mut files = BTreeMap::new();
    for (name, expected) in &meta.files {
        let path = dir.join(name);
        let bytes = std::fs::read(&path)
            .map_err(|error| InstallationCatalogError::Read(format!("{name}: {error}")))?;
        if hex_encode(&file_digest(&bytes)) != *expected {
            return Err(InstallationCatalogError::DigestMismatch {
                id: meta.id.clone(),
                file: name.clone(),
            });
        }
        files.insert(name.clone(), bytes);
    }
    if hex_encode(&content_digest(&files)) != meta.digest {
        return Err(InstallationCatalogError::DigestMismatch {
            id: meta.id.clone(),
            file: "<aggregate>".to_owned(),
        });
    }

    let manifest_bytes = files
        .get("manifest.json")
        .ok_or_else(|| InstallationCatalogError::InvalidMeta("manifest.json 缺失".to_owned()))?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|error| InstallationCatalogError::InvalidMeta(error.to_string()))?;
    validate_manifest(&manifest)?;
    if manifest.id.0 != meta.id || manifest.version != meta.version {
        return Err(InstallationCatalogError::MetadataMismatch);
    }
    let installation = InstalledInstallation {
        dir: dir.to_path_buf(),
        manifest,
        meta,
        files,
    };
    let _ = installation.reference()?;
    Ok(installation)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use floatile_core::install::InstallMeta;

    fn write_install(root: &Path, id: &str, version: &str) -> PathBuf {
        let dir = root.join(id).join(version);
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::create_dir_all(dir.join("logic")).unwrap();
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "id": id,
            "name": "Clock",
            "version": version,
            "publisher": { "id": "dev.floatile", "name": "Floatile" },
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 100, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": []
        })
        .to_string()
        .into_bytes();
        let mut files = BTreeMap::from([
            ("logic/plugin.wasm".to_owned(), b"wasm".to_vec()),
            ("manifest.json".to_owned(), manifest),
            ("ui/widget.ftui".to_owned(), b"{}".to_vec()),
        ]);
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let meta = InstallMeta {
            manifest_version: 1,
            id: id.to_owned(),
            version: version.to_owned(),
            engine_api_version: "1.0.0".to_owned(),
            ui_api_version: "1.0.0".to_owned(),
            installed_at: 1,
            source: "test".to_owned(),
            trust: floatile_core::install::InstallationTrust::Unsigned,
            files: files
                .iter()
                .map(|(name, bytes)| (name.clone(), hex_encode(&file_digest(bytes))))
                .collect(),
            digest: hex_encode(&content_digest(&files)),
        };
        std::fs::write(dir.join("install.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        files.clear();
        dir
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "floatile-installation-catalog-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn exact_and_highest_share_verified_content() {
        let root = temp_root("highest");
        write_install(&root, "dev.floatile.clock", "1.0.0");
        write_install(&root, "dev.floatile.clock", "1.2.0");
        let exact = load_exact(&root, "dev.floatile.clock", "1.0.0")
            .unwrap()
            .unwrap();
        let highest = load_highest(&root, "dev.floatile.clock").unwrap().unwrap();
        assert_eq!(exact.meta.version, "1.0.0");
        assert_eq!(highest.meta.version, "1.2.0");
        assert!(
            load_reference(&root, &exact.reference().unwrap())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn catalog_lists_each_plugin_once_at_highest_version_in_stable_order() {
        let root = temp_root("list");
        write_install(&root, "dev.floatile.zulu", "1.0.0");
        write_install(&root, "dev.floatile.alpha", "1.0.0");
        write_install(&root, "dev.floatile.alpha", "2.0.0");
        std::fs::create_dir_all(root.join("not-a-plugin-id")).unwrap();

        let installations = list_highest(&root).unwrap();
        let identities: Vec<_> = installations
            .iter()
            .map(|installation| {
                (
                    installation.meta.id.as_str(),
                    installation.meta.version.as_str(),
                )
            })
            .collect();
        assert_eq!(
            identities,
            vec![
                ("dev.floatile.alpha", "2.0.0"),
                ("dev.floatile.zulu", "1.0.0")
            ]
        );
        assert!(list_highest(&root.join("missing")).unwrap().is_empty());
    }

    #[test]
    fn catalog_lists_all_versions_in_stable_semver_order() {
        let root = temp_root("list-all");
        write_install(&root, "dev.floatile.zulu", "1.0.0");
        write_install(&root, "dev.floatile.alpha", "1.0.0");
        write_install(&root, "dev.floatile.alpha", "2.0.0");

        let installations = list_all(&root).unwrap();
        let identities: Vec<_> = installations
            .iter()
            .map(|installation| {
                (
                    installation.meta.id.as_str(),
                    installation.meta.version.as_str(),
                )
            })
            .collect();
        assert_eq!(
            identities,
            vec![
                ("dev.floatile.alpha", "2.0.0"),
                ("dev.floatile.alpha", "1.0.0"),
                ("dev.floatile.zulu", "1.0.0"),
            ]
        );
    }

    #[test]
    fn tampered_file_is_rejected() {
        let root = temp_root("tamper");
        let dir = write_install(&root, "dev.floatile.clock", "1.0.0");
        std::fs::write(dir.join("logic/plugin.wasm"), b"tampered").unwrap();
        assert!(matches!(
            load_exact(&root, "dev.floatile.clock", "1.0.0"),
            Err(InstallationCatalogError::DigestMismatch { .. })
        ));
    }
}
