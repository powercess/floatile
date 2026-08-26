//! `floatile preview`：临时构建、安装并用正式 shell runtime 打开有界真实窗口。

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use thiserror::Error;

use crate::build::{BuildError, build_project};
use crate::install::{InstallError, install_package};
use crate::package::PackageLimits;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewReport {
    pub schema_version: u32,
    pub status: &'static str,
    pub severity: &'static str,
    pub code: String,
    pub warnings: Vec<crate::CommandWarning>,
    pub running: bool,
    pub detail: String,
}

#[derive(Debug, Error)]
pub enum PreviewError {
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error("预览宿主不可用: {0}")]
    HostUnavailable(String),
    #[error("预览宿主输出无效: {0}")]
    HostProtocol(String),
    #[error("无法准备预览临时目录: {0}")]
    Temporary(String),
}

impl PreviewError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Build(error) => error.code(),
            Self::Install(error) => error.code(),
            Self::HostUnavailable(_) => "FPREVIEW_HOST_UNAVAILABLE",
            Self::HostProtocol(_) => "FPREVIEW_HOST_PROTOCOL",
            Self::Temporary(_) => "FPREVIEW_TEMP",
        }
    }

    pub fn public_detail(&self) -> &'static str {
        match self {
            Self::Build(_) => "预览项目构建失败",
            Self::Install(_) => "预览包临时安装失败",
            Self::HostUnavailable(_) => "找不到或无法启动 Floatile 真实预览宿主",
            Self::HostProtocol(_) => "真实预览宿主返回了无效结果",
            Self::Temporary(_) => "无法准备预览临时目录",
        }
    }
}

pub fn preview_project(project: &Path, duration: Duration) -> Result<PreviewReport, PreviewError> {
    PreviewSession::start(project, duration)?.wait()
}

/// 一个正在运行的真实预览子进程。Drop 会先终止宿主，再清理临时安装。
pub struct PreviewSession {
    child: Option<Child>,
    _temporary: PreviewTemporaryDirectory,
}

impl PreviewSession {
    pub fn start(project: &Path, duration: Duration) -> Result<Self, PreviewError> {
        let temporary = PreviewTemporaryDirectory::create()?;
        let package = temporary.path.join("plugin.floatile");
        let manifest = build_project(project, &package)?;
        let archive =
            std::fs::read(&package).map_err(|error| PreviewError::Temporary(error.to_string()))?;
        let _installed = install_package(
            &archive,
            &temporary.store,
            "preview.floatile",
            &PackageLimits::default(),
        )?;
        let host = preview_host_path()?;
        let child = std::process::Command::new(host)
            .arg(&temporary.store)
            .arg(&manifest.id.0)
            .arg(duration.as_millis().to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| PreviewError::HostUnavailable(error.to_string()))?;
        Ok(Self {
            child: Some(child),
            _temporary: temporary,
        })
    }

    pub fn wait(mut self) -> Result<PreviewReport, PreviewError> {
        let child = self
            .child
            .take()
            .ok_or_else(|| PreviewError::HostProtocol("preview host already reaped".to_owned()))?;
        let output = child
            .wait_with_output()
            .map_err(|error| PreviewError::HostUnavailable(error.to_string()))?;
        let outcome: HostOutcome = serde_json::from_slice(&output.stdout)
            .map_err(|error| PreviewError::HostProtocol(error.to_string()))?;
        Ok(PreviewReport {
            schema_version: crate::OUTPUT_SCHEMA_VERSION,
            status: if outcome.running { "ok" } else { "error" },
            severity: if outcome.running { "info" } else { "error" },
            code: outcome.code,
            warnings: Vec::new(),
            running: outcome.running,
            detail: outcome.detail,
        })
    }
}

impl Drop for PreviewSession {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct HostOutcome {
    running: bool,
    code: String,
    detail: String,
}

pub(crate) fn preview_host_path() -> Result<PathBuf, PreviewError> {
    if let Some(path) = std::env::var_os("FLOATTILE_PREVIEW_HOST").filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(path));
    }
    let executable = std::env::current_exe()
        .map_err(|error| PreviewError::HostUnavailable(error.to_string()))?;
    let name = if cfg!(windows) {
        "floatile-preview-host.exe"
    } else {
        "floatile-preview-host"
    };
    executable
        .parent()
        .map(|parent| parent.join(name))
        .ok_or_else(|| PreviewError::HostUnavailable("CLI executable has no parent".to_owned()))
}

struct PreviewTemporaryDirectory {
    path: PathBuf,
    store: PathBuf,
}

impl PreviewTemporaryDirectory {
    fn create() -> Result<Self, PreviewError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path =
            std::env::temp_dir().join(format!("floatile-preview-{}-{nonce}", std::process::id()));
        let store = path.join("plugins");
        std::fs::create_dir_all(&store)
            .map_err(|error| PreviewError::Temporary(error.to_string()))?;
        Ok(Self { path, store })
    }
}

impl Drop for PreviewTemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
