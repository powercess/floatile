//! ADR-0002 spike：运行时用 `slint-interpreter` 编译 renderer 生成的插件 UI 源码。
//!
//! 证明的事实（ADR 证据）：
//! 1. renderer 输出的 `component PluginUI inherits Rectangle { in property <string> … }`
//!    可以由 `slint_interpreter::ComponentCompiler` 在进程内**运行时**编译（不依赖
//!    `slint!` 宏的构建期嵌入、不依赖 slint-build）。
//! 2. 编译产物在窗口 backend 可用时可实例化，绑定槽位属性可写、可读——即
//!    「运行时把任意已验证 widget.ftui 渲染进窗口」所需的最小能力成立。
//! 3. 该 spike 使用与宿主一致的 `slint`/`slint-interpreter` 1.17 系列与 renderer 输出，
//!    与现有 `i-slint-compiler` 依赖链重叠，无新增许可面（见 ADR-0002 §依赖）。
//!
//! 无头 CI（无 DISPLAY/WAYLAND）下：断言 1 必须通过（纯编译，无 backend 需求）；
//! 断言 2 需要窗口 backend，无显示环境会**明确跳过并留痕**（spike 实验证据，
//! 非生产门禁），在 Xvfb/真实桌面上全量验证。

#![allow(clippy::unwrap_used, clippy::expect_used)] // spike 测试：局部、清晰。

use floatile_renderer::render_component;
use floatile_ui_schema::ir::{Binding, Component, PropValue, UiDocument};
use floatile_ui_schema::{UI_API_VERSION, validate_document};
use serde_json::json;

/// 构造与参考时钟等价的最小 IR（与 floatile-renderer 契约测试同源）。
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
    assert!(validate_document(&doc).is_ok(), "spike IR 应通过结构校验");
    doc
}

fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[test]
fn runtime_compiles_renderer_output() {
    let doc = clock_doc();
    let rendered = render_component(&doc).expect("renderer 应生成插件 UI 源码");
    assert_eq!(rendered.bindings.len(), 1);
    assert_eq!(rendered.bindings[0].path, "$.time");

    // ADR-0002 核心断言：renderer 输出的源码可在运行时（非构建期）编译。
    let compiler = slint_interpreter::Compiler::default();
    let compiled = tokio::runtime::Runtime::new()
        .expect("spike 需要 tokio runtime")
        .block_on(compiler.build_from_source(rendered.source.clone(), "runtime-plugin-ui".into()));
    let definition = compiled
        .component("ClockPluginUI")
        .expect("slint-interpreter 应在运行时编译 renderer 生成的源码并暴露组件");
    assert_eq!(
        definition.name(),
        "ClockPluginUI",
        "编译产物应为 renderer 声明的宿主组件名"
    );
}

#[test]
fn runtime_instance_projects_state() {
    if !has_display() {
        // 无头环境：实例化需要窗口 backend。明确跳过并留痕（Xvfb/桌面环境全量验证）。
        eprintln!(
            "SKIP: no DISPLAY/WAYLAND; instance projection verified under Xvfb (see ADR-0002 §验证)"
        );
        return;
    }
    let doc = clock_doc();
    let rendered = render_component(&doc).expect("renderer 应生成插件 UI 源码");

    let compiler = slint_interpreter::Compiler::default();
    let compiled = tokio::runtime::Runtime::new()
        .expect("spike 需要 tokio runtime")
        .block_on(compiler.build_from_source(rendered.source.clone(), "runtime-plugin-ui".into()));
    let definition = compiled
        .component("ClockPluginUI")
        .expect("slint-interpreter 应在运行时编译 renderer 生成的源码并暴露组件");

    // 沿 renderer binding 槽位投影权威 State（与 shell 静态接线同一语义）。
    let instance = definition.create().expect("组件应可实例化");
    let prop = rendered.bindings[0].prop.as_str();
    instance
        .set_property(prop, slint_interpreter::Value::String("12:34:56".into()))
        .expect("binding 槽位属性应可写");
    let value = instance.get_property(prop).expect("binding 槽位属性应可读");
    assert_eq!(
        value,
        slint_interpreter::Value::String("12:34:56".into()),
        "投影后的属性值应等于写入的 State"
    );
}
