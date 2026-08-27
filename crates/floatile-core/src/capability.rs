//! 能力注册表与权限决策的纯模型（PermissionBroker 的输入，无 I/O）。
//!
//! 事实源：`docs/security/permission-model.md`。本模块只定义能力集合、授权结构
//! 与决策输入；执行、配额记账与脱敏审计在 `floatile-services`。

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::{InstanceId, PluginId};

/// P0 宿主能力（固有 + 声明，manifest-v1 §4、permission-model §1）。
///
/// 固有能力（UI/log/clock）固定当前实例 scope，不写入 manifest permissions，
/// 但仍经过 Broker 的身份、schema、配额与审计路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum CapabilityId {
    UiUpdateState,
    LogWrite,
    ClockRead,
    StorageRead,
    StorageWrite,
    TimerSchedule,
    ThemeSubscribe,
    SystemCpu,
    SystemMemory,
    NetworkHttps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityExposure {
    Inherent,
    Declared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityParamKind {
    None,
    Storage,
    Timer,
    Metrics,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CapabilityRisk {
    Inherent,
    L0,
    L2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityExecution {
    Sync,
    SyncAndOperation,
}

/// Capability Registry 的单项稳定元数据。执行实现仍属于 Broker/services，
/// 但名称、暴露方式、参数族、风险、WIT/SDK 映射不再由各层重复声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDefinition {
    pub id: CapabilityId,
    pub name: &'static str,
    pub exposure: CapabilityExposure,
    pub params: CapabilityParamKind,
    pub risk: CapabilityRisk,
    pub execution: CapabilityExecution,
    pub wit_interface: &'static str,
    pub wit_functions: &'static [&'static str],
    pub sdk_surface: &'static str,
    pub author_section: Option<&'static str>,
    pub audit_redaction: &'static str,
}

pub const CAPABILITY_REGISTRY: &[CapabilityDefinition] = &[
    CapabilityDefinition {
        id: CapabilityId::UiUpdateState,
        name: "ui:update-state",
        exposure: CapabilityExposure::Inherent,
        params: CapabilityParamKind::None,
        risk: CapabilityRisk::Inherent,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-ui",
        wit_functions: &["update-state"],
        sdk_surface: "ctx.state",
        author_section: None,
        audit_redaction: "size-and-error-path-only",
    },
    CapabilityDefinition {
        id: CapabilityId::LogWrite,
        name: "log:write",
        exposure: CapabilityExposure::Inherent,
        params: CapabilityParamKind::None,
        risk: CapabilityRisk::Inherent,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-log",
        wit_functions: &["log"],
        sdk_surface: "ctx.log",
        author_section: None,
        audit_redaction: "message-length-only",
    },
    CapabilityDefinition {
        id: CapabilityId::ClockRead,
        name: "clock:read",
        exposure: CapabilityExposure::Inherent,
        params: CapabilityParamKind::None,
        risk: CapabilityRisk::Inherent,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-clock",
        wit_functions: &["now"],
        sdk_surface: "ctx.clock",
        author_section: None,
        audit_redaction: "no-result-values",
    },
    CapabilityDefinition {
        id: CapabilityId::StorageRead,
        name: "storage:read",
        exposure: CapabilityExposure::Declared,
        params: CapabilityParamKind::Storage,
        risk: CapabilityRisk::L0,
        execution: CapabilityExecution::SyncAndOperation,
        wit_interface: "host-storage",
        wit_functions: &["get", "submit-get", "take-get-result"],
        sdk_surface: "ctx.storage",
        author_section: Some("storage"),
        audit_redaction: "key-metadata-no-values",
    },
    CapabilityDefinition {
        id: CapabilityId::StorageWrite,
        name: "storage:write",
        exposure: CapabilityExposure::Declared,
        params: CapabilityParamKind::Storage,
        risk: CapabilityRisk::L0,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-storage",
        wit_functions: &["set", "delete"],
        sdk_surface: "ctx.storage",
        author_section: Some("storage"),
        audit_redaction: "key-and-size-no-values",
    },
    CapabilityDefinition {
        id: CapabilityId::TimerSchedule,
        name: "timer:schedule",
        exposure: CapabilityExposure::Declared,
        params: CapabilityParamKind::Timer,
        risk: CapabilityRisk::L0,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-timer",
        wit_functions: &["schedule", "cancel"],
        sdk_surface: "ctx.timer",
        author_section: Some("timer"),
        audit_redaction: "delay-and-budget-only",
    },
    CapabilityDefinition {
        id: CapabilityId::ThemeSubscribe,
        name: "theme:subscribe",
        exposure: CapabilityExposure::Declared,
        params: CapabilityParamKind::None,
        risk: CapabilityRisk::L0,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-theme",
        wit_functions: &["get-token", "subscribe", "unsubscribe"],
        sdk_surface: "ctx.theme",
        author_section: Some("theme"),
        audit_redaction: "token-name-no-values",
    },
    CapabilityDefinition {
        id: CapabilityId::SystemCpu,
        name: "system:cpu",
        exposure: CapabilityExposure::Declared,
        params: CapabilityParamKind::Metrics,
        risk: CapabilityRisk::L0,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-metrics",
        wit_functions: &["cpu-percent"],
        sdk_surface: "ctx.metrics",
        author_section: Some("metrics"),
        audit_redaction: "result-bucket-only",
    },
    CapabilityDefinition {
        id: CapabilityId::SystemMemory,
        name: "system:memory",
        exposure: CapabilityExposure::Declared,
        params: CapabilityParamKind::None,
        risk: CapabilityRisk::L0,
        execution: CapabilityExecution::Sync,
        wit_interface: "host-metrics",
        wit_functions: &["memory"],
        sdk_surface: "ctx.metrics",
        author_section: Some("metrics"),
        audit_redaction: "result-bucket-only",
    },
    CapabilityDefinition {
        id: CapabilityId::NetworkHttps,
        name: "network:https",
        exposure: CapabilityExposure::Declared,
        params: CapabilityParamKind::Network,
        risk: CapabilityRisk::L2,
        execution: CapabilityExecution::SyncAndOperation,
        wit_interface: "host-http",
        wit_functions: &["submit", "take-result"],
        sdk_surface: "ctx.http",
        author_section: Some("httpTemplates"),
        audit_redaction: "template-origin-status-and-size-only",
    },
];

impl CapabilityId {
    pub fn name(&self) -> &'static str {
        self.definition().name
    }

    pub fn definition(&self) -> &'static CapabilityDefinition {
        &CAPABILITY_REGISTRY[*self as usize]
    }

    /// 固有能力：安装时不提示，固定当前实例 scope，不可放宽。
    pub fn is_inherent(&self) -> bool {
        self.definition().exposure == CapabilityExposure::Inherent
    }

    /// 按 manifest capability 字符串解析；未注册的能力返回 `None`。
    pub fn from_name(name: &str) -> Option<Self> {
        CAPABILITY_REGISTRY
            .iter()
            .find(|definition| definition.name == name)
            .map(|definition| definition.id)
    }
}

impl Serialize for CapabilityId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for CapabilityId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Self::from_name(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("未知 capability `{name}`")))
    }
}

/// 能力参数（scope/配额）。P0 默认值见 permission-model §1.2；数字在
/// evil/clock/10-instance 数据后冻结。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityParams {
    /// storage:read/write：键范围与写配额。`keys` 为空表示插件全部私有键。
    Storage { keys: Vec<String>, max_bytes: u64 },
    /// timer:schedule：每分钟上限与活跃计时器上限。
    Timer {
        max_per_minute: u32,
        max_active: u32,
    },
    /// system:cpu：采样频率上限（Hz）。
    Metrics { sample_rate_hz: u32 },
    /// network:https：精确 HTTPS origin 白名单与宿主执行预算。
    Network {
        origins: Vec<String>,
        max_requests_per_minute: u32,
        max_response_bytes: u64,
        max_timeout_ms: u64,
    },
}

/// 授权来源（permission-model §2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveGrant {
    Prompted,
    Explicit,
    #[serde(rename = "derived-from-install")]
    DerivedFromInstall,
}

/// 信任级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLevel {
    None,
    Dev,
    #[serde(rename = "signed-untrusted")]
    SignedUntrusted,
    #[serde(rename = "signed-trusted")]
    SignedTrusted,
}

/// 单条授权：能力 + 参数上限 + 来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub capability: CapabilityId,
    pub params: Option<CapabilityParams>,
    pub effective: EffectiveGrant,
}

/// 插件级授权（安装时上限，用户 grant 只能收窄）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grants {
    pub plugin: PluginId,
    pub caps: Vec<Grant>,
    pub trust: TrustLevel,
}

/// 实例级授权：每实例可收窄，不可放宽。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceGrant {
    pub instance: InstanceId,
    pub caps: Vec<Grant>,
}

impl Grants {
    pub fn find(&self, capability: CapabilityId) -> Option<&Grant> {
        self.caps.iter().find(|g| g.capability == capability)
    }
}

/// 权限决策结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allowed,
    Denied { reason: DenyReason },
}

/// 拒绝原因（稳定、可观测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// 未在 grants 中声明。
    NotGranted,
    /// 参数超出 scope（键范围、速率等）。
    ScopeViolation,
    /// 超出配额（字节、次数上限）。
    QuotaExceeded,
    /// 环境能力不可用（如 Wayland 无穿透）。
    EnvironmentUnavailable,
    /// 请求参数非法。
    InvalidInput,
}

/// 能力参数解析/校验错误。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CapabilityError {
    #[error("未知能力 `{0}`")]
    UnknownCapability(String),
    #[error("能力 `{capability}` 参数非法: {detail}")]
    InvalidParams {
        capability: &'static str,
        detail: String,
    },
    #[error("能力未在插件授权中声明: {0}")]
    NotGranted(String),
    #[error("实例授权超出插件授权上限: {0}")]
    ExceedsPluginGrant(String),
}

/// 从 manifest 的 JSON params 解析为该能力的 `CapabilityParams`。
///
/// 未知字段一律拒绝；参数缺失时使用该能力默认值（permission-model §1.2）。
pub fn parse_capability_params(
    capability: CapabilityId,
    json: Option<&serde_json::Value>,
) -> Result<Option<CapabilityParams>, CapabilityError> {
    let Some(json) = json else {
        return default_params(capability);
    };
    let obj = json
        .as_object()
        .ok_or_else(|| CapabilityError::InvalidParams {
            capability: capability.name(),
            detail: "params 必须是 JSON object".to_owned(),
        })?;

    let params = match capability.definition().params {
        CapabilityParamKind::Storage => {
            let mut keys = Vec::new();
            let mut max_bytes = 64 * 1024;
            for (k, v) in obj {
                match k.as_str() {
                    "keys" => {
                        let arr = v.as_array().ok_or_else(|| CapabilityError::InvalidParams {
                            capability: capability.name(),
                            detail: "`keys` 必须是字符串数组".to_owned(),
                        })?;
                        for key in arr {
                            let key =
                                key.as_str().ok_or_else(|| CapabilityError::InvalidParams {
                                    capability: capability.name(),
                                    detail: "`keys` 元素必须是字符串".to_owned(),
                                })?;
                            validate_storage_key(key)?;
                            keys.push(key.to_owned());
                        }
                    }
                    "maxBytes" => {
                        max_bytes = v.as_u64().ok_or_else(|| CapabilityError::InvalidParams {
                            capability: capability.name(),
                            detail: "`maxBytes` 必须是非负整数".to_owned(),
                        })?;
                    }
                    other => {
                        return Err(CapabilityError::InvalidParams {
                            capability: capability.name(),
                            detail: format!("未知字段 `{other}`"),
                        });
                    }
                }
            }
            CapabilityParams::Storage { keys, max_bytes }
        }
        CapabilityParamKind::Timer => {
            let mut max_per_minute = 60;
            let mut max_active = 8;
            for (k, v) in obj {
                match k.as_str() {
                    "maxPerMinute" => {
                        max_per_minute = v
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or_else(|| CapabilityError::InvalidParams {
                                capability: capability.name(),
                                detail: "`maxPerMinute` 必须是非负 32 位整数".to_owned(),
                            })?;
                    }
                    "maxActive" => {
                        max_active =
                            v.as_u64()
                                .and_then(|n| u32::try_from(n).ok())
                                .ok_or_else(|| CapabilityError::InvalidParams {
                                    capability: capability.name(),
                                    detail: "`maxActive` 必须是非负 32 位整数".to_owned(),
                                })?;
                    }
                    other => {
                        return Err(CapabilityError::InvalidParams {
                            capability: capability.name(),
                            detail: format!("未知字段 `{other}`"),
                        });
                    }
                }
            }
            CapabilityParams::Timer {
                max_per_minute,
                max_active,
            }
        }
        CapabilityParamKind::Metrics => {
            let mut sample_rate_hz = 1;
            for (k, v) in obj {
                match k.as_str() {
                    "sampleRateHz" => {
                        sample_rate_hz = v
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or_else(|| CapabilityError::InvalidParams {
                                capability: capability.name(),
                                detail: "`sampleRateHz` 必须是非负 32 位整数".to_owned(),
                            })?;
                    }
                    other => {
                        return Err(CapabilityError::InvalidParams {
                            capability: capability.name(),
                            detail: format!("未知字段 `{other}`"),
                        });
                    }
                }
            }
            CapabilityParams::Metrics { sample_rate_hz }
        }
        CapabilityParamKind::Network => {
            let mut origins = Vec::new();
            let mut max_requests_per_minute = 30;
            let mut max_response_bytes = 256 * 1024;
            let mut max_timeout_ms = 10_000;
            for (k, v) in obj {
                match k.as_str() {
                    "origins" => {
                        let values =
                            v.as_array().ok_or_else(|| CapabilityError::InvalidParams {
                                capability: capability.name(),
                                detail: "`origins` 必须是字符串数组".to_owned(),
                            })?;
                        if values.is_empty() || values.len() > 16 {
                            return Err(CapabilityError::InvalidParams {
                                capability: capability.name(),
                                detail: "`origins` 必须包含 1..=16 项".to_owned(),
                            });
                        }
                        for value in values {
                            let origin =
                                value
                                    .as_str()
                                    .ok_or_else(|| CapabilityError::InvalidParams {
                                        capability: capability.name(),
                                        detail: "`origins` 元素必须是字符串".to_owned(),
                                    })?;
                            validate_https_origin(origin)?;
                            if origins.iter().any(|item| item == origin) {
                                return Err(CapabilityError::InvalidParams {
                                    capability: capability.name(),
                                    detail: format!("重复 origin `{origin}`"),
                                });
                            }
                            origins.push(origin.to_owned());
                        }
                    }
                    "maxRequestsPerMinute" => {
                        max_requests_per_minute = bounded_u32(v, capability, k, 1, 600)?;
                    }
                    "maxResponseBytes" => {
                        max_response_bytes = bounded_u64(v, capability, k, 1, 1024 * 1024)?;
                    }
                    "maxTimeoutMs" => {
                        max_timeout_ms = bounded_u64(v, capability, k, 100, 30_000)?;
                    }
                    other => {
                        return Err(CapabilityError::InvalidParams {
                            capability: capability.name(),
                            detail: format!("未知字段 `{other}`"),
                        });
                    }
                }
            }
            if origins.is_empty() {
                return Err(CapabilityError::InvalidParams {
                    capability: capability.name(),
                    detail: "`origins` 不得省略".to_owned(),
                });
            }
            CapabilityParams::Network {
                origins,
                max_requests_per_minute,
                max_response_bytes,
                max_timeout_ms,
            }
        }
        // 无参数能力：任何 params 都拒绝。
        CapabilityParamKind::None => {
            return Err(CapabilityError::InvalidParams {
                capability: capability.name(),
                detail: "该能力不接受 params".to_owned(),
            });
        }
    };
    Ok(Some(params))
}

fn default_params(capability: CapabilityId) -> Result<Option<CapabilityParams>, CapabilityError> {
    Ok(Some(match capability.definition().params {
        CapabilityParamKind::Storage => CapabilityParams::Storage {
            keys: Vec::new(),
            max_bytes: 64 * 1024,
        },
        CapabilityParamKind::Timer => CapabilityParams::Timer {
            max_per_minute: 60,
            max_active: 8,
        },
        CapabilityParamKind::Metrics => CapabilityParams::Metrics { sample_rate_hz: 1 },
        CapabilityParamKind::Network => {
            return Err(CapabilityError::InvalidParams {
                capability: capability.name(),
                detail: "`origins` 不得省略".to_owned(),
            });
        }
        CapabilityParamKind::None => return Ok(None),
    }))
}

fn bounded_u32(
    value: &serde_json::Value,
    capability: CapabilityId,
    field: &str,
    min: u32,
    max: u32,
) -> Result<u32, CapabilityError> {
    let value = value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| CapabilityError::InvalidParams {
            capability: capability.name(),
            detail: format!("`{field}` 必须是 32 位整数"),
        })?;
    if !(min..=max).contains(&value) {
        return Err(CapabilityError::InvalidParams {
            capability: capability.name(),
            detail: format!("`{field}` 必须在 {min}..={max}"),
        });
    }
    Ok(value)
}

fn bounded_u64(
    value: &serde_json::Value,
    capability: CapabilityId,
    field: &str,
    min: u64,
    max: u64,
) -> Result<u64, CapabilityError> {
    let value = value
        .as_u64()
        .ok_or_else(|| CapabilityError::InvalidParams {
            capability: capability.name(),
            detail: format!("`{field}` 必须是非负整数"),
        })?;
    if !(min..=max).contains(&value) {
        return Err(CapabilityError::InvalidParams {
            capability: capability.name(),
            detail: format!("`{field}` 必须在 {min}..={max}"),
        });
    }
    Ok(value)
}

fn validate_https_origin(origin: &str) -> Result<(), CapabilityError> {
    let invalid = !origin.starts_with("https://")
        || origin.len() > 255
        || origin[8..].is_empty()
        || origin[8..].contains('/')
        || origin.contains('@')
        || origin.contains('?')
        || origin.contains('#')
        || origin.chars().any(char::is_whitespace);
    if invalid {
        return Err(CapabilityError::InvalidParams {
            capability: "network:https",
            detail: format!("`{origin}` 不是精确 HTTPS origin"),
        });
    }
    Ok(())
}

fn validate_storage_key(key: &str) -> Result<(), CapabilityError> {
    if key.is_empty() {
        return Err(CapabilityError::InvalidParams {
            capability: "storage:*",
            detail: "空存储键".to_owned(),
        });
    }
    if key.contains('\0') || key.chars().count() > 256 {
        return Err(CapabilityError::InvalidParams {
            capability: "storage:*",
            detail: "存储键含 NUL 或超过 256 字符".to_owned(),
        });
    }
    Ok(())
}

/// 决策：grant 是否存在、请求参数是否在授权范围内。
pub fn decide(grant: Option<&Grant>, request: Option<&CapabilityParams>) -> PermissionDecision {
    let Some(grant) = grant else {
        return PermissionDecision::Denied {
            reason: DenyReason::NotGranted,
        };
    };
    let Some(request) = request else {
        return if grant.params.is_none() {
            PermissionDecision::Allowed
        } else {
            PermissionDecision::Denied {
                reason: DenyReason::ScopeViolation,
            }
        };
    };
    let Some(granted) = &grant.params else {
        // 授权无参数，请求却带参数 → 超 scope。
        return PermissionDecision::Denied {
            reason: DenyReason::ScopeViolation,
        };
    };
    match (granted, request) {
        (
            CapabilityParams::Storage { keys, max_bytes },
            CapabilityParams::Storage {
                keys: rk,
                max_bytes: rb,
            },
        ) => {
            let keys_ok = keys.is_empty() || rk.iter().all(|k| keys.contains(k));
            if !keys_ok {
                return PermissionDecision::Denied {
                    reason: DenyReason::ScopeViolation,
                };
            }
            if rb > max_bytes {
                return PermissionDecision::Denied {
                    reason: DenyReason::QuotaExceeded,
                };
            }
            PermissionDecision::Allowed
        }
        (
            CapabilityParams::Timer {
                max_per_minute,
                max_active,
            },
            CapabilityParams::Timer {
                max_per_minute: rp,
                max_active: ra,
            },
        ) => {
            if rp > max_per_minute || ra > max_active {
                return PermissionDecision::Denied {
                    reason: DenyReason::QuotaExceeded,
                };
            }
            PermissionDecision::Allowed
        }
        (
            CapabilityParams::Metrics { sample_rate_hz },
            CapabilityParams::Metrics { sample_rate_hz: rr },
        ) => {
            if rr > sample_rate_hz {
                return PermissionDecision::Denied {
                    reason: DenyReason::QuotaExceeded,
                };
            }
            PermissionDecision::Allowed
        }
        (
            CapabilityParams::Network {
                origins,
                max_requests_per_minute,
                max_response_bytes,
                max_timeout_ms,
            },
            CapabilityParams::Network {
                origins: requested_origins,
                max_requests_per_minute: requested_rate,
                max_response_bytes: requested_bytes,
                max_timeout_ms: requested_timeout,
            },
        ) => {
            if !requested_origins
                .iter()
                .all(|origin| origins.contains(origin))
            {
                return PermissionDecision::Denied {
                    reason: DenyReason::ScopeViolation,
                };
            }
            if requested_rate > max_requests_per_minute
                || requested_bytes > max_response_bytes
                || requested_timeout > max_timeout_ms
            {
                return PermissionDecision::Denied {
                    reason: DenyReason::QuotaExceeded,
                };
            }
            PermissionDecision::Allowed
        }
        _ => PermissionDecision::Denied {
            reason: DenyReason::InvalidInput,
        },
    }
}

/// 实例授权收窄：每项能力必须存在于插件授权中，且参数不得放宽。
pub fn narrow_instance(
    plugin: &Grants,
    instance: InstanceId,
    caps: Vec<Grant>,
) -> Result<InstanceGrant, CapabilityError> {
    for cap in &caps {
        let granted = plugin.find(cap.capability).ok_or_else(|| {
            CapabilityError::NotGranted(format!("实例要求 {} 但插件未授权", cap.capability.name()))
        })?;
        if !params_within(granted.params.as_ref(), cap.params.as_ref()) {
            return Err(CapabilityError::ExceedsPluginGrant(format!(
                "{} 实例参数超出插件授权",
                cap.capability.name()
            )));
        }
    }
    Ok(InstanceGrant { instance, caps })
}

fn params_within(plugin: Option<&CapabilityParams>, instance: Option<&CapabilityParams>) -> bool {
    match (plugin, instance) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(_), None) => true,
        (Some(p), Some(i)) => match (p, i) {
            (
                CapabilityParams::Storage { keys, max_bytes },
                CapabilityParams::Storage {
                    keys: ik,
                    max_bytes: ib,
                },
            ) => {
                let keys_ok = keys.is_empty() || ik.iter().all(|k| keys.contains(k));
                keys_ok && ib <= max_bytes
            }
            (
                CapabilityParams::Timer {
                    max_per_minute,
                    max_active,
                },
                CapabilityParams::Timer {
                    max_per_minute: ip,
                    max_active: ia,
                },
            ) => ip <= max_per_minute && ia <= max_active,
            (
                CapabilityParams::Metrics { sample_rate_hz },
                CapabilityParams::Metrics { sample_rate_hz: ir },
            ) => ir <= sample_rate_hz,
            (
                CapabilityParams::Network {
                    origins,
                    max_requests_per_minute,
                    max_response_bytes,
                    max_timeout_ms,
                },
                CapabilityParams::Network {
                    origins: instance_origins,
                    max_requests_per_minute: instance_rate,
                    max_response_bytes: instance_bytes,
                    max_timeout_ms: instance_timeout,
                },
            ) => {
                instance_origins
                    .iter()
                    .all(|origin| origins.contains(origin))
                    && instance_rate <= max_requests_per_minute
                    && instance_bytes <= max_response_bytes
                    && instance_timeout <= max_timeout_ms
            }
            _ => false,
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn registry_is_unique_indexed_and_serde_round_trips() {
        let mut names = BTreeSet::new();
        for (index, definition) in CAPABILITY_REGISTRY.iter().enumerate() {
            assert_eq!(definition.id as usize, index);
            assert!(names.insert(definition.name), "重复 capability 名称");
            assert_eq!(
                CapabilityId::from_name(definition.name),
                Some(definition.id)
            );
            assert_eq!(definition.id.definition(), definition);
            assert!(!definition.wit_interface.is_empty());
            assert!(!definition.wit_functions.is_empty());
            assert!(!definition.sdk_surface.is_empty());
            assert!(!definition.audit_redaction.is_empty());
            assert_eq!(
                serde_json::from_value::<CapabilityId>(json!(definition.name)).unwrap(),
                definition.id
            );
            assert_eq!(
                serde_json::to_value(definition.id).unwrap(),
                json!(definition.name)
            );
            if definition.exposure == CapabilityExposure::Declared {
                assert!(definition.author_section.is_some());
            }
        }
        assert!(CapabilityId::from_name("storage:read-extra").is_none());
        assert!(serde_json::from_value::<CapabilityId>(json!("unknown:capability")).is_err());
    }

    #[test]
    fn registry_covers_every_wit_capability_interface() {
        let wit = include_str!("../../../wit/floatile-widget.wit");
        let world = wit.split("world floatile-widget {").nth(1).unwrap();
        let imported: BTreeSet<_> = world
            .lines()
            .map(str::trim)
            .filter_map(|line| line.strip_prefix("import "))
            .filter_map(|line| line.strip_suffix(';'))
            .filter(|interface| *interface != "host-operation")
            .collect();
        let registered: BTreeSet<_> = CAPABILITY_REGISTRY
            .iter()
            .map(|definition| definition.wit_interface)
            .collect();
        assert_eq!(imported, registered);

        for interface in registered {
            let marker = format!("interface {interface} {{");
            let body = wit
                .split(&marker)
                .nth(1)
                .unwrap_or_else(|| panic!("WIT 缺少 interface {interface}"));
            let mut depth = 1usize;
            let mut functions = BTreeSet::new();
            for line in body.lines() {
                let trimmed = line.trim();
                if depth == 1
                    && let Some((name, _)) = trimmed.split_once(": func(")
                {
                    functions.insert(name);
                }
                depth += trimmed
                    .chars()
                    .filter(|character| *character == '{')
                    .count();
                depth -= trimmed
                    .chars()
                    .filter(|character| *character == '}')
                    .count();
                if depth == 0 {
                    break;
                }
            }
            let mapped: BTreeSet<_> = CAPABILITY_REGISTRY
                .iter()
                .filter(|definition| definition.wit_interface == interface)
                .flat_map(|definition| definition.wit_functions.iter().copied())
                .collect();
            assert_eq!(functions, mapped, "{interface} function 映射漂移");
        }
    }

    fn plugin_grants() -> Grants {
        Grants {
            plugin: PluginId("dev.floatile.clock".into()),
            trust: TrustLevel::Dev,
            caps: vec![
                Grant {
                    capability: CapabilityId::TimerSchedule,
                    params: Some(CapabilityParams::Timer {
                        max_per_minute: 60,
                        max_active: 2,
                    }),
                    effective: EffectiveGrant::DerivedFromInstall,
                },
                Grant {
                    capability: CapabilityId::StorageWrite,
                    params: Some(CapabilityParams::Storage {
                        keys: vec!["settings".into()],
                        max_bytes: 4096,
                    }),
                    effective: EffectiveGrant::DerivedFromInstall,
                },
            ],
        }
    }

    #[test]
    fn deny_by_default_when_not_granted() {
        let grants = plugin_grants();
        assert_eq!(
            decide(grants.find(CapabilityId::ClockRead), None),
            PermissionDecision::Denied {
                reason: DenyReason::NotGranted
            }
        );
    }

    #[test]
    fn allows_within_quota() {
        let grants = plugin_grants();
        let grant = grants.find(CapabilityId::TimerSchedule).unwrap();
        assert_eq!(
            decide(
                Some(grant),
                Some(&CapabilityParams::Timer {
                    max_per_minute: 60,
                    max_active: 1
                })
            ),
            PermissionDecision::Allowed
        );
    }

    #[test]
    fn rejects_over_quota() {
        let grants = plugin_grants();
        let grant = grants.find(CapabilityId::TimerSchedule).unwrap();
        assert_eq!(
            decide(
                Some(grant),
                Some(&CapabilityParams::Timer {
                    max_per_minute: 120,
                    max_active: 1
                })
            ),
            PermissionDecision::Denied {
                reason: DenyReason::QuotaExceeded
            }
        );
        let storage = grants.find(CapabilityId::StorageWrite).unwrap();
        assert_eq!(
            decide(
                Some(storage),
                Some(&CapabilityParams::Storage {
                    keys: vec!["settings".into()],
                    max_bytes: 8192
                })
            ),
            PermissionDecision::Denied {
                reason: DenyReason::QuotaExceeded
            }
        );
    }

    #[test]
    fn rejects_out_of_scope_key() {
        let grants = plugin_grants();
        let storage = grants.find(CapabilityId::StorageWrite).unwrap();
        assert_eq!(
            decide(
                Some(storage),
                Some(&CapabilityParams::Storage {
                    keys: vec!["other".into()],
                    max_bytes: 100
                })
            ),
            PermissionDecision::Denied {
                reason: DenyReason::ScopeViolation
            }
        );
    }

    #[test]
    fn instance_narrowing_only() {
        let grants = plugin_grants();
        let ok = narrow_instance(
            &grants,
            InstanceId(1),
            vec![Grant {
                capability: CapabilityId::TimerSchedule,
                params: Some(CapabilityParams::Timer {
                    max_per_minute: 30,
                    max_active: 1,
                }),
                effective: EffectiveGrant::DerivedFromInstall,
            }],
        );
        assert!(ok.is_ok());
        // 放宽失败。
        let widen = narrow_instance(
            &grants,
            InstanceId(1),
            vec![Grant {
                capability: CapabilityId::TimerSchedule,
                params: Some(CapabilityParams::Timer {
                    max_per_minute: 90,
                    max_active: 1,
                }),
                effective: EffectiveGrant::DerivedFromInstall,
            }],
        );
        assert!(widen.is_err());
        // 未授权能力失败。
        let unlisted = narrow_instance(
            &grants,
            InstanceId(1),
            vec![Grant {
                capability: CapabilityId::ClockRead,
                params: None,
                effective: EffectiveGrant::DerivedFromInstall,
            }],
        );
        assert!(unlisted.is_err());
    }

    #[test]
    fn parses_params_and_rejects_unknown_fields() {
        let timer = parse_capability_params(
            CapabilityId::TimerSchedule,
            Some(&json!({"maxPerMinute": 30, "maxActive": 2})),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            timer,
            CapabilityParams::Timer {
                max_per_minute: 30,
                max_active: 2
            }
        );

        assert!(
            parse_capability_params(CapabilityId::TimerSchedule, Some(&json!({"bogus": 1})),)
                .is_err()
        );

        assert!(
            parse_capability_params(
                CapabilityId::ThemeSubscribe,
                Some(&json!({"maxPerMinute": 1})),
            )
            .is_err()
        );
    }

    #[test]
    fn defaults_applied_without_params() {
        let timer = parse_capability_params(CapabilityId::TimerSchedule, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            timer,
            CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 8
            }
        );
        assert!(
            parse_capability_params(CapabilityId::ThemeSubscribe, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn network_scope_requires_exact_https_origins_and_bounded_budgets() {
        let params = parse_capability_params(
            CapabilityId::NetworkHttps,
            Some(&json!({
                "origins": ["https://api.example.com"],
                "maxRequestsPerMinute": 12,
                "maxResponseBytes": 4096,
                "maxTimeoutMs": 2000
            })),
        )
        .unwrap();
        assert!(matches!(params, Some(CapabilityParams::Network { .. })));
        for origin in [
            "http://api.example.com",
            "https://user@example.com",
            "https://api.example.com/path",
            "https://api.example.com?token=x",
        ] {
            assert!(
                parse_capability_params(
                    CapabilityId::NetworkHttps,
                    Some(&json!({ "origins": [origin] })),
                )
                .is_err(),
                "应拒绝 {origin}"
            );
        }
        assert!(parse_capability_params(CapabilityId::NetworkHttps, None).is_err());
    }
}
