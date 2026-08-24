//! 持久化插件实例领域模型（PP-M1）。
//!
//! `InstallationRef` 固定到已经校验的插件版本与内容摘要；`PluginInstance` 表示用户可独立
//! 配置和启停的运行单元。运行时 `State`、宿主窗口和 observed lifecycle 不在此持久模型中。

use std::fmt;

use semver::Version;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::install::{InstallMeta, hex_decode, hex_encode};
use crate::manifest::validate_plugin_id;
use crate::{InstanceId, PluginId};

/// 单实例配置的规范 JSON 字节上限。
pub const MAX_INSTANCE_CONFIG_BYTES: usize = 64 * 1024;
/// 单实例配置的最大 JSON 嵌套深度。
pub const MAX_INSTANCE_CONFIG_DEPTH: usize = 16;

/// 已安装插件内容的 SHA-256 摘要。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstallationDigest([u8; 32]);

impl InstallationDigest {
    /// 从 64 字符小写或大写 hex 解析摘要。
    pub fn from_hex(value: &str) -> Result<Self, InstanceModelError> {
        let bytes = hex_decode(value).ok_or(InstanceModelError::InvalidDigest)?;
        let digest: [u8; 32] = bytes
            .try_into()
            .map_err(|_| InstanceModelError::InvalidDigest)?;
        Ok(Self(digest))
    }

    /// 从原始 SHA-256 字节构造摘要。
    pub const fn from_bytes(value: [u8; 32]) -> Self {
        Self(value)
    }

    /// 返回原始 SHA-256 字节。
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 返回规范小写 hex。
    pub fn to_hex(self) -> String {
        hex_encode(&self.0)
    }
}

impl fmt::Display for InstallationDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex_encode(&self.0))
    }
}

impl Serialize for InstallationDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for InstallationDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(serde::de::Error::custom)
    }
}

/// 对一份不可变 Installation 的精确引用。
///
/// `version` 仅用于诊断和目录解析；`digest` 才是实例恢复时的内容身份。宿主不得静默把实例
/// 切换到同插件的更高版本。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct InstallationRef {
    plugin: PluginId,
    version: String,
    digest: InstallationDigest,
}

impl<'de> Deserialize<'de> for InstallationRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            plugin: PluginId,
            version: String,
            digest: InstallationDigest,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.plugin, wire.version, wire.digest).map_err(serde::de::Error::custom)
    }
}

impl InstallationRef {
    /// 构造并验证插件 ID 与 semver。
    pub fn new(
        plugin: PluginId,
        version: impl Into<String>,
        digest: InstallationDigest,
    ) -> Result<Self, InstanceModelError> {
        validate_plugin_id(&plugin.0)
            .map_err(|_| InstanceModelError::InvalidPluginId(plugin.0.clone()))?;
        let version = version.into();
        Version::parse(&version)
            .map_err(|_| InstanceModelError::InvalidVersion(version.clone()))?;
        Ok(Self {
            plugin,
            version,
            digest,
        })
    }

    /// 从安装器写入并由 PluginManager 复核的元数据构造引用。
    pub fn from_install_meta(meta: &InstallMeta) -> Result<Self, InstanceModelError> {
        Self::new(
            PluginId(meta.id.clone()),
            meta.version.clone(),
            InstallationDigest::from_hex(&meta.digest)?,
        )
    }

    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub const fn digest(&self) -> InstallationDigest {
        self.digest
    }

    /// 重新验证从持久化或其他边界恢复的引用。
    pub fn validate(&self) -> Result<(), InstanceModelError> {
        Self::new(self.plugin.clone(), self.version.clone(), self.digest).map(|_| ())
    }
}

/// 可持久化的非敏感实例配置；调用方仍必须按 Installation 的 config schema 校验具体值。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct InstanceConfig(Map<String, Value>);

impl InstanceConfig {
    /// 接受根为 object、大小与深度均有界的 JSON 配置。
    pub fn new(value: Value) -> Result<Self, InstanceModelError> {
        let Value::Object(map) = value else {
            return Err(InstanceModelError::ConfigRootNotObject);
        };
        let size = serde_json::to_vec(&map)
            .map_err(|error| InstanceModelError::InvalidConfig(error.to_string()))?
            .len();
        if size > MAX_INSTANCE_CONFIG_BYTES {
            return Err(InstanceModelError::ConfigTooLarge {
                actual: size,
                maximum: MAX_INSTANCE_CONFIG_BYTES,
            });
        }
        let depth = object_depth(&map, 1);
        if depth > MAX_INSTANCE_CONFIG_DEPTH {
            return Err(InstanceModelError::ConfigTooDeep {
                actual: depth,
                maximum: MAX_INSTANCE_CONFIG_DEPTH,
            });
        }
        Ok(Self(map))
    }

    pub fn empty() -> Self {
        Self(Map::new())
    }

    pub fn as_object(&self) -> &Map<String, Value> {
        &self.0
    }

    pub fn to_value(&self) -> Value {
        Value::Object(self.0.clone())
    }

    /// 按 Installation 随包提供的 JSON Schema 校验 canonical Config。
    ///
    /// 错误只返回 JSON Pointer，不包含配置值，避免把可能敏感的用户输入带入日志。
    pub fn validate_schema(&self, schema: &Value) -> Result<(), InstanceModelError> {
        // 插件 schema 不得触发 JSON Schema resolver 的宿主网络/文件 I/O。
        // 包内单文档 fragment 仍可用于 `$defs`/`definitions` 复用。
        if schema_contains_external_reference(schema) {
            return Err(InstanceModelError::InvalidConfigSchema);
        }
        let validator = jsonschema::validator_for(schema)
            .map_err(|_| InstanceModelError::InvalidConfigSchema)?;
        let value = self.to_value();
        validator.validate(&value).map_err(|errors| {
            let paths = errors
                .take(8)
                .map(|error| {
                    let path = error.instance_path.to_string();
                    if path.is_empty() {
                        "/".to_owned()
                    } else {
                        path
                    }
                })
                .collect();
            InstanceModelError::ConfigSchemaMismatch { paths }
        })
    }
}

fn schema_contains_external_reference(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(schema_contains_external_reference),
        Value::Object(map) => map.iter().any(|(key, value)| {
            let is_reference = matches!(key.as_str(), "$ref" | "$dynamicRef" | "$recursiveRef");
            (is_reference
                && value
                    .as_str()
                    .is_none_or(|reference| !reference.starts_with('#')))
                || schema_contains_external_reference(value)
        }),
        _ => false,
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self::empty()
    }
}

impl<'de> Deserialize<'de> for InstanceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// 宿主重启后应恢复的实例意图；实际运行状态由 shell/runtime 维护，不写成持久真相。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceDesiredState {
    Stopped,
    Running,
}

impl InstanceDesiredState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Running => "running",
        }
    }

    pub fn parse(value: &str) -> Result<Self, InstanceModelError> {
        match value {
            "stopped" => Ok(Self::Stopped),
            "running" => Ok(Self::Running),
            other => Err(InstanceModelError::InvalidDesiredState(other.to_owned())),
        }
    }
}

/// 一条可跨宿主重启恢复的插件实例记录。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PluginInstance {
    id: InstanceId,
    installation: InstallationRef,
    config: InstanceConfig,
    desired_state: InstanceDesiredState,
    generation: u64,
    created_at: u64,
    updated_at: u64,
}

impl<'de> Deserialize<'de> for PluginInstance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            id: InstanceId,
            installation: InstallationRef,
            config: InstanceConfig,
            desired_state: InstanceDesiredState,
            generation: u64,
            created_at: u64,
            updated_at: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::restore(
            wire.id,
            wire.installation,
            wire.config,
            wire.desired_state,
            wire.generation,
            wire.created_at,
            wire.updated_at,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl PluginInstance {
    /// 从可信创建路径或持久化行恢复实例，并复验领域不变量。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: InstanceId,
        installation: InstallationRef,
        config: InstanceConfig,
        desired_state: InstanceDesiredState,
        generation: u64,
        created_at: u64,
        updated_at: u64,
    ) -> Result<Self, InstanceModelError> {
        installation.validate()?;
        if updated_at < created_at {
            return Err(InstanceModelError::TimestampOrder {
                created_at,
                updated_at,
            });
        }
        Ok(Self {
            id,
            installation,
            config,
            desired_state,
            generation,
            created_at,
            updated_at,
        })
    }

    pub const fn id(&self) -> InstanceId {
        self.id
    }

    pub fn installation(&self) -> &InstallationRef {
        &self.installation
    }

    pub fn config(&self) -> &InstanceConfig {
        &self.config
    }

    pub const fn desired_state(&self) -> InstanceDesiredState {
        self.desired_state
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn created_at(&self) -> u64 {
        self.created_at
    }

    pub const fn updated_at(&self) -> u64 {
        self.updated_at
    }
}

/// 插件实例领域输入或持久化数据不满足不变量。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstanceModelError {
    #[error("非法插件 id `{0}`")]
    InvalidPluginId(String),
    #[error("非法安装版本 `{0}`")]
    InvalidVersion(String),
    #[error("安装 digest 必须是 32 字节 SHA-256 hex")]
    InvalidDigest,
    #[error("实例配置根必须是 object")]
    ConfigRootNotObject,
    #[error("实例配置大小 {actual} 超过上限 {maximum}")]
    ConfigTooLarge { actual: usize, maximum: usize },
    #[error("实例配置深度 {actual} 超过上限 {maximum}")]
    ConfigTooDeep { actual: usize, maximum: usize },
    #[error("实例配置无效: {0}")]
    InvalidConfig(String),
    #[error("实例配置 schema 无效")]
    InvalidConfigSchema,
    #[error("实例配置不符合 schema，路径: {paths:?}")]
    ConfigSchemaMismatch { paths: Vec<String> },
    #[error("未知实例 desired state `{0}`")]
    InvalidDesiredState(String),
    #[error("实例 updated_at {updated_at} 早于 created_at {created_at}")]
    TimestampOrder { created_at: u64, updated_at: u64 },
}

fn object_depth(map: &Map<String, Value>, depth: usize) -> usize {
    map.values()
        .map(|value| value_depth(value, depth + 1))
        .max()
        .unwrap_or(depth)
}

fn value_depth(value: &Value, depth: usize) -> usize {
    match value {
        Value::Object(map) => object_depth(map, depth),
        Value::Array(values) => values
            .iter()
            .map(|value| value_depth(value, depth + 1))
            .max()
            .unwrap_or(depth),
        _ => depth,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    #[test]
    fn config_schema_accepts_valid_config_and_redacts_values_on_reject() {
        let schema = json!({
            "type": "object",
            "required": ["timezone"],
            "additionalProperties": false,
            "properties": { "timezone": { "type": "string", "maxLength": 32 } }
        });
        let valid = InstanceConfig::new(json!({"timezone": "Asia/Shanghai"})).unwrap();
        assert_eq!(valid.validate_schema(&schema), Ok(()));

        let secret = "secret-value-must-not-appear";
        let invalid = InstanceConfig::new(json!({"timezone": 7, "token": secret})).unwrap();
        let error = invalid.validate_schema(&schema).unwrap_err();
        assert!(matches!(
            error,
            InstanceModelError::ConfigSchemaMismatch { .. }
        ));
        assert!(!error.to_string().contains(secret));
    }

    #[test]
    fn invalid_config_schema_is_rejected() {
        let config = InstanceConfig::empty();
        assert_eq!(
            config.validate_schema(&json!({"type": 7})),
            Err(InstanceModelError::InvalidConfigSchema)
        );
    }

    #[test]
    fn external_config_schema_references_are_rejected_without_resolution() {
        let config = InstanceConfig::new(json!({"timezone": "UTC"})).unwrap();
        assert_eq!(
            config.validate_schema(&json!({"$ref": "https://example.invalid/schema.json"})),
            Err(InstanceModelError::InvalidConfigSchema)
        );
        assert!(
            config
                .validate_schema(&json!({
                    "$defs": { "zone": { "type": "string" } },
                    "type": "object",
                    "properties": { "timezone": { "$ref": "#/$defs/zone" } }
                }))
                .is_ok()
        );
    }

    use super::*;

    fn digest() -> InstallationDigest {
        InstallationDigest::from_bytes([0x5a; 32])
    }

    fn installation() -> InstallationRef {
        InstallationRef::new(PluginId("dev.floatile.clock".into()), "1.2.3", digest()).unwrap()
    }

    #[test]
    fn installation_ref_round_trips_canonical_digest() {
        let reference = installation();
        assert_eq!(reference.version(), "1.2.3");
        assert_eq!(reference.digest().to_hex(), "5a".repeat(32));
        let json = serde_json::to_string(&reference).unwrap();
        let restored: InstallationRef = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, reference);
    }

    #[test]
    fn installation_ref_rejects_invalid_identity() {
        assert!(matches!(
            InstallationRef::new(PluginId("UPPER".into()), "1.0.0", digest()),
            Err(InstanceModelError::InvalidPluginId(_))
        ));
        assert!(matches!(
            InstallationRef::new(PluginId("dev.floatile.clock".into()), "latest", digest()),
            Err(InstanceModelError::InvalidVersion(_))
        ));
        assert!(matches!(
            InstallationDigest::from_hex("deadbeef"),
            Err(InstanceModelError::InvalidDigest)
        ));
    }

    #[test]
    fn config_requires_bounded_object() {
        assert!(matches!(
            InstanceConfig::new(json!([1, 2, 3])),
            Err(InstanceModelError::ConfigRootNotObject)
        ));
        assert_eq!(
            InstanceConfig::new(json!({"timezone": "UTC"}))
                .unwrap()
                .to_value(),
            json!({"timezone": "UTC"})
        );

        let oversized = "x".repeat(MAX_INSTANCE_CONFIG_BYTES);
        assert!(matches!(
            InstanceConfig::new(json!({"value": oversized})),
            Err(InstanceModelError::ConfigTooLarge { .. })
        ));

        let mut deep = json!(true);
        for _ in 0..MAX_INSTANCE_CONFIG_DEPTH {
            deep = json!({"child": deep});
        }
        assert!(matches!(
            InstanceConfig::new(deep),
            Err(InstanceModelError::ConfigTooDeep { .. })
        ));
    }

    #[test]
    fn plugin_instance_rejects_reversed_timestamps() {
        let error = PluginInstance::restore(
            InstanceId(2),
            installation(),
            InstanceConfig::empty(),
            InstanceDesiredState::Stopped,
            0,
            20,
            19,
        )
        .unwrap_err();
        assert_eq!(
            error,
            InstanceModelError::TimestampOrder {
                created_at: 20,
                updated_at: 19
            }
        );
    }

    #[test]
    fn deserialization_preserves_instance_invariants() {
        let invalid_reference = json!({
            "plugin": "UPPER",
            "version": "1.0.0",
            "digest": "5a".repeat(32)
        });
        assert!(serde_json::from_value::<InstallationRef>(invalid_reference).is_err());

        let instance = PluginInstance::restore(
            InstanceId(2),
            installation(),
            InstanceConfig::empty(),
            InstanceDesiredState::Stopped,
            0,
            10,
            10,
        )
        .unwrap();
        let mut invalid_instance = serde_json::to_value(instance).unwrap();
        invalid_instance["updated_at"] = json!(9);
        assert!(serde_json::from_value::<PluginInstance>(invalid_instance).is_err());
    }
}
