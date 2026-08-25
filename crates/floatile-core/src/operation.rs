//! 宿主托管异步 Operation 的纯领域模型（PP-M2）。
//!
//! Operation 的真实执行、deadline、取消和结果暂存在 `floatile-services`；本模块只定义
//! 跨层共享的身份、generation 隔离与稳定终态，不做 I/O 或调度。

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::{CapabilityId, InstanceId, PluginId};

/// 进程内唯一的 Operation 标识；`0` 保留为无效值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(NonZeroU64);

impl OperationId {
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Operation 的不可伪造宿主归属；generation 是实例每次启动/重启的隔离边界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationOwner {
    pub plugin: PluginId,
    pub instance: InstanceId,
    pub generation: u64,
}

impl OperationOwner {
    pub fn new(plugin: PluginId, instance: InstanceId, generation: u64) -> Self {
        Self {
            plugin,
            instance,
            generation,
        }
    }
}

/// Operation 失败终态；字符串由 `code` 提供，供日志、WIT adapter 和 contract tests 使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationFailure {
    Timeout,
    Cancelled,
    Unavailable,
    Internal,
    ResultDropped,
}

impl OperationFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
            Self::ResultDropped => "result-dropped",
        }
    }
}

/// 唯一终态；成功结果由 capability-specific adapter 通过 typed `take` 一次性领取。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationTerminal {
    Succeeded,
    Failed(OperationFailure),
}

impl OperationTerminal {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed(failure) => failure.code(),
        }
    }
}

/// 进入 runtime completion lane 的固定元数据；不携带 capability payload 或 secret。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationCompletion {
    pub id: OperationId,
    pub owner: OperationOwner,
    pub capability: CapabilityId,
    pub terminal: OperationTerminal,
}

impl OperationCompletion {
    pub fn is_current_for(&self, owner: &OperationOwner) -> bool {
        self.owner == *owner
    }
}

/// runtime 对完成信号的宿主侧处置；不属于当前 WIT 合约。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationCompletionDisposition {
    Delivered,
    StaleGeneration,
    QueueFull,
    ActorClosed,
}

impl OperationCompletionDisposition {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::StaleGeneration => "stale-generation",
            Self::QueueFull => "queue-full",
            Self::ActorClosed => "actor-closed",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn operation_id_reserves_zero() {
        assert_eq!(OperationId::new(0), None);
        assert_eq!(OperationId::new(7).map(OperationId::get), Some(7));
        assert!(serde_json::from_str::<OperationId>("0").is_err());
        assert_eq!(serde_json::from_str::<OperationId>("7").unwrap().get(), 7);
    }

    #[test]
    fn completion_requires_exact_owner_generation() {
        let owner = OperationOwner::new(PluginId("dev.floatile.test".into()), InstanceId(2), 4);
        let completion = OperationCompletion {
            id: OperationId::new(1).unwrap(),
            owner: owner.clone(),
            capability: CapabilityId::TimerSchedule,
            terminal: OperationTerminal::Succeeded,
        };
        assert!(completion.is_current_for(&owner));
        assert!(!completion.is_current_for(&OperationOwner::new(
            owner.plugin.clone(),
            owner.instance,
            owner.generation + 1,
        )));
    }
}
