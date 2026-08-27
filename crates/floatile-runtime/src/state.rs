//! 实例宿主状态与 WIT host adapter 实现。
//!
//! `InstanceHostState` 是 Store 的数据类型，实现全部宿主 import trait。
//! 所有能力调用经 `Broker` 授权并审计；`host-ui` 的 State Patch 在这里原子
//! 应用并把有界 `UiUpdate` 投递到 UI 通道。

use std::time::Instant;

use floatile_core::OperationId;
use floatile_core::types::InstanceId;
use floatile_plugin_api::floatile::widget::{
    host_clock, host_http, host_log, host_metrics, host_operation, host_storage, host_theme,
    host_timer, host_ui,
};
use floatile_services::broker::Broker;
use floatile_services::errors::{LogError, MetricsError, StorageError, ThemeError, TimerError};
use floatile_services::{
    LogLevel, MemorySnapshot, OperationCancelError, OperationSubmitError, OperationTakeError,
};
use floatile_ui_schema::schema::JsonSchema;
use floatile_ui_schema::{
    MAX_PATCH_BYTES, MAX_STATE_BYTES, MAX_UPDATE_RATE_PER_SEC, merge_patch, validate_value,
};
use serde_json::Value;
use tokio::sync::mpsc;
use wasmtime::StoreLimits;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

/// 已原子应用并验证过的 State 快照（投递给 shell/UI）。
#[derive(Debug, Clone)]
pub struct UiUpdate {
    pub instance: InstanceId,
    pub seq: u64,
    pub state: Value,
}

/// Store 宿主状态。
pub struct InstanceHostState {
    pub broker: Broker,
    pub limits: StoreLimits,
    ui: UiState,
    instance: InstanceId,
    /// 空 WASI 上下文：满足 wasm32-wasip2 工具链 std 注入的 import，但不授予任何
    /// 文件/网络/环境能力（零 ambient capability，见 p0-design §9）。
    wasi_ctx: WasiCtx,
    wasi_table: ResourceTable,
}

impl WasiView for InstanceHostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.wasi_table,
        }
    }
}

struct UiState {
    state: Value,
    schema: JsonSchema,
    tx: mpsc::Sender<UiUpdate>,
    seq: u64,
    window_start: Instant,
    window_count: u32,
}

impl InstanceHostState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instance: InstanceId,
        broker: Broker,
        max_memory: usize,
        initial_state: Value,
        state_schema: JsonSchema,
        ui_tx: mpsc::Sender<UiUpdate>,
    ) -> Self {
        Self {
            broker,
            limits: wasmtime::StoreLimitsBuilder::new()
                .memory_size(max_memory)
                .build(),
            ui: UiState {
                state: initial_state,
                schema: state_schema,
                tx: ui_tx,
                seq: 0,
                window_start: Instant::now(),
                window_count: 0,
            },
            instance,
            wasi_ctx: WasiCtxBuilder::new().build(),
            wasi_table: ResourceTable::new(),
        }
    }
}

// ---- host-ui ----

impl host_ui::Host for InstanceHostState {
    async fn update_state(&mut self, patch_json: String) -> Result<(), host_ui::UiError> {
        use host_ui::UiError;
        // 固有能力：仍经 Broker 裁决与审计（当前实例 scope）。
        self.broker
            .authorize(
                floatile_core::CapabilityId::UiUpdateState,
                None,
                &format!("ui update-state patch={}B", patch_json.len()),
            )
            .map_err(|_| UiError::NotAllowed)?;

        // 频率限制。
        let now = Instant::now();
        if now.duration_since(self.ui.window_start) >= std::time::Duration::from_secs(1) {
            self.ui.window_start = now;
            self.ui.window_count = 0;
        }
        if self.ui.window_count >= MAX_UPDATE_RATE_PER_SEC {
            return Err(UiError::UpdateRateExceeded);
        }
        self.ui.window_count += 1;

        // 大小与解析。
        if patch_json.len() > MAX_PATCH_BYTES {
            return Err(UiError::PatchTooLarge);
        }
        let patch: Value = serde_json::from_str(&patch_json).map_err(|_| UiError::InvalidJson)?;
        if !patch.is_object() {
            return Err(UiError::InvalidJson);
        }

        // 在副本上原子应用并完整校验；失败旧 State 不变。
        let mut next = self.ui.state.clone();
        merge_patch(&mut next, &patch);
        validate_value(&self.ui.schema, &next, "$", 0)
            .map_err(|e| UiError::SchemaMismatch(e.to_string()))?;
        let bytes = serde_json::to_vec(&next).map_err(|_| UiError::Internal)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(UiError::StateTooLarge);
        }

        self.ui.seq += 1;
        self.ui.state = next.clone();
        self.ui
            .tx
            .try_send(UiUpdate {
                instance: self.instance,
                seq: self.ui.seq,
                state: next,
            })
            .map_err(|_| UiError::QueueFull)
    }
}

// ---- host-log ----

impl host_log::Host for InstanceHostState {
    async fn log(
        &mut self,
        level: host_log::LogLevel,
        message: String,
    ) -> Result<(), host_log::LogError> {
        let level = match level {
            host_log::LogLevel::Debug => LogLevel::Debug,
            host_log::LogLevel::Info => LogLevel::Info,
            host_log::LogLevel::Warn => LogLevel::Warn,
            host_log::LogLevel::Error => LogLevel::Error,
        };
        self.broker
            .log(level, &message)
            .map_err(|e: LogError| match e {
                LogError::RateExceeded => host_log::LogError::RateExceeded,
                LogError::MessageTooLarge => host_log::LogError::MessageTooLarge,
            })
    }
}

// ---- host-clock ----

impl host_clock::Host for InstanceHostState {
    async fn now(&mut self) -> host_clock::WallTime {
        let snap = self.broker.clock_now();
        host_clock::WallTime {
            unix_millis: snap.unix_millis,
            utc_offset_minutes: snap.utc_offset_minutes,
        }
    }
}

// ---- host-timer ----

impl host_timer::Host for InstanceHostState {
    async fn schedule(&mut self, delay_ms: u64) -> Result<u32, host_timer::TimerError> {
        self.broker
            .timer_schedule(delay_ms)
            .map_err(|e: TimerError| match e {
                TimerError::NotAllowed => host_timer::TimerError::NotAllowed,
                TimerError::BudgetExceeded => host_timer::TimerError::BudgetExceeded,
                TimerError::InvalidDelay => host_timer::TimerError::InvalidDelay,
                TimerError::InvalidTimerId => host_timer::TimerError::InvalidTimerId,
                TimerError::Unavailable => host_timer::TimerError::Unavailable,
            })
    }

    async fn cancel(&mut self, timer_id: u32) -> Result<(), host_timer::TimerError> {
        self.broker
            .timer_cancel(timer_id)
            .map_err(|e: TimerError| match e {
                TimerError::NotAllowed => host_timer::TimerError::NotAllowed,
                TimerError::BudgetExceeded => host_timer::TimerError::BudgetExceeded,
                TimerError::InvalidDelay => host_timer::TimerError::InvalidDelay,
                TimerError::InvalidTimerId => host_timer::TimerError::InvalidTimerId,
                TimerError::Unavailable => host_timer::TimerError::Unavailable,
            })
    }
}

// ---- host-storage ----

fn map_storage(e: StorageError) -> host_storage::StorageError {
    use host_storage::StorageError as Target;
    match e {
        StorageError::NotAllowed => Target::NotAllowed,
        StorageError::InvalidKey => Target::InvalidKey,
        StorageError::QuotaExceeded => Target::QuotaExceeded,
        StorageError::Unavailable => Target::Unavailable,
        StorageError::Internal => Target::Internal,
    }
}

impl host_storage::Host for InstanceHostState {
    async fn get(&mut self, key: String) -> Result<Option<String>, host_storage::StorageError> {
        self.broker.storage_get(&key).map_err(map_storage)
    }

    async fn set(&mut self, key: String, value: String) -> Result<(), host_storage::StorageError> {
        self.broker.storage_set(&key, &value).map_err(map_storage)
    }

    async fn delete(&mut self, key: String) -> Result<(), host_storage::StorageError> {
        self.broker.storage_delete(&key).map_err(map_storage)
    }

    async fn submit_get(
        &mut self,
        key: String,
        timeout_ms: u64,
    ) -> Result<u64, host_operation::OperationError> {
        self.broker
            .submit_storage_get(&key, std::time::Duration::from_millis(timeout_ms))
            .map(OperationId::get)
            .map_err(map_operation_submit)
    }

    async fn take_get_result(
        &mut self,
        id: u64,
    ) -> Result<Option<String>, host_operation::OperationError> {
        let id = OperationId::new(id).ok_or(host_operation::OperationError::InvalidOperationId)?;
        self.broker
            .take_storage_get_result(id)
            .map_err(map_operation_take)
    }
}

impl host_http::Host for InstanceHostState {
    async fn submit(
        &mut self,
        template_id: String,
        _connection_id: u64,
        _query: Vec<host_http::QueryParam>,
    ) -> Result<u64, host_operation::OperationError> {
        self.broker
            .submit_https_unconfigured(&template_id)
            .map(OperationId::get)
            .map_err(map_operation_submit)
    }

    async fn take_result(
        &mut self,
        id: u64,
    ) -> Result<host_http::HttpResponse, host_operation::OperationError> {
        let id = OperationId::new(id).ok_or(host_operation::OperationError::InvalidOperationId)?;
        self.broker
            .take_https_result::<(u16, Vec<u8>)>(id)
            .map(|(status, body)| host_http::HttpResponse { status, body })
            .map_err(map_operation_take)
    }
}

fn map_operation_submit(error: OperationSubmitError) -> host_operation::OperationError {
    match error {
        OperationSubmitError::PermissionDenied(_) => host_operation::OperationError::NotAllowed,
        OperationSubmitError::InvalidInput => host_operation::OperationError::InvalidInput,
        OperationSubmitError::QueueFull => host_operation::OperationError::QueueFull,
        OperationSubmitError::InvalidDeadline => host_operation::OperationError::InvalidDeadline,
        OperationSubmitError::Unavailable => host_operation::OperationError::Unavailable,
        OperationSubmitError::IdExhausted => host_operation::OperationError::Internal,
    }
}

fn map_operation_take(error: OperationTakeError) -> host_operation::OperationError {
    match error {
        OperationTakeError::NotAvailable => host_operation::OperationError::ResultNotAvailable,
        OperationTakeError::CapabilityMismatch | OperationTakeError::TypeMismatch => {
            host_operation::OperationError::CapabilityMismatch
        }
        OperationTakeError::PermissionDenied(_) => host_operation::OperationError::NotAllowed,
        OperationTakeError::Unavailable => host_operation::OperationError::Unavailable,
    }
}

impl host_operation::Host for InstanceHostState {
    async fn cancel(&mut self, id: u64) -> Result<(), host_operation::OperationError> {
        let id = OperationId::new(id).ok_or(host_operation::OperationError::InvalidOperationId)?;
        self.broker
            .cancel_operation_by_id(id)
            .map_err(|error| match error {
                OperationCancelError::NotActive => host_operation::OperationError::NotActive,
                OperationCancelError::PermissionDenied(_) => {
                    host_operation::OperationError::NotAllowed
                }
                OperationCancelError::Unavailable => host_operation::OperationError::Unavailable,
            })
    }
}

// ---- host-metrics ----

fn map_metrics(e: MetricsError) -> host_metrics::MetricsError {
    use host_metrics::MetricsError as Target;
    match e {
        MetricsError::NotAllowed => Target::NotAllowed,
        MetricsError::RateExceeded => Target::RateExceeded,
        MetricsError::Unavailable => Target::Unavailable,
    }
}

impl host_metrics::Host for InstanceHostState {
    async fn cpu_percent(&mut self) -> Result<f64, host_metrics::MetricsError> {
        self.broker.metrics_cpu_percent().map_err(map_metrics)
    }

    async fn memory(&mut self) -> Result<host_metrics::MemorySnapshot, host_metrics::MetricsError> {
        let MemorySnapshot {
            rss_kib,
            virtual_kib,
        } = self.broker.metrics_memory().map_err(map_metrics)?;
        Ok(host_metrics::MemorySnapshot {
            rss_kib,
            virtual_kib,
        })
    }
}

// ---- host-theme ----

fn map_theme(e: ThemeError) -> host_theme::ThemeError {
    use host_theme::ThemeError as Target;
    match e {
        ThemeError::NotAllowed => Target::NotAllowed,
        ThemeError::UnknownToken => Target::UnknownToken,
        ThemeError::InvalidSubscription => Target::InvalidSubscription,
        ThemeError::Unavailable => Target::Unavailable,
    }
}

impl host_theme::Host for InstanceHostState {
    async fn get_token(&mut self, name: String) -> Result<Option<String>, host_theme::ThemeError> {
        self.broker.theme_get_token(&name).map_err(map_theme)
    }

    async fn subscribe(&mut self) -> Result<u32, host_theme::ThemeError> {
        self.broker.theme_subscribe().map_err(map_theme)
    }

    async fn unsubscribe(&mut self, id: u32) -> Result<(), host_theme::ThemeError> {
        self.broker.theme_unsubscribe(id).map_err(map_theme)
    }
}

impl InstanceHostState {
    /// 计时器事件已投递并由实例处理，释放槽位。
    pub fn timer_complete(&mut self, id: u32) {
        self.broker.timer_complete(id);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use floatile_core::capability::{Grants, TrustLevel, narrow_instance};
    use floatile_core::types::PluginId;
    use floatile_services::{AuditSink, TimerSink};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn clock_schema() -> JsonSchema {
        JsonSchema::Object {
            required: vec![],
            properties: BTreeMap::from([(
                "time".into(),
                JsonSchema::String {
                    max_length: Some(32),
                },
            )]),
            additional_properties: false,
        }
    }

    fn test_state() -> (InstanceHostState, mpsc::Receiver<UiUpdate>) {
        let plugin = Grants {
            plugin: PluginId("t".into()),
            trust: TrustLevel::Dev,
            caps: vec![],
        };
        let grants = narrow_instance(&plugin, InstanceId(1), vec![]).unwrap();
        let broker = Broker::new(
            PluginId("t".into()),
            0,
            grants,
            AuditSink::new("t", 1),
            Arc::new(|_| {}) as TimerSink,
        );
        let (tx, rx) = mpsc::channel(8);
        let state = InstanceHostState::new(
            InstanceId(1),
            broker,
            16 * 1024 * 1024,
            json!({}),
            clock_schema(),
            tx,
        );
        (state, rx)
    }

    #[tokio::test]
    async fn valid_patch_applies_and_pushes_update() {
        let (mut state, mut rx) = test_state();
        let result = host_ui::Host::update_state(&mut state, r#"{"time":"12:34:56"}"#.into()).await;
        assert!(result.is_ok());
        let update = rx.try_recv().expect("应有 UiUpdate");
        assert_eq!(update.state["time"], json!("12:34:56"));
        // 原子应用：重复 patch 叠加。
        let result = host_ui::Host::update_state(&mut state, r#"{"time":"23:00:00"}"#.into()).await;
        assert!(result.is_ok());
        let update = rx.try_recv().unwrap();
        assert_eq!(update.state["time"], json!("23:00:00"));
    }

    #[tokio::test]
    async fn invalid_json_rejected_without_mutation() {
        let (mut state, _rx) = test_state();
        let result = host_ui::Host::update_state(&mut state, "not json".into()).await;
        assert!(matches!(result, Err(host_ui::UiError::InvalidJson)));
        // 非 object patch 也拒绝。
        let result = host_ui::Host::update_state(&mut state, "42".into()).await;
        assert!(matches!(result, Err(host_ui::UiError::InvalidJson)));
    }

    #[tokio::test]
    async fn schema_mismatch_keeps_old_state() {
        let (mut state, _rx) = test_state();
        // time 是 string；patch 改成 number → schema-mismatch。
        let result = host_ui::Host::update_state(&mut state, r#"{"time":5}"#.into()).await;
        assert!(matches!(result, Err(host_ui::UiError::SchemaMismatch(_))));
        // 未知字段 → additionalProperties=false 拒绝。
        let result = host_ui::Host::update_state(&mut state, r#"{"nope":1}"#.into()).await;
        assert!(matches!(result, Err(host_ui::UiError::SchemaMismatch(_))));
    }

    #[tokio::test]
    async fn patch_too_large_rejected() {
        let (mut state, _rx) = test_state();
        let big = format!(r#"{{"time":"{}"}}"#, "x".repeat(MAX_PATCH_BYTES));
        let result = host_ui::Host::update_state(&mut state, big).await;
        assert!(matches!(result, Err(host_ui::UiError::PatchTooLarge)));
    }
}
