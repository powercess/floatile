//! PermissionBroker：唯一宿主能力入口（授权 + 执行 + 脱敏审计）。
//!
//! WIT adapter 只持本门面与 instance context；本门面持有全部能力服务，任何
//! capability 调用都必须先经 `decide` 授权，拒绝与允许都写脱敏审计。固有能力
//! （ui/log/clock）固定当前实例 scope 并合并进实例授权。

use std::any::Any;
use std::future::Future;
use std::time::Duration;

use floatile_core::types::PluginId;
use floatile_core::{
    CapabilityId, CapabilityParams, DenyReason, EffectiveGrant, Grant, InstanceGrant,
    OperationCompletion, OperationCompletionDisposition, OperationFailure, OperationId,
    PermissionDecision, decide,
};

use crate::audit::AuditSink;
use crate::clock::{Clock, ClockSnapshot};
use crate::errors::{LogError, MetricsError, StorageError, ThemeError, TimerError};
use crate::log::{LogLevel, LogService};
use crate::metrics::{MemorySnapshot, MetricsService};
use crate::operation::{
    OperationCancelError, OperationRegistry, OperationServiceError, OperationSubmitError,
    OperationTakeError,
};
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
    plugin: PluginId,
    generation: u64,
    grants: InstanceGrant,
    audit: AuditSink,
    clock: Clock,
    log: LogService,
    timer: TimerService,
    storage: StorageService,
    metrics: MetricsService,
    theme: ThemeService,
    operations: Option<OperationRegistry>,
}

impl Broker {
    /// `instance_grants` 来自 `narrow_instance`（插件授权收窄）；固有能力自动合并。
    pub fn new(
        plugin: PluginId,
        generation: u64,
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
        let log = LogService::new(plugin.0.clone(), instance.0);

        Self {
            plugin,
            generation,
            grants,
            audit,
            clock: Clock,
            log,
            timer,
            storage: StorageService::new(storage_max_bytes),
            metrics: MetricsService::new(metrics_rate),
            theme: ThemeService::new(),
            operations: None,
        }
    }

    /// 绑定本 instance generation 的 Operation registry；身份不一致时拒绝组合。
    pub fn with_operations(
        mut self,
        operations: OperationRegistry,
    ) -> Result<Self, OperationServiceError> {
        let owner = operations.owner();
        if owner.plugin != self.plugin
            || owner.instance != self.grants.instance
            || owner.generation != self.generation
        {
            return Err(OperationServiceError::OwnerMismatch);
        }
        self.operations = Some(operations);
        Ok(self)
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

    fn authorize_existing_grant(
        &self,
        capability: CapabilityId,
        detail: &str,
    ) -> Result<(), DenyReason> {
        let request = self
            .grants
            .caps
            .iter()
            .find(|grant| grant.capability == capability)
            .and_then(|grant| grant.params.as_ref());
        self.authorize(capability, request, detail)
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

    // ---- 宿主托管异步 Operation（PP-M2 spike）----

    /// 在同一个 Broker 调用中完成授权、脱敏审计与有界提交；没有可分离的公开 execute 步骤。
    pub fn submit_operation<T, F>(
        &self,
        capability: CapabilityId,
        request: Option<&CapabilityParams>,
        timeout: Duration,
        audit_detail: &str,
        work: F,
    ) -> Result<OperationId, OperationSubmitError>
    where
        T: Any + Send + 'static,
        F: Future<Output = Result<T, OperationFailure>> + Send + 'static,
    {
        self.authorize(capability, request, audit_detail)
            .map_err(OperationSubmitError::PermissionDenied)?;
        let result = match &self.operations {
            Some(operations) => operations.submit(capability, timeout, work),
            None => Err(OperationSubmitError::Unavailable),
        };
        if let Err(error) = result {
            let reason = match error {
                OperationSubmitError::PermissionDenied(reason) => reason,
                OperationSubmitError::QueueFull | OperationSubmitError::IdExhausted => {
                    DenyReason::QuotaExceeded
                }
                OperationSubmitError::InvalidDeadline => DenyReason::InvalidInput,
                OperationSubmitError::Unavailable => DenyReason::EnvironmentUnavailable,
            };
            self.audit.record(
                capability,
                false,
                Some(reason),
                &format!("operation submit failed={}", error.code()),
            );
        }
        result
    }

    /// 主动取消仍持续经过 capability 授权，并只命中当前 Broker instance 的 active registry。
    pub fn cancel_operation(
        &self,
        capability: CapabilityId,
        id: OperationId,
    ) -> Result<(), OperationCancelError> {
        self.authorize_existing_grant(capability, &format!("operation={} action=cancel", id.get()))
            .map_err(OperationCancelError::PermissionDenied)?;
        self.operations
            .as_ref()
            .ok_or(OperationCancelError::Unavailable)?
            .cancel(capability, id)
    }

    /// capability-specific adapter 一次性领取 typed result；领取时重新授权，支持未来动态撤权。
    pub fn take_operation_result<T: Any + Send + 'static>(
        &self,
        capability: CapabilityId,
        id: OperationId,
    ) -> Result<T, OperationTakeError> {
        self.authorize_existing_grant(
            capability,
            &format!("operation={} action=take-result", id.get()),
        )
        .map_err(OperationTakeError::PermissionDenied)?;
        self.operations
            .as_ref()
            .ok_or(OperationTakeError::Unavailable)?
            .take(capability, id)
    }

    /// runtime 在 completion 成为当前 generation 的 guest event 前记录唯一终态；不记录结果值。
    pub fn audit_operation_completion(
        &self,
        completion: &OperationCompletion,
        disposition: OperationCompletionDisposition,
    ) -> bool {
        let Some(operations) = &self.operations else {
            return false;
        };
        if !completion.is_current_for(operations.owner()) {
            return false;
        }
        self.audit.record(
            completion.capability,
            true,
            None,
            &format!(
                "operation={} terminal={} delivery={}",
                completion.id.get(),
                completion.terminal.code(),
                disposition.code()
            ),
        );
        true
    }

    /// runtime 丢弃旧 generation、过载或关闭后的成功结果，避免宿主内存滞留。
    pub fn discard_operation_result(&self, id: OperationId) -> bool {
        self.operations
            .as_ref()
            .is_some_and(|operations| operations.discard_result(id))
    }

    /// instance stop/delete 时取消全部 active operation；每项仍只产生一个 cancelled 终态。
    pub fn cancel_all_operations(&self) -> usize {
        self.operations
            .as_ref()
            .map_or(0, OperationRegistry::cancel_all)
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.cancel_all_operations();
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
            0,
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
            0,
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
            0,
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
            0,
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
    fn deny_is_audited_with_redacted_record() {
        use crate::audit::AuditListener;
        use std::sync::mpsc;
        use std::time::Duration;

        let (tx, rx) = mpsc::sync_channel(1);
        let listener: AuditListener = Arc::new(move |event| {
            // 通道容量 1，够用且不阻塞；失败即拒绝实例化。
            let _ = tx.send(event.clone());
        });
        let broker = Broker::new(
            PluginId("dev.floatile.clock".into()),
            0,
            test_grants(),
            AuditSink::new("dev.floatile.clock", 7).with_listener(listener),
            sink(),
        );
        // 未授权能力 → deny，且写结构化脱敏审计。
        assert_eq!(
            broker.authorize(CapabilityId::SystemMemory, None, "metrics memory"),
            Err(DenyReason::NotGranted)
        );
        let event = rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_else(|_| panic!("deny 应同步产生审计事件"));
        assert_eq!(event.decision, "deny");
        assert_eq!(event.capability, "system:memory");
        assert_eq!(event.plugin, "dev.floatile.clock");
        assert_eq!(event.instance, 7);
    }
}
