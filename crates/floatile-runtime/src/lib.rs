//! Floatile 插件运行时：Wasmtime Component 加载、实例 actor 与 State Patch。
//!
//! `WidgetManager` 创建引擎并派生 `WidgetHandle`（每实例有界串行 actor）。实例
//! 生命周期 `constructor/start/handle-event/stop` 严格串行；fuel/内存/超时/trap
//! 只终止当前实例，宿主与其他实例存活。所有宿主能力经 `PermissionBroker`。

mod error;
pub mod harness;
pub mod operation;
mod state;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use floatile_core::capability::InstanceGrant;
use floatile_core::types::{InstanceId, PluginId};
use floatile_core::{
    CapabilityId, OperationCompletion, OperationFailure, OperationOwner, OperationTerminal,
};
use floatile_plugin_api::FloatileWidget;
use floatile_plugin_api::exports::floatile::widget::widget_contract::{
    GuestWidgetInstance, WidgetEvent, WidgetInit, WidgetInstance, WidgetMode,
};
use floatile_plugin_api::floatile::widget::host_operation::{
    OperationCapability, OperationCompletion as WitOperationCompletion,
    OperationTerminal as WitOperationTerminal,
};
use floatile_services::{
    AuditListener, AuditSink, Broker, HttpsService, OperationLimits, OperationRegistry, TimerSink,
};
use floatile_ui_schema::schema::JsonSchema;
use floatile_ui_schema::validate_value;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Store};

pub use error::{InstanceError, RuntimeError};
pub use operation::{
    OperationBridgeError, OperationCompletionBridge, OperationDelivery, RuntimeOperationEvents,
};
pub use state::UiUpdate;

/// 实例队列容量（有界背压）。
const QUEUE_CAPACITY: usize = 64;
/// 默认单次调用 fuel 预算（wasm32-wasip2 std 初始化会消耗一定量；恶意循环用
/// 更小预算单独测试）。
const DEFAULT_FUEL_PER_CALL: u64 = 1_000_000_000;
/// 默认单次 guest 调用墙钟预算。
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(2);
/// Wasmtime epoch 的宿主节拍；deadline 向上取整到该粒度。
const EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(10);
/// 默认每实例线性内存上限。
const DEFAULT_MAX_MEMORY: usize = 16 * 1024 * 1024;

/// 创建实例所需的配置（manifest 校验、UI IR 验证与 grants 收窄在调用方完成）。
pub struct WidgetConfig {
    pub plugin: PluginId,
    pub instance: InstanceId,
    /// 本次实例启动 generation；Operation completion 只可回到相同 generation。
    pub generation: u64,
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
    call_timeout: Duration,
    epoch_ticker: Arc<EpochTicker>,
    /// 可选脱敏审计持久化 sink:注入到每个实例 Broker 的 AuditSink。
    audit_listener: Option<AuditListener>,
}

impl WidgetManager {
    pub fn new() -> Result<Self, RuntimeError> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true)
            .wasm_backtrace_max_frames(std::num::NonZero::new(32));
        let engine = wasmtime::Engine::new(&config)?;
        let epoch_ticker = Arc::new(EpochTicker::spawn(engine.clone())?);
        Ok(Self {
            engine,
            max_memory: DEFAULT_MAX_MEMORY,
            fuel_per_call: DEFAULT_FUEL_PER_CALL,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            epoch_ticker,
            audit_listener: None,
        })
    }

    /// 注入一个共享脱敏审计接收器(shell 用它把 Broker 审计持久化到 SQLite)。
    ///
    /// 传给每个实例 Broker 的 `AuditSink.with_listener`:允许/拒绝都经此回调解脱敏
    /// 记录。传 `None` 关闭持久化(仅保留 tracing 输出)。同一 listener 会被多个
    /// 实例共享,必须线程安全(默认 `Arc<dyn Fn(...)>` 满足)。
    pub fn with_audit_listener(mut self, listener: Option<AuditListener>) -> Self {
        self.audit_listener = listener;
        self
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

    /// 设置 constructor/lifecycle/event/timer 单次 guest 调用的墙钟预算。
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// 派生一个插件实例（加载组件 + 启动串行 actor）。
    pub fn spawn(&self, config: WidgetConfig) -> Result<WidgetHandle, RuntimeError> {
        self.spawn_with_https(config, None)
    }

    /// Spawn with the host-owned PP-M5 HTTPS service for this exact instance generation.
    pub fn spawn_with_https(
        &self,
        config: WidgetConfig,
        https: Option<HttpsService>,
    ) -> Result<WidgetHandle, RuntimeError> {
        let (cmd_tx, cmd_rx) = mpsc::channel(QUEUE_CAPACITY);
        let (ui_tx, ui_rx) = mpsc::channel(QUEUE_CAPACITY);

        let actor_tx = cmd_tx.clone();
        let engine = self.engine.clone();
        let max_memory = self.max_memory;
        let fuel_per_call = self.fuel_per_call;
        let call_timeout = self.call_timeout;
        let epoch_ticker = Arc::clone(&self.epoch_ticker);
        let audit_listener = self.audit_listener.clone();
        let actor_failure = Arc::new(parking_lot::Mutex::new(None));
        let task_failure = Arc::clone(&actor_failure);
        let join = tokio::spawn(async move {
            let result = run_actor(
                engine,
                config,
                https,
                max_memory,
                fuel_per_call,
                call_timeout,
                epoch_ticker,
                audit_listener,
                ui_tx,
                cmd_rx,
                actor_tx,
            )
            .await;
            if let Err(error) = &result {
                *task_failure.lock() = Some(error.to_string());
            }
            result
        });

        Ok(WidgetHandle {
            cmd: cmd_tx,
            ui: ui_rx,
            join,
            actor_failure,
        })
    }
}

/// 为共享 Wasmtime Engine 推进固定 epoch；Store 用相对 tick deadline 独立计时。
struct EpochTicker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn spawn(engine: wasmtime::Engine) -> Result<Self, RuntimeError> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("floatile-wasm-epoch".to_owned())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    std::thread::park_timeout(EPOCH_TICK_INTERVAL);
                    if thread_stop.load(Ordering::Acquire) {
                        break;
                    }
                    engine.increment_epoch();
                }
            })
            .map_err(|error| {
                RuntimeError::InstanceFailed(format!("启动 Wasmtime epoch ticker 失败: {error}"))
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

/// 实例句柄：命令投递 + UI State 接收。
pub struct WidgetHandle {
    cmd: mpsc::Sender<InstanceCommand>,
    ui: mpsc::Receiver<UiUpdate>,
    join: tokio::task::JoinHandle<Result<(), RuntimeError>>,
    actor_failure: Arc<parking_lot::Mutex<Option<String>>>,
}

enum InstanceCommand {
    Start(oneshot::Sender<Result<(), InstanceError>>),
    Event(WidgetEvent, oneshot::Sender<Result<(), InstanceError>>),
    Timer(u32),
    OperationCompleted(OperationCompletion),
    Shutdown,
}

impl WidgetHandle {
    pub async fn start(&self) -> Result<(), InstanceError> {
        let (tx, rx) = oneshot::channel();
        self.send(InstanceCommand::Start(tx)).await?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(self.actor_stopped_error().await),
        }
    }

    /// 投递一个统一事件（UI/timer/mode/config/theme/suspend/resume）。
    pub async fn handle_event(&self, event: WidgetEvent) -> Result<(), InstanceError> {
        let (tx, rx) = oneshot::channel();
        self.send(InstanceCommand::Event(event, tx)).await?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(self.actor_stopped_error().await),
        }
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

    async fn actor_stopped_error(&self) -> InstanceError {
        // Dropping the command receiver wakes the caller before the actor task's wrapper
        // necessarily records `run_actor`'s result. Give that wrapper a bounded chance to
        // publish the setup/call error so cross-platform failures retain their root cause.
        for _ in 0..4 {
            if let Some(detail) = self.actor_failure.lock().clone() {
                return InstanceError::Failed(detail);
            }
            tokio::task::yield_now().await;
        }
        InstanceError::Failed("actor 未返回调用结果".to_owned())
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_actor(
    engine: wasmtime::Engine,
    config: WidgetConfig,
    https: Option<HttpsService>,
    max_memory: usize,
    fuel_per_call: u64,
    call_timeout: Duration,
    _epoch_ticker: Arc<EpochTicker>,
    audit_listener: Option<AuditListener>,
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
    let timer_actor = actor_tx.clone();
    let sink: TimerSink = Arc::new(move |id| {
        if timer_actor.try_send(InstanceCommand::Timer(id)).is_err() {
            tracing::warn!(
                plugin_id = %sink_plugin,
                instance_id = instance_id,
                timer_id = id,
                "timer 事件队列满，丢弃",
            );
        }
    });
    // Broker：所有宿主能力入口；审计可选落到注入的持久化 sink。
    let mut audit = AuditSink::new(plugin_id.clone(), instance_id);
    if let Some(listener) = &audit_listener {
        audit = audit.with_listener(Arc::clone(listener));
    }
    let owner = OperationOwner::new(config.plugin.clone(), config.instance, config.generation);
    let (operations, mut operation_completions) =
        OperationRegistry::new(owner, OperationLimits::default()).map_err(|error| {
            RuntimeError::InstanceFailed(format!("初始化 Operation registry 失败: {error}"))
        })?;
    let operation_actor = actor_tx.clone();
    let completion_audit = audit.clone();
    let completion_results = operations.result_discarder();
    let mut broker = Broker::new(
        config.plugin.clone(),
        config.generation,
        config.grants,
        audit,
        sink,
    )
    .with_operations(operations)
    .map_err(|error| {
        RuntimeError::InstanceFailed(format!("绑定 Operation registry 失败: {error}"))
    })?;
    if let Some(https) = https {
        broker = broker.with_https(https);
    }
    tokio::spawn(async move {
        while let Some(completion) = operation_completions.recv().await {
            let capability = completion.capability;
            let id = completion.id;
            let terminal = completion.terminal;
            let disposition =
                match operation_actor.try_send(InstanceCommand::OperationCompleted(completion)) {
                    Ok(()) => floatile_core::OperationCompletionDisposition::Delivered,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        completion_results.discard(id);
                        floatile_core::OperationCompletionDisposition::QueueFull
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        completion_results.discard(id);
                        floatile_core::OperationCompletionDisposition::ActorClosed
                    }
                };
            completion_audit.record(
                capability,
                true,
                None,
                &format!(
                    "operation={} terminal={} delivery={}",
                    id.get(),
                    terminal.code(),
                    disposition.code()
                ),
            );
            if disposition == floatile_core::OperationCompletionDisposition::ActorClosed {
                break;
            }
        }
    });

    let initial_state_json = serde_json::to_string(&config.initial_state)
        .map_err(|e| RuntimeError::InstanceFailed(format!("initial state 序列化失败: {e}")))?;

    // 宿主下发的 initial State 必须先通过 schema 校验（与 update-state 同一
    // 校验器），fail-fast 防止把与契约不符的初始状态交给实例造成下游漂移。
    validate_value(&config.state_schema, &config.initial_state, "$", 0).map_err(|e| {
        RuntimeError::InstanceFailed(format!("initial state 未通过 schema 校验: {e}"))
    })?;

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

    // instantiate 会运行组件的 `_initialize` start 段，先给足 fuel。constructor 与
    // 后续每次 guest 回调都重新装填同一预算，避免把“每次调用”误实现为实例生命周期
    // 总预算。
    reset_setup_budget(&mut store, fuel_per_call, call_timeout)?;
    let _ = store.fuel_async_yield_interval(Some(100_000));

    let bindings = FloatileWidget::instantiate_async(&mut store, &component, &linker)
        .await
        .map_err(|error| setup_call_error("instantiate", call_timeout, error))?;
    let contract = bindings.floatile_widget_widget_contract();
    let widget = contract.widget_instance();
    let init = WidgetInit {
        config_json: config.config_json,
        initial_state_json,
    };
    reset_setup_budget(&mut store, fuel_per_call, call_timeout)?;
    let resource = widget
        .call_constructor(&mut store, &init)
        .await
        .map_err(|error| setup_call_error("constructor", call_timeout, error))?;

    let mut stopped = false;
    let mut failed = false;
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            InstanceCommand::Start(tx) => {
                let result = match reset_call_budget(&mut store, fuel_per_call, call_timeout) {
                    Ok(()) => match widget.call_start(&mut store, resource).await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(guest_err)) => {
                            Err(InstanceError::Rejected(format!("{guest_err:?}")))
                        }
                        Err(wasm_err) => Err(instance_call_error("start", call_timeout, wasm_err)),
                    },
                    Err(error) => Err(error),
                };
                failed = result.is_err();
                let _ = tx.send(result);
            }
            InstanceCommand::Event(event, tx) => {
                let result = match reset_call_budget(&mut store, fuel_per_call, call_timeout) {
                    Ok(()) => match widget.call_handle_event(&mut store, resource, &event).await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(guest_err)) => {
                            Err(InstanceError::Rejected(format!("{guest_err:?}")))
                        }
                        Err(wasm_err) => {
                            Err(instance_call_error("handle-event", call_timeout, wasm_err))
                        }
                    },
                    Err(error) => Err(error),
                };
                failed = result.is_err();
                let _ = tx.send(result);
            }
            InstanceCommand::Timer(id) => {
                let result = match reset_call_budget(&mut store, fuel_per_call, call_timeout) {
                    Ok(()) => match widget
                        .call_handle_event(&mut store, resource, &WidgetEvent::Timer(id))
                        .await
                    {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(guest_err)) => {
                            Err(InstanceError::Rejected(format!("{guest_err:?}")))
                        }
                        Err(wasm_err) => Err(instance_call_error("timer", call_timeout, wasm_err)),
                    },
                    Err(error) => Err(error),
                };
                if result.is_err() {
                    failed = true;
                } else {
                    store.data_mut().timer_complete(id);
                }
            }
            InstanceCommand::OperationCompleted(completion) => {
                if handle_operation_completion(
                    &mut store,
                    &widget,
                    resource,
                    completion,
                    fuel_per_call,
                    call_timeout,
                )
                .await
                .is_err()
                {
                    failed = true;
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
    let _ = reset_setup_budget(&mut store, fuel_per_call, call_timeout);
    let _ = widget.call_stop(&mut store, resource).await;
    let _ = reset_setup_budget(&mut store, fuel_per_call, call_timeout);
    let _ = resource.resource_drop_async(&mut store).await;
    if stopped && !failed {
        Ok(())
    } else {
        Err(RuntimeError::InstanceFailed(
            "actor 因 trap/超时/终止退出".to_owned(),
        ))
    }
}

async fn handle_operation_completion(
    store: &mut Store<state::InstanceHostState>,
    widget: &GuestWidgetInstance<'_>,
    resource: WidgetInstance,
    completion: OperationCompletion,
    fuel_per_call: u64,
    call_timeout: Duration,
) -> Result<(), InstanceError> {
    let capability = match completion.capability {
        CapabilityId::StorageRead => OperationCapability::StorageRead,
        CapabilityId::NetworkHttps => OperationCapability::HttpsRequest,
        _ => {
            store.data().broker.discard_operation_result(completion.id);
            return Ok(());
        }
    };
    let terminal = match completion.terminal {
        OperationTerminal::Succeeded => WitOperationTerminal::Succeeded,
        OperationTerminal::Failed(OperationFailure::Timeout) => WitOperationTerminal::Timeout,
        OperationTerminal::Failed(OperationFailure::Cancelled) => WitOperationTerminal::Cancelled,
        OperationTerminal::Failed(OperationFailure::Unavailable) => {
            WitOperationTerminal::Unavailable
        }
        OperationTerminal::Failed(OperationFailure::Internal) => WitOperationTerminal::Internal,
        OperationTerminal::Failed(OperationFailure::ResultDropped) => {
            WitOperationTerminal::ResultDropped
        }
    };
    let event = WidgetEvent::OperationCompleted(WitOperationCompletion {
        id: completion.id.get(),
        capability,
        terminal,
    });
    reset_call_budget(store, fuel_per_call, call_timeout)?;
    match widget.call_handle_event(store, resource, &event).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(guest_err)) => Err(InstanceError::Rejected(format!("{guest_err:?}"))),
        Err(wasm_err) => Err(instance_call_error(
            "operation-completed",
            call_timeout,
            wasm_err,
        )),
    }
}

fn reset_call_budget(
    store: &mut Store<state::InstanceHostState>,
    fuel_per_call: u64,
    call_timeout: Duration,
) -> Result<(), InstanceError> {
    store
        .set_fuel(fuel_per_call)
        .map_err(|error| InstanceError::Failed(format!("重置调用 fuel 失败: {error}")))?;
    store.set_epoch_deadline(timeout_epoch_ticks(call_timeout));
    store.epoch_deadline_trap();
    Ok(())
}

fn reset_setup_budget(
    store: &mut Store<state::InstanceHostState>,
    fuel_per_call: u64,
    call_timeout: Duration,
) -> Result<(), RuntimeError> {
    store.set_fuel(fuel_per_call)?;
    store.set_epoch_deadline(timeout_epoch_ticks(call_timeout));
    store.epoch_deadline_trap();
    Ok(())
}

fn timeout_epoch_ticks(timeout: Duration) -> u64 {
    let ticks = timeout.as_nanos().div_ceil(EPOCH_TICK_INTERVAL.as_nanos());
    u64::try_from(ticks).map_or(u64::MAX, |ticks| ticks.max(1))
}

fn setup_call_error(operation: &str, timeout: Duration, error: wasmtime::Error) -> RuntimeError {
    if is_epoch_interrupt(&error) {
        RuntimeError::InstanceFailed(format!("{operation} 超过墙钟预算 {timeout:?}: {error}"))
    } else {
        RuntimeError::InstanceFailed(format!("{operation} trap: {error}"))
    }
}

fn instance_call_error(
    operation: &str,
    timeout: Duration,
    error: wasmtime::Error,
) -> InstanceError {
    if is_epoch_interrupt(&error) {
        InstanceError::Failed(format!("{operation} 超过墙钟预算 {timeout:?}: {error}"))
    } else {
        InstanceError::Failed(error.to_string())
    }
}

fn is_epoch_interrupt(error: &wasmtime::Error) -> bool {
    matches!(
        error.downcast_ref::<wasmtime::Trap>(),
        Some(wasmtime::Trap::Interrupt)
    )
}

/// 实例失败后，把所有待处理命令的错误回给调用方，避免等待超时。
fn drain_pending(cmd_rx: &mut mpsc::Receiver<InstanceCommand>, message: &str) {
    while let Ok(cmd) = cmd_rx.try_recv() {
        match cmd {
            InstanceCommand::Start(tx) | InstanceCommand::Event(_, tx) => {
                let _ = tx.send(Err(InstanceError::Failed(message.to_owned())));
            }
            InstanceCommand::Timer(_)
            | InstanceCommand::OperationCompleted(_)
            | InstanceCommand::Shutdown => {}
        }
    }
}
