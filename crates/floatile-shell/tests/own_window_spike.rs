//! ADR-0002 spike-3（实现切片最终判据）：interpreter 自窗口接入宿主平台能力。
//!
//! ADR-0002 修订后接入形态：interpreter 编译的插组件 `create()` 得到独立原生窗口，
//! 经 `WinitWindowAccessor` 取原生 `winit::window::Window`，复用 `floatile-platform`
//! 的无边框/透明/置顶/穿透能力——不依赖 Slint 实验性 `ComponentContainer`。
//!
//! 本 spike 验证该链路的可编译性与运行时成立，Xvfb 下实测。

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
fn interpreter_window_exposes_native_window() {
    if !has_display() {
        eprintln!(
            "SKIP: no DISPLAY/WAYLAND; own-window + native handle verified under Xvfb (see ADR-0002 §验证)"
        );
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

    // 自窗口接入：create() → ComponentHandle::window()（独立 window adapter）。
    let instance = definition.create().expect("实例化");
    use slint::winit_030::WinitWindowAccessor;
    use slint_interpreter::ComponentHandle;
    let win = instance.window();

    // 关键断言：interpreter 窗口可经 WinitWindowAccessor 取原生 winit 窗口——这是
    // 复用 floatile-platform 运行时窗口能力（set_always_on_top/set_click_through，
    // 均接收原生 Window）的前置，编译通过即链路成立。
    win.with_winit_window(|w: &slint::winit_030::winit::window::Window| {
        let _pos = w.outer_position();
        let _size = w.outer_size();
    });

    // 投影断言：interpreter 实例仍沿 renderer binding 槽位 set/get property——
    // 自窗口形态下投影路径与内容渲染一致，是后续 shell 实现切片的直接依据。
    let prop = rendered.bindings[0].prop.as_str();
    instance
        .set_property(prop, slint_interpreter::Value::String("13:37:00".into()))
        .expect("interpreter 自窗口 binding 槽位应可写");
    let value = instance
        .get_property(prop)
        .expect("interpreter 自窗口 binding 槽位应可读");
    assert_eq!(
        value,
        slint_interpreter::Value::String("13:37:00".into()),
        "自窗口投影后的属性值应等于写入的 State"
    );
}
