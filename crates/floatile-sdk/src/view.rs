//! `View` builder — 声明式 UI 组件树构建，编译期确定。
//!
//! `View` 是 `floatile_ui_schema::Component` 的类型别名；View builder
//! 提供符合 P0 组件集的便利构造器。运行期插件不调用 View（运行期只有 State
//! Patch 流向宿主 UI）；View 用于构建时生成 `widget.ftui` 文档。

use crate::JsonSchema;
pub use floatile_ui_schema::Component as View;
pub use floatile_ui_schema::ir::{Binding, EventSchema, PropValue, StateSchema, UiDocument};

/// Column 容器。
pub fn column(children: Vec<View>) -> View {
    View {
        kind: "Column".into(),
        children,
        ..Default::default()
    }
}

/// Row 容器。
pub fn row(children: Vec<View>) -> View {
    View {
        kind: "Row".into(),
        children,
        ..Default::default()
    }
}

/// Stack 容器。
pub fn stack(children: Vec<View>) -> View {
    View {
        kind: "Stack".into(),
        children,
        ..Default::default()
    }
}

/// Text 组件：绑定 State 路径。
pub fn text_bind(path: &str) -> View {
    let mut props = std::collections::BTreeMap::new();
    props.insert(
        "text".into(),
        PropValue::Binding(Binding::State {
            bind: path.to_owned(),
        }),
    );
    View {
        kind: "Text".into(),
        props,
        ..Default::default()
    }
}

/// Text 组件：字面量。
pub fn text_literal(s: &str) -> View {
    let mut props = std::collections::BTreeMap::new();
    props.insert(
        "text".into(),
        PropValue::Literal(serde_json::Value::String(s.to_owned())),
    );
    View {
        kind: "Text".into(),
        props,
        ..Default::default()
    }
}

/// Button 组件。
pub fn button(label: &str) -> View {
    let mut props = std::collections::BTreeMap::new();
    props.insert(
        "label".into(),
        PropValue::Literal(serde_json::Value::String(label.to_owned())),
    );
    View {
        kind: "Button".into(),
        props,
        ..Default::default()
    }
}

/// 设置布局 props（padding、gap 等）。
pub fn with_props(
    mut view: View,
    props: impl IntoIterator<Item = (&'static str, PropValue)>,
) -> View {
    for (k, v) in props {
        view.props.insert(k.into(), v);
    }
    view
}

/// 从 View（根组件）+ State schema + 事件声明 构造 `widget.ftui` 文档。
pub fn into_document(
    root: View,
    schema: JsonSchema,
    initial_state_json: serde_json::Value,
    events: std::collections::BTreeMap<String, EventSchema>,
) -> UiDocument {
    UiDocument {
        ui_api_version: floatile_ui_schema::UI_API_VERSION.to_owned(),
        state: StateSchema {
            initial: initial_state_json,
            schema,
        },
        events,
        root,
    }
}
