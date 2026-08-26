//! 恶意/对抗性 WASM 插件 fixture —— 只用于安全验收（P0 安全验收 §3）。
//!
//! 不是真实插件：它在 `start`/`event` 里按宿主下发的 initial State 的 `mode`
//! 字段执行对抗行为，验证宿主在下列情况下「拒绝 + 审计 + 宿主存活」：
//!
//! - `deny`：调用未声明/未授权能力（storage:get、metrics:memory），应被
//!   Broker 拒绝并写脱敏审计；插件记录后继续，实例存活。
//! - `bad-patch`：提交超限/类型错误/未知字段的 State Patch，应被宿主校验拒绝，
//!   状态不部分改写；实例存活。
//! - `loop`：无限 CPU 循环，fuel 预算耗尽应 trap 终止本实例，宿主与其他
//!   实例存活。
//! - `alloc`：申请超限线性内存，StoreLimits 应终止本实例，宿主存活。
//!
//! 构建（产出组件）：
//! ```text
//! cargo build -p floatile-evil-wasm --target wasm32-wasip2
//! wasm-tools validate target/wasm32-wasip2/debug/floatile_evil_wasm.wasm
//! ```

#[cfg(target_arch = "wasm32")]
use floatile_sdk::impl_export_widget;
use floatile_sdk::{
    Context, FromWidgetEvent, LogLevel, OperationCapability, OperationTerminal, State, Widget,
    WidgetEvent, view, view::View,
};
use serde::{Deserialize, Serialize};

/// State 只携带攻击模式；宿主测试用 `initial_state`(已验证 schema)选择行为。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
pub struct EvilState {
    pub mode: String,
}

/// 触发事件：`Ui("trigger")` 或 timer 触发执行 `trigger` 攻击路径。
#[derive(Debug)]
pub enum EvilEvent {
    Trigger,
    OperationCompleted(u64),
    Unknown,
}

impl FromWidgetEvent for EvilEvent {
    fn from_widget_event(event: WidgetEvent) -> Option<Self> {
        match event {
            WidgetEvent::Ui(u) if u.name == "trigger" => Some(EvilEvent::Trigger),
            WidgetEvent::Ui(_) => Some(EvilEvent::Unknown),
            WidgetEvent::Timer(_) => Some(EvilEvent::Trigger),
            WidgetEvent::OperationCompleted(completion)
                if completion.capability == OperationCapability::StorageRead
                    && completion.terminal == OperationTerminal::Succeeded =>
            {
                Some(EvilEvent::OperationCompleted(completion.id))
            }
            _ => None,
        }
    }
}

/// host 编译仅用于保证 workspace 可构建;完整攻击逻辑只在 wasm 目标实际导出执行。
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Default)]
struct Evil {
    mode: String,
}

/// wasm 目标是唯一实例化 `Evil` 的上下文(经 impl_export_widget);host 编译时
/// 这些仅被导出适配调用的实现与私有攻击方法视为未使用,故按目标豁免 dead_code。
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl Widget for Evil {
    type State = EvilState;
    type Event = EvilEvent;

    fn view(_state: &Self::State) -> View {
        view::column(vec![])
    }

    fn init(&mut self, initial: &Self::State) {
        // 记住宿主下发的攻击模式(构造期在 export 适配层调用)。
        self.mode = initial.mode.clone();
    }

    fn start(&mut self, ctx: &mut Context<Self>) {
        match self.mode.as_str() {
            // loop 不进 start:让实例先成功启动,测试再以事件触发无限循环,fuel 才 trap。
            "deny" => self.deny_call(ctx),
            "bad-patch" => self.bad_patch(ctx),
            "alloc" => self.alloc_memory(),
            "operation" => {
                let _ = ctx.storage().submit_get("settings", 1_000);
            }
            mode => {
                let _ = ctx.log(LogLevel::Info, &format!("start mode {mode}"));
            }
        }
    }

    fn event(&mut self, event: Self::Event, ctx: &mut Context<Self>) {
        match event {
            EvilEvent::Trigger => match self.mode.as_str() {
                "loop" => self.loop_forever(),
                "deny" => self.deny_call(ctx),
                "bad-patch" => self.bad_patch(ctx),
                _ => {}
            },
            EvilEvent::Unknown => {}
            EvilEvent::OperationCompleted(id) => {
                if ctx.storage().take_get_result(id).is_ok() {
                    let _ = ctx.state().update(r#"{"mode":"operation-complete"}"#);
                }
            }
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl Evil {
    /// 调用两个未声明能力；每次失败都记录(固有能力 log:write 会成功并留审计)。
    fn deny_call(&self, ctx: &mut Context<Evil>) {
        match ctx.storage().get("secret") {
            Ok(_) => {
                let _ = ctx.log(LogLevel::Error, "storage GET unexpectedly allowed");
            }
            Err(_) => {
                let _ = ctx.log(LogLevel::Info, "storage GET denied as expected");
            }
        }
        match ctx.metrics().memory() {
            Ok(_) => {
                let _ = ctx.log(LogLevel::Error, "metrics MEMORY unexpectedly allowed");
            }
            Err(_) => {
                let _ = ctx.log(LogLevel::Info, "metrics MEMORY denied as expected");
            }
        }
    }

    /// 依次提交过大 / 类型错误 / 未知字段的 State Patch；全部应被宿主拒绝。
    fn bad_patch(&self, ctx: &mut Context<Evil>) {
        // 超限：> MAX_PATCH_BYTES(16KiB)。
        let big = "x".repeat(17 * 1024);
        let oversize = format!(r#"{{"mode":"bad","junk":"{big}"}}"#);
        let _ = ctx.state().update(&oversize);

        // 类型错误：mode 应为字符串。
        let _ = ctx.state().update(r#"{"mode":123}"#);

        // 未知字段：schema additional_properties=false。
        let _ = ctx.state().update(r#"{"mode":"bad","sneaky":true}"#);
    }

    /// 申请远超线性内存上限(默认 16 MiB)的内存。
    fn alloc_memory(&self) {
        let _buf: Vec<u8> = vec![0u8; 64 * 1024 * 1024];
        // 保持 buf 存活防止优化器消除分配。
        std::hint::black_box(&_buf);
    }

    fn loop_forever(&self) -> ! {
        loop {
            std::hint::black_box(1u64);
        }
    }
}

// ---- 导出（仅 wasm 目标；host 编译用于 build_ftui 时不需要导出）----
#[cfg(target_arch = "wasm32")]
impl_export_widget!(Evil);
