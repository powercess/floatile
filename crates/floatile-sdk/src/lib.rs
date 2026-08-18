//! Floatile WASI guest SDK（面向 `wasm32-wasip2`）。
//!
//! 绑定由 `wit/floatile-widget.wit` 单一事实源生成；本 crate 只 re-export
//! 绑定与导出宏，不包含任何宿主依赖。示例见 `plugins/clock-wasm`。
//!
//! WIT 已迁移到 ADR-0001 目标契约形状（统一 `constructor/start/handle-event/stop`
//! lifecycle、`host-ui.update-state`、canonical initial State 与稳定 guest error）。
//! 面向普通作者的 `State / View / Event / Context` 安全包装、proc macro 与 UI IR
//! 生成仍在 S5 实现中，不能把本层 raw binding re-export 当作最终作者 SDK。

wit_bindgen::generate!({
    world: "floatile-widget",
    path: "../../wit/floatile-widget.wit",
    // 让插件 crate 通过 `floatile_sdk::export_widget!(Type)` 导出实现；
    // 宏展开引用 SDK crate 内的绑定模块，而不是调用方 crate。
    default_bindings_module: "floatile_sdk",
    pub_export_macro: true,
    export_macro_name: "export_widget",
});

/// 宿主要求的引擎 API 版本；与 `wit/floatile-widget.wit` 的
/// `package floatile:widget@x.y.z` 对应（WIT 变更时同步）。
pub const ENGINE_API_VERSION: &str = "1.0.0";

// 宿主能力命名空间（包名嵌套结构），插件用 `host_ui::update_state` 等调用。
pub use floatile::widget::host_clock;
pub use floatile::widget::host_log;
pub use floatile::widget::host_metrics;
pub use floatile::widget::host_storage;
pub use floatile::widget::host_theme;
pub use floatile::widget::host_timer;
pub use floatile::widget::host_ui;

// 契约类型与 trait。
pub use exports::floatile::widget::widget_contract::{
    Guest, GuestWidgetInstance, UiEvent, WidgetError, WidgetEvent, WidgetInit, WidgetMode,
};

// 常用错误/枚举/记录类型（跨边界值由 WIT 生成）。
pub use floatile::widget::host_clock::WallTime;
pub use floatile::widget::host_log::{LogError, LogLevel};
pub use floatile::widget::host_metrics::{MemorySnapshot, MetricsError};
pub use floatile::widget::host_storage::StorageError;
pub use floatile::widget::host_theme::ThemeError;
pub use floatile::widget::host_timer::TimerError;
pub use floatile::widget::host_ui::UiError;
