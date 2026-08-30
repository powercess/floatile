//! 参考时钟插件的作者模型与 UI 单一事实源。
//!
//! 本 crate 只产生 `rlib`：宿主构建脚本与测试可读取同一 `Widget::view`，而
//! `clock-wasm` 仍是唯一产生 WASM `cdylib` 的 guest 外壳。

use floatile_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
pub struct ClockState {
    pub time: String,
    pub running: bool,
}

#[derive(Debug)]
pub enum ClockEvent {
    Start,
    Tick,
}

impl FromWidgetEvent for ClockEvent {
    fn from_widget_event(event: WidgetEvent) -> Option<Self> {
        match event {
            WidgetEvent::Ui(event) if event.name == "start" => Some(Self::Start),
            WidgetEvent::Timer(_) => Some(Self::Tick),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct Clock;

impl Widget for Clock {
    type State = ClockState;
    type Event = ClockEvent;

    fn view(_state: &Self::State) -> View {
        view::column(vec![view::text_bind("$.time")])
    }

    fn start(&mut self, ctx: &mut Context<Self>) -> WidgetResult {
        let _ = ctx.log(LogLevel::Info, "clock started");
        match ctx.timer().schedule(1000) {
            Ok(id) => {
                let _ = ctx.log(LogLevel::Debug, &format!("timer scheduled: {id}"));
            }
            Err(error) => {
                let _ = ctx.log(LogLevel::Warn, &format!("timer denied: {error:?}"));
            }
        }
        Ok(())
    }

    fn event(&mut self, event: ClockEvent, ctx: &mut Context<Self>) -> WidgetResult {
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
                let _ = ctx.timer().schedule(1000);
            }
        }
        Ok(())
    }

    fn stop(&mut self) {
        let ctx = Context::<Self>::new();
        let _ = ctx.log(LogLevel::Info, "clock stopped");
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ftui_json() -> String {
    floatile_sdk::build::build_ftui::<Clock>(std::collections::BTreeMap::new())
}
