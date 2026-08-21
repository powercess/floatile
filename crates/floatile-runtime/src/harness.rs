//! 作者级测试工具（`floatile test` / 集成测试）。
//!
//! `WidgetHarness` 是 `floatile-runtime` 之上的一层叙述性测试 API：把
//! `WidgetManager`/`WidgetHandle` 的 spawn/start/event/state/audit 收进一个可链式
//! 拼接的 builder，让插件作者与 AI Agent 用 `grant/start/emit/wait_for_state/
//! advance_time/audit` 描述无桌面逻辑测试。默认不启动窗口、不碰真实 SQLite/网络/
//! 文件/系统指标；所有宿主能力仍走生产 `PermissionBroker`（deny-by-default），
//! 拒绝/配额/脱敏审计照常生效——不绕过 Broker 语义。
//!
//! 说明：`advance_time` 按真实时钟推进（guest 计时器经 Broker 落到 tokio 定时器），
//! 在 P0 用真实短时延驱动；后续若引入确定性虚拟时钟再收紧。

use std::sync::Arc;
use std::time::Duration;

use floatile_core::capability::{
    CapabilityId, CapabilityParams, EffectiveGrant, Grant, Grants, InstanceGrant, TrustLevel,
    narrow_instance,
};
use floatile_core::types::{InstanceId, PluginId};
use floatile_plugin_api::exports::floatile::widget::widget_contract::{UiEvent, WidgetEvent};
use floatile_services::AuditEvent;
use floatile_ui_schema::schema::JsonSchema;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::state::UiUpdate;
use crate::{RuntimeError, WidgetConfig, WidgetHandle, WidgetManager};

/// 测试工具错误（稳定 code `FTEST_*` 由 CLI 层做映射）。
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("引擎/运行时错误: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("实例错误: {0}")]
    Instance(String),
    #[error("授权错误: {0}")]
    Grant(String),
    #[error("等待 State 超时（{0:?}）")]
    Timeout(Duration),
    #[error("UI 通道已关闭")]
    Closed,
}

impl From<crate::InstanceError> for HarnessError {
    fn from(e: crate::InstanceError) -> Self {
        Self::Instance(e.to_string())
    }
}

/// 构造一个插件实例的测试句柄。
pub struct WidgetHarness {
    plugin: PluginId,
    instance: InstanceId,
    wasm: Vec<u8>,
    initial_state: Value,
    state_schema: JsonSchema,
    config_json: String,
    grants: Vec<Grant>,
    trust: TrustLevel,
    fuel_per_call: Option<u64>,
    max_memory: Option<usize>,
}

impl WidgetHarness {
    /// 构造：`plugin` 为反域名 id，`wasm` 为插件 Component 字节。
    pub fn new(plugin: PluginId, wasm: Vec<u8>) -> Self {
        Self {
            plugin,
            instance: InstanceId(1),
            wasm,
            initial_state: Value::Object(Default::default()),
            state_schema: JsonSchema::default(),
            config_json: "{}".into(),
            grants: Vec::new(),
            trust: TrustLevel::Dev,
            fuel_per_call: None,
            max_memory: None,
        }
    }

    /// 设置实例 id（默认 1）。
    pub fn instance(mut self, instance: InstanceId) -> Self {
        self.instance = instance;
        self
    }

    /// canonical initial State（默认 `{}`）。
    pub fn initial_state(mut self, state: Value) -> Self {
        self.initial_state = state;
        self
    }

    /// State schema（默认宽松 object）。
    pub fn state_schema(mut self, schema: JsonSchema) -> Self {
        self.state_schema = schema;
        self
    }

    /// manifest 校验后的 config JSON（默认 `{}`）。
    pub fn config_json(mut self, config: impl Into<String>) -> Self {
        self.config_json = config.into();
        self
    }

    /// 声明一项能力授权（`params` 可省略以使用默认配额）。
    pub fn grant(mut self, capability: CapabilityId, params: Option<CapabilityParams>) -> Self {
        self.grants.push(Grant {
            capability,
            params,
            effective: EffectiveGrant::DerivedFromInstall,
        });
        self
    }

    /// 批量设置授权（覆盖已有；供按 manifest `permissions` 重建）。
    pub fn grant_all(mut self, grants: Vec<Grant>) -> Self {
        self.grants = grants;
        self
    }

    /// 覆盖信任级别（默认 Dev）。
    pub fn trust(mut self, trust: TrustLevel) -> Self {
        self.trust = trust;
        self
    }

    /// 覆盖单次宿主调用 fuel 预算（测试 trap 用）。
    pub fn fuel_per_call(mut self, fuel: u64) -> Self {
        self.fuel_per_call = Some(fuel);
        self
    }

    /// 覆盖每实例线性内存上限（测试 StoreLimits 用）。
    pub fn max_memory(mut self, bytes: usize) -> Self {
        self.max_memory = Some(bytes);
        self
    }

    /// 创建引擎、派生并启动实例 actor，返回可驱动的句柄。
    pub fn build(self) -> Result<HarnessInstance, HarnessError> {
        let mut manager = WidgetManager::new()?;
        if let Some(fuel) = self.fuel_per_call {
            manager = manager.with_fuel_per_call(fuel);
        }
        if let Some(mem) = self.max_memory {
            manager = manager.with_max_memory(mem);
        }
        let audit: Arc<Mutex<Vec<AuditEvent>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let audit = Arc::clone(&audit);
            let listener: floatile_services::AuditListener =
                Arc::new(move |event| audit.lock().push(event.clone()));
            manager = manager.with_audit_listener(Some(listener));
        }

        let plugin_grants = Grants {
            plugin: self.plugin.clone(),
            caps: self.grants.clone(),
            trust: self.trust,
        };
        let grants: InstanceGrant = narrow_instance(&plugin_grants, self.instance, self.grants)
            .map_err(|e| HarnessError::Grant(e.to_string()))?;

        let config = WidgetConfig {
            plugin: self.plugin,
            instance: self.instance,
            wasm: self.wasm,
            initial_state: self.initial_state,
            state_schema: self.state_schema,
            config_json: self.config_json,
            grants,
        };
        let handle = manager.spawn(config)?;
        Ok(HarnessInstance { handle, audit })
    }
}

/// 已启动实例的可测试句柄：投递事件、读权威 State、断言审计。
pub struct HarnessInstance {
    handle: WidgetHandle,
    audit: Arc<Mutex<Vec<AuditEvent>>>,
}

impl HarnessInstance {
    /// 通知实例 start（触发 guest `constructor` 之后的 `start`）。
    pub async fn start(&self) -> Result<(), HarnessError> {
        self.handle.start().await?;
        Ok(())
    }

    /// 投递一个统一事件并等待 guest 处理完成。
    pub async fn emit(&self, event: WidgetEvent) -> Result<(), HarnessError> {
        self.handle.handle_event(event).await?;
        Ok(())
    }

    /// 便捷：投递一个 UI 事件（`name` + JSON payload）。
    pub async fn emit_ui(&self, name: &str, payload_json: &str) -> Result<(), HarnessError> {
        self.emit(WidgetEvent::Ui(UiEvent {
            name: name.into(),
            payload_json: payload_json.into(),
        }))
        .await
    }

    /// 权威 State 快照接收端（原子应用、已通过 schema 校验）。
    pub fn ui_updates(&mut self) -> &mut mpsc::Receiver<UiUpdate> {
        self.handle.ui_updates()
    }

    /// 轮询 State 通道直到谓词满足或超时。
    ///
    /// 返回满足谓词的那份 State 快照；通道关闭返回 `Closed`，超时返回 `Timeout`。
    pub async fn wait_for_state<F>(
        &mut self,
        timeout: Duration,
        mut pred: F,
    ) -> Result<Value, HarnessError>
    where
        F: FnMut(&Value) -> bool,
    {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(HarnessError::Timeout(timeout));
            }
            match tokio::time::timeout(remaining, self.handle.ui_updates().recv()).await {
                Ok(Some(update)) => {
                    if pred(&update.state) {
                        return Ok(update.state);
                    }
                }
                Ok(None) => return Err(HarnessError::Closed),
                Err(_) => return Err(HarnessError::Timeout(timeout)),
            }
        }
    }

    /// 按真实时钟推进（guest 计时器经 Broker 落到 tokio 定时器）。
    pub async fn advance_time(&self, dur: Duration) {
        tokio::time::sleep(dur).await;
    }

    /// 在 `timeout` 内累计收到的权威 State 更新数量（超时或通道关闭即停）。
    pub async fn count_state_updates(&mut self, timeout: Duration) -> usize {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut n = 0usize;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return n;
            }
            match tokio::time::timeout(remaining, self.handle.ui_updates().recv()).await {
                Ok(Some(_)) => n += 1,
                Ok(None) | Err(_) => return n,
            }
        }
    }

    /// 当前已收集的脱敏审计记录（含 allow 与 deny）。
    pub fn audit(&self) -> Vec<AuditEvent> {
        self.audit.lock().clone()
    }

    /// 审计断言：谓词作用于全部审计记录。
    pub fn assert_audit<F>(&self, pred: F) -> bool
    where
        F: Fn(&[AuditEvent]) -> bool,
    {
        pred(&self.audit())
    }

    /// 有预算地停止实例。
    pub async fn shutdown(self) -> Result<(), RuntimeError> {
        self.handle.shutdown().await
    }
}
