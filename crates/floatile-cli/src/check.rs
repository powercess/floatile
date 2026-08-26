//! `check`：在自动清理的临时目录中执行作者项目完整构建与包复验。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use thiserror::Error;

use crate::build::{BuildError, build_project};
use crate::inspect::{InspectError, InspectReport, inspect_package};
use crate::package::PackageLimits;

/// `floatile check --json` 的成功结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub schema_version: u32,
    pub status: &'static str,
    pub code: &'static str,
    pub phases: CheckPhases,
    pub warnings: Vec<CheckWarning>,
    pub inspection: InspectReport,
}

/// 检查阶段是否完整通过。字段顺序也是人类输出的稳定阶段顺序。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckPhases {
    pub metadata: bool,
    pub wasm: bool,
    pub ui: bool,
    pub manifest: bool,
    pub package: bool,
}

/// 预留的版本化 warning 结构；当前检查链无 warning，成功时为空数组。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum CheckError {
    #[error("无法创建检查临时目录: {0}")]
    TemporaryDirectory(String),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Inspect(#[from] InspectError),
}

impl CheckError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::TemporaryDirectory(_) => "FCHECK_TEMP",
            Self::Build(error) => error.code(),
            Self::Inspect(error) => error.code(),
        }
    }

    /// JSON/Agent 输出使用有界稳定描述，不回显 cargo stderr 或宿主绝对路径。
    pub fn public_detail(&self) -> &'static str {
        match self {
            Self::TemporaryDirectory(_) => "无法准备临时检查目录",
            Self::Build(BuildError::Project(_)) => "项目配置无效",
            Self::Build(BuildError::CargoMetadata(_)) => "Cargo 项目元数据检查失败",
            Self::Build(BuildError::WasmBuild(_)) => "WASM Component 构建失败",
            Self::Build(BuildError::BuildFtui(_)) => "Floatile UI 生成失败",
            Self::Build(BuildError::Package(_)) => "生成包未通过安全校验",
            Self::Build(BuildError::Io(_)) => "项目输入或临时产物 I/O 失败",
            Self::Inspect(InspectError::Io(_)) => "临时检查产物读取失败",
            Self::Inspect(InspectError::Package(_)) => "临时检查产物未通过安全校验",
        }
    }

    pub fn phases(&self) -> CheckPhases {
        match self {
            Self::TemporaryDirectory(_) | Self::Build(BuildError::CargoMetadata(_)) => {
                CheckPhases::default()
            }
            Self::Build(BuildError::WasmBuild(_)) => CheckPhases {
                metadata: true,
                ..CheckPhases::default()
            },
            Self::Build(BuildError::BuildFtui(_)) => CheckPhases {
                metadata: true,
                wasm: true,
                ..CheckPhases::default()
            },
            Self::Build(BuildError::Project(_) | BuildError::Io(_)) => CheckPhases {
                metadata: true,
                wasm: true,
                ui: true,
                ..CheckPhases::default()
            },
            Self::Build(BuildError::Package(_)) | Self::Inspect(_) => CheckPhases {
                metadata: true,
                wasm: true,
                ui: true,
                manifest: true,
                package: false,
            },
        }
    }
}

/// 构建并完整复验作者项目，但不保留 `.floatile` 产物。
pub fn check_project(project_dir: &Path) -> Result<CheckReport, CheckError> {
    let temporary = CheckTemporaryDirectory::create()?;
    let package_path = temporary.path.join("plugin.floatile");
    build_project(project_dir, &package_path)?;
    let inspection = inspect_package(&package_path, &PackageLimits::default())?;
    Ok(CheckReport {
        schema_version: 1,
        status: "ok",
        code: "ok",
        phases: CheckPhases {
            metadata: true,
            wasm: true,
            ui: true,
            manifest: true,
            package: true,
        },
        warnings: Vec::new(),
        inspection,
    })
}

struct CheckTemporaryDirectory {
    path: PathBuf,
}

impl CheckTemporaryDirectory {
    fn create() -> Result<Self, CheckError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path =
            std::env::temp_dir().join(format!("floatile-check-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path)
            .map_err(|error| CheckError::TemporaryDirectory(error.to_string()))?;
        Ok(Self { path })
    }
}

impl Drop for CheckTemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_errors_map_to_stable_completed_phases() {
        let wasm = CheckError::Build(BuildError::WasmBuild("private path".to_owned()));
        assert_eq!(wasm.code(), "FBUILD_WASM_BUILD");
        assert_eq!(
            wasm.phases(),
            CheckPhases {
                metadata: true,
                ..CheckPhases::default()
            }
        );
        assert!(!wasm.public_detail().contains("private path"));

        let package = CheckError::Build(BuildError::Package(
            crate::package::PackageError::MissingManifest,
        ));
        assert!(package.phases().manifest);
        assert!(!package.phases().package);
    }
}
