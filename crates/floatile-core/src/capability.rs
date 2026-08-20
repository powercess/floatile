//! 能力注册表与权限决策的纯模型（PermissionBroker 的输入，无 I/O）。
//!
//! 事实源：`docs/security/permission-model.md`。本模块只定义能力集合、授权结构
//! 与决策输入；执行、配额记账与脱敏审计在 `floatile-services`。

use serde::{Deserialize, Serialize};

use crate::types::{InstanceId, PluginId};

/// P0 宿主能力（固有 + 声明，manifest-v1 §4、permission-model §1）。
///
/// 固有能力（UI/log/clock）固定当前实例 scope，不写入 manifest permissions，
/// 但仍经过 Broker 的身份、schema、配额与审计路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityId {
    #[serde(rename = "ui:update-state")]
    UiUpdateState,
    #[serde(rename = "log:write")]
    LogWrite,
    #[serde(rename = "clock:read")]
    ClockRead,
    #[serde(rename = "storage:read")]
    StorageRead,
    #[serde(rename = "storage:write")]
    StorageWrite,
    #[serde(rename = "timer:schedule")]
    TimerSchedule,
    #[serde(rename = "theme:subscribe")]
    ThemeSubscribe,
    #[serde(rename = "system:cpu")]
    SystemCpu,
    #[serde(rename = "system:memory")]
    SystemMemory,
}

impl CapabilityId {
    pub fn name(&self) -> &'static str {
        match self {
            Self::UiUpdateState => "ui:update-state",
            Self::LogWrite => "log:write",
            Self::ClockRead => "clock:read",
            Self::StorageRead => "storage:read",
            Self::StorageWrite => "storage:write",
            Self::TimerSchedule => "timer:schedule",
            Self::ThemeSubscribe => "theme:subscribe",
            Self::SystemCpu => "system:cpu",
            Self::SystemMemory => "system:memory",
        }
    }

    /// 固有能力：安装时不提示，固定当前实例 scope，不可放宽。
    pub fn is_inherent(&self) -> bool {
        matches!(self, Self::UiUpdateState | Self::LogWrite | Self::ClockRead)
    }

    /// 按 manifest capability 字符串解析；未注册的能力返回 `None`。
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "ui:update-state" => Self::UiUpdateState,
            "log:write" => Self::LogWrite,
            "clock:read" => Self::ClockRead,
            "storage:read" => Self::StorageRead,
            "storage:write" => Self::StorageWrite,
            "timer:schedule" => Self::TimerSchedule,
            "theme:subscribe" => Self::ThemeSubscribe,
            "system:cpu" => Self::SystemCpu,
            "system:memory" => Self::SystemMemory,
            _ => return None,
        })
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

    let params = match capability {
        CapabilityId::StorageRead | CapabilityId::StorageWrite => {
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
        CapabilityId::TimerSchedule => {
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
        CapabilityId::SystemCpu => {
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
        // 无参数能力：任何 params 都拒绝。
        CapabilityId::UiUpdateState
        | CapabilityId::LogWrite
        | CapabilityId::ClockRead
        | CapabilityId::ThemeSubscribe
        | CapabilityId::SystemMemory => {
            return Err(CapabilityError::InvalidParams {
                capability: capability.name(),
                detail: "该能力不接受 params".to_owned(),
            });
        }
    };
    Ok(Some(params))
}

fn default_params(capability: CapabilityId) -> Result<Option<CapabilityParams>, CapabilityError> {
    Ok(Some(match capability {
        CapabilityId::StorageRead | CapabilityId::StorageWrite => CapabilityParams::Storage {
            keys: Vec::new(),
            max_bytes: 64 * 1024,
        },
        CapabilityId::TimerSchedule => CapabilityParams::Timer {
            max_per_minute: 60,
            max_active: 8,
        },
        CapabilityId::SystemCpu => CapabilityParams::Metrics { sample_rate_hz: 1 },
        _ => return Ok(None),
    }))
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
            _ => false,
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
