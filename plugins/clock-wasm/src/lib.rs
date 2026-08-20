//! 参考时钟 WASM 插件（S5c — 使用作者 SDK，不手写 WIT/manifest/UI IR）。
//!
//! 构建（产出组件）：
//! ```text
//! cargo build -p floatile-clock-wasm --target wasm32-wasip2
//! wasm-tools validate target/wasm32-wasip2/debug/floatile_clock_wasm.wasm
//! ```

#[cfg(target_arch = "wasm32")]
use floatile_sdk::impl_export_widget;
use floatile_sdk::{
    Context, FromWidgetEvent, LogLevel, State, Widget, WidgetEvent, view, view::View,
};
use serde::{Deserialize, Serialize};

// ---- State（derive State 生成 schema + initial）----
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
pub struct ClockState {
    pub time: String,
    pub running: bool,
}

// ---- 作者定义的事件类型 ----
#[derive(Debug)]
pub enum ClockEvent {
    Start,
    Tick,
}

impl FromWidgetEvent for ClockEvent {
    fn from_widget_event(event: WidgetEvent) -> Option<Self> {
        match event {
            WidgetEvent::Ui(u) if u.name == "start" => Some(ClockEvent::Start),
            WidgetEvent::Timer(_) => Some(ClockEvent::Tick),
            _ => None,
        }
    }
}

// ---- Widget ----
// host 编译时仅用于 build_ftui（view 是静态方法，不构造实例）。
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct Clock;

impl Widget for Clock {
    type State = ClockState;
    type Event = ClockEvent;

    fn view(_state: &Self::State) -> View {
        view::column(vec![view::text_bind("$.time")])
    }

    fn start(&mut self, ctx: &mut Context<Self>) {
        let _ = ctx.log(LogLevel::Info, "clock started");
        match ctx.timer().schedule(1000) {
            Ok(id) => {
                let _ = ctx.log(LogLevel::Debug, &format!("timer scheduled: {id}"));
            }
            Err(error) => {
                let _ = ctx.log(LogLevel::Warn, &format!("timer denied: {error:?}"));
            }
        }
    }

    fn event(&mut self, event: ClockEvent, ctx: &mut Context<Self>) {
        match event {
            ClockEvent::Start => {
                let _ = ctx.log(LogLevel::Info, "clock start command received");
                let _ = ctx.state().update(r#"{"running":true}"#);
            }
            ClockEvent::Tick => {
                let time = ctx.clock().now();
                let seconds = (time.unix_millis / 1000) % 86_400;
                let hh = seconds / 3600;
                let mm = (seconds % 3600) / 60;
                let ss = seconds % 60;
                let text = format!("{hh:02}:{mm:02}:{ss:02}");
                let patch = format!(r#"{{"time":"{text}"}}"#);
                let _ = ctx.state().update(&patch);
                let _ = ctx.log(LogLevel::Debug, &format!("tick {text}"));
                // 重新调度一次性计时器。
                let _ = ctx.timer().schedule(1000);
            }
        }
    }

    fn stop(&mut self) {
        let ctx = Context::<Self>::new();
        let _ = ctx.log(LogLevel::Info, "clock stopped");
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock
    }
}

// ---- 导出（仅 wasm 目标；host 编译时用于 build_ftui）----
#[cfg(target_arch = "wasm32")]
impl_export_widget!(Clock);

// ---- 宿主侧 build 入口（`floatile build` 调用）----
#[cfg(not(target_arch = "wasm32"))]
pub fn __floatile_ftui_json() -> String {
    floatile_sdk::build::build_ftui::<Clock>(std::collections::BTreeMap::new())
}
