//! 参考时钟 WASM 插件（S5 最小垂直切片）。
//!
//! 构建（产出组件）：
//! ```text
//! cargo build -p floatile-clock-wasm --target wasm32-wasip2
//! wasm-tools validate target/wasm32-wasip2/debug/floatile_clock_wasm.wasm
//! ```

use std::cell::Cell;

use floatile_sdk::{
    Guest, GuestWidgetInstance, LogLevel, UiEvent, WidgetMode, export_widget, host_log, host_timer,
};

struct Clock {
    running: Cell<bool>,
    ticks: Cell<u64>,
}

impl GuestWidgetInstance for Clock {
    fn new(_config: String) -> Self {
        host_log::log(LogLevel::Info, "clock constructed");
        Self {
            running: Cell::new(false),
            ticks: Cell::new(0),
        }
    }

    fn handle_ui_event(&self, event: UiEvent) {
        host_log::log(LogLevel::Info, &format!("ui event: {}", event.name));
        if event.name == "start" {
            self.running.set(true);
            // v1 计时器为一次性语义：每次 on-tick 后重新 schedule。
            match host_timer::schedule(1000) {
                Ok(id) => host_log::log(LogLevel::Debug, &format!("timer scheduled: {id}")),
                Err(error) => {
                    host_log::log(LogLevel::Warn, &format!("timer schedule denied: {error:?}"))
                }
            }
        }
    }

    fn on_tick(&self, _timer_id: u32) {
        self.ticks.set(self.ticks.get() + 1);
        host_log::log(LogLevel::Info, &format!("tick {}", self.ticks.get()));
        if self.running.get() {
            let _ = host_timer::schedule(1000);
        }
    }

    fn on_mode_changed(&self, mode: WidgetMode) {
        host_log::log(LogLevel::Debug, &format!("mode: {mode:?}"));
    }

    fn destroy(&self) {
        host_log::log(LogLevel::Info, "destroy");
    }
}

impl Guest for Clock {
    type WidgetInstance = Clock;
}

export_widget!(Clock);
