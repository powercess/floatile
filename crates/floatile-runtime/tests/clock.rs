//! 集成测试：加载 clock-wasm，执行 lifecycle 与 State Patch 链路。
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
use floatile_ui_schema::schema::JsonSchema;
use serde_json::json;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 读取 clock-wasm 组件；未构建时先构建。
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

fn clock_grants(instance: u64) -> floatile_core::InstanceGrant {
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
        InstanceId(instance),
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
        generation: 0,
        wasm: clock_wasm_bytes(),
        initial_state: json!({"time": "", "running": false}),
        state_schema: clock_state_schema(),
        config_json: "{}".into(),
        grants: clock_grants(1),
    };
    manager.spawn(config).expect("spawn 失败")
}

#[tokio::test(flavor = "multi_thread")]
async fn loads_starts_and_receives_state_update() {
    let mut handle = spawn_clock();

    // constructor + start。
    if let Err(e) = handle.start().await {
        let actor_err = handle.into_result().await;
        panic!("start 应成功，实际 {e:?}；actor: {actor_err:?}");
    }

    // clock 在 start 时 schedule 1000ms 计时器；到期后 on_tick → update-state。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut got_update = false;
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            maybe = handle.ui_updates().recv() => {
                match maybe {
                    Some(update) => {
                        if update.state.get("time").is_some() {
                            got_update = true;
                            let time = update.state["time"].as_str().unwrap();
                            assert_eq!(time.len(), 8, "HH:MM:SS 格式，实际 {time}");
                            break;
                        }
                    }
                    None => panic!("UI 通道关闭"),
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }
    assert!(got_update, "4 秒内未收到带 time 的 State 更新");

    // UI 事件路径：typed 事件桥验证。
    // Ui{name:"start"} → ClockEvent::Start → ctx.state().update(r#"{"running":true}"#) → UiUpdate。
    handle
        .handle_event(WidgetEvent::Ui(UiEvent {
            name: "start".into(),
            payload_json: "{}".into(),
        }))
        .await
        .expect("ui event 应成功");

    // 断言 typed 事件链产出 running=true 的 State 更新。
    let running_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut got_running = false;
    while tokio::time::Instant::now() < running_deadline {
        tokio::select! {
            maybe = handle.ui_updates().recv() => {
                match maybe {
                    Some(update) => {
                        if update.state.get("running").is_some_and(|v| v == &json!(true)) {
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
    assert!(
        got_running,
        "typed 事件链：Ui(start) → ClockEvent::Start → running=true 未在 2 秒内收到"
    );

    handle.shutdown().await.expect("shutdown 应正常返回");
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_is_clean() {
    let handle = spawn_clock();
    handle.start().await.expect("start 应成功");
    handle.shutdown().await.expect("shutdown 应正常返回");
}

/// 无 timer grant 时：clock 调用 host-timer 被 Broker 拒绝（guest 记录并继续），
/// 宿主与实例都存活，start 仍成功。
#[tokio::test(flavor = "multi_thread")]
async fn denied_capability_does_not_kill_host() {
    let plugin = Grants {
        plugin: PluginId("dev.floatile.clock".into()),
        trust: TrustLevel::Dev,
        caps: vec![], // 不授予 timer:schedule
    };
    let grants = narrow_instance(&plugin, InstanceId(2), vec![]).unwrap();
    let manager = WidgetManager::new().unwrap();
    let config = WidgetConfig {
        plugin: PluginId("dev.floatile.clock".into()),
        instance: InstanceId(2),
        generation: 0,
        wasm: clock_wasm_bytes(),
        initial_state: json!({"time": "", "running": false}),
        state_schema: clock_state_schema(),
        config_json: "{}".into(),
        grants,
    };
    let handle = manager.spawn(config).expect("spawn 失败");
    // timer 被拒：clock 在 start 里 schedule 失败仅记录日志，不中断实例。
    handle
        .start()
        .await
        .expect("start 应成功（timer 拒绝被吞掉）");
    // 实例仍可用：正常关闭。
    handle.shutdown().await.expect("shutdown 应正常返回");
}

/// fuel 预算耗尽 → 实例 trap 终止；同引擎仍可派生新实例（宿主存活）。
#[tokio::test(flavor = "multi_thread")]
async fn fuel_exhaustion_kills_instance_but_host_survives() {
    let bad = WidgetManager::new().unwrap().with_fuel_per_call(1);
    let config = WidgetConfig {
        plugin: PluginId("dev.floatile.clock".into()),
        instance: InstanceId(3),
        generation: 0,
        wasm: clock_wasm_bytes(),
        initial_state: json!({"time": "", "running": false}),
        state_schema: clock_state_schema(),
        config_json: "{}".into(),
        grants: clock_grants(3),
    };
    let handle = bad.spawn(config).expect("spawn 应成功（组件可解析）");
    let result = handle.start().await;
    assert!(
        matches!(result, Err(floatile_runtime::InstanceError::Failed(_))),
        "fuel=1 应导致实例失败，实际 {result:?}"
    );

    // 宿主存活：同引擎（默认燃料）再派生一个实例正常工作。
    let manager = WidgetManager::new().unwrap();
    let handle2 = manager
        .spawn(WidgetConfig {
            plugin: PluginId("dev.floatile.clock".into()),
            instance: InstanceId(4),
            generation: 0,
            wasm: clock_wasm_bytes(),
            initial_state: json!({"time": "", "running": false}),
            state_schema: clock_state_schema(),
            config_json: "{}".into(),
            grants: clock_grants(4),
        })
        .expect("新实例 spawn 成功");
    if let Err(start_error) = handle2.start().await {
        let actor_result = handle2.into_result().await;
        panic!("新实例 start 应成功: {start_error}; actor={actor_result:?}");
    }
    handle2.shutdown().await.expect("新实例 shutdown 正常");
}
