//! `Widget` trait：作者插件的行为契约。
//!
//! 一个 Floatile widget 实现本 trait 并通过 `impl_export_widget!(Type)`
//! 导出为 WASM Component。所有 host 能力调用经 `Context` → WIT → Broker 路径。

use crate::view::View;
use crate::{Context, WidgetEvent};

/// 从宿主级 [`WidgetEvent`] 转换为作者定义的事件类型。
///
/// 返回 `Some(event)` 表示投递到 `Widget::event`；`None` 表示静默忽略。
/// 恒等实现（`type Event = WidgetEvent`）始终返回 `Some`，模板默认使用。
pub trait FromWidgetEvent: Sized {
    fn from_widget_event(event: WidgetEvent) -> Option<Self>;
}

/// 恒等转换：`type Event = WidgetEvent` 时无需作者手写转换。
impl FromWidgetEvent for WidgetEvent {
    fn from_widget_event(event: WidgetEvent) -> Option<Self> {
        Some(event)
    }
}

/// 标准 Widget 契约。`State` 由 `#[derive(State)]` 生成 schema。
///
/// - `view`：构建期定义 UI 组件树（host 侧编译为 widget.ftui）
/// - `start`：实例启动，可 schedule 计时器、初始化
/// - `event`：统一事件入口（UI / timer / mode / config / theme / suspend / resume）
/// - `stop`：实例销毁前的通知（尽力而为，不能保证执行）
///
/// `Default` 是导出宏 `impl_export_widget!` 构造实例所需的构造约束。
pub trait Widget: Sized + Default {
    type State: crate::State;
    type Event: FromWidgetEvent;
    fn view(state: &Self::State) -> View;
    fn start(&mut self, ctx: &mut Context<Self>);
    fn event(&mut self, event: Self::Event, ctx: &mut Context<Self>);
    fn stop(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_widget_event_maps_and_ignores() {
        enum E {
            A,
            B,
        }
        impl FromWidgetEvent for E {
            fn from_widget_event(event: WidgetEvent) -> Option<Self> {
                match event {
                    WidgetEvent::Ui(u) if u.name == "a" => Some(E::A),
                    WidgetEvent::Timer(_) => Some(E::B),
                    _ => None,
                }
            }
        }

        // Ui "a" → Some(A)
        let ev = WidgetEvent::Ui(crate::UiEvent {
            name: "a".into(),
            payload_json: "{}".into(),
        });
        assert!(E::from_widget_event(ev).is_some_and(|e| matches!(e, E::A)));

        // Timer → Some(B)
        let ev = WidgetEvent::Timer(1);
        assert!(E::from_widget_event(ev).is_some_and(|e| matches!(e, E::B)));

        // Ui "other" → None
        let ev = WidgetEvent::Ui(crate::UiEvent {
            name: "other".into(),
            payload_json: "{}".into(),
        });
        assert!(E::from_widget_event(ev).is_none());

        // 恒等 WidgetEvent 转换始终 Some
        let ev = WidgetEvent::Timer(42);
        assert!(WidgetEvent::from_widget_event(ev).is_some());
    }
}
