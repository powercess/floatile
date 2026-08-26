//! 插件运行时上下文：包装 WIT host 调用，所有能力经 `PermissionBroker` 仲裁。
//!
//! `Context` 不持有 State（host 为权威）；每个方法对应一个 WIT import 函数，
//! 作者通过 `ctx.state().update(patch)` 提交 JSON Merge Patch，host 侧做
//! schema 校验与原子应用。

use std::marker::PhantomData;

use crate::{LogError, LogLevel, OperationError, StorageError, ThemeError, TimerError, UiError};
use crate::{
    host_clock, host_log, host_metrics, host_operation, host_storage, host_theme, host_timer,
    host_ui,
};

pub struct Context<W> {
    _widget: PhantomData<fn(&mut W)>,
}

impl<W> Default for Context<W> {
    fn default() -> Self {
        Self {
            _widget: PhantomData,
        }
    }
}

impl<W> Context<W> {
    pub fn new() -> Self {
        Self {
            _widget: PhantomData,
        }
    }

    // ---- state（固有能力 ui:update-state）----

    pub fn state(&self) -> StateCtx {
        StateCtx
    }

    // ---- log（固有能力 log:write）----

    pub fn log(&self, level: LogLevel, message: &str) -> Result<(), LogError> {
        host_log::log(level, message)
    }

    // ---- clock（固有能力 clock:read）----

    pub fn clock(&self) -> ClockCtx {
        ClockCtx
    }

    // ---- timer（声明能力 timer:schedule）----

    pub fn timer(&self) -> TimerCtx {
        TimerCtx
    }

    // ---- storage（声明能力 storage:read/write）----

    pub fn storage(&self) -> StorageCtx {
        StorageCtx
    }

    // ---- operation（宿主托管异步工作）----

    pub fn operation(&self) -> OperationCtx {
        OperationCtx
    }

    // ---- metrics（声明能力 system:cpu/memory）----

    pub fn metrics(&self) -> MetricsCtx {
        MetricsCtx
    }

    // ---- theme（声明能力 theme:subscribe）----

    pub fn theme(&self) -> ThemeCtx {
        ThemeCtx
    }
}

/// `ctx.state()` — State Patch 更新（JSON Merge Patch → host 侧原子应用）。
pub struct StateCtx;

impl StateCtx {
    /// 提交 State Patch（JSON Merge Patch）；宿主原子校验并应用。
    pub fn update(&self, patch_json: &str) -> Result<(), UiError> {
        host_ui::update_state(patch_json)
    }
}

/// `ctx.clock()` — wall clock（只读，不暴露系统句柄）。
pub struct ClockCtx;

impl ClockCtx {
    pub fn now(&self) -> host_clock::WallTime {
        host_clock::now()
    }
}

/// `ctx.timer()` — 计时器（声明能力，需 grant）。
pub struct TimerCtx;

impl TimerCtx {
    pub fn schedule(&self, delay_ms: u64) -> Result<u32, TimerError> {
        host_timer::schedule(delay_ms)
    }
    pub fn cancel(&self, id: u32) -> Result<(), TimerError> {
        host_timer::cancel(id)
    }
}

/// `ctx.storage()` — 插件私有 KV（声明能力，需 grant）。
pub struct StorageCtx;

impl StorageCtx {
    pub fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        host_storage::get(key)
    }
    pub fn set(&self, key: &str, value: &str) -> Result<(), StorageError> {
        host_storage::set(key, value)
    }
    pub fn delete(&self, key: &str) -> Result<(), StorageError> {
        host_storage::delete(key)
    }

    /// 提交异步私有 KV 读取；完成后由 `WidgetEvent::OperationCompleted` 通知。
    pub fn submit_get(&self, key: &str, timeout_ms: u64) -> Result<u64, OperationError> {
        host_storage::submit_get(key, timeout_ms)
    }

    /// 一次性领取已成功完成的异步 KV 读取结果。
    pub fn take_get_result(&self, id: u64) -> Result<Option<String>, OperationError> {
        host_storage::take_get_result(id)
    }
}

/// `ctx.operation()` — 通用 Operation 生命周期操作；payload 仍由 capability-specific API 持有。
pub struct OperationCtx;

impl OperationCtx {
    pub fn cancel(&self, id: u64) -> Result<(), OperationError> {
        host_operation::cancel(id)
    }
}

/// `ctx.metrics()` — 进程指标（声明能力，需 grant）。
pub struct MetricsCtx;

impl MetricsCtx {
    pub fn cpu_percent(&self) -> Result<f64, host_metrics::MetricsError> {
        host_metrics::cpu_percent()
    }
    pub fn memory(&self) -> Result<host_metrics::MemorySnapshot, host_metrics::MetricsError> {
        host_metrics::memory()
    }
}

/// `ctx.theme()` — 主题 token（声明能力，需 grant）。
pub struct ThemeCtx;

impl ThemeCtx {
    pub fn get_token(&self, name: &str) -> Result<Option<String>, ThemeError> {
        host_theme::get_token(name)
    }
    pub fn subscribe(&self) -> Result<u32, ThemeError> {
        host_theme::subscribe()
    }
    pub fn unsubscribe(&self, id: u32) -> Result<(), ThemeError> {
        host_theme::unsubscribe(id)
    }
}
