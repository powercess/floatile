//! manifest.json v1 的纯模型与校验（`floatile` 插件包元数据）。
//!
//! 事实源：`docs/plugin-sdk/manifest-v1.md`。本模块是 manifest schema 的可执行
//! 单源（serde 类型 + 校验函数）；CLI 与 PluginManager 复用同一实现，不另写一套
//! 平行字段定义。zip 归档、WASM world、UI IR 与 config schema 的深度校验属于
//! CLI/runtime 切片。

use std::fmt;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityId, parse_capability_params};
use crate::constants::{ENGINE_API_VERSION, MANIFEST_VERSION};
use crate::types::{LogicalSize, PluginId};

/// 插件类型：P0/MVP 仅 `widget`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Widget,
}

/// 包内规范化相对路径（`ui/`、`logic/`、assets 等）。
///
/// 构造时校验形式：必须是规范化相对路径，拒绝绝对路径、`..`/`.` 段、
/// 反斜杠变体、NUL 与空段。重复路径/大小写碰撞/symlink 是包级校验（CLI）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackagePath(String);

impl PackagePath {
    /// 构造并校验路径形式。
    pub fn parse(path: &str) -> Result<Self, ManifestError> {
        validate_package_path(path)?;
        Ok(Self(path.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackagePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for PackagePath {
    type Error = ManifestError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<PackagePath> for String {
    fn from(value: PackagePath) -> Self {
        value.0
    }
}

/// 发布者元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publisher {
    pub id: String,
    pub name: String,
}

/// 包入口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entrypoints {
    pub ui: PackagePath,
    pub logic: PackagePath,
}

/// 窗口尺寸（逻辑像素；default 必须在 min..max 内）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sizes {
    pub default: LogicalSize,
    pub min: LogicalSize,
    pub max: LogicalSize,
    pub resizable: bool,
}

/// 声明能力（manifest 是安装授权上限）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionDecl {
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// 用户配置 schema 引用（包内路径）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigRef {
    pub schema: PackagePath,
}

/// 插件私有 KV 迁移版本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDecl {
    pub migration_version: u64,
}

/// 构建诊断元数据（不参与信任/授权）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildMeta {
    pub sdk: String,
    pub sdk_version: String,
}

/// manifest.json v1。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "manifestVersion")]
    pub manifest_version: u32,
    pub id: PluginId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub version: String,
    pub publisher: Publisher,
    #[serde(rename = "engineApiVersion")]
    pub engine_api_version: String,
    #[serde(rename = "uiApiVersion")]
    pub ui_api_version: String,
    #[serde(rename = "type")]
    pub kind: PluginKind,
    pub entrypoints: Entrypoints,
    pub sizes: Sizes,
    pub permissions: Vec<PermissionDecl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<ConfigRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageDecl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildMeta>,
}

/// manifest 校验错误（稳定 code `FMAN_*`）。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ManifestError {
    #[error("不支持的 manifestVersion {0}，需要 {MANIFEST_VERSION}")]
    UnsupportedManifestVersion(u32),
    #[error("非法插件 id `{0}`")]
    InvalidPluginId(String),
    #[error("非法名称: {0}")]
    InvalidName(String),
    #[error("非法 semver `{0}`")]
    InvalidSemver(String),
    #[error("engineApiVersion `{version}` 与宿主 {engine} 不兼容")]
    IncompatibleEngineApiVersion { version: String, engine: String },
    #[error("不支持的 uiApiVersion `{0}`，需要 major 1")]
    UnsupportedUiApiVersion(String),
    #[error("不支持的插件类型 `{0}`")]
    UnsupportedPluginType(String),
    #[error("非法入口: {0}")]
    InvalidEntrypoint(String),
    #[error("非法尺寸: {0}")]
    InvalidSizes(String),
    #[error("未知能力 `{0}`")]
    UnknownCapability(String),
    #[error("声明了固有能力 `{0}`，不得写入 permissions")]
    InherentCapabilityDeclared(String),
    #[error("能力 `{capability}` 参数非法: {detail}")]
    InvalidCapabilityParams { capability: String, detail: String },
    #[error("非法包路径 `{0}`")]
    InvalidPackagePath(String),
}

impl ManifestError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedManifestVersion(_) => "FMAN_UNSUPPORTED_MANIFEST_VERSION",
            Self::InvalidPluginId(_) => "FMAN_INVALID_PLUGIN_ID",
            Self::InvalidName(_) => "FMAN_INVALID_NAME",
            Self::InvalidSemver(_) => "FMAN_INVALID_SEMVER",
            Self::IncompatibleEngineApiVersion { .. } => "FMAN_INCOMPATIBLE_ENGINE_API",
            Self::UnsupportedUiApiVersion(_) => "FMAN_UNSUPPORTED_UI_API",
            Self::UnsupportedPluginType(_) => "FMAN_UNSUPPORTED_PLUGIN_TYPE",
            Self::InvalidEntrypoint(_) => "FMAN_INVALID_ENTRYPOINT",
            Self::InvalidSizes(_) => "FMAN_INVALID_SIZES",
            Self::UnknownCapability(_) => "FMAN_UNKNOWN_CAPABILITY",
            Self::InherentCapabilityDeclared(_) => "FMAN_INHERENT_CAPABILITY_DECLARED",
            Self::InvalidCapabilityParams { .. } => "FMAN_INVALID_CAPABILITY_PARAMS",
            Self::InvalidPackagePath(_) => "FMAN_INVALID_PACKAGE_PATH",
        }
    }
}

/// 校验完整 manifest（纯逻辑，无 I/O）。
pub fn validate_manifest(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.manifest_version != MANIFEST_VERSION {
        return Err(ManifestError::UnsupportedManifestVersion(
            manifest.manifest_version,
        ));
    }
    validate_plugin_id(&manifest.id.0)?;
    validate_name(&manifest.name)?;
    if let Some(desc) = &manifest.description
        && desc.chars().count() > 1024
    {
        return Err(ManifestError::InvalidName(
            "description 超过 1024 字符".to_owned(),
        ));
    }
    Version::parse(&manifest.version)
        .map_err(|_| ManifestError::InvalidSemver(manifest.version.clone()))?;
    if manifest.publisher.id.is_empty() {
        return Err(ManifestError::InvalidName(
            "publisher.id 不能为空".to_owned(),
        ));
    }
    if manifest.publisher.name.chars().count() > 256 {
        return Err(ManifestError::InvalidName(
            "publisher.name 超过 256 字符".to_owned(),
        ));
    }
    check_api_major(&manifest.engine_api_version, ENGINE_API_VERSION)?;
    if major_of(&manifest.ui_api_version) != Some(1) {
        return Err(ManifestError::UnsupportedUiApiVersion(
            manifest.ui_api_version.clone(),
        ));
    }
    if manifest.kind != PluginKind::Widget {
        return Err(ManifestError::UnsupportedPluginType(format!(
            "{:?}",
            manifest.kind
        )));
    }
    validate_entrypoints(&manifest.entrypoints)?;
    validate_sizes(&manifest.sizes)?;
    for decl in &manifest.permissions {
        validate_permission(decl)?;
    }
    Ok(())
}

fn validate_plugin_id(id: &str) -> Result<(), ManifestError> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        && !id.starts_with('.')
        && !id.ends_with('.')
        && !id.contains("..");
    if !valid {
        return Err(ManifestError::InvalidPluginId(id.to_owned()));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), ManifestError> {
    if name.is_empty() || name.chars().count() > 256 {
        return Err(ManifestError::InvalidName(
            "name 必须非空且不超过 256 字符".to_owned(),
        ));
    }
    Ok(())
}

fn check_api_major(version: &str, engine: &str) -> Result<(), ManifestError> {
    if major_of(version) == major_of(engine) {
        Ok(())
    } else {
        Err(ManifestError::IncompatibleEngineApiVersion {
            version: version.to_owned(),
            engine: engine.to_owned(),
        })
    }
}

fn major_of(version: &str) -> Option<u64> {
    version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
}

fn validate_entrypoints(entrypoints: &Entrypoints) -> Result<(), ManifestError> {
    // PackagePath 构造已校验形式；此处校验扩展名与类型约定。
    if !entrypoints.ui.as_str().ends_with(".ftui") {
        return Err(ManifestError::InvalidEntrypoint(format!(
            "ui 必须是 .ftui，实际 {}",
            entrypoints.ui
        )));
    }
    if !entrypoints.logic.as_str().ends_with(".wasm") {
        return Err(ManifestError::InvalidEntrypoint(format!(
            "logic 必须是 .wasm，实际 {}",
            entrypoints.logic
        )));
    }
    Ok(())
}

fn validate_sizes(sizes: &Sizes) -> Result<(), ManifestError> {
    let positive = |s: &LogicalSize| {
        s.width.is_finite() && s.height.is_finite() && s.width > 0.0 && s.height > 0.0
    };
    if !positive(&sizes.default) || !positive(&sizes.min) || !positive(&sizes.max) {
        return Err(ManifestError::InvalidSizes(
            "尺寸必须为正有限逻辑像素".to_owned(),
        ));
    }
    if sizes.min.width > sizes.max.width || sizes.min.height > sizes.max.height {
        return Err(ManifestError::InvalidSizes("min 必须不大于 max".to_owned()));
    }
    if sizes.default.width < sizes.min.width
        || sizes.default.width > sizes.max.width
        || sizes.default.height < sizes.min.height
        || sizes.default.height > sizes.max.height
    {
        return Err(ManifestError::InvalidSizes(
            "default 必须在 min..max 内".to_owned(),
        ));
    }
    Ok(())
}

fn validate_permission(decl: &PermissionDecl) -> Result<(), ManifestError> {
    let Some(capability) = CapabilityId::from_name(&decl.capability) else {
        return Err(ManifestError::UnknownCapability(decl.capability.clone()));
    };
    if capability.is_inherent() {
        return Err(ManifestError::InherentCapabilityDeclared(
            decl.capability.clone(),
        ));
    }
    parse_capability_params(capability, decl.params.as_ref())
        .map(|_| ())
        .map_err(|e| match e {
            crate::capability::CapabilityError::InvalidParams { detail, .. } => {
                ManifestError::InvalidCapabilityParams {
                    capability: decl.capability.clone(),
                    detail,
                }
            }
            other => ManifestError::UnknownCapability(other.to_string()),
        })
}

/// 校验包路径形式（manifest-v1 §6.3 的纯路径部分）。
///
/// 拒绝：绝对路径、`..`/`.` 段、反斜杠变体、NUL、首尾斜杠、空段、`.` 前缀。
fn validate_package_path(path: &str) -> Result<(), ManifestError> {
    if path.is_empty() {
        return Err(ManifestError::InvalidPackagePath("空路径".to_owned()));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(ManifestError::InvalidPackagePath(format!(
            "{path}: 不得为绝对路径"
        )));
    }
    if path.contains('\0') {
        return Err(ManifestError::InvalidPackagePath(format!("{path}: 含 NUL")));
    }
    if path.contains('\\') {
        return Err(ManifestError::InvalidPackagePath(format!(
            "{path}: 含反斜杠变体"
        )));
    }
    if path.starts_with('.') {
        return Err(ManifestError::InvalidPackagePath(format!(
            "{path}: 不得以 `.` 开头"
        )));
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(ManifestError::InvalidPackagePath(format!(
                "{path}: 存在空段"
            )));
        }
        if segment == "." || segment == ".." {
            return Err(ManifestError::InvalidPackagePath(format!(
                "{path}: 含 {segment} 段"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn size(w: f32, h: f32) -> LogicalSize {
        LogicalSize {
            width: w,
            height: h,
        }
    }

    fn valid_manifest() -> Manifest {
        Manifest {
            manifest_version: 1,
            id: PluginId("dev.floatile.clock".into()),
            name: "World Clock".into(),
            description: None,
            version: "0.1.0".into(),
            publisher: Publisher {
                id: "dev.floatile".into(),
                name: "Floatile Labs".into(),
            },
            engine_api_version: "1.0.0".into(),
            ui_api_version: "1.0.0".into(),
            kind: PluginKind::Widget,
            entrypoints: Entrypoints {
                ui: PackagePath::parse("ui/widget.ftui").unwrap(),
                logic: PackagePath::parse("logic/plugin.wasm").unwrap(),
            },
            sizes: Sizes {
                default: size(240.0, 120.0),
                min: size(160.0, 80.0),
                max: size(800.0, 600.0),
                resizable: true,
            },
            permissions: vec![PermissionDecl {
                capability: "timer:schedule".into(),
                params: Some(json!({"maxPerMinute": 60, "maxActive": 2})),
            }],
            config: None,
            storage: None,
            build: None,
        }
    }

    #[test]
    fn accepts_valid_manifest() {
        assert!(validate_manifest(&valid_manifest()).is_ok());
    }

    #[test]
    fn rejects_manifest_version() {
        let mut m = valid_manifest();
        m.manifest_version = 2;
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::UnsupportedManifestVersion(2))
        ));
    }

    #[test]
    fn rejects_bad_plugin_id() {
        for bad in ["", "..evil", "UPPER", "dev..x", ".dev"] {
            let mut m = valid_manifest();
            m.id = PluginId(bad.into());
            assert!(validate_manifest(&m).is_err(), "id {bad} should fail");
        }
    }

    #[test]
    fn rejects_bad_semver() {
        let mut m = valid_manifest();
        m.version = "not-a-version".into();
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::InvalidSemver(_))
        ));
    }

    #[test]
    fn rejects_engine_api_mismatch() {
        let mut m = valid_manifest();
        m.engine_api_version = "2.0.0".into();
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::IncompatibleEngineApiVersion { .. })
        ));
        // minor 不同但 major 相同 → 允许（兼容降级）。
        m.engine_api_version = "1.2.0".into();
        assert!(validate_manifest(&m).is_ok());
    }

    #[test]
    fn rejects_ui_api_major() {
        let mut m = valid_manifest();
        m.ui_api_version = "2.0.0".into();
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::UnsupportedUiApiVersion(_))
        ));
    }

    #[test]
    fn rejects_unknown_and_inherent_capabilities() {
        let mut m = valid_manifest();
        m.permissions = vec![PermissionDecl {
            capability: "network:https".into(),
            params: None,
        }];
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::UnknownCapability(_))
        ));

        m.permissions = vec![PermissionDecl {
            capability: "ui:update-state".into(),
            params: None,
        }];
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::InherentCapabilityDeclared(_))
        ));
    }

    #[test]
    fn rejects_bad_capability_params() {
        let mut m = valid_manifest();
        m.permissions = vec![PermissionDecl {
            capability: "timer:schedule".into(),
            params: Some(json!({"bogus": 1})),
        }];
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::InvalidCapabilityParams { .. })
        ));
    }

    #[test]
    fn rejects_sizes_out_of_bounds() {
        let mut m = valid_manifest();
        m.sizes.default = size(900.0, 120.0); // 超 max
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::InvalidSizes(_))
        ));
        m.sizes = Sizes {
            default: size(240.0, 120.0),
            min: size(300.0, 80.0), // min > max
            max: size(200.0, 600.0),
            resizable: true,
        };
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::InvalidSizes(_))
        ));
    }

    #[test]
    fn rejects_bad_entrypoint_extension() {
        let mut m = valid_manifest();
        m.entrypoints.ui = PackagePath::parse("ui/view.slint").unwrap();
        assert!(matches!(
            validate_manifest(&m),
            Err(ManifestError::InvalidEntrypoint(_))
        ));
    }

    #[test]
    fn rejects_malformed_package_path() {
        for bad in [
            "/abs/path",
            "../escape",
            "a/./b",
            "a//b",
            "a\\b",
            "",
            ".hidden",
            "a\0b",
        ] {
            assert!(
                PackagePath::parse(bad).is_err(),
                "path `{bad}` should be rejected"
            );
        }
        // 合法相对路径。
        for good in ["ui/widget.ftui", "assets/icon.png", "config.schema.json"] {
            assert!(PackagePath::parse(good).is_ok(), "path `{good}` ok");
        }
    }

    #[test]
    fn roundtrips_json() {
        let m = valid_manifest();
        let json = serde_json::to_value(&m).unwrap();
        let back: Manifest = serde_json::from_value(json).unwrap();
        assert_eq!(back, m);
    }
}
