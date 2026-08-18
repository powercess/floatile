//! Floatile 插件运行时：Wasmtime Component 加载、实例 actor 与 State Patch。
//!
//! `WidgetManager` 创建引擎并派生 `WidgetHandle`（每实例有界串行 actor）。实例
//! 生命周期 `constructor/start/handle-event/stop` 严格串行；fuel/内存/超时/trap
//! 只终止当前实例，宿主与其他实例存活。所有宿主能力经 `PermissionBroker`。

mod error;
mod state;

use std::sync::Arc;

use floatile_core::capability::InstanceGrant;
use floatile_core::types::{InstanceId, PluginId};
use floatile_plugin_api::FloatileWidget;
use floatile_plugin_api::exports::floatile::widget::widget_contract::{
    WidgetEvent, WidgetInit, WidgetMode,
};
use floatile_services::{AuditSink, Broker, TimerSink};
use floatile_ui_schema::schema::JsonSchema;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Store};

pub use error::{InstanceError, RuntimeError};
pub use state::UiUpdate;

/// 实例队列容量（有界背压）。
const QUEUE_CAPACITY: usize = 64;
/// 默认单次调用 fuel 预算（wasm32-wasip2 std 初始化会消耗一定量；恶意循环用
/// 更小预算单独测试）。
const DEFAULT_FUEL_PER_CALL: u64 = 1_000_000_000;
/// 默认每实例线性内存上限。
const DEFAULT_MAX_MEMORY: usize = 16 * 1024 * 1024;

/// 创建实例所需的配置（manifest 校验、UI IR 验证与 grants 收窄在调用方完成）。
pub struct WidgetConfig {
    pub plugin: PluginId,
    pub instance: InstanceId,
    /// WASM Component 字节。
    pub wasm: Vec<u8>,
    /// canonical initial State（来自已验证 UI IR）。
    pub initial_state: Value,
    /// State schema（来自已验证 UI IR）。
    pub state_schema: JsonSchema,
    /// manifest 校验后的 config JSON。
    pub config_json: String,
    /// 实例收窄授权（固有能力的合并由 Broker 完成）。
    pub grants: InstanceGrant,
}

/// 共享引擎与默认预算。
pub struct WidgetManager {
    engine: wasmtime::Engine,
    max_memory: usize,
    fuel_per_call: u64,
}

impl WidgetManager {
    pub fn new() -> Result<Self, RuntimeError> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .wasm_backtrace_max_frames(std::num::NonZero::new(32));
        let engine = wasmtime::Engine::new(&config)?;
        Ok(Self {
            engine,
            max_memory: DEFAULT_MAX_MEMORY,
            fuel_per_call: DEFAULT_FUEL_PER_CALL,
        })
    }

    /// 设置每实例线性内存上限（字节）。
    pub fn with_max_memory(mut self, bytes: usize) -> Self {
        self.max_memory = bytes;
        self
    }

    /// 设置每次宿主调用的 fuel 预算（耗尽即 trap）。
    pub fn with_fuel_per_call(mut self, fuel: u64) -> Self {
        self.fuel_per_call = fuel;
        self
    }

    /// 派生一个插件实例（加载组件 + 启动串行 actor）。
    pub fn spawn(&self, config: WidgetConfig) -> Result<WidgetHandle, RuntimeError> {
        let (cmd_tx, cmd_rx) = mpsc::channel(QUEUE_CAPACITY);
        let (ui_tx, ui_rx) = mpsc::channel(QUEUE_CAPACITY);

        let actor_tx = cmd_tx.clone();
        let engine = self.engine.clone();
        let max_memory = self.max_memory;
        let fuel_per_call = self.fuel_per_call;
        let join = tokio::spawn(run_actor(
            engine,
            config,
            max_memory,
            fuel_per_call,
            ui_tx,
            cmd_rx,
            actor_tx,
        ));

        Ok(WidgetHandle {
            cmd: cmd_tx,
            ui: ui_rx,
            join,
        })
    }
}

/// 实例句柄：命令投递 + UI State 接收。
pub struct WidgetHandle {
    cmd: mpsc::Sender<InstanceCommand>,
    ui: mpsc::Receiver<UiUpdate>,
    join: tokio::task::JoinHandle<Result<(), RuntimeError>>,
}

enum InstanceCommand {
    Start(oneshot::Sender<Result<(), InstanceError>>),
    Event(WidgetEvent, oneshot::Sender<Result<(), InstanceError>>),
    Timer(u32),
    Shutdown,
}

impl WidgetHandle {
    pub async fn start(&self) -> Result<(), InstanceError> {
        let (tx, rx) = oneshot::channel();
        self.send(InstanceCommand::Start(tx)).await?;
        rx.await
            .map_err(|_| InstanceError::Failed("actor 未返回调用结果".to_owned()))?
    }

    /// 投递一个统一事件（UI/timer/mode/config/theme/suspend/resume）。
    pub async fn handle_event(&self, event: WidgetEvent) -> Result<(), InstanceError> {
        let (tx, rx) = oneshot::channel();
        self.send(InstanceCommand::Event(event, tx)).await?;
        rx.await
            .map_err(|_| InstanceError::Failed("actor 未返回调用结果".to_owned()))?
    }

    /// 展示模式切换通知。
    pub async fn set_mode(&self, mode: WidgetMode) -> Result<(), InstanceError> {
        self.handle_event(WidgetEvent::ModeChanged(mode)).await
    }

    /// 已原子应用并验证的 State 快照接收端。
    pub fn ui_updates(&mut self) -> &mut mpsc::Receiver<UiUpdate> {
        &mut self.ui
    }

    /// 停止实例（有预算的 stop + resource drop）。
    pub async fn shutdown(self) -> Result<(), RuntimeError> {
        let _ = self.cmd.send(InstanceCommand::Shutdown).await;
        match self.join.await {
            Ok(result) => result,
            Err(e) => Err(RuntimeError::InstanceFailed(format!("actor 任务终止: {e}"))),
        }
    }

    /// 等待 actor 结束并返回其最终结果（含 setup/构造阶段的错误）。
    pub async fn into_result(self) -> Result<(), RuntimeError> {
        match self.join.await {
            Ok(result) => result,
            Err(e) => Err(RuntimeError::InstanceFailed(format!("actor 任务终止: {e}"))),
        }
    }

    async fn send(&self, cmd: InstanceCommand) -> Result<(), InstanceError> {
        self.cmd
            .send(cmd)
            .await
            .map_err(|_| InstanceError::Failed("实例已终止（命令通道关闭）".to_owned()))
    }
}

async fn run_actor(
    engine: wasmtime::Engine,
    config: WidgetConfig,
    max_memory: usize,
    fuel_per_call: u64,
    ui_tx: mpsc::Sender<UiUpdate>,
    mut cmd_rx: mpsc::Receiver<InstanceCommand>,
    actor_tx: mpsc::Sender<InstanceCommand>,
) -> Result<(), RuntimeError> {
    let component = Component::from_binary(&engine, &config.wasm)
        .map_err(|e| RuntimeError::Component(format!("WASM 组件解析失败: {e}")))?;

    let plugin_id = config.plugin.0.clone();
    let instance_id = config.instance.0;

    // Broker：所有宿主能力入口；计时器到期经 sink 送回 actor 队列。
    let sink_plugin = plugin_id.clone();
    let sink: TimerSink = Arc::new(move |id| {
        if actor_tx.try_send(InstanceCommand::Timer(id)).is_err() {
            tracing::warn!(
                plugin_id = %sink_plugin,
                instance_id = instance_id,
                timer_id = id,
                "timer 事件队列满，丢弃",
            );
        }
    });
    let broker = Broker::new(
        config.plugin.clone(),
        config.grants,
        AuditSink::new(plugin_id.clone(), instance_id),
        sink,
    );

    let initial_state_json = serde_json::to_string(&config.initial_state)
        .map_err(|e| RuntimeError::InstanceFailed(format!("initial state 序列化失败: {e}")))?;
    let host = state::InstanceHostState::new(
        config.instance,
        broker,
        max_memory,
        config.initial_state,
        config.state_schema,
        ui_tx,
    );
    let mut store = Store::new(&engine, host);
    store.limiter(|s| &mut s.limits);

    let mut linker = Linker::new(&engine);
    // 空 WASI 上下文：满足 wasm32-wasip2 std 的 import，零 ambient capability。
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    FloatileWidget::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)?;

    // instantiate 会运行组件的 `_initialize` start 段，先给足 fuel。
    store.set_fuel(fuel_per_call)?;
    let _ = store.fuel_async_yield_interval(Some(100_000));

    let bindings = FloatileWidget::instantiate_async(&mut store, &component, &linker).await?;
    let contract = bindings.floatile_widget_widget_contract();
    let widget = contract.widget_instance();
    let init = WidgetInit {
        config_json: config.config_json,
        initial_state_json,
    };
    let resource = widget
        .call_constructor(&mut store, &init)
        .await
        .map_err(|e| RuntimeError::InstanceFailed(format!("constructor trap: {e}")))?;

    let mut stopped = false;
    let mut failed = false;
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            InstanceCommand::Start(tx) => {
                let result = match widget.call_start(&mut store, resource).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(guest_err)) => Err(InstanceError::Rejected(format!("{guest_err:?}"))),
                    Err(wasm_err) => Err(InstanceError::Failed(wasm_err.to_string())),
                };
                failed = result.is_err();
                let _ = tx.send(result);
            }
            InstanceCommand::Event(event, tx) => {
                let result = match widget.call_handle_event(&mut store, resource, &event).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(guest_err)) => Err(InstanceError::Rejected(format!("{guest_err:?}"))),
                    Err(wasm_err) => Err(InstanceError::Failed(wasm_err.to_string())),
                };
                failed = result.is_err();
                let _ = tx.send(result);
            }
            InstanceCommand::Timer(id) => {
                let result = match widget
                    .call_handle_event(&mut store, resource, &WidgetEvent::Timer(id))
                    .await
                {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(guest_err)) => Err(InstanceError::Rejected(format!("{guest_err:?}"))),
                    Err(wasm_err) => Err(InstanceError::Failed(wasm_err.to_string())),
                };
                if result.is_err() {
                    failed = true;
                } else {
                    store.data_mut().timer_complete(id);
                }
            }
            InstanceCommand::Shutdown => {
                stopped = true;
                break;
            }
        }
        if failed {
            drain_pending(&mut cmd_rx, "实例已终止");
            break;
        }
    }

    // 清理：有预算的 stop + resource drop（尽力而为）。
    let _ = widget.call_stop(&mut store, resource).await;
    let _ = resource.resource_drop_async(&mut store).await;
    if stopped && !failed {
        Ok(())
    } else {
        Err(RuntimeError::InstanceFailed(
            "actor 因 trap/超时/终止退出".to_owned(),
        ))
    }
}

/// 实例失败后，把所有待处理命令的错误回给调用方，避免等待超时。
fn drain_pending(cmd_rx: &mut mpsc::Receiver<InstanceCommand>, message: &str) {
    while let Ok(cmd) = cmd_rx.try_recv() {
        match cmd {
            InstanceCommand::Start(tx) | InstanceCommand::Event(_, tx) => {
                let _ = tx.send(Err(InstanceError::Failed(message.to_owned())));
            }
            _ => {}
        }
    }
}
