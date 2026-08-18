//! 宿主侧 build-time UI IR 生成。
//!
//! `build_ftui` 在构建期（host，非 wasm）调用作者的 `Widget::view` 与
//! `State::schema/initial`，生成 `widget.ftui` JSON。插件必须把 wasm 导出胶水
//! （`impl_export_widget!`）用 `#[cfg(target_arch = "wasm32")]` 隔离，才能同时
//! 编译为 host（build 助手）与 wasm（运行时组件）。

use std::collections::BTreeMap;

use crate::Widget;
use crate::state::State;
use crate::view::into_document;

/// 生成 `widget.ftui` 文档的 JSON 字符串（host-only）。
///
/// 使用 `State::initial()` 作为初始状态调用 `Widget::view`，并把 `State::schema()`
/// 作为 State schema，事件声明由 `events` 提供（S5c 暂为空集合）。
#[cfg(not(target_arch = "wasm32"))]
pub fn build_ftui<W: Widget>(events: BTreeMap<String, crate::view::EventSchema>) -> String {
    let state = W::State::initial();
    let schema = W::State::schema();
    let initial_json = serde_json::to_value(&state).unwrap_or_default();
    let root = W::view(&state);
    let doc = into_document(root, schema, initial_json, events);
    serde_json::to_string(&doc).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod tests {
    // 让 derive 宏生成的 `floatile_sdk::...` 路径解析到本 crate。
    use super::*;
    use crate as floatile_sdk;
    use crate::{Context, Widget, WidgetEvent};

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, crate::State)]
    struct S {
        time: String,
        running: bool,
    }

    struct W;
    impl Widget for W {
        type State = S;
        fn view(_: &S) -> crate::view::View {
            crate::view::column(vec![crate::view::text_bind("$.time")])
        }
        fn start(&mut self, _: &mut Context<Self>) {}
        fn event(&mut self, _: WidgetEvent, _: &mut Context<Self>) {}
    }

    #[test]
    fn builds_ftui_and_validates() {
        let json = build_ftui::<W>(BTreeMap::new());
        let doc: crate::view::UiDocument = serde_json::from_str(&json).unwrap();
        crate::validate_document(&doc).unwrap();
        assert_eq!(doc.root.kind, "Column");
        // State schema 有 time/running 字段。
        match doc.state.schema {
            crate::JsonSchema::Object { properties, .. } => {
                assert!(properties.contains_key("time"));
                assert!(properties.contains_key("running"));
            }
            other => panic!("期望 Object schema，实际 {other:?}"),
        }
    }
}
