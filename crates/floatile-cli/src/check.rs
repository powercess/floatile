//! `check`：在自动清理的临时目录中执行作者项目完整构建与包复验。

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use floatile_core::manifest::PermissionDecl;
use floatile_core::{CapabilityExposure, CapabilityId};
use serde::Serialize;
use thiserror::Error;

use crate::build::{BuildError, build_project};
use crate::inspect::{InspectError, InspectReport, inspect_package};
use crate::output::{CommandWarning, OUTPUT_SCHEMA_VERSION};
use crate::package::{PackageLimits, imported_capabilities, validate_package};

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

/// 版本化 warning 结构。
pub type CheckWarning = CommandWarning;

#[derive(Debug, Error)]
pub enum CheckError {
    #[error("无法创建检查临时目录: {0}")]
    TemporaryDirectory(String),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Inspect(#[from] InspectError),
    #[error("组件使用了 manifest 未声明的能力: {0:?}")]
    CapabilityMissing(Vec<String>),
}

impl CheckError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::TemporaryDirectory(_) => "FCHECK_TEMP",
            Self::Build(error) => error.code(),
            Self::Inspect(error) => error.code(),
            Self::CapabilityMissing(_) => "FCHECK_CAPABILITY_MISSING",
        }
    }

    /// JSON/Agent 输出使用有界稳定描述，不回显 cargo stderr 或宿主绝对路径。
    pub fn public_detail(&self) -> Cow<'static, str> {
        match self {
            Self::TemporaryDirectory(_) => Cow::Borrowed("无法准备临时检查目录"),
            Self::Build(BuildError::Project(_)) => Cow::Borrowed("项目配置无效"),
            Self::Build(BuildError::CargoMetadata(_)) => Cow::Borrowed("Cargo 项目元数据检查失败"),
            Self::Build(BuildError::WasmBuild(_)) => Cow::Borrowed("WASM Component 构建失败"),
            Self::Build(BuildError::BuildFtui(_)) => Cow::Borrowed("Floatile UI 生成失败"),
            Self::Build(BuildError::Package(_)) => Cow::Borrowed("生成包未通过安全校验"),
            Self::Build(BuildError::Io(_)) => Cow::Borrowed("项目输入或临时产物 I/O 失败"),
            Self::Inspect(InspectError::Io(_)) => Cow::Borrowed("临时检查产物读取失败"),
            Self::Inspect(InspectError::Package(_)) => Cow::Borrowed("临时检查产物未通过安全校验"),
            Self::CapabilityMissing(capabilities) => Cow::Owned(format!(
                "组件使用了 manifest 未声明的宿主能力: {}",
                capabilities.join(", ")
            )),
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
            Self::CapabilityMissing(_) => CheckPhases {
                metadata: true,
                wasm: true,
                ui: true,
                manifest: true,
                package: true,
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
    let archive =
        std::fs::read(&package_path).map_err(|error| InspectError::Io(error.to_string()))?;
    let validated =
        validate_package(&archive, &PackageLimits::default()).map_err(InspectError::Package)?;
    let used = imported_capabilities(&validated.wasm).map_err(InspectError::Package)?;
    let (missing, warnings) = capability_drift(&validated.manifest.permissions, &used);
    if !missing.is_empty() {
        return Err(CheckError::CapabilityMissing(missing));
    }
    Ok(CheckReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        status: "ok",
        code: "ok",
        phases: CheckPhases {
            metadata: true,
            wasm: true,
            ui: true,
            manifest: true,
            package: true,
        },
        warnings,
        inspection,
    })
}

fn capability_drift(
    permissions: &[PermissionDecl],
    used: &std::collections::BTreeSet<CapabilityId>,
) -> (Vec<String>, Vec<CheckWarning>) {
    let declared: std::collections::BTreeSet<_> = permissions
        .iter()
        .filter_map(|permission| CapabilityId::from_name(&permission.capability))
        .collect();
    let missing = used
        .difference(&declared)
        .filter(|capability| capability.definition().exposure == CapabilityExposure::Declared)
        .map(|capability| capability.name().to_owned())
        .collect();
    let warnings = declared
        .difference(used)
        .map(|capability| CheckWarning {
            code: "FCHECK_CAPABILITY_UNUSED".to_owned(),
            message: format!("manifest 声明的能力 `{}` 未被组件导入", capability.name()),
        })
        .collect();
    (missing, warnings)
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

    #[test]
    fn capability_drift_ignores_inherent_and_orders_diagnostics() {
        let permissions = vec![PermissionDecl {
            capability: "storage:write".to_owned(),
            params: None,
        }];
        let used = [
            CapabilityId::ClockRead,
            CapabilityId::StorageRead,
            CapabilityId::TimerSchedule,
        ]
        .into_iter()
        .collect();

        let (missing, warnings) = capability_drift(&permissions, &used);
        assert_eq!(missing, ["storage:read", "timer:schedule"]);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "FCHECK_CAPABILITY_UNUSED");
        assert!(warnings[0].message.contains("storage:write"));

        let error = CheckError::CapabilityMissing(missing);
        assert_eq!(error.code(), "FCHECK_CAPABILITY_MISSING");
        assert!(error.public_detail().contains("storage:read"));
        assert!(error.phases().package);
    }
}
