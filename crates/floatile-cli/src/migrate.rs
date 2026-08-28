//! Versioned author-project migrations. The default mode is read-only.

use std::io::Write;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::OUTPUT_SCHEMA_VERSION;
use crate::project::parse_floatile_toml;

const MAX_PROJECT_CONFIG_BYTES: u64 = 256 * 1024;
const SDK_SECTION: &str = "\n[sdk]\nlanguage = \"rust\"\nversion = \"0.1.0\"\n";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationChange {
    pub code: &'static str,
    pub file: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub schema_version: u32,
    pub status: &'static str,
    pub mode: &'static str,
    pub changed: bool,
    pub target_sdk_version: &'static str,
    pub changes: Vec<MigrationChange>,
    pub warnings: Vec<crate::CommandWarning>,
}

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("项目配置缺失")]
    MissingConfig,
    #[error("项目配置不是普通文件")]
    UnsafeConfig,
    #[error("项目配置超过迁移预算")]
    ConfigTooLarge,
    #[error("项目配置无效")]
    InvalidConfig,
    #[error("无法读取项目配置")]
    Read,
    #[error("无法写入项目配置")]
    Write,
}

impl MigrationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingConfig => "FMIGRATE_CONFIG_MISSING",
            Self::UnsafeConfig => "FMIGRATE_CONFIG_UNSAFE",
            Self::ConfigTooLarge => "FMIGRATE_CONFIG_SIZE",
            Self::InvalidConfig => "FMIGRATE_CONFIG_INVALID",
            Self::Read => "FMIGRATE_READ",
            Self::Write => "FMIGRATE_WRITE",
        }
    }
}

pub fn migrate_project(project_dir: &Path, write: bool) -> Result<MigrationReport, MigrationError> {
    let path = project_dir.join("floatile.toml");
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            MigrationError::MissingConfig
        } else {
            MigrationError::Read
        }
    })?;
    if !metadata.file_type().is_file() {
        return Err(MigrationError::UnsafeConfig);
    }
    if metadata.len() > MAX_PROJECT_CONFIG_BYTES {
        return Err(MigrationError::ConfigTooLarge);
    }

    let source = std::fs::read_to_string(&path).map_err(|_| MigrationError::Read)?;
    parse_floatile_toml(&source).map_err(|_| MigrationError::InvalidConfig)?;
    let document: toml::Value =
        toml::from_str(&source).map_err(|_| MigrationError::InvalidConfig)?;
    let needs_sdk = document.get("sdk").is_none();
    let changes = if needs_sdk {
        vec![MigrationChange {
            code: "FMIGRATE_SDK_EXPLICIT",
            file: "floatile.toml",
            description: "显式记录 Rust SDK language/version，为双语言构建保留单一项目事实",
        }]
    } else {
        Vec::new()
    };

    if write && needs_sdk {
        let mut migrated = source;
        if !migrated.ends_with('\n') {
            migrated.push('\n');
        }
        migrated.push_str(SDK_SECTION.trim_start_matches('\n'));
        replace_recoverably(&path, migrated.as_bytes())?;
    }

    Ok(MigrationReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        status: "ok",
        mode: if write { "write" } else { "dry-run" },
        changed: write && needs_sdk,
        target_sdk_version: env!("CARGO_PKG_VERSION"),
        changes,
        warnings: Vec::new(),
    })
}

fn replace_recoverably(path: &Path, bytes: &[u8]) -> Result<(), MigrationError> {
    let parent = path.parent().ok_or(MigrationError::Write)?;
    let suffix = std::process::id();
    let staging = parent.join(format!(".floatile.toml.migrate-{suffix}"));
    let backup = parent.join(format!(".floatile.toml.backup-{suffix}"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .map_err(|_| MigrationError::Write)?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&staging);
        return Err(MigrationError::Write);
    }
    drop(file);

    if std::fs::rename(path, &backup).is_err() {
        let _ = std::fs::remove_file(&staging);
        return Err(MigrationError::Write);
    }
    if std::fs::rename(&staging, path).is_err() {
        let _ = std::fs::rename(&backup, path);
        let _ = std::fs::remove_file(&staging);
        return Err(MigrationError::Write);
    }
    std::fs::remove_file(&backup).map_err(|_| MigrationError::Write)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn legacy_project() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "floatile-migrate-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("floatile.toml"),
            "[plugin]\nid = \"dev.floatile.legacy\"\nname = \"Legacy\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn dry_run_reports_without_writing() {
        let dir = legacy_project();
        let before = std::fs::read_to_string(dir.join("floatile.toml")).unwrap();
        let report = migrate_project(&dir, false).unwrap();
        assert_eq!(report.mode, "dry-run");
        assert!(!report.changed);
        assert_eq!(report.changes.len(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("floatile.toml")).unwrap(),
            before
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_adds_explicit_sdk_once() {
        let dir = legacy_project();
        let report = migrate_project(&dir, true).unwrap();
        assert!(report.changed);
        let migrated = std::fs::read_to_string(dir.join("floatile.toml")).unwrap();
        assert!(migrated.contains("[sdk]"));
        assert_eq!(migrate_project(&dir, true).unwrap().changes.len(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
