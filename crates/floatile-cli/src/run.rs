//! `floatile run`：构建、安装、创建持久实例并由正式宿主按精确 Installation 运行。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use floatile_core::{InstanceConfig, InstanceDesiredState};
use floatile_store::installation::load_exact;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::build::{BuildError, build_project};
use crate::install::{InstallError, install_package};
use crate::instance::{InstanceCommandError, create_instance};
use crate::package::PackageLimits;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub schema_version: u32,
    pub status: &'static str,
    pub severity: &'static str,
    pub code: String,
    pub warnings: Vec<crate::CommandWarning>,
    pub instance_id: u64,
    pub plugin_id: String,
    pub version: String,
    pub running: bool,
    pub detail: String,
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error(transparent)]
    Inspect(#[from] crate::InspectError),
    #[error("安装目录复验失败: {0}")]
    Catalog(#[from] floatile_store::installation::InstallationCatalogError),
    #[error(transparent)]
    Instance(#[from] InstanceCommandError),
    #[error("同版本安装内容不同")]
    InstallationConflict,
    #[error("运行宿主不可用: {0}")]
    HostUnavailable(String),
    #[error("运行宿主输出无效: {0}")]
    HostProtocol(String),
    #[error("数据目录不可用: {0}")]
    DataDirectory(String),
    #[error("临时构建目录不可用: {0}")]
    Temporary(String),
}

impl RunError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Build(error) => error.code(),
            Self::Install(error) => error.code(),
            Self::Inspect(error) => error.code(),
            Self::Catalog(error) => error.code(),
            Self::Instance(error) => error.code(),
            Self::InstallationConflict => "FRUN_INSTALLATION_CONFLICT",
            Self::HostUnavailable(_) => "FRUN_HOST_UNAVAILABLE",
            Self::HostProtocol(_) => "FRUN_HOST_PROTOCOL",
            Self::DataDirectory(_) => "FRUN_DATA_DIRECTORY",
            Self::Temporary(_) => "FRUN_TEMP",
        }
    }

    pub fn public_detail(&self) -> &'static str {
        match self {
            Self::Build(_) => "运行项目构建失败",
            Self::Install(_) => "插件安装失败",
            Self::Inspect(_) => "运行包安全检查失败",
            Self::Catalog(_) => "已安装内容无法通过完整性复验",
            Self::Instance(_) => "无法创建持久插件实例",
            Self::InstallationConflict => "相同插件版本已安装不同内容；请升级版本号",
            Self::HostUnavailable(_) => "找不到或无法启动 Floatile 运行宿主",
            Self::HostProtocol(_) => "Floatile 运行宿主返回了无效结果",
            Self::DataDirectory(_) => "无法解析 Floatile 数据目录",
            Self::Temporary(_) => "无法准备运行构建目录",
        }
    }
}

pub fn default_run_paths() -> Result<(PathBuf, PathBuf), RunError> {
    let data = floatile_platform::data_dir()
        .map_err(|error| RunError::DataDirectory(error.to_string()))?;
    Ok((data.join("layout.db"), data.join("plugins")))
}

pub fn run_project(
    project: &Path,
    database: &Path,
    plugin_store: &Path,
    duration: Duration,
) -> Result<RunReport, RunError> {
    let temporary = RunTemporaryPackage::create()?;
    let manifest = build_project(project, &temporary.path)?;
    let archive =
        std::fs::read(&temporary.path).map_err(|error| RunError::Temporary(error.to_string()))?;
    let expected = crate::inspect_package_bytes(&archive, &PackageLimits::default())?;
    match install_package(
        &archive,
        plugin_store,
        "run.floatile",
        &PackageLimits::default(),
    ) {
        Ok(_) => {}
        Err(InstallError::AlreadyInstalled { .. }) => {
            let installed = load_exact(plugin_store, &manifest.id.0, &manifest.version)?
                .ok_or(RunError::InstallationConflict)?;
            if installed.meta.digest != expected.digest {
                return Err(RunError::InstallationConflict);
            }
        }
        Err(error) => return Err(error.into()),
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    let instance = create_instance(
        database,
        plugin_store,
        &manifest.id.0,
        &manifest.version,
        InstanceConfig::empty(),
        InstanceDesiredState::Running,
        now,
    )?;
    let host = crate::preview::preview_host_path()
        .map_err(|error| RunError::HostUnavailable(error.to_string()))?;
    let output = Command::new(host)
        .arg("--instance")
        .arg(plugin_store)
        .arg(database)
        .arg(instance.instance_id.to_string())
        .arg(duration.as_millis().to_string())
        .output()
        .map_err(|error| RunError::HostUnavailable(error.to_string()))?;
    let outcome: HostOutcome = serde_json::from_slice(&output.stdout)
        .map_err(|error| RunError::HostProtocol(error.to_string()))?;
    Ok(RunReport {
        schema_version: crate::OUTPUT_SCHEMA_VERSION,
        status: if outcome.running { "ok" } else { "error" },
        severity: if outcome.running { "info" } else { "error" },
        code: outcome.code,
        warnings: Vec::new(),
        instance_id: instance.instance_id,
        plugin_id: instance.plugin_id,
        version: instance.version,
        running: outcome.running,
        detail: outcome.detail,
    })
}

#[derive(Debug, Deserialize)]
struct HostOutcome {
    running: bool,
    code: String,
    detail: String,
}

struct RunTemporaryPackage {
    path: PathBuf,
}

impl RunTemporaryPackage {
    fn create() -> Result<Self, RunError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "floatile-run-{}-{nonce}.floatile",
            std::process::id()
        ));
        Ok(Self { path })
    }
}

impl Drop for RunTemporaryPackage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
