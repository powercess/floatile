//! `test`：无桌面无头测试。构建项目 → 提取 `wasm/widget.ftui/manifest` → 用
//! `WidgetHarness` 跑生命周期冒烟（load/start/shutdown + 宿主存活），输出稳定 JSON。
//!
//! 所有宿主能力仍走生产 `PermissionBroker`（deny-by-default），不绕过 Broker 语义；
//! 只做无窗口的确定性冒烟，真实窗口行为由平台验收覆盖。

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use floatile_core::capability::{
    CapabilityId, EffectiveGrant, Grant, TrustLevel, parse_capability_params,
};
use floatile_core::manifest::Manifest;
use floatile_runtime::harness::WidgetHarness;
use floatile_ui_schema::UiDocument;
use zip::ZipArchive;

use crate::build::{BuildError, build_project};
use crate::dev::ensure_project;

/// 测试错误（稳定 code `FTEST_*`）。
#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error("项目准备失败: {0}")]
    Build(#[from] BuildError),
    #[error("包读取失败: {0}")]
    Zip(String),
    #[error("manifest 解析失败: {0}")]
    Manifest(String),
    #[error("widget.ftui 解析失败: {0}")]
    Ftui(String),
    #[error("运行时失败: {0}")]
    Runtime(String),
}

impl TestError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Build(_) => "FBUILD",
            Self::Zip(_) => "FTEST_ZIP",
            Self::Manifest(_) => "FTEST_MANIFEST",
            Self::Ftui(_) => "FTEST_FTUI",
            Self::Runtime(_) => "FTEST_RUNTIME",
        }
    }
}

/// 无头测试各阶段结果（稳定 JSON 结构）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TestStatus {
    pub ok: bool,
    pub code: String,
    pub detail: String,
    pub phases: TestPhases,
}

/// 生命周期冒烟各阶段。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TestPhases {
    pub build: bool,
    pub load: bool,
    pub start: bool,
    pub state_updates: usize,
    pub shutdown: bool,
}

/// 从 `.floatile` zip 读取一个条目。
fn read_zip_entry(bytes: &[u8], name: &str) -> Result<Vec<u8>, TestError> {
    let mut zip = ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| TestError::Zip(format!("打开 zip: {e}")))?;
    let mut entry = zip
        .by_name(name)
        .map_err(|e| TestError::Zip(format!("缺少 {name}: {e}")))?;
    let mut buf = Vec::new();
    entry
        .read_to_end(&mut buf)
        .map_err(|e| TestError::Zip(format!("读取 {name}: {e}")))?;
    Ok(buf)
}

/// 对项目执行无头测试：构建 → 提取 → 生命周期冒烟。
pub fn test_project(
    project_dir: &Path,
    out: &Path,
    state_timeout: Duration,
) -> Result<TestStatus, TestError> {
    ensure_project(project_dir)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TestError::Zip(format!("创建 {}: {e}", parent.display())))?;
    }
    build_project(project_dir, out)?;
    let bytes =
        std::fs::read(out).map_err(|e| TestError::Zip(format!("读取 {}: {e}", out.display())))?;

    let wasm = read_zip_entry(&bytes, "logic/plugin.wasm")?;
    let manifest_bytes = read_zip_entry(&bytes, "manifest.json")?;
    let pkg: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| TestError::Manifest(format!("解析 manifest.json: {e}")))?;
    let ftui_bytes = read_zip_entry(&bytes, "ui/widget.ftui")?;
    let doc: UiDocument = serde_json::from_slice(&ftui_bytes)
        .map_err(|e| TestError::Ftui(format!("解析 widget.ftui: {e}")))?;

    // 从 manifest permissions 重建授权：params 经单一源 `parse_capability_params`
    // 由 manifest JSON 转回（固有能力由 Broker 自动合并，拒绝会照常审计）。
    let grants = pkg
        .permissions
        .iter()
        .filter_map(|p| {
            let capability = CapabilityId::from_name(&p.capability)?;
            let params = parse_capability_params(capability, p.params.as_ref())
                .ok()
                .flatten();
            Some(Grant {
                capability,
                params,
                effective: EffectiveGrant::DerivedFromInstall,
            })
        })
        .collect::<Vec<_>>();

    let harness = WidgetHarness::new(pkg.id.clone(), wasm)
        .initial_state(doc.state.initial)
        .state_schema(doc.state.schema)
        .config_json("{}")
        .trust(TrustLevel::Dev)
        .grant_all(grants);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| TestError::Runtime(format!("创建运行时: {e}")))?;
    rt.block_on(run_smoke(harness, state_timeout))
}

async fn run_smoke(harness: WidgetHarness, timeout: Duration) -> Result<TestStatus, TestError> {
    let instance = harness
        .build()
        .map_err(|e| TestError::Runtime(e.to_string()))?;

    let start = instance.start().await;
    if let Err(e) = start {
        // start 失败（构造/trap）：实例已终止，无法再驱动。
        return Ok(TestStatus {
            ok: false,
            code: "FTEST_START".into(),
            detail: e.to_string(),
            phases: TestPhases {
                build: true,
                load: true,
                start: false,
                state_updates: 0,
                shutdown: false,
            },
        });
    }

    let mut instance = instance;
    let state_updates = instance.count_state_updates(timeout).await;
    let shutdown = instance.shutdown().await;
    let shutdown_ok = shutdown.is_ok();
    let ok = shutdown_ok;

    Ok(TestStatus {
        ok,
        code: if ok {
            "ok".into()
        } else {
            "FTEST_SHUTDOWN".into()
        },
        detail: if ok {
            "生命周期冒烟通过：load/start/shutdown + 宿主存活".into()
        } else {
            format!("shutdown 失败: {shutdown:?}")
        },
        phases: TestPhases {
            build: true,
            load: true,
            start: true,
            state_updates,
            shutdown: shutdown_ok,
        },
    })
}
