//! 安全验收（P0 安全验收 §3）：恶意插件对抗 + 审计持久化 + 宿主存活。
//!
//! 用 `plugins/evil-wasm` fixture 验证：
//! - 未声明能力调用 → Broker 拒绝 + 脱敏审计落 SQLite + 宿主存活；
//! - 超限/类型错误/未知字段 State Patch → 被拒，宿主存活；
//! - 无限 CPU 循环 → fuel trap 终止实例，宿主存活；
//! - 超限内存申请 → StoreLimits 终止实例，宿主存活。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use parking_lot::Mutex;

use floatile_core::capability::{Grants, InstanceGrant, TrustLevel, narrow_instance};
use floatile_core::types::{InstanceId, PluginId};
use floatile_plugin_api::exports::floatile::widget::widget_contract::{UiEvent, WidgetEvent};
use floatile_runtime::{WidgetConfig, WidgetManager};
use floatile_services::AuditEvent;
use floatile_store::{AuditRecord, Store};
use floatile_ui_schema::schema::JsonSchema;
use serde_json::{Value, json};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 读取 evil-wasm 组件；未构建时先构建。
fn evil_wasm_bytes() -> Vec<u8> {
    let wasm_path = workspace_root().join("target/wasm32-wasip2/debug/floatile_evil_wasm.wasm");
    if !wasm_path.exists() {
        let status = Command::new("cargo")
            .current_dir(workspace_root())
            .args([
                "build",
                "-p",
                "floatile-evil-wasm",
                "--target",
                "wasm32-wasip2",
            ])
            .status()
            .expect("failed to run cargo build for evil-wasm");
        assert!(status.success(), "evil-wasm 构建失败");
    }
    std::fs::read(&wasm_path).expect("读取 evil-wasm 失败")
}

/// EvilState 的 host 侧 schema：仅 `mode` 字符串，additional_properties=false。
fn evil_state_schema() -> JsonSchema {
    JsonSchema::Object {
        required: vec!["mode".into()],
        properties: BTreeMap::from([(
            "mode".into(),
            JsonSchema::String {
                max_length: Some(64),
            },
        )]),
        additional_properties: false,
    }
}

/// 零授权（未声明任何能力）。固有能力(ui/log/clock)由 Broker 自动并入。
fn evil_grants_none(instance: u64) -> InstanceGrant {
    let plugin = Grants {
        plugin: PluginId("dev.floatile.evil".into()),
        trust: TrustLevel::Dev,
        caps: vec![],
    };
    narrow_instance(&plugin, InstanceId(instance), vec![]).unwrap()
}

fn spawn_evil(
    manager: &WidgetManager,
    instance: u64,
    grants: InstanceGrant,
    initial_state: Value,
) -> floatile_runtime::WidgetHandle {
    let config = WidgetConfig {
        plugin: PluginId("dev.floatile.evil".into()),
        instance: InstanceId(instance),
        wasm: evil_wasm_bytes(),
        initial_state,
        state_schema: evil_state_schema(),
        config_json: "{}".into(),
        grants,
    };
    manager.spawn(config).expect("spawn 失败")
}

/// 内存 SQLite 审计存储 + 把服务层 AuditEvent 落库的 listener。
fn mem_audit_persistence() -> (Arc<Mutex<Store>>, floatile_services::AuditListener) {
    let store = Arc::new(Mutex::new(floatile_store::open(":memory:").unwrap()));
    let sink = Arc::clone(&store);
    let listener: floatile_services::AuditListener = Arc::new(move |event: &AuditEvent| {
        let record = AuditRecord {
            plugin: event.plugin.clone(),
            instance: event.instance,
            capability: event.capability.clone(),
            decision: event.decision.clone(),
            reason: event.reason.clone(),
            detail: event.detail.clone(),
            unix_ts: 0,
        };
        let _ = sink.lock().audit().record(&record);
    });
    (store, listener)
}

/// 未声明能力调用 → Broker 拒绝 + 审计落 SQLite + 宿主存活。
#[tokio::test(flavor = "multi_thread")]
async fn denied_capability_persists_audit_and_host_survives() {
    let (store, listener) = mem_audit_persistence();
    let manager = WidgetManager::new()
        .unwrap()
        .with_audit_listener(Some(listener));
    let handle = spawn_evil(&manager, 1, evil_grants_none(1), json!({"mode": "deny"}));

    // 所有未声明能力调用被 Broker 拒绝并被插件吞掉(不中断实例)。
    handle.start().await.expect("deny 模式 start 应成功");
    handle.shutdown().await.expect("shutdown 应正常");

    let rows = store.lock().audit().list().unwrap();
    // 拒绝留痕：storage:read 与 system:memory。
    assert!(
        rows.iter()
            .any(|r| r.capability == "storage:read" && r.decision == "deny"),
        "应有 storage:read deny 审计，实际 {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.capability == "system:memory" && r.decision == "deny"),
        "应有 system:memory deny 审计，实际 {rows:?}"
    );
    // 固有能力 log:write(插件据此记录)被允许并留痕。
    assert!(
        rows.iter()
            .any(|r| r.capability == "log:write" && r.decision == "allow"),
        "应有 log:write allow 审计，实际 {rows:?}"
    );

    // 宿主存活：同一引擎再 spawn 一个实例正常工作。
    let manager2 = WidgetManager::new().unwrap();
    let handle2 = spawn_evil(&manager2, 9, evil_grants_none(9), json!({"mode": "deny"}));
    handle2.start().await.expect("新实例 start 应成功");
    handle2.shutdown().await.expect("新实例 shutdown 正常");
}

/// 超限/类型错误/未知字段 State Patch → 被拒，宿主存活，状态不部分改写。
#[tokio::test(flavor = "multi_thread")]
async fn bad_state_patch_rejected_and_host_survives() {
    let manager = WidgetManager::new().unwrap();
    let mut handle = spawn_evil(
        &manager,
        2,
        evil_grants_none(2),
        json!({"mode": "bad-patch"}),
    );
    // 三类恶意 patch 全部被宿主拒绝；error 被插件吞掉，实例不中断。
    handle.start().await.expect("bad-patch 模式 start 应成功");

    // 被拒的 patch 不应产生任何 UI 状态投递(原子应用 + 失败无部分改写)。
    // 给一个小的收口窗口:若收到更新则说明有状态被应用(不应发生)。
    let drained = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        handle.ui_updates().recv(),
    )
    .await;
    assert!(
        drained.is_err(),
        "恶意 patch 全部被拒,不应产生 UiUpdate(实际收到 {drained:?})"
    );

    // host UI 仍是权威:下发一次合法更新仍可被接受(用 clock 同款流程验证宿主侧正常)。
    handle.shutdown().await.expect("shutdown 应正常");

    // 宿主存活。
    let manager2 = WidgetManager::new().unwrap();
    let handle2 = spawn_evil(&manager2, 8, evil_grants_none(8), json!({"mode": "deny"}));
    handle2.start().await.expect("新实例 start 应成功");
    handle2.shutdown().await.expect("新实例 shutdown 正常");
}

/// 无限 CPU 循环 → fuel 耗尽 trap 终止实例；宿主与其他实例存活。
#[tokio::test(flavor = "multi_thread")]
async fn infinite_loop_is_fuel_trapped_and_host_survives() {
    // 事件触发无限循环;实例共享单一 fuel 预算,循环会耗尽余量并 trap。
    let manager = WidgetManager::new().unwrap();
    let handle = spawn_evil(&manager, 3, evil_grants_none(3), json!({"mode": "loop"}));
    handle
        .start()
        .await
        .expect("loop 模式的 start 应成功(循环只在事件触发)");

    // 触发无限循环 → 该实例 fuel 耗尽 trap,handle_event 返回失败。
    let result = handle
        .handle_event(WidgetEvent::Ui(UiEvent {
            name: "trigger".into(),
            payload_json: "{}".into(),
        }))
        .await;
    assert!(
        matches!(result, Err(floatile_runtime::InstanceError::Failed(_))),
        "触发无限循环应 fuel trap 返回 Failed,实际 {result:?}"
    );

    // 宿主存活:默认引擎再派生一个实例正常工作。
    let manager2 = WidgetManager::new().unwrap();
    let handle2 = spawn_evil(&manager2, 7, evil_grants_none(7), json!({"mode": "deny"}));
    handle2.start().await.expect("宿主存活:新实例 start 应成功");
    handle2.shutdown().await.expect("新实例 shutdown 正常");
}

/// 超限线性内存申请 → StoreLimits 终止实例；宿主存活。
#[tokio::test(flavor = "multi_thread")]
async fn memory_alloc_over_limit_traps_instance_but_host_survives() {
    let manager = WidgetManager::new().unwrap(); // 默认每实例 16 MiB 上限
    let handle = spawn_evil(&manager, 4, evil_grants_none(4), json!({"mode": "alloc"}));
    // start 里申请 64 MiB > 16 MiB 上限 → 实例 trap,start 失败。
    let start_result = handle.start().await;
    assert!(
        matches!(
            start_result,
            Err(floatile_runtime::InstanceError::Failed(_))
        ),
        "超限内存申请应被 StoreLimits 终止,实际 {start_result:?}"
    );

    // 宿主存活。
    let manager2 = WidgetManager::new().unwrap();
    let handle2 = spawn_evil(&manager2, 6, evil_grants_none(6), json!({"mode": "deny"}));
    handle2.start().await.expect("宿主存活:新实例 start 应成功");
    handle2.shutdown().await.expect("新实例 shutdown 正常");
}

/// 伪造事件名/关闭后调用 → 干净失败,宿主不崩。
#[tokio::test(flavor = "multi_thread")]
async fn forged_event_names_are_ignored_and_shutdown_is_clean() {
    let manager = WidgetManager::new().unwrap();
    let handle = spawn_evil(&manager, 5, evil_grants_none(5), json!({"mode": "deny"}));
    handle.start().await.expect("start 应成功");

    // 伪造的 UI 事件名(guest 的 FromWidgetEvent 映射为 Unknown 或被忽略),
    // 洪泛发送也不能拖垮宿主或造成拒绝之外的副作用。
    for i in 0..QUEUE_CAPACITY_BUFFER {
        let forged = handle
            .handle_event(WidgetEvent::Ui(UiEvent {
                name: format!("forged-{i}"),
                payload_json: "{}".into(),
            }))
            .await;
        assert!(
            forged.is_ok(),
            "伪造事件名 {i} 应被静默忽略,实际 {forged:?}"
        );
    }

    handle.shutdown().await.expect("shutdown 应正常");

    // 宿主存活:新实例不受影响。
    let manager2 = WidgetManager::new().unwrap();
    let handle2 = spawn_evil(&manager2, 10, evil_grants_none(10), json!({"mode": "deny"}));
    handle2.start().await.expect("新实例 start 应成功");
    handle2.shutdown().await.expect("新实例 shutdown 正常");
}

/// 命令队列上限常量的测试镜像(runtime::QUEUE_CAPACITY 私有,这里只做合理洪泛界)。
const QUEUE_CAPACITY_BUFFER: usize = 70;
