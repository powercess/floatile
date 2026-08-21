//! 集成测试：`WidgetHarness` 作者级测试工具驱动 clock-wasm。
//! 覆盖 start→计时器→State 更新、UI 事件回投、审计捕获、拒绝存活与 fuel trap。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use floatile_core::capability::{CapabilityId, CapabilityParams};
use floatile_core::types::PluginId;
use floatile_runtime::harness::WidgetHarness;
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

fn clock_schema() -> JsonSchema {
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

/// 授权 timer:schedule 的 clock harness（行为契约同 runtime 集成测试）。
fn clock_harness() -> WidgetHarness {
    WidgetHarness::new(PluginId("dev.floatile.clock".into()), clock_wasm_bytes())
        .initial_state(json!({"time": "", "running": false}))
        .state_schema(clock_schema())
        .grant(
            CapabilityId::TimerSchedule,
            Some(CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 4,
            }),
        )
}

#[tokio::test(flavor = "multi_thread")]
async fn start_then_receives_timer_state_update() {
    let mut h = clock_harness().build().unwrap();
    h.start().await.expect("start 应成功");

    // clock 在 start 时 schedule 1000ms 计时器；到期后 on_tick → update-state 带 time。
    let state = h
        .wait_for_state(Duration::from_secs(5), |s| s.get("time").is_some())
        .await
        .expect("5 秒内应收到达 time 的 State 更新");
    let time = state["time"].as_str().unwrap();
    assert_eq!(time.len(), 8, "HH:MM:SS 格式，实际 {time}");

    h.shutdown().await.expect("shutdown 应正常");
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_event_updates_state() {
    let mut h = clock_harness().build().unwrap();
    h.start().await.expect("start 应成功");

    // 先让首个计时器更新到达，再投 UI 事件，串行处理。
    h.wait_for_state(Duration::from_secs(5), |s| s.get("time").is_some())
        .await
        .expect("先收到时间更新");
    h.emit_ui("start", "{}").await.expect("UI 事件应被实例接受");

    let state = h
        .wait_for_state(Duration::from_secs(3), |s| {
            s.get("running") == Some(&json!(true))
        })
        .await
        .expect("Ui(start) → ClockEvent::Start → running=true");
    assert_eq!(state["running"], json!(true));

    h.shutdown().await.expect("shutdown 应正常");
}

#[tokio::test(flavor = "multi_thread")]
async fn audit_records_allow_decision() {
    let mut h = clock_harness().build().unwrap();
    h.start().await.expect("start 应成功");
    h.wait_for_state(Duration::from_secs(5), |s| s.get("time").is_some())
        .await
        .expect("收到时间更新");

    assert!(
        h.assert_audit(|events| events
            .iter()
            .any(|e| e.capability == "timer:schedule" && e.decision == "allow")),
        "应存在 timer:schedule allow 审计，实际: {:?}",
        h.audit()
    );

    h.shutdown().await.expect("shutdown 应正常");
}

#[tokio::test(flavor = "multi_thread")]
async fn count_state_updates_collects_ticks() {
    let mut h = clock_harness().build().unwrap();
    h.start().await.expect("start 应成功");
    // clock 1 Hz 更新；3 秒应至少观察到 1 次。
    let n = h.count_state_updates(Duration::from_secs(3)).await;
    assert!(n >= 1, "clock 应产生至少 1 次 State 更新，实际 {n}");
    h.shutdown().await.expect("shutdown 应正常");
}

#[tokio::test(flavor = "multi_thread")]
async fn denied_capability_survives_harness_and_audits_deny() {
    // 不授予 timer:schedule：clock 调用被 Broker 拒绝（guest 记录并继续）。
    let h = WidgetHarness::new(PluginId("dev.floatile.clock".into()), clock_wasm_bytes())
        .initial_state(json!({"time": "", "running": false}))
        .state_schema(clock_schema())
        .build()
        .unwrap();
    // 拒绝被吞掉，实例仍存活。
    h.start()
        .await
        .expect("start 应成功（timer 拒绝不中断实例）");

    h.advance_time(Duration::from_millis(1200)).await;
    assert!(
        h.assert_audit(|events| events
            .iter()
            .any(|e| e.capability == "timer:schedule" && e.decision == "deny")),
        "应存在 timer:schedule deny 审计，实际: {:?}",
        h.audit()
    );

    h.shutdown().await.expect("实例仍可正常关闭");
}

#[tokio::test(flavor = "multi_thread")]
async fn fuel_exhaustion_fails_instance_but_engine_survives() {
    // fuel=1：构造/start 应 trap，start 返回错误。
    let h = clock_harness().fuel_per_call(1).build().unwrap();
    assert!(h.start().await.is_err(), "fuel=1 应导致实例失败");

    // 宿主/引擎存活：新的正常实例工作。
    let mut h2 = clock_harness().build().unwrap();
    h2.start().await.expect("新实例 start 应成功");
    h2.wait_for_state(Duration::from_secs(5), |s| s.get("time").is_some())
        .await
        .expect("新实例应正常更新");
    h2.shutdown().await.expect("新实例 shutdown 应正常");
}
