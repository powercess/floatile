//! Rust/TypeScript Clock 共用行为与权限向量。

use std::collections::BTreeMap;
use std::time::Duration;

use floatile_runtime::harness::HarnessInstance;
use floatile_ui_schema::schema::JsonSchema;
use serde_json::json;

pub fn state_schema() -> JsonSchema {
    JsonSchema::Object {
        required: vec!["time".into(), "running".into()],
        properties: BTreeMap::from([
            (
                "time".into(),
                JsonSchema::String {
                    max_length: Some(64),
                },
            ),
            ("running".into(), JsonSchema::Boolean),
        ]),
        additional_properties: false,
    }
}

pub async fn assert_reference_behavior(mut clock: HarnessInstance) {
    clock.start().await.expect("start 应成功");
    let tick = clock
        .wait_for_state(Duration::from_secs(5), |state| {
            state
                .get("time")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value.len() == 8)
        })
        .await
        .expect("应收到 HH:MM:SS tick");
    assert_eq!(tick["time"].as_str().expect("time string").len(), 8);

    clock
        .emit_ui("start", "{}")
        .await
        .expect("UI start 事件应成功");
    let running = clock
        .wait_for_state(Duration::from_secs(3), |state| {
            state.get("running") == Some(&json!(true))
        })
        .await
        .expect("应收到 running=true");
    assert_eq!(running["running"], json!(true));
    clock.shutdown().await.expect("shutdown 应成功");
}

pub async fn assert_timer_denied(clock: HarnessInstance) {
    clock
        .start()
        .await
        .expect("timer 拒绝由 guest 降级，实例应继续存活");
    clock.advance_time(Duration::from_millis(1200)).await;
    assert!(
        clock.assert_audit(|events| events
            .iter()
            .any(|event| { event.capability == "timer:schedule" && event.decision == "deny" })),
        "应存在 timer:schedule deny 审计，实际: {:?}",
        clock.audit()
    );
    clock.shutdown().await.expect("实例仍可正常关闭");
}
