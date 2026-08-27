//! 集成测试：`floatile test` 无头冒烟对真实 clock-wasm 全链路。
//! 覆盖 build → 提取 wasm/widget.ftui/manifest → 生命周期冒烟（load/start/state/shutdown）
//! 与稳定 JSON 结构。无需窗口，headless 可跑。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Both cases invoke Cargo for the same project and consume the same release artifact. Keep that
/// producer/consumer sequence exclusive: Windows cannot safely replace/read the artifact across
/// the two concurrent test threads, while each individual author flow remains fully exercised.
fn author_build_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn test_project_runs_clock_smoke() {
    let _build_guard = author_build_lock();
    let project_dir = workspace_root().join("plugins/clock-wasm");
    let out =
        std::env::temp_dir().join(format!("floatile-test-e2e-{}.floatile", std::process::id()));
    let _ = std::fs::remove_file(&out);

    let status = floatile_cli::test_project(&project_dir, &out, Duration::from_secs(4))
        .expect("test_project 应成功");

    assert!(status.ok, "clock-wasm 冒烟应通过: {status:?}");
    assert!(status.phases.start, "start 阶段应成功");
    assert!(status.phases.shutdown, "shutdown 阶段应成功");
    assert!(
        status.phases.state_updates >= 1,
        "clock 应产生至少 1 次 State 更新: {:?}",
        status.phases
    );

    // 稳定 JSON 契约：可解析且字段稳定。
    let parsed: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
    assert_eq!(parsed["schemaVersion"], 1);
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["severity"], "info");
    assert_eq!(parsed["warnings"], serde_json::json!([]));
    assert_eq!(parsed["ok"], serde_json::json!(true));
    assert_eq!(parsed["code"], "ok");
    assert_eq!(parsed["phases"]["build"], serde_json::json!(true));
    assert_eq!(
        parsed["phases"]["state_updates"],
        status.phases.state_updates
    );

    let _ = std::fs::remove_file(&out);
}

#[test]
fn test_scenario_injects_event_and_exercises_broker_denial() {
    let _build_guard = author_build_lock();
    let project_dir = workspace_root().join("plugins/clock-wasm");
    let out = std::env::temp_dir().join(format!(
        "floatile-test-scenario-{}.floatile",
        std::process::id()
    ));
    let status = floatile_cli::test_project_with_scenario(
        &project_dir,
        &out,
        Duration::from_millis(300),
        floatile_cli::TestScenario {
            ui_events: vec![("start".to_owned(), "{}".to_owned())],
            deny_all: true,
            advance_time: Duration::from_millis(20),
        },
    )
    .expect("scenario should run");
    assert!(status.ok, "scenario failed: {status:?}");
    assert_eq!(status.phases.events, 1);
    assert!(status.phases.state_updates >= 1);
    assert!(status.phases.audit_denials >= 1);
    let _ = std::fs::remove_file(out);
}
