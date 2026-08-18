//! 参考时钟 WASM 插件（S5 最小垂直切片，迁移到 ADR-0001 统一 lifecycle）。
//!
//! 构建（产出组件）：
//! ```text
//! cargo build -p floatile-clock-wasm --target wasm32-wasip2
//! wasm-tools validate target/wasm32-wasip2/debug/floatile_clock_wasm.wasm
//! ```

use std::cell::Cell;

use floatile_sdk::{
    Guest, GuestWidgetInstance, LogLevel, WidgetError, WidgetEvent, WidgetInit, export_widget,
    host_clock, host_log, host_timer, host_ui,
};

struct Clock {
    running: Cell<bool>,
    ticks: Cell<u64>,
}

impl Clock {
    /// 从宿主 wall-clock 格式化当前时间并提交 State Patch。
    ///
    /// P0 契约迁移阶段只验证 `host-clock` + `host-ui.update-state` 链路；
    /// 真实格式化与时区由作者级 SDK 提供，这里用 UTC 秒数演示能力面。
    fn refresh(&self) {
        let time = host_clock::now();
        let seconds = (time.unix_millis / 1000) % 86_400;
        let hh = seconds / 3600;
        let mm = (seconds % 3600) / 60;
        let ss = seconds % 60;
        let text = format!("{hh:02}:{mm:02}:{ss:02}");
        let patch = format!(r#"{{"time":"{text}"}}"#);
        let _ = match host_ui::update_state(&patch) {
            Ok(()) => host_log::log(LogLevel::Debug, &format!("state patch applied: {text}")),
            Err(error) => {
                host_log::log(LogLevel::Warn, &format!("update-state rejected: {error:?}"))
            }
        };
    }
}

impl GuestWidgetInstance for Clock {
    fn new(init: WidgetInit) -> Self {
        let _ = host_log::log(
            LogLevel::Info,
            &format!(
                "clock constructed; config={} initial_state={}",
                init.config_json, init.initial_state_json
            ),
        );
        Self {
            running: Cell::new(false),
            ticks: Cell::new(0),
        }
    }

    fn start(&self) -> Result<(), WidgetError> {
        let _ = host_log::log(LogLevel::Info, "clock started");
        self.running.set(true);
        // v1 计时器为一次性语义：每次 on-tick 后重新 schedule。
        match host_timer::schedule(1000) {
            Ok(id) => {
                let _ = host_log::log(LogLevel::Debug, &format!("timer scheduled: {id}"));
            }
            Err(error) => {
                let _ = host_log::log(LogLevel::Warn, &format!("timer schedule denied: {error:?}"));
            }
        }
        Ok(())
    }

    fn handle_event(&self, event: WidgetEvent) -> Result<(), WidgetError> {
        match event {
            WidgetEvent::Ui(ui_event) => {
                let _ = host_log::log(
                    LogLevel::Info,
                    &format!(
                        "ui event: {} payload={}",
                        ui_event.name, ui_event.payload_json
                    ),
                );
                Ok(())
            }
            WidgetEvent::Timer(_timer_id) => {
                self.ticks.set(self.ticks.get() + 1);
                let _ = host_log::log(LogLevel::Info, &format!("tick {}", self.ticks.get()));
                self.refresh();
                if self.running.get() {
                    let _ = host_timer::schedule(1000);
                }
                Ok(())
            }
            WidgetEvent::ModeChanged(mode) => {
                let _ = host_log::log(LogLevel::Debug, &format!("mode: {mode:?}"));
                Ok(())
            }
            WidgetEvent::ConfigChanged(_config) => Ok(()),
            WidgetEvent::ThemeChanged(_theme) => Ok(()),
            WidgetEvent::Suspend => {
                let _ = host_log::log(LogLevel::Debug, "suspend");
                Ok(())
            }
            WidgetEvent::Resume => {
                let _ = host_log::log(LogLevel::Debug, "resume");
                Ok(())
            }
        }
    }

    fn stop(&self) {
        let _ = host_log::log(LogLevel::Info, "clock stopped");
    }
}

impl Guest for Clock {
    type WidgetInstance = Clock;
}

export_widget!(Clock);
