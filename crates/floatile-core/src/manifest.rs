//! manifest.json v1 的纯模型与校验（`floatile` 插件包元数据）。
//!
//! 事实源：`docs/plugin-sdk/manifest-v1.md`。本模块是 manifest schema 的可执行
//! 单源（serde 类型 + 校验函数）；CLI 与 PluginManager 复用同一实现，不另写一套
//! 平行字段定义。zip 归档、WASM world、UI IR 与 config schema 的深度校验属于
//! CLI/runtime 切片。

use std::fmt;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::capability::{
    CAPABILITY_REGISTRY, CapabilityExposure, CapabilityId, CapabilityParamKind,
    parse_capability_params,
};
use crate::constants::{ENGINE_API_VERSION, MANIFEST_VERSION};
use crate::types::{LogicalSize, PluginId};

/// 插件类型：P0/MVP 仅 `widget`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Widget,
}

/// 包内规范化相对路径（`ui/`、`logic/`、assets 等）。
///
/// 构造时校验形式：必须是规范化相对路径，拒绝绝对路径、`..`/`.` 段、
/// 反斜杠变体、NUL 与空段。重复路径/大小写碰撞/symlink 是包级校验（CLI）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Publisher {
    pub id: String,
    pub name: String,
}

/// 包入口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Entrypoints {
    pub ui: PackagePath,
    pub logic: PackagePath,
}

/// 窗口尺寸（逻辑像素；default 必须在 min..max 内）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Sizes {
    pub default: LogicalSize,
    pub min: LogicalSize,
    pub max: LogicalSize,
    pub resizable: bool,
}

/// 声明能力（manifest 是安装授权上限）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PermissionDecl {
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// 用户配置 schema 引用（包内路径）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConfigRef {
    pub schema: PackagePath,
}

/// 插件私有 KV 迁移版本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StorageDecl {
    pub migration_version: u64,
}

/// 构建诊断元数据（不参与信任/授权）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BuildMeta {
    pub sdk: String,
    pub sdk_version: String,
}

/// 宿主拥有的 HTTPS 请求模板。guest 只能选择模板并填写声明过的查询参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpTemplateDecl {
    pub id: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub query_params: Vec<String>,
    pub credential_header: String,
    #[serde(default = "default_http_statuses")]
    pub allowed_statuses: Vec<u16>,
    #[serde(default = "default_http_max_bytes")]
    pub max_response_bytes: u64,
    #[serde(default = "default_http_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_http_statuses() -> Vec<u16> {
    vec![200]
}

const fn default_http_max_bytes() -> u64 {
    256 * 1024
}

const fn default_http_timeout_ms() -> u64 {
    10_000
}

/// manifest.json v1。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
    #[serde(
        default,
        rename = "httpTemplates",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub http_templates: Vec<HttpTemplateDecl>,
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
    #[error("非法 HTTPS 模板: {0}")]
    InvalidHttpTemplate(String),
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
            Self::InvalidHttpTemplate(_) => "FMAN_INVALID_HTTP_TEMPLATE",
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
    validate_http_templates(manifest)?;
    Ok(())
}

fn validate_http_templates(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.http_templates.len() > 16 {
        return Err(ManifestError::InvalidHttpTemplate(
            "httpTemplates 最多 16 项".to_owned(),
        ));
    }
    let network = manifest
        .permissions
        .iter()
        .find(|permission| permission.capability == "network:https")
        .map(|permission| {
            parse_capability_params(CapabilityId::NetworkHttps, permission.params.as_ref())
        })
        .transpose()
        .map_err(|error| ManifestError::InvalidHttpTemplate(error.to_string()))?
        .flatten();
    let network = match network {
        Some(crate::capability::CapabilityParams::Network {
            origins,
            max_response_bytes,
            max_timeout_ms,
            ..
        }) => Some((origins, max_response_bytes, max_timeout_ms)),
        _ => None,
    };
    let mut ids = std::collections::BTreeSet::new();
    for template in &manifest.http_templates {
        let valid_id = !template.id.is_empty()
            && template.id.len() <= 64
            && template
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
        if !valid_id || !ids.insert(template.id.as_str()) {
            return Err(ManifestError::InvalidHttpTemplate(format!(
                "模板 id `{}` 非法或重复",
                template.id
            )));
        }
        if template.method != "GET" {
            return Err(ManifestError::InvalidHttpTemplate(format!(
                "模板 `{}` 当前仅允许 GET",
                template.id
            )));
        }
        let origin = https_url_origin(&template.url).ok_or_else(|| {
            ManifestError::InvalidHttpTemplate(format!(
                "模板 `{}` URL 必须是无凭证、无 fragment 的固定 HTTPS URL",
                template.id
            ))
        })?;
        let Some((origins, max_bytes, max_timeout)) = &network else {
            return Err(ManifestError::InvalidHttpTemplate(
                "声明 httpTemplates 必须同时声明 network:https".to_owned(),
            ));
        };
        if !origins.iter().any(|allowed| allowed == origin)
            || template.max_response_bytes == 0
            || template.max_response_bytes > *max_bytes
            || !(100..=30_000).contains(&template.timeout_ms)
            || template.timeout_ms > *max_timeout
        {
            return Err(ManifestError::InvalidHttpTemplate(format!(
                "模板 `{}` 超出 network:https origin 或预算",
                template.id
            )));
        }
        if !matches!(
            template.credential_header.as_str(),
            "authorization" | "x-api-key"
        ) {
            return Err(ManifestError::InvalidHttpTemplate(format!(
                "模板 `{}` credentialHeader 仅允许 authorization/x-api-key",
                template.id
            )));
        }
        if template.query_params.len() > 16
            || template.query_params.iter().any(|name| {
                name.is_empty()
                    || name.len() > 64
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
            || template
                .query_params
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != template.query_params.len()
        {
            return Err(ManifestError::InvalidHttpTemplate(format!(
                "模板 `{}` queryParams 非法或重复",
                template.id
            )));
        }
        if template.allowed_statuses.is_empty()
            || template.allowed_statuses.len() > 32
            || template
                .allowed_statuses
                .iter()
                .any(|status| !(100..=599).contains(status))
        {
            return Err(ManifestError::InvalidHttpTemplate(format!(
                "模板 `{}` allowedStatuses 非法",
                template.id
            )));
        }
    }
    Ok(())
}

fn https_url_origin(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty()
        || authority.contains('@')
        || authority.starts_with('[')
        || authority.eq_ignore_ascii_case("localhost")
        || url.contains('#')
        || url.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(&url[.."https://".len() + authority_end])
}

pub(crate) fn validate_plugin_id(id: &str) -> Result<(), ManifestError> {
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

/// 由单一源 serde 模型生成 manifest.json 的独立 JSON Schema（manifest-v1 单源产物）。
///
/// 供外部工具/编辑器校验 `manifest.json`，避免手写第二份平行 schema 造成 drift。
/// 字段名与 serde 序列化一致（`manifestVersion` 等 camelCase rename），并附加
/// `additionalProperties: false` + required 列表，落实「未知字段拒绝」策略。
pub fn manifest_json_schema() -> serde_json::Value {
    let mut root = schemars::schema_for!(Manifest).to_value();
    if let Some(obj) = root.as_object_mut() {
        if let Some(permission) = obj
            .get_mut("$defs")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|definitions| definitions.get_mut("PermissionDecl"))
        {
            *permission = permission_schema_from_registry();
        }
        obj.insert(
            "$schema".into(),
            serde_json::json!("http://json-schema.org/draft-07/schema#"),
        );
        obj.insert("additionalProperties".into(), serde_json::json!(false));
        // 顶层必填字段即 Manifest 的全部序列化字段。
        obj.insert(
            "required".into(),
            serde_json::json!([
                "manifestVersion",
                "id",
                "name",
                "version",
                "publisher",
                "engineApiVersion",
                "uiApiVersion",
                "type",
                "entrypoints",
                "sizes",
                "permissions"
            ]),
        );
        obj.insert("title".into(), serde_json::json!("Floatile manifest v1"));
    }
    root
}

fn permission_schema_from_registry() -> serde_json::Value {
    let alternatives: Vec<_> = CAPABILITY_REGISTRY
        .iter()
        .filter(|definition| definition.exposure == CapabilityExposure::Declared)
        .map(|definition| {
            let params = match definition.params {
                CapabilityParamKind::None => serde_json::Value::Bool(false),
                CapabilityParamKind::Storage => serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "keys": { "type": "array", "items": { "type": "string" } },
                        "maxBytes": { "type": "integer", "minimum": 0 }
                    }
                }),
                CapabilityParamKind::Timer => serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "maxPerMinute": { "type": "integer", "minimum": 0, "maximum": u32::MAX },
                        "maxActive": { "type": "integer", "minimum": 0, "maximum": u32::MAX }
                    }
                }),
                CapabilityParamKind::Metrics => serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "sampleRateHz": { "type": "integer", "minimum": 0, "maximum": u32::MAX }
                    }
                }),
                CapabilityParamKind::Network => serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["origins"],
                    "properties": {
                        "origins": {
                            "type": "array", "minItems": 1, "maxItems": 16,
                            "items": { "type": "string", "pattern": "^https://[^/?#@]+$" }
                        },
                        "maxRequestsPerMinute": { "type": "integer", "minimum": 1, "maximum": 600 },
                        "maxResponseBytes": { "type": "integer", "minimum": 1, "maximum": 1048576 },
                        "maxTimeoutMs": { "type": "integer", "minimum": 100, "maximum": 30000 }
                    }
                }),
            };
            serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["capability"],
                "properties": {
                    "capability": { "const": definition.name },
                    "params": params
                }
            })
        })
        .collect();
    serde_json::json!({
        "description": "声明能力（由 Capability Registry 生成）",
        "oneOf": alternatives
    })
}

/// 用生成的独立 JSON Schema 校验一个 manifest JSON（结构无 drift 的落地校验）。
///
/// P0 是结构校验（字段名/类型/必填/额外字段），不做 #ref 全连通求值；用于
/// `manifest_json_schema` 与 `Manifest` 序列化保持一致性的自检与外部工具校验。
pub fn validate_manifest_json_with_schema(manifest: &serde_json::Value) -> Result<(), String> {
    let schema = manifest_json_schema();
    let validator =
        jsonschema::validator_for(&schema).map_err(|e| format!("manifest schema 自身非法: {e}"))?;
    validator.validate(manifest).map_err(|errors| {
        let mut list: Vec<String> = errors.map(|e| e.to_string()).collect();
        if list.is_empty() {
            list.push("未知校验错误".into());
        }
        list.join("; ")
    })
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
            http_templates: Vec::new(),
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
    fn validates_connection_bound_https_templates() {
        let mut manifest = valid_manifest();
        manifest.permissions.push(PermissionDecl {
            capability: "network:https".into(),
            params: Some(serde_json::json!({
                "origins": ["https://api.example.com"],
                "maxResponseBytes": 4096,
                "maxTimeoutMs": 2000
            })),
        });
        manifest.http_templates.push(HttpTemplateDecl {
            id: "balance".into(),
            method: "GET".into(),
            url: "https://api.example.com/v1/balance".into(),
            query_params: vec!["account".into()],
            credential_header: "authorization".into(),
            allowed_statuses: vec![200],
            max_response_bytes: 4096,
            timeout_ms: 2000,
        });
        assert!(validate_manifest(&manifest).is_ok());

        manifest.http_templates[0].url = "https://evil.example/steal".into();
        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::InvalidHttpTemplate(_))
        ));
        manifest.http_templates[0].url = "https://api.example.com/v1/balance".into();
        manifest.http_templates[0].credential_header = "cookie".into();
        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::InvalidHttpTemplate(_))
        ));
    }

    #[test]
    fn manifest_json_schema_validates_own_serialization() {
        // 单源一致性：生成的 manifest.schema.json 必须能通过自身校验合法 manifest 的
        // 序列化，证明 schema 产物与 serde 模型无 drift。
        let value = serde_json::to_value(valid_manifest()).unwrap();
        assert!(
            validate_manifest_json_with_schema(&value).is_ok(),
            "manifest schema 应接受自身序列化"
        );
    }

    #[test]
    fn manifest_json_schema_rejects_unknown_field() {
        let value = serde_json::to_value(valid_manifest()).unwrap();
        let mut map = value.as_object().unwrap().clone();
        map.insert("evil".into(), json!(true));
        let value = serde_json::Value::Object(map);
        assert!(
            validate_manifest_json_with_schema(&value).is_err(),
            "manifest schema 应拒绝未知字段"
        );
    }

    #[test]
    fn manifest_permission_schema_is_generated_from_declared_registry() {
        let schema = manifest_json_schema();
        let alternatives = schema["$defs"]["PermissionDecl"]["oneOf"]
            .as_array()
            .unwrap();
        let actual: std::collections::BTreeSet<_> = alternatives
            .iter()
            .map(|alternative| {
                alternative["properties"]["capability"]["const"]
                    .as_str()
                    .unwrap()
            })
            .collect();
        let expected: std::collections::BTreeSet<_> = CAPABILITY_REGISTRY
            .iter()
            .filter(|definition| definition.exposure == CapabilityExposure::Declared)
            .map(|definition| definition.name)
            .collect();
        assert_eq!(actual, expected);

        for invalid in ["unknown:capability", "clock:read"] {
            let mut value = serde_json::to_value(valid_manifest()).unwrap();
            value["permissions"][0]["capability"] = json!(invalid);
            value["permissions"][0]
                .as_object_mut()
                .unwrap()
                .remove("params");
            assert!(
                validate_manifest_json_with_schema(&value).is_err(),
                "schema 不应接受 {invalid}"
            );
        }
    }

    #[test]
    fn manifest_json_schema_is_draft07_object() {
        let schema = manifest_json_schema();
        assert_eq!(
            schema["$schema"],
            json!("http://json-schema.org/draft-07/schema#")
        );
        assert!(schema["type"] == json!("object"));
        assert_eq!(schema["additionalProperties"], json!(false));
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
            capability: "network:raw".into(),
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
