//! TypeScript runtime 候选的手动集成证据。
//!
//! 组件由 `spikes/typescript-runtime` 构建；默认门禁不下载 npm 依赖，故这些测试
//! 显式 ignored，由 spike 的 `pnpm test` 在构建组件后执行。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use floatile_core::capability::{
    CapabilityId, CapabilityParams, EffectiveGrant, Grant, Grants, InstanceGrant, TrustLevel,
    narrow_instance,
};
use floatile_core::types::{InstanceId, PluginId};
use floatile_plugin_api::exports::floatile::widget::widget_contract::{UiEvent, WidgetEvent};
use floatile_runtime::harness::WidgetHarness;
use floatile_runtime::{InstanceError, WidgetConfig, WidgetManager};
use serde_json::json;

#[path = "support/clock_behavior.rs"]
mod clock_behavior;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleSuite {
    schema_version: u64,
    engine_api_version: String,
    vectors: Vec<LifecycleVector>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleVector {
    id: String,
    callback: String,
    message: Option<String>,
    expected_host_outcome: String,
}

fn lifecycle_suite() -> LifecycleSuite {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/sdk-lifecycle-v1.json"
    )))
    .expect("lifecycle conformance vectors must parse")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn component_bytes() -> Vec<u8> {
    let path = std::env::var_os("FLOATILE_TYPESCRIPT_CLOCK_WASM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            workspace_root()
                .join("target/typescript-runtime-spike/clock-typescript-starlingmonkey.wasm")
        });
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "读取 TypeScript clock component {} 失败: {error}；先运行 spike 构建",
            path.display()
        )
    })
}

fn harness_builder(with_timer: bool) -> WidgetHarness {
    let builder = WidgetHarness::new(
        PluginId("dev.floatile.clock-typescript-spike".into()),
        component_bytes(),
    )
    .initial_state(json!({"time": "", "running": false}))
    .state_schema(clock_behavior::state_schema());
    if with_timer {
        builder.grant(
            CapabilityId::TimerSchedule,
            Some(CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 4,
            }),
        )
    } else {
        builder
    }
}

fn harness(with_timer: bool) -> floatile_runtime::harness::HarnessInstance {
    harness_builder(with_timer)
        .build()
        .expect("TypeScript clock spawn 应成功")
}

fn timer_grants(instance: InstanceId) -> InstanceGrant {
    let plugin = PluginId("dev.floatile.clock-typescript-spike".into());
    let timer = Grant {
        capability: CapabilityId::TimerSchedule,
        params: Some(CapabilityParams::Timer {
            max_per_minute: 60,
            max_active: 4,
        }),
        effective: EffectiveGrant::DerivedFromInstall,
    };
    narrow_instance(
        &Grants {
            plugin,
            caps: vec![timer.clone()],
            trust: TrustLevel::Dev,
        },
        instance,
        vec![timer],
    )
    .expect("收窄 timer grant")
}

fn widget_config(instance: InstanceId, wasm: Vec<u8>, config_json: &str) -> WidgetConfig {
    WidgetConfig {
        plugin: PluginId("dev.floatile.clock-typescript-spike".into()),
        instance,
        generation: 0,
        wasm,
        initial_state: json!({"time": "", "running": false}),
        state_schema: clock_behavior::state_schema(),
        config_json: config_json.into(),
        grants: timer_grants(instance),
    }
}

async fn measure_instances(count: u64) {
    let wasm = component_bytes();
    let baseline = floatile_platform::process_metrics().ok();
    let manager = WidgetManager::new().expect("创建共享 engine");
    let started = Instant::now();
    let mut handles = Vec::new();
    for id in 1..=count {
        handles.push(
            manager
                .spawn(widget_config(InstanceId(id), wasm.clone(), "{}"))
                .expect("spawn TypeScript clock"),
        );
    }
    for handle in &handles {
        handle.start().await.expect("start TypeScript clock");
    }
    let startup = started.elapsed();

    for handle in &mut handles {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let update = handle.ui_updates().recv().await.expect("UI channel");
                if update
                    .state
                    .get("time")
                    .and_then(|value| value.as_str())
                    .is_some_and(|value| value.len() == 8)
                {
                    break;
                }
            }
        })
        .await
        .expect("4 秒内应收到每个实例的首个 tick");
    }
    let first_tick = started.elapsed();
    let measured = floatile_platform::process_metrics().ok();
    while let Some(handle) = handles.pop() {
        handle.shutdown().await.expect("shutdown TypeScript clock");
    }
    let rss_bytes = measured.map(|snapshot| snapshot.rss_bytes);
    let rss_delta_bytes = baseline
        .zip(measured)
        .map(|(before, after)| after.rss_bytes.saturating_sub(before.rss_bytes));
    println!(
        "{{\"instances\":{count},\"componentBytes\":{},\"startupMs\":{},\"allFirstTicksMs\":{},\"rssBytes\":{},\"rssDeltaBytes\":{}}}",
        wasm.len(),
        startup.as_millis(),
        first_tick.as_millis(),
        rss_bytes.map_or_else(|| "null".into(), |value| value.to_string()),
        rss_delta_bytes.map_or_else(|| "null".into(), |value| value.to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires spikes/typescript-runtime pnpm build"]
async fn typescript_clock_matches_reference_behavior() {
    clock_behavior::assert_reference_behavior(harness(true)).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires spikes/typescript-runtime pnpm build"]
async fn typescript_clock_denied_timer_is_brokered_and_instance_survives() {
    clock_behavior::assert_timer_denied(harness(false)).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires spikes/typescript-runtime pnpm build"]
async fn typescript_clock_lifecycle_errors_match_shared_vectors() {
    let wasm = component_bytes();
    let manager = WidgetManager::new().expect("创建共享 engine");
    let suite = lifecycle_suite();
    assert_eq!(suite.schema_version, 1);
    assert_eq!(
        suite.engine_api_version,
        floatile_plugin_api::ENGINE_API_VERSION
    );

    for (index, vector) in suite.vectors.iter().enumerate() {
        let instance = InstanceId(201 + u64::try_from(index).expect("bounded vector index"));
        let handle = manager
            .spawn(widget_config(
                instance,
                wasm.clone(),
                &serde_json::json!({"mode": format!("conformance-{}", vector.id)}).to_string(),
            ))
            .expect("TypeScript conformance fixture spawn");
        let result = if vector.callback == "start" {
            handle.start().await
        } else {
            handle.start().await.expect("event vector should start");
            handle
                .handle_event(WidgetEvent::Ui(UiEvent {
                    name: "trigger".into(),
                    payload_json: "{}".into(),
                }))
                .await
        };
        assert_eq!(vector.expected_host_outcome, "rejected");
        assert!(
            matches!(result, Err(InstanceError::Rejected(ref message))
                if vector.message.as_ref().is_none_or(|expected| message.contains(expected))),
            "TypeScript conformance vector {} should remain a guest rejection, got {result:?}",
            vector.id
        );
    }

    let survivor = manager
        .spawn(widget_config(InstanceId(209), wasm, "{}"))
        .expect("survivor spawn");
    survivor
        .start()
        .await
        .expect("host survives TypeScript rejection");
    survivor.shutdown().await.expect("survivor should stop");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires spikes/typescript-runtime pnpm build"]
async fn typescript_clock_timeout_isolated_from_peer() {
    let wasm = component_bytes();
    let manager = WidgetManager::new().expect("创建共享 engine");
    let looping = manager
        .spawn(widget_config(
            InstanceId(101),
            wasm.clone(),
            r#"{"mode":"loop"}"#,
        ))
        .expect("loop fixture spawn");
    let peer = manager
        .spawn(widget_config(InstanceId(102), wasm, "{}"))
        .expect("peer spawn");
    assert!(looping.start().await.is_err(), "无限循环必须被预算终止");

    peer.start().await.expect("peer start 应成功");
    peer.shutdown().await.expect("peer shutdown 应成功");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires spikes/typescript-runtime pnpm build"]
async fn typescript_clock_memory_limit_isolated_from_peer() {
    let constrained = harness_builder(true)
        .max_memory(1024 * 1024)
        .build()
        .expect("低内存 fixture spawn");
    assert!(
        constrained.start().await.is_err(),
        "低于 JS runtime 所需的线性内存必须失败"
    );

    let peer = harness(true);
    peer.start().await.expect("peer start 应成功");
    peer.shutdown().await.expect("peer shutdown 应成功");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual resource evidence; requires TypeScript component"]
async fn resource_evidence_single_instance() {
    measure_instances(1).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual resource evidence; requires TypeScript component"]
async fn resource_evidence_ten_instances() {
    measure_instances(10).await;
}
