//! PermissionBroker：唯一宿主能力入口（授权 + 执行 + 脱敏审计）。
//!
//! WIT adapter 只持本门面与 instance context；本门面持有全部能力服务，任何
//! capability 调用都必须先经 `decide` 授权，拒绝与允许都写脱敏审计。固有能力
//! （ui/log/clock）固定当前实例 scope 并合并进实例授权。

use floatile_core::types::PluginId;
use floatile_core::{
    CapabilityId, CapabilityParams, DenyReason, EffectiveGrant, Grant, InstanceGrant,
    PermissionDecision, decide,
};

use crate::audit::AuditSink;
use crate::clock::{Clock, ClockSnapshot};
use crate::errors::{LogError, MetricsError, StorageError, ThemeError, TimerError};
use crate::log::{LogLevel, LogService};
use crate::metrics::{MemorySnapshot, MetricsService};
use crate::storage::StorageService;
use crate::theme::ThemeService;
use crate::timer::{TimerService, TimerSink};

/// 固有能力（固定 scope，安装时不提示；合并进实例授权）。
const INHERENT: &[CapabilityId] = &[
    CapabilityId::UiUpdateState,
    CapabilityId::LogWrite,
    CapabilityId::ClockRead,
];

/// 按实例构造的 Broker。
pub struct Broker {
    grants: InstanceGrant,
    audit: AuditSink,
    clock: Clock,
    log: LogService,
    timer: TimerService,
    storage: StorageService,
    metrics: MetricsService,
    theme: ThemeService,
}

impl Broker {
    /// `instance_grants` 来自 `narrow_instance`（插件授权收窄）；固有能力自动合并。
    pub fn new(
        plugin: PluginId,
        instance_grants: InstanceGrant,
        audit: AuditSink,
        timer_sink: TimerSink,
    ) -> Self {
        let mut caps = instance_grants.caps.clone();
        for inherent in INHERENT {
            if !caps.iter().any(|g| g.capability == *inherent) {
                caps.push(Grant {
                    capability: *inherent,
                    params: None,
                    effective: EffectiveGrant::DerivedFromInstall,
                });
            }
        }
        let grants = InstanceGrant {
            instance: instance_grants.instance,
            caps,
        };

        let mut storage_max_bytes = 64 * 1024;
        let mut timer_quota = None;
        let mut metrics_rate = 1;
        for grant in &grants.caps {
            match (&grant.capability, &grant.params) {
                (CapabilityId::StorageWrite, Some(CapabilityParams::Storage { max_bytes, .. })) => {
                    storage_max_bytes = *max_bytes as usize;
                }
                (
                    CapabilityId::TimerSchedule,
                    Some(CapabilityParams::Timer {
                        max_per_minute,
                        max_active,
                    }),
                ) => {
                    timer_quota = Some((*max_per_minute, *max_active));
                }
                (CapabilityId::SystemCpu, Some(CapabilityParams::Metrics { sample_rate_hz })) => {
                    metrics_rate = *sample_rate_hz;
                }
                _ => {}
            }
        }

        let mut timer = TimerService::new(timer_sink);
        if let Some((max_per_minute, max_active)) = timer_quota {
            timer.set_quota(max_per_minute, max_active);
        }
        let instance = grants.instance;

        Self {
            grants,
            audit,
            clock: Clock,
            log: LogService::new(plugin.0.clone(), instance.0),
            timer,
            storage: StorageService::new(storage_max_bytes),
            metrics: MetricsService::new(metrics_rate),
            theme: ThemeService::new(),
        }
    }

    /// 纯授权检查（用于 UI State 等由 runtime 执行、Broker 只裁决的能力）。
    pub fn authorize(
        &self,
        capability: CapabilityId,
        request: Option<&CapabilityParams>,
        detail: &str,
    ) -> Result<(), DenyReason> {
        let grant = self.grants.caps.iter().find(|g| g.capability == capability);
        match decide(grant, request) {
            PermissionDecision::Allowed => {
                self.audit.record(capability, true, None, detail);
                Ok(())
            }
            PermissionDecision::Denied { reason } => {
                self.audit.record(capability, false, Some(reason), detail);
                Err(reason)
            }
        }
    }

    // ---- 固有能力 ----

    pub fn clock_now(&self) -> ClockSnapshot {
        // 固有能力仍经 Broker 授权与审计（当前实例 scope；固有 grants 恒定存在）。
        let _ = self.authorize(CapabilityId::ClockRead, None, "clock");
        self.clock.now()
    }

    pub fn log(&mut self, level: LogLevel, message: &str) -> Result<(), LogError> {
        self.authorize(CapabilityId::LogWrite, None, &redact_size(message))?;
        self.log.log(level, message)
    }

    // ---- 声明能力 ----

    pub fn timer_schedule(&mut self, delay_ms: u64) -> Result<u32, TimerError> {
        self.authorize(
            CapabilityId::TimerSchedule,
            Some(&CapabilityParams::Timer {
                max_per_minute: 1,
                max_active: 1,
            }),
            "schedule timer",
        )?;
        self.timer.schedule(delay_ms)
    }

    pub fn timer_cancel(&mut self, id: u32) -> Result<(), TimerError> {
        self.authorize(
            CapabilityId::TimerSchedule,
            Some(&CapabilityParams::Timer {
                max_per_minute: 1,
                max_active: 1,
            }),
            "cancel timer",
        )?;
        self.timer.cancel(id)
    }

    /// 计时器事件已由实例处理，释放槽位（不经过能力授权，仅簿记）。
    pub fn timer_complete(&mut self, id: u32) {
        self.timer.complete(id);
    }

    pub fn storage_get(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.authorize(
            CapabilityId::StorageRead,
            Some(&CapabilityParams::Storage {
                keys: vec![key.to_owned()],
                max_bytes: 0,
            }),
            &format!("storage get key={key}"),
        )?;
        self.storage.get(key)
    }

    pub fn storage_set(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        self.authorize(
            CapabilityId::StorageWrite,
            Some(&CapabilityParams::Storage {
                keys: vec![key.to_owned()],
                max_bytes: value.len() as u64,
            }),
            &format!("storage set key={key} len={}", value.len()),
        )?;
        self.storage.set(key, value)
    }

    pub fn storage_delete(&mut self, key: &str) -> Result<(), StorageError> {
        self.authorize(
            CapabilityId::StorageWrite,
            Some(&CapabilityParams::Storage {
                keys: vec![key.to_owned()],
                max_bytes: 0,
            }),
            &format!("storage delete key={key}"),
        )?;
        self.storage.delete(key)
    }

    pub fn metrics_cpu_percent(&mut self) -> Result<f64, MetricsError> {
        self.authorize(
            CapabilityId::SystemCpu,
            Some(&CapabilityParams::Metrics { sample_rate_hz: 1 }),
            "metrics cpu-percent",
        )?;
        self.metrics.cpu_percent()
    }

    pub fn metrics_memory(&self) -> Result<MemorySnapshot, MetricsError> {
        self.authorize(CapabilityId::SystemMemory, None, "metrics memory")?;
        self.metrics.memory()
    }

    pub fn theme_get_token(&self, name: &str) -> Result<Option<String>, ThemeError> {
        self.authorize(CapabilityId::ThemeSubscribe, None, "theme get-token")?;
        self.theme.get_token(name)
    }

    pub fn theme_subscribe(&mut self) -> Result<u32, ThemeError> {
        self.authorize(CapabilityId::ThemeSubscribe, None, "theme subscribe")?;
        self.theme.subscribe()
    }

    pub fn theme_unsubscribe(&mut self, id: u32) -> Result<(), ThemeError> {
        self.authorize(CapabilityId::ThemeSubscribe, None, "theme unsubscribe")?;
        self.theme.unsubscribe(id)
    }
}

/// 审计脱敏：消息只记长度，不落内容。
fn redact_size(value: &str) -> String {
    format!("message len={}", value.chars().count())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use floatile_core::{CapabilityParams, Grants, InstanceId, narrow_instance};
    use std::sync::{Arc, Mutex};

    fn test_grants() -> InstanceGrant {
        let plugin = Grants {
            plugin: PluginId("dev.floatile.clock".into()),
            trust: floatile_core::TrustLevel::Dev,
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
                    capability: CapabilityId::StorageRead,
                    params: Some(CapabilityParams::Storage {
                        keys: vec!["settings".into()],
                        max_bytes: 1024,
                    }),
                    effective: EffectiveGrant::DerivedFromInstall,
                },
                Grant {
                    capability: CapabilityId::StorageWrite,
                    params: Some(CapabilityParams::Storage {
                        keys: vec!["settings".into()],
                        max_bytes: 1024,
                    }),
                    effective: EffectiveGrant::DerivedFromInstall,
                },
            ],
        };
        narrow_instance(
            &plugin,
            InstanceId(7),
            vec![
                Grant {
                    capability: CapabilityId::TimerSchedule,
                    params: Some(CapabilityParams::Timer {
                        max_per_minute: 30,
                        max_active: 1,
                    }),
                    effective: EffectiveGrant::DerivedFromInstall,
                },
                Grant {
                    capability: CapabilityId::StorageRead,
                    params: Some(CapabilityParams::Storage {
                        keys: vec!["settings".into()],
                        max_bytes: 1024,
                    }),
                    effective: EffectiveGrant::DerivedFromInstall,
                },
                Grant {
                    capability: CapabilityId::StorageWrite,
                    params: Some(CapabilityParams::Storage {
                        keys: vec!["settings".into()],
                        max_bytes: 512,
                    }),
                    effective: EffectiveGrant::DerivedFromInstall,
                },
            ],
        )
        .unwrap()
    }

    fn sink() -> TimerSink {
        let delivered = Arc::new(Mutex::new(Vec::new()));
        Arc::new(move |id| {
            delivered.lock().unwrap().push(id);
        })
    }

    #[test]
    fn inherent_caps_allowed_without_grant() {
        let broker = Broker::new(
            PluginId("dev.floatile.clock".into()),
            test_grants(),
            AuditSink::new("dev.floatile.clock", 7),
            sink(),
        );
        assert!(
            broker
                .authorize(CapabilityId::ClockRead, None, "clock")
                .is_ok()
        );
        assert!(
            broker
                .authorize(CapabilityId::UiUpdateState, None, "ui")
                .is_ok()
        );
        assert!(
            broker
                .authorize(CapabilityId::LogWrite, None, "log")
                .is_ok()
        );
    }

    #[test]
    fn unlisted_capability_denied_and_audited() {
        let broker = Broker::new(
            PluginId("dev.floatile.clock".into()),
            test_grants(),
            AuditSink::new("dev.floatile.clock", 7),
            sink(),
        );
        assert_eq!(
            broker.authorize(CapabilityId::SystemMemory, None, "metrics"),
            Err(DenyReason::NotGranted)
        );
    }

    #[test]
    fn storage_respects_scope_and_quota() {
        let mut broker = Broker::new(
            PluginId("dev.floatile.clock".into()),
            test_grants(),
            AuditSink::new("dev.floatile.clock", 7),
            sink(),
        );
        // 允许的键。
        broker.storage_set("settings", "v").unwrap();
        assert_eq!(
            broker.storage_get("settings").unwrap().as_deref(),
            Some("v")
        );
        // 键不在授权范围 → NotAllowed。
        assert!(matches!(
            broker.storage_set("other", "v"),
            Err(StorageError::NotAllowed)
        ));
        // 超配额 → QuotaExceeded。
        let big = "x".repeat(600);
        assert!(matches!(
            broker.storage_set("settings", &big),
            Err(StorageError::QuotaExceeded)
        ));
    }

    #[tokio::test]
    async fn timer_denied_without_quota_budget() {
        // 实例授权 maxActive=1：连续 schedule 两个未到期计时器第二个应超限。
        let mut broker = Broker::new(
            PluginId("dev.floatile.clock".into()),
            test_grants(),
            AuditSink::new("dev.floatile.clock", 7),
            sink(),
        );
        let first = broker.timer_schedule(10_000).unwrap();
        let second = broker.timer_schedule(10_000);
        assert!(matches!(second, Err(TimerError::BudgetExceeded)));
        broker.timer_cancel(first).unwrap();
        // 释放后可以再调度。
        assert!(broker.timer_schedule(10_000).is_ok());
    }

    #[test]
    fn deny_is_audited_via_tracing() {
        use tracing::field::{Field, Visit};
        use tracing::span;
        use tracing::{Event, Metadata, Subscriber};

        /// 结构化捕获审计事件：直接用最小 subscriber 记录字段，不依赖
        /// tracing-subscriber fmt 的渲染/writer 细节（避免平台相关的脆弱性）。
        #[derive(Default)]
        struct AuditRecord {
            capability: String,
            decision: String,
            plugin: String,
            reason: String,
            instance: u64,
        }

        #[derive(Default)]
        struct Capture(Mutex<Option<AuditRecord>>);

        impl Visit for AuditRecord {
            fn record_str(&mut self, field: &Field, value: &str) {
                match field.name() {
                    "capability" => self.capability = value.to_owned(),
                    "decision" => self.decision = value.to_owned(),
                    "plugin_id" => self.plugin = value.to_owned(),
                    _ => {}
                }
            }
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                // `%`（Display）与 `?`（Debug）字段都经 record_debug 到达；
                // Display 包装值的 Debug 输出即其显示文本。
                match field.name() {
                    "plugin_id" => self.plugin = format!("{value:?}"),
                    "reason" => self.reason = format!("{value:?}"),
                    _ => {}
                }
            }
            fn record_u64(&mut self, field: &Field, value: u64) {
                if field.name() == "instance_id" {
                    self.instance = value;
                }
            }
        }

        impl Subscriber for Capture {
            fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
                span::Id::from_u64(1)
            }
            fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}
            fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}
            fn event(&self, event: &Event<'_>) {
                if event.metadata().target() != "floatile::audit" {
                    return;
                }
                let mut record = AuditRecord::default();
                event.record(&mut record);
                let Ok(mut captured) = self.0.lock() else {
                    return;
                };
                *captured = Some(record);
            }
            fn enter(&self, _span: &span::Id) {}
            fn exit(&self, _span: &span::Id) {}
        }

        let capture = Arc::new(Capture::default());
        let guard = tracing::dispatcher::set_default(&tracing::Dispatch::new(capture.clone()));
        let broker = Broker::new(
            PluginId("dev.floatile.clock".into()),
            test_grants(),
            AuditSink::new("dev.floatile.clock", 7),
            sink(),
        );
        // 未授权能力 → deny，且写脱敏审计（结构化字段）。
        assert_eq!(
            broker.authorize(CapabilityId::SystemMemory, None, "metrics memory"),
            Err(DenyReason::NotGranted)
        );
        drop(guard);

        let captured = capture
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match captured.as_ref() {
            Some(record) => {
                assert_eq!(record.decision, "deny");
                assert_eq!(record.capability, "system:memory");
                assert_eq!(record.plugin, "dev.floatile.clock");
                assert_eq!(record.instance, 7);
            }
            None => panic!("应捕获到 floatile::audit 审计事件"),
        }
    }
}
