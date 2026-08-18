//! `Widget` trait：作者插件的行为契约。
//!
//! 一个 Floatile widget 实现本 trait 并通过 `impl_export_widget!(Type)`
//! 导出为 WASM Component。所有 host 能力调用经 `Context` → WIT → Broker 路径。

use crate::view::View;
use crate::{Context, WidgetEvent};

/// 标准 Widget 契约。`State` 由 `#[derive(State)]` 生成 schema。
///
/// - `view`：构建期定义 UI 组件树（host 侧编译为 widget.ftui）
/// - `start`：实例启动，可 schedule 计时器、初始化
/// - `event`：统一事件入口（UI / timer / mode / config / theme / suspend / resume）
/// - `stop`：实例销毁前的通知（尽力而为，不能保证执行）
pub trait Widget: Sized {
    type State: crate::State;
    fn view(state: &Self::State) -> View;
    fn start(&mut self, ctx: &mut Context<Self>);
    fn event(&mut self, event: WidgetEvent, ctx: &mut Context<Self>);
    fn stop(&mut self) {}
}
