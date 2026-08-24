//! 运行时第三方插件 UI 渲染集成测试（ADR-0002 实现切片，Xvfb 门禁）。
//!
//! 无头 CI（无 DISPLAY/WAYLAND）下整体 SKIP 并留痕（窗口实例化需要窗口 backend）；
//! 在 Xvfb/真实桌面上验证 ADR-0002 spike-3 判据之上的宿主 API 全链：
//! `render_ftui → compile_component → RuntimePluginWindow::create_on_ui_thread →
//! project_state → register_events`，即「已安装插件 UI 在运行时编译成宿主窗口、
//! 同包双窗口 State 隔离、binding 槽位 State 投影、输入事件回投」四条断言。
//!
//! 注意保持单一 `#[test]`：Slint 事件循环每进程只可创建一次（`EventLoop can't be
//! recreated`），多个测试在同一二进制里实例化窗口会冲突；窗口路径的证据都收敛到
//! 这一个用例。renderer 输出契约一致性由 `floatile-renderer` 契约测试与
//! `runtime_render_spike` 承担；F12 恶意 IR 拒绝由 `floatile_shell::runtime_ui`
//! 单测（headless）承担。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 集成测试：局部、清晰。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use floatile_platform::capability::probe;
use floatile_shell::runtime_ui::{
    PLUGIN_COMPONENT_NAME, RuntimePluginWindow, compile_component, render_ftui,
};
use floatile_ui_schema::ir::{Component, EventSchema, PropValue, UiDocument};
use floatile_ui_schema::schema::JsonSchema;
use slint_interpreter::Value;

fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// 构造带 State 绑定 + 一个声明输入事件的 IR（与参考时钟同源，事件用于回投验证）。
fn widget_doc() -> UiDocument {
    UiDocument {
        ui_api_version: floatile_ui_schema::UI_API_VERSION.into(),
        state: floatile_ui_schema::ir::StateSchema {
            initial: serde_json::json!({"time": "00:00:00", "running": false}),
            schema: JsonSchema::Object {
                required: vec![],
                properties: BTreeMap::from([
                    (
                        "time".into(),
                        JsonSchema::String {
                            max_length: Some(32),
                        },
                    ),
                    ("running".into(), JsonSchema::Boolean),
                ]),
                additional_properties: false,
            },
        },
        events: BTreeMap::from([(
            "toggle".into(),
            EventSchema {
                payload: JsonSchema::Object {
                    required: vec![],
                    properties: BTreeMap::new(),
                    additional_properties: false,
                },
            },
        )]),
        root: Component {
            kind: "Column".into(),
            children: vec![
                Component {
                    kind: "Text".into(),
                    props: BTreeMap::from([(
                        "text".into(),
                        PropValue::Binding(floatile_ui_schema::ir::Binding::State {
                            bind: "$.time".into(),
                        }),
                    )]),
                    ..Default::default()
                },
                Component {
                    kind: "Button".into(),
                    props: BTreeMap::from([(
                        "label".into(),
                        PropValue::Literal(serde_json::json!("Go")),
                    )]),
                    events: BTreeMap::from([(
                        "activate".into(),
                        floatile_ui_schema::ir::EmittedEvent {
                            emit: "toggle".into(),
                            payload: serde_json::json!({}),
                        },
                    )]),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    }
}

#[test]
fn runtime_window_renders_projects_state_and_forwards_events() {
    if !has_display() {
        eprintln!("SKIP: runtime window test needs a display backend");
        return;
    }

    let doc = widget_doc();
    let ui_bytes = serde_json::to_vec(&doc).unwrap();
    let rendered = render_ftui(&ui_bytes).expect("合法 IR 应渲染");
    assert_eq!(rendered.bindings.len(), 1);
    let prop = rendered.bindings[0].prop.clone();
    let callback = rendered.events[0].callback.clone();

    // 编译 + 实例化为宿主窗口（自窗口接入形态，非实验性 ComponentContainer）。
    let definition = compile_component(&rendered).expect("interpreter 运行时编译");
    assert_eq!(definition.name(), PLUGIN_COMPONENT_NAME);
    let caps = probe();
    let window =
        RuntimePluginWindow::create_on_ui_thread(&definition, rendered.bindings.clone(), &caps)
            .expect("interpreter 自窗口应可实例化");
    let second_window =
        RuntimePluginWindow::create_on_ui_thread(&definition, rendered.bindings.clone(), &caps)
            .expect("同一插件定义应可实例化第二个独立窗口");

    // 沿 binding 槽位投影权威 State → 再读出确认往返（ADR-0002 spike-3 断言之一）。
    window
        .project_state(&serde_json::json!({"time": "13:37:00", "running": true}))
        .expect("投影应成功");
    second_window
        .project_state(&serde_json::json!({"time": "08:00:00", "running": false}))
        .expect("第二实例投影应成功");
    let projected = window
        .instance()
        .get_property(&prop)
        .expect("binding 槽位可读");
    assert_eq!(
        projected,
        Value::String("13:37:00".into()),
        "binding 槽位应投影到权威 State 文本"
    );
    let second_projected = second_window
        .instance()
        .get_property(&prop)
        .expect("第二实例 binding 槽位可读");
    assert_eq!(
        second_projected,
        Value::String("08:00:00".into()),
        "同一插件的两个窗口必须保留各自独立 State"
    );
    assert_eq!(
        window.instance().get_property(&prop).unwrap(),
        Value::String("13:37:00".into()),
        "第二实例投影不得改写第一实例窗口"
    );

    // 跨线程弱引用可取得（worker 投影路径前置）。
    let _weak: slint::Weak<slint_interpreter::ComponentInstance> = window.weak();

    // 声明事件 → interpreter callback → sink；事件名来自 renderer 槽位，非插件自由文本。
    let received = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let sink = {
        let received = Arc::clone(&received);
        Arc::new(move |name: &str, payload: String| {
            received.lock().unwrap().push((name.to_owned(), payload));
        })
    };
    window
        .register_events(&rendered.events, sink)
        .expect("事件回调注册");
    let _: Value = window
        .instance()
        .invoke(&callback, &[])
        .expect("生成的输入事件回调应可触发");
    let got = received.lock().unwrap();
    assert_eq!(got.len(), 1, "事件应回投一次");
    assert_eq!(got[0].0, "toggle", "事件名来自 renderer 槽位");
}
