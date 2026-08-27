//! Floatile WASI guest SDK（面向 `wasm32-wasip2`）。
//!
//! 低层绑定由 `wit/floatile-widget.wit` 单一事实源生成（本 crate 只 re-export
//! 绑定与导出宏，不包含任何宿主依赖）。作者层（`Widget<State,Event>` / `View` /
//! `Context` / `#[derive(State)]`）提供不手写 WIT/manifest/UI IR 的开发体验。

// ---- 低层 WIT 绑定 ----
wit_bindgen::generate!({
    world: "floatile-widget",
    // `wit/` 是根事实源生成的 crate 内发行快照；漂移测试禁止手工分叉。
    path: "wit/floatile-widget.wit",
    default_bindings_module: "floatile_sdk",
    pub_export_macro: true,
    export_macro_name: "export_widget",
});

pub const ENGINE_API_VERSION: &str = "1.2.0";

// 供 `impl_export_widget!` 在 guest 侧解析宿主下发的 canonical initial State。
pub use serde_json;

// ---- 低层 WIT host modules ----
pub use floatile::widget::host_clock;
pub use floatile::widget::host_http;
pub use floatile::widget::host_log;
pub use floatile::widget::host_metrics;
pub use floatile::widget::host_operation;
pub use floatile::widget::host_storage;
pub use floatile::widget::host_theme;
pub use floatile::widget::host_timer;
pub use floatile::widget::host_ui;

// ---- 低层 WIT 契约类型 ----
pub use exports::floatile::widget::widget_contract::{
    Guest, GuestWidgetInstance, UiEvent, WidgetError, WidgetEvent, WidgetInit, WidgetMode,
};
pub use floatile::widget::host_log::{LogError, LogLevel};
pub use floatile::widget::host_metrics::MemorySnapshot;
pub use floatile::widget::host_operation::{
    OperationCapability, OperationCompletion, OperationError, OperationTerminal,
};
pub use floatile::widget::host_storage::StorageError;
pub use floatile::widget::host_theme::ThemeError;
pub use floatile::widget::host_timer::TimerError;
pub use floatile::widget::host_ui::UiError;

// ---- 作者层 ----
#[cfg(not(target_arch = "wasm32"))]
pub mod build;
pub mod context;
pub mod export;
pub mod state;
pub mod view;
pub mod widget;

pub use context::Context;
pub use floatile_sdk_macros::State;
pub use floatile_ui_schema::{JsonSchema, merge_patch, validate_document};
pub use state::State;
pub use widget::FromWidgetEvent;
pub use widget::Widget;
