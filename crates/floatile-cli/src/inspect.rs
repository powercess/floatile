//! `inspect`：完整校验 `.floatile` 后输出只读、可复现的包元数据与摘要。

use std::path::Path;

use floatile_core::install::{content_digest, file_digest, hex_encode};
use floatile_core::manifest::{PermissionDecl, PluginKind};
use serde::Serialize;
use thiserror::Error;

use crate::package::{PackageError, PackageLimits, validate_package};

/// `floatile inspect --json` 的版本化成功结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectReport {
    pub schema_version: u32,
    pub status: &'static str,
    pub package: PackageSummary,
    pub compatibility: CompatibilitySummary,
    pub permissions: Vec<PermissionDecl>,
    pub budget: PackageBudget,
    pub entries: Vec<EntrySummary>,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub kind: PluginKind,
    pub manifest_version: u32,
    pub publisher_id: String,
    pub publisher_name: String,
    pub ui_entrypoint: String,
    pub logic_entrypoint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilitySummary {
    pub status: &'static str,
    pub engine_api_version: String,
    pub ui_api_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageBudget {
    pub archive_bytes: usize,
    pub max_archive_bytes: usize,
    pub entry_count: usize,
    pub max_entries: usize,
    pub file_count: usize,
    pub uncompressed_bytes: usize,
    pub max_uncompressed_bytes: usize,
    pub largest_entry_bytes: usize,
    pub max_single_entry_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Debug, Error)]
pub enum InspectError {
    #[error("读取包失败: {0}")]
    Io(String),
    #[error("包校验失败: {0}")]
    Package(#[from] PackageError),
}

impl InspectError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "FINSPECT_IO",
            Self::Package(error) => error.code(),
        }
    }
}

/// 从文件读取并检查包。错误不包含输入路径，避免 JSON 诊断泄漏宿主绝对路径。
pub fn inspect_package(path: &Path, limits: &PackageLimits) -> Result<InspectReport, InspectError> {
    let bytes = std::fs::read(path).map_err(|error| InspectError::Io(error.to_string()))?;
    inspect_package_bytes(&bytes, limits)
}

/// 对内存中的包执行完整安全校验后生成检查报告。
pub fn inspect_package_bytes(
    archive_bytes: &[u8],
    limits: &PackageLimits,
) -> Result<InspectReport, InspectError> {
    let validated = validate_package(archive_bytes, limits)?;
    let manifest = &validated.manifest;
    let mut entries = Vec::with_capacity(validated.files.len());
    let mut uncompressed_bytes = 0usize;
    let mut largest_entry_bytes = 0usize;
    for (path, bytes) in &validated.files {
        uncompressed_bytes = uncompressed_bytes.saturating_add(bytes.len());
        largest_entry_bytes = largest_entry_bytes.max(bytes.len());
        entries.push(EntrySummary {
            path: path.clone(),
            bytes: bytes.len(),
            sha256: hex_encode(&file_digest(bytes)),
        });
    }

    Ok(InspectReport {
        schema_version: 1,
        status: "ok",
        package: PackageSummary {
            id: manifest.id.0.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            kind: manifest.kind,
            manifest_version: manifest.manifest_version,
            publisher_id: manifest.publisher.id.clone(),
            publisher_name: manifest.publisher.name.clone(),
            ui_entrypoint: manifest.entrypoints.ui.as_str().to_owned(),
            logic_entrypoint: manifest.entrypoints.logic.as_str().to_owned(),
        },
        compatibility: CompatibilitySummary {
            status: "compatible",
            engine_api_version: manifest.engine_api_version.clone(),
            ui_api_version: manifest.ui_api_version.clone(),
        },
        permissions: manifest.permissions.clone(),
        budget: PackageBudget {
            archive_bytes: archive_bytes.len(),
            max_archive_bytes: limits.max_archive_bytes,
            entry_count: validated.entry_names.len(),
            max_entries: limits.max_entries,
            file_count: validated.files.len(),
            uncompressed_bytes,
            max_uncompressed_bytes: limits.max_uncompressed_total,
            largest_entry_bytes,
            max_single_entry_bytes: limits.max_single_entry,
        },
        entries,
        digest: hex_encode(&content_digest(&validated.files)),
    })
}
