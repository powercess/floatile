#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use floatile_core::capability::{
    CapabilityId, CapabilityParams, EffectiveGrant, Grant, Grants, TrustLevel, narrow_instance,
};
use floatile_core::types::{InstanceId, PluginId};
use floatile_plugin_api::exports::floatile::widget::widget_contract::{UiEvent, WidgetEvent};
use floatile_runtime::{WidgetConfig, WidgetManager};
use floatile_shell::{project_plugin_ui, resolve_plugin_view_state};
use floatile_ui_schema::UiDocument;
use floatile_ui_schema::schema::JsonSchema;
use serde_json::json;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn clock_wasm_bytes() -> Vec<u8> {
    let wasm_path = workspace_root().join("target/wasm32-wasip2/debug/floatile_clock_wasm.wasm");
    if !wasm_path.exists() {
        let status = Command::new("cargo")
            .current_dir(workspace_root())
            .args([
                "build",
                "-p",
                "floatile-clock-wasm",
                "--target",
                "wasm32-wasip2",
            ])
            .status()
            .expect("failed to run cargo build for clock-wasm");
        assert!(status.success(), "clock-wasm 构建失败");
    }
    std::fs::read(&wasm_path).expect("读取 clock-wasm 失败")
}

fn clock_ftui_document() -> UiDocument {
    let json = floatile_clock_wasm::__floatile_ftui_json();
    serde_json::from_str(&json).expect("clock ftui JSON 应可解析")
}

fn clock_grants() -> floatile_core::InstanceGrant {
    let plugin = Grants {
        plugin: PluginId("dev.floatile.clock".into()),
        trust: TrustLevel::Dev,
        caps: vec![Grant {
            capability: CapabilityId::TimerSchedule,
            params: Some(CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 4,
            }),
            effective: EffectiveGrant::DerivedFromInstall,
        }],
    };
    narrow_instance(
        &plugin,
        InstanceId(1),
        vec![Grant {
            capability: CapabilityId::TimerSchedule,
            params: Some(CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 4,
            }),
            effective: EffectiveGrant::DerivedFromInstall,
        }],
    )
    .unwrap()
}

fn clock_state_schema() -> JsonSchema {
    JsonSchema::Object {
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
    }
}

fn spawn_clock() -> floatile_runtime::WidgetHandle {
    let manager = WidgetManager::new().expect("引擎创建失败");
    let config = WidgetConfig {
        plugin: PluginId("dev.floatile.clock".into()),
        instance: InstanceId(1),
        wasm: clock_wasm_bytes(),
        initial_state: json!({"time": "", "running": false}),
        state_schema: clock_state_schema(),
        config_json: "{}".into(),
        grants: clock_grants(),
    };
    manager.spawn(config).expect("spawn 失败")
}

#[test]
fn host_generated_clock_ftui_projects_to_shell_text_binding() {
    let doc = clock_ftui_document();
    let projection = project_plugin_ui(&doc).expect("clock ftui 应能投影为 shell 文本");
    assert_eq!(projection.text_binding, "$.time");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_clock_updates_resolve_through_shell_projection() {
    let projection = project_plugin_ui(&clock_ftui_document()).expect("clock ftui 应可投影");
    let mut handle = spawn_clock();
    handle.start().await.expect("start 应成功");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut projected = None;
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            maybe = handle.ui_updates().recv() => {
                match maybe {
                    Some(update) => {
                        if update.state.get("time").is_some() {
                            let view = resolve_plugin_view_state(&projection, &update.state)
                                .expect("shell 应能解析 runtime 状态");
                            projected = Some(view.time_text);
                            break;
                        }
                    }
                    None => panic!("UI 通道关闭"),
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }

    let text = projected.expect("4 秒内未收到可投影的 time 更新");
    assert_eq!(text.len(), 8, "HH:MM:SS 格式，实际 {text}");

    handle
        .handle_event(WidgetEvent::Ui(UiEvent {
            name: "start".into(),
            payload_json: "{}".into(),
        }))
        .await
        .expect("ui event 应成功");

    let running_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut got_running = false;
    while tokio::time::Instant::now() < running_deadline {
        tokio::select! {
            maybe = handle.ui_updates().recv() => {
                match maybe {
                    Some(update) => {
                        if update.state.get("running").is_some_and(|v| v == &json!(true)) {
                            let view = resolve_plugin_view_state(&projection, &update.state)
                                .expect("running patch 后仍可投影");
                            assert!(!view.time_text.is_empty() || update.state["time"] == json!(""));
                            got_running = true;
                            break;
                        }
                    }
                    None => panic!("UI 通道关闭（typed event 链）"),
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }
    assert!(got_running, "2 秒内未收到 running=true 更新");

    handle.shutdown().await.expect("shutdown 应正常返回");
}
