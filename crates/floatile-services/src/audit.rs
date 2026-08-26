//! 脱敏审计（target `floatile::audit`）。
//!
//! 原则：不记录 secret、完整 State/Storage value 或未脱敏 WIT 参数；值只以长度
//! 或哈希摘要出现。审计日志仅供宿主查看，插件读不到。
//!
//! `record` 同时：(1) 写结构化 `tracing` 事件（target `floatile::audit`）供
//! 宿主日志消费；(2) 若注入了 `listener`（`with_listener`），同步回调结构化
//! 记录——测试用它做确定性断言，未来的 SQLite 审计持久化也复用同一结构。

use std::sync::Arc;

use floatile_core::{CapabilityId, DenyReason};

/// 一次能力决策的结构化、已脱敏审计记录。
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub plugin: String,
    pub instance: u64,
    pub capability: String,
    pub decision: String,
    pub reason: Option<String>,
    pub detail: String,
}

/// 同步审计接收器（测试断言或未来的持久化后端）。
pub type AuditListener = Arc<dyn Fn(&AuditEvent) + Send + Sync>;

/// 审计目标标识。
#[derive(Clone)]
pub struct AuditSink {
    plugin: String,
    instance: u64,
    listener: Option<AuditListener>,
}

impl std::fmt::Debug for AuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditSink")
            .field("plugin", &self.plugin)
            .field("instance", &self.instance)
            .field("listener", &self.listener.is_some())
            .finish()
    }
}

impl AuditSink {
    pub fn new(plugin: impl Into<String>, instance: u64) -> Self {
        Self {
            plugin: plugin.into(),
            instance,
            listener: None,
        }
    }

    /// 注入同步审计接收器（测试断言或未来的持久化后端）。不替换 tracing 输出。
    pub fn with_listener(mut self, listener: AuditListener) -> Self {
        self.listener = Some(listener);
        self
    }

    /// 记录一次能力决策。`detail` 必须已脱敏（长度/哈希，不落值）。
    pub fn record(
        &self,
        capability: CapabilityId,
        allowed: bool,
        reason: Option<DenyReason>,
        detail: &str,
    ) {
        let event = AuditEvent {
            plugin: self.plugin.clone(),
            instance: self.instance,
            capability: capability.name().to_owned(),
            decision: if allowed { "allow" } else { "deny" }.to_owned(),
            reason: reason.map(|r| format!("{r:?}")),
            detail: detail.to_owned(),
        };
        if let Some(listener) = &self.listener {
            listener(&event);
        }
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
