//! ADR-0002 spike-2（被否决接入形态的否定证据保留）。
//!
//! 评估过把 interpreter 组件经 `slint::ComponentFactory` 注入宿主静态窗口的
//! `ComponentContainer` 嵌入路径。已确认该路径**技术上可编译**（interpreter 实例
//! 满足 `ComponentFactory::new` 的 `HasStaticVTable + ComponentHandle` 约束），但
//! `ComponentContainer` 是 Slint 1.17 实验性 API（生产 `builtin()` 显式移除，需
//! `SLINT_ENABLE_EXPERIMENTAL_FEATURES` 编译环境变量），ADR-0002 决策 2 已**拒绝**
//! 该路径。本文件保留「桥接可编译」证据并验证 interpreter 实例自带独立窗口
//! （支持最终采用的自窗口形态）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // spike 测试：局部、清晰。

use floatile_renderer::render_component;
use floatile_ui_schema::ir::{Binding, Component, PropValue, UiDocument};
use floatile_ui_schema::{UI_API_VERSION, validate_document};
use serde_json::json;

fn clock_doc() -> UiDocument {
    let doc = UiDocument {
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
        root: Component {
            kind: "Column".into(),
            children: vec![Component {
                kind: "Text".into(),
                props: std::collections::BTreeMap::from([(
                    "text".into(),
                    PropValue::Binding(Binding::State {
                        bind: "$.time".into(),
                    }),
                )]),
                ..Default::default()
            }],
            ..Default::default()
        },
    };
    assert!(validate_document(&doc).is_ok());
    doc
}

fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[test]
fn host_window_can_embed_interpreter_factory() {
    let doc = clock_doc();
    let rendered = render_component(&doc).expect("renderer 应生成插件 UI 源码");

    // 运行时编译 renderer 输出。
    let compiler = slint_interpreter::Compiler::default();
    let compiled = tokio::runtime::Runtime::new()
        .expect("spike 需要 tokio runtime")
        .block_on(compiler.build_from_source(rendered.source.clone(), "runtime-plugin-ui".into()));
    let definition = compiled
        .component("ClockPluginUI")
        .expect("interpreter 应编译 renderer 输出");

    // 把 interpreter 的 ComponentDefinition 包成 `slint::ComponentFactory`，试图注入
    // 宿主静态窗口的 ComponentContainer。本测试只验证"桥接能否编译/构造"这一前置条件，
    // 实例化嵌入需窗口 backend（见 SKIP 分支）。
    let factory = slint::ComponentFactory::new(
        move |_ctx| -> Option<slint_interpreter::ComponentInstance> { definition.create().ok() },
    );
    // 若此处报 HasStaticVTable 约束错误，说明 interpreter 组件无法作为静态宿主
    // 窗口的子组件——接入形态需改为整窗 interpreter 或其他方案。
    let _ = factory;
}

// 备用判据：整窗 interpreter 自窗口能否作为"内容承载"（若上面桥接失败则走此形态）。
#[test]
fn interpreter_instance_has_own_window() {
    if !has_display() {
        eprintln!("SKIP: no DISPLAY/WAYLAND; own-window check needs a display");
        return;
    }
    let doc = clock_doc();
    let rendered = render_component(&doc).expect("renderer 生成");
    let compiler = slint_interpreter::Compiler::default();
    let compiled = tokio::runtime::Runtime::new()
        .expect("rt")
        .block_on(compiler.build_from_source(rendered.source.clone(), "runtime-ui".into()));
    let definition = compiled
        .component("ClockPluginUI")
        .expect("interpreter 编译");
    let instance = definition.create().expect("实例化");
    // renderer 输出是内容组件（Rectangle, 非 Window）。interpreter 会为它建窗口。
    use slint_interpreter::ComponentHandle;
    let _ = instance.window();
    eprintln!("interpreter instance has its own window adapter (content component)");
}
