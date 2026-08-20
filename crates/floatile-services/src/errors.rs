//! 能力服务错误（与 `wit/floatile-widget.wit` 的 error variant 一一对应）。
//!
//! 这是跨边界稳定错误契约的一部分；runtime 的 WIT adapter 将它们 1:1 映射为
//! 绑定生成的错误类型。自由文本不作为测试或 Agent 判断依据。

use thiserror::Error;

/// host-log 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LogError {
    #[error("log rate exceeded")]
    RateExceeded,
    #[error("message too large")]
    MessageTooLarge,
}

/// host-timer 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TimerError {
    #[error("timer not allowed")]
    NotAllowed,
    #[error("timer budget exceeded")]
    BudgetExceeded,
    #[error("invalid delay")]
    InvalidDelay,
    #[error("invalid timer id")]
    InvalidTimerId,
    #[error("timer unavailable")]
    Unavailable,
}

/// host-storage 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StorageError {
    #[error("storage not allowed")]
    NotAllowed,
    #[error("invalid storage key")]
    InvalidKey,
    #[error("storage quota exceeded")]
    QuotaExceeded,
    #[error("storage unavailable")]
    Unavailable,
    #[error("storage internal error")]
    Internal,
}

/// host-metrics 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MetricsError {
    #[error("metrics not allowed")]
    NotAllowed,
    #[error("metrics rate exceeded")]
    RateExceeded,
    #[error("metrics unavailable")]
    Unavailable,
}

/// host-theme 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ThemeError {
    #[error("theme not allowed")]
    NotAllowed,
    #[error("unknown theme token")]
    UnknownToken,
    #[error("invalid theme subscription")]
    InvalidSubscription,
    #[error("theme unavailable")]
    Unavailable,
}

use floatile_core::DenyReason;

/// Broker 拒绝 → 服务错误。log 无 not-allowed 变体（固有能力实际不会拒绝），
/// 以 rate-exceeded 兜底。
impl From<DenyReason> for LogError {
    fn from(_: DenyReason) -> Self {
        Self::RateExceeded
    }
}

impl From<DenyReason> for TimerError {
    fn from(reason: DenyReason) -> Self {
        match reason {
            DenyReason::QuotaExceeded => Self::BudgetExceeded,
            _ => Self::NotAllowed,
        }
    }
}

impl From<DenyReason> for StorageError {
    fn from(reason: DenyReason) -> Self {
        match reason {
            DenyReason::QuotaExceeded => Self::QuotaExceeded,
            _ => Self::NotAllowed,
        }
    }
}

impl From<DenyReason> for MetricsError {
    fn from(reason: DenyReason) -> Self {
        match reason {
            DenyReason::QuotaExceeded => Self::RateExceeded,
            _ => Self::NotAllowed,
        }
    }
}

impl From<DenyReason> for ThemeError {
    fn from(_: DenyReason) -> Self {
        Self::NotAllowed
    }
}
