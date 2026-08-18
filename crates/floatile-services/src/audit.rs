//! 脱敏审计（target `floatile::audit`）。
//!
//! 原则：不记录 secret、完整 State/Storage value 或未脱敏 WIT 参数；值只以长度
//! 或哈希摘要出现。审计日志仅供宿主查看，插件读不到。

use floatile_core::{CapabilityId, DenyReason};

/// 审计目标标识。
#[derive(Debug, Clone)]
pub struct AuditSink {
    plugin: String,
    instance: u64,
}

impl AuditSink {
    pub fn new(plugin: impl Into<String>, instance: u64) -> Self {
        Self {
            plugin: plugin.into(),
            instance,
        }
    }

    /// 记录一次能力决策。`detail` 必须已脱敏（长度/哈希，不落值）。
    pub fn record(
        &self,
        capability: CapabilityId,
        allowed: bool,
        reason: Option<DenyReason>,
        detail: &str,
    ) {
        tracing::event!(
            target: "floatile::audit",
            tracing::Level::INFO,
            plugin_id = %self.plugin,
            instance_id = self.instance,
            capability = capability.name(),
            decision = if allowed { "allow" } else { "deny" },
            reason = ?reason,
            detail = %detail,
        );
    }
}

/// FNV-1a 64 位哈希：用于记录值摘要，不用于加密。
pub fn fnv1a(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
