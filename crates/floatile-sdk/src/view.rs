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

/// Grid 容器：列数是受验证的静态布局提示。
pub fn grid(columns: u32, children: Vec<View>) -> View {
    let mut props = std::collections::BTreeMap::new();
    props.insert(
        "columns".into(),
        PropValue::Literal(serde_json::json!(columns)),
    );
    View {
        kind: "Grid".into(),
        props,
        children,
        ..Default::default()
    }
}

/// 响应式容器：窗口宽度低于 breakpoint 时纵向，否则横向。
pub fn responsive(breakpoint: f64, children: Vec<View>) -> View {
    View {
        kind: "Responsive".into(),
        props: std::collections::BTreeMap::from([(
            "breakpoint".into(),
            PropValue::Literal(serde_json::json!(breakpoint)),
        )]),
        children,
        ..Default::default()
    }
}

/// 静态 List 容器。
pub fn list(children: Vec<View>) -> View {
    View {
        kind: "List".into(),
        children,
        ..Default::default()
    }
}

/// 动态字符串 List：绑定具有显式 `maxItems` 的 string array State。
pub fn list_bind(path: &str) -> View {
    let mut props = std::collections::BTreeMap::new();
    props.insert(
        "items".into(),
        PropValue::Binding(Binding::State {
            bind: path.to_owned(),
        }),
    );
    View {
        kind: "List".into(),
        props,
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

/// Badge 组件：标签绑定 State 路径，tone 只能使用宿主定义的语义值。
pub fn badge_bind(path: &str, tone: &str) -> View {
    let mut props = std::collections::BTreeMap::new();
    props.insert(
        "label".into(),
        PropValue::Binding(Binding::State {
            bind: path.to_owned(),
        }),
    );
    props.insert(
        "tone".into(),
        PropValue::Literal(serde_json::Value::String(tone.to_owned())),
    );
    View {
        kind: "Badge".into(),
        props,
        ..Default::default()
    }
}

/// Progress 组件：绑定 0..=100 的数值 State 路径。
pub fn progress_bind(path: &str) -> View {
    let mut props = std::collections::BTreeMap::new();
    props.insert(
        "value".into(),
        PropValue::Binding(Binding::State {
            bind: path.to_owned(),
        }),
    );
    View {
        kind: "Progress".into(),
        props,
        ..Default::default()
    }
}

/// Sparkline：绑定有界 number array，并提供屏幕阅读器可用的替代标签。
pub fn sparkline_bind(path: &str, label: &str, tone: &str) -> View {
    let props = std::collections::BTreeMap::from([
        (
            "values".into(),
            PropValue::Binding(Binding::State {
                bind: path.to_owned(),
            }),
        ),
        (
            "label".into(),
            PropValue::Literal(serde_json::Value::String(label.to_owned())),
        ),
        (
            "tone".into(),
            PropValue::Literal(serde_json::Value::String(tone.to_owned())),
        ),
    ]);
    View {
        kind: "Sparkline".into(),
        props,
        ..Default::default()
    }
}

/// If 控制节点：按 boolean State 绑定选择 then/else 分支。
pub fn if_bind(path: &str, then_view: View, else_view: Option<View>) -> View {
    View {
        kind: "If".into(),
        when: Some(Binding::State {
            bind: path.to_owned(),
        }),
        then: Some(Box::new(then_view)),
        else_: else_view.map(Box::new),
        ..Default::default()
    }
}

/// 标准 loading/empty/error/content 四态页面。
///
/// 优先级固定为 loading → error → empty → content，避免多个状态同时为真时
/// 呈现不确定结果。三个路径都必须绑定 boolean State 字段。
pub fn page_state(
    loading_path: &str,
    error_path: &str,
    empty_path: &str,
    loading: View,
    error: View,
    empty: View,
    content: View,
) -> View {
    if_bind(
        loading_path,
        loading,
        Some(if_bind(
            error_path,
            error,
            Some(if_bind(empty_path, empty, Some(content))),
        )),
    )
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn page_state_has_deterministic_priority() {
        let view = page_state(
            "$.loading",
            "$.error",
            "$.empty",
            text_literal("loading"),
            text_literal("error"),
            text_literal("empty"),
            text_literal("content"),
        );
        assert_eq!(view.kind, "If");
        assert_eq!(
            view.when,
            Some(Binding::State {
                bind: "$.loading".into()
            })
        );
        let error_branch = view.else_.expect("page state 必须包含 error 分支");
        assert_eq!(
            error_branch.when,
            Some(Binding::State {
                bind: "$.error".into()
            })
        );
        let empty_branch = error_branch.else_.expect("page state 必须包含 empty 分支");
        assert_eq!(
            empty_branch.when,
            Some(Binding::State {
                bind: "$.empty".into()
            })
        );
    }

    #[test]
    fn badge_and_progress_builders_emit_registry_components() {
        let badge = badge_bind("$.status", "success");
        assert_eq!(badge.kind, "Badge");
        let progress = progress_bind("$.percent");
        assert_eq!(progress.kind, "Progress");
    }

    #[test]
    fn list_and_grid_builders_emit_bounded_layout_contract() {
        let list = list_bind("$.items");
        assert_eq!(list.kind, "List");
        let grid = grid(2, vec![text_literal("one"), text_literal("two")]);
        assert_eq!(grid.kind, "Grid");
        assert_eq!(
            grid.props.get("columns"),
            Some(&PropValue::Literal(serde_json::json!(2)))
        );
    }

    #[test]
    fn sparkline_builder_includes_accessible_label() {
        let sparkline = sparkline_bind("$.trend", "Usage trend", "info");
        assert_eq!(sparkline.kind, "Sparkline");
        assert_eq!(
            sparkline.props.get("label"),
            Some(&PropValue::Literal(serde_json::json!("Usage trend")))
        );
    }

    #[test]
    fn responsive_builder_includes_static_breakpoint() {
        let view = responsive(420.0, vec![text_literal("content")]);
        assert_eq!(view.kind, "Responsive");
        assert_eq!(
            view.props.get("breakpoint"),
            Some(&PropValue::Literal(serde_json::json!(420.0)))
        );
    }
}
