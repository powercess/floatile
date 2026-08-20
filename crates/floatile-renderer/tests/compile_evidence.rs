//! renderer 契约/预算/转义集成测试(纯逻辑,离线可运行)。
//!
//! renderer 生成文本的"可编译证据"由 `floatile-shell/build.rs` 在真实 cargo
//! 构建中以 `slint-build` 编译承担(slint-build 仅在 cargo build 上下文可运行,
//! 测试进程内直接调用会返回 `NotRunViaCargo`)。本文件验证 renderer 输出的
//! 契约向量:**生成的组件命名、绑定槽位、预算上限、结构化转义**均与
//! `floatile_ui_schema` 单源一致,不依赖 Slint 编译器。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use floatile_ui_schema::ir::{Binding, Component, PropValue, UiDocument};
use floatile_ui_schema::{UI_API_VERSION, validate_document};
use serde_json::json;

fn doc(root: Component) -> UiDocument {
    UiDocument {
        ui_api_version: UI_API_VERSION.into(),
        state: floatile_ui_schema::ir::StateSchema {
            initial: json!({"time": "00:00:00", "running": false}),
            schema: floatile_ui_schema::JsonSchema::Object {
                required: vec![],
                properties: std::collections::BTreeMap::from([
                    (
                        "time".into(),
                        floatile_ui_schema::JsonSchema::String {
                            max_length: Some(32),
                        },
                    ),
                    ("running".into(), floatile_ui_schema::JsonSchema::Boolean),
                ]),
                additional_properties: false,
            },
        },
        events: std::collections::BTreeMap::new(),
        root,
    }
}

fn text_bind(path: &str) -> Component {
    Component {
        kind: "Text".into(),
        props: std::collections::BTreeMap::from([(
            "text".into(),
            PropValue::Binding(Binding::State { bind: path.into() }),
        )]),
        ..Default::default()
    }
}

fn column(children: Vec<Component>) -> Component {
    Component {
        kind: "Column".into(),
        children,
        ..Default::default()
    }
}

/// renderer 生成的 binding 槽位与生成属性一致(契约向量)。
#[test]
fn rendered_binding_slots_match_generated_source() {
    let d = doc(column(vec![text_bind("$.time")]));
    validate_document(&d).unwrap();
    let rendered = floatile_renderer::render_component(&d).unwrap();
    assert_eq!(rendered.bindings.len(), 1);
    assert_eq!(rendered.bindings[0].path, "$.time");
    assert_eq!(rendered.bindings[0].prop, "prop_time");
    assert!(rendered.source.contains("in property <string> prop_time"));
    assert!(rendered.source.contains("text: root.prop_time;"));
}

/// 恶意 IR(超预算)被拒绝,不生成宿主 UI(validate 层先拦截,renderer 复验兜底)。
#[test]
fn rendered_rejects_budget_exceeded() {
    // 深度 40 的嵌套 Column 超过 MAX_TREE_DEPTH=32;validate_document 先拒绝,
    // renderer 复验作为第二道防线(任一层的拒绝都是安全结果)。
    let mut root = text_bind("$.time");
    for _ in 0..40 {
        root = column(vec![root]);
    }
    let d = doc(root);
    let err = floatile_renderer::render_component(&d).unwrap_err();
    match err.code() {
        "RNDR_INVALID_IR" | "RNDR_BUDGET_EXCEEDED" => {}
        other => panic!("期望拒绝码,实际 {other}"),
    }
}

/// 生成文本不含插件原始拼接:字面量中的引号/换行被结构化转义。
#[test]
fn rendered_escapes_plugin_literals() {
    let root = Component {
        kind: "Text".into(),
        props: std::collections::BTreeMap::from([(
            "text".into(),
            PropValue::Literal(json!("a\"b\nc")),
        )]),
        ..Default::default()
    };
    let d = doc(root);
    let rendered = floatile_renderer::render_component(&d).unwrap();
    assert!(rendered.source.contains(r#"text: "a\"b\nc";"#));
    assert!(!rendered.source.contains("a\"b\nc"));
}

/// 不接受的组件(registry 之外/未映射)稳定拒绝,不产生宿主 UI。
#[test]
fn rendered_rejects_unmapped_components() {
    let root = Component {
        kind: "ListView".into(), // 非 registry 组件
        ..Default::default()
    };
    let d = doc(root);
    let err = floatile_renderer::render_component(&d).unwrap_err();
    assert_eq!(err.code(), "RNDR_INVALID_IR");
}
