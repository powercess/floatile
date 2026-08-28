//! 持久插件实例管理（PP-M1 CLI 入口的可测试实现）。

use std::path::Path;

use floatile_core::{
    InstanceConfig, InstanceDesiredState, InstanceId, PluginInstance, RollbackPlanError,
    instance::InstanceModelError, plan_rollback,
};
use floatile_store::installation::{
    ConfigValidationError, InstallationCatalogError, InstalledInstallation, load_exact,
    load_reference,
};
use serde::Serialize;

/// CLI/Agent 稳定实例视图；`observed_state` 不持久化，因此不在此结构伪造。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    pub schema_version: u32,
    pub instance_id: u64,
    pub plugin_id: String,
    pub version: String,
    pub digest: String,
    pub config: serde_json::Value,
    pub desired_state: InstanceDesiredState,
    pub generation: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl From<&PluginInstance> for InstanceView {
    fn from(instance: &PluginInstance) -> Self {
        Self {
            schema_version: 1,
            instance_id: instance.id().0,
            plugin_id: instance.installation().plugin().0.clone(),
            version: instance.installation().version().to_owned(),
            digest: instance.installation().digest().to_string(),
            config: instance.config().to_value(),
            desired_state: instance.desired_state(),
            generation: instance.generation(),
            created_at: instance.created_at(),
            updated_at: instance.updated_at(),
        }
    }
}

/// 实例命令错误（稳定 code `FINSTANCE_*`）。
#[derive(Debug, thiserror::Error)]
pub enum InstanceCommandError {
    #[error("{0}")]
    InvalidArguments(String),
    #[error("插件 {id}@{version} 未安装")]
    InstallationMissing { id: String, version: String },
    #[error("安装目录校验失败: {0}")]
    Installation(#[from] InstallationCatalogError),
    #[error("实例配置无效: {0}")]
    Config(#[from] InstanceModelError),
    #[error("插件未声明 config schema，只允许空配置")]
    ConfigNotDeclared,
    #[error("配置 schema 缺失或损坏")]
    ConfigSchemaMissing,
    #[error("配置 schema JSON 无效")]
    ConfigSchemaInvalid,
    #[error("实例数据库失败: {0}")]
    Store(#[from] floatile_store::StoreError),
    #[error("实例 {0} 不存在")]
    NotFound(u64),
    #[error("实例 {0} 正在运行；请先 stop 再修改配置或删除")]
    MustBeStopped(u64),
    #[error("实例 {0} 未更新；记录可能已被并发修改")]
    ConcurrentUpdate(u64),
    #[error("无法创建实例数据库目录: {0}")]
    DatabaseDirectory(String),
    #[error("回滚策略拒绝: {0}")]
    Rollback(#[from] RollbackPlanError),
    #[error("历史安装信任校验失败: {0}")]
    Trust(#[from] crate::install::InstallError),
}

impl InstanceCommandError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArguments(_) => "FINSTANCE_INVALID_ARGUMENTS",
            Self::InstallationMissing { .. } => "FINSTANCE_INSTALLATION_MISSING",
            Self::Installation(error) => error.code(),
            Self::Config(_) => "FINSTANCE_CONFIG_INVALID",
            Self::ConfigNotDeclared => "FINSTANCE_CONFIG_NOT_DECLARED",
            Self::ConfigSchemaMissing => "FINSTANCE_CONFIG_SCHEMA_MISSING",
            Self::ConfigSchemaInvalid => "FINSTANCE_CONFIG_SCHEMA_INVALID",
            Self::Store(_) => "FINSTANCE_STORE",
            Self::NotFound(_) => "FINSTANCE_NOT_FOUND",
            Self::MustBeStopped(_) => "FINSTANCE_MUST_BE_STOPPED",
            Self::ConcurrentUpdate(_) => "FINSTANCE_CONCURRENT_UPDATE",
            Self::DatabaseDirectory(_) => "FINSTANCE_DATABASE_DIRECTORY",
            Self::Rollback(_) => "FINSTANCE_ROLLBACK_REJECTED",
            Self::Trust(error) => error.code(),
        }
    }
}

/// 创建固定到精确 Installation 的实例。
pub fn create_instance(
    database: &Path,
    plugin_store: &Path,
    plugin_id: &str,
    version: &str,
    config: InstanceConfig,
    desired_state: InstanceDesiredState,
    unix_ts: u64,
) -> Result<InstanceView, InstanceCommandError> {
    let installation = load_exact(plugin_store, plugin_id, version)?.ok_or_else(|| {
        InstanceCommandError::InstallationMissing {
            id: plugin_id.to_owned(),
            version: version.to_owned(),
        }
    })?;
    validate_config(&installation, &config)?;
    let reference = installation.reference()?;
    let store = open_database(database)?;
    let instance = store
        .instances()
        .create(&reference, &config, desired_state, unix_ts)?;
    Ok(InstanceView::from(&instance))
}

pub fn list_instances(database: &Path) -> Result<Vec<InstanceView>, InstanceCommandError> {
    let store = open_database(database)?;
    Ok(store
        .instances()
        .list()?
        .iter()
        .map(InstanceView::from)
        .collect())
}

pub fn get_instance(
    database: &Path,
    instance_id: InstanceId,
) -> Result<InstanceView, InstanceCommandError> {
    let store = open_database(database)?;
    let instance = store
        .instances()
        .get(instance_id)?
        .ok_or(InstanceCommandError::NotFound(instance_id.0))?;
    Ok(InstanceView::from(&instance))
}

/// 配置只允许在 stopped 状态修改；动态热配置留给后续明确的确认/失败语义。
pub fn configure_instance(
    database: &Path,
    plugin_store: &Path,
    instance_id: InstanceId,
    config: InstanceConfig,
    unix_ts: u64,
) -> Result<InstanceView, InstanceCommandError> {
    let store = open_database(database)?;
    let instance = require_instance(&store, instance_id)?;
    require_stopped(&instance)?;
    let installation = load_reference(plugin_store, instance.installation())?.ok_or_else(|| {
        InstanceCommandError::InstallationMissing {
            id: instance.installation().plugin().0.clone(),
            version: instance.installation().version().to_owned(),
        }
    })?;
    validate_config(&installation, &config)?;
    if !store
        .instances()
        .update_config(instance_id, &config, unix_ts.max(instance.updated_at()))?
    {
        return Err(InstanceCommandError::ConcurrentUpdate(instance_id.0));
    }
    get_from_store(&store, instance_id)
}

pub fn set_instance_desired_state(
    database: &Path,
    instance_id: InstanceId,
    desired_state: InstanceDesiredState,
    unix_ts: u64,
) -> Result<InstanceView, InstanceCommandError> {
    let store = open_database(database)?;
    let instance = require_instance(&store, instance_id)?;
    if !store.instances().set_desired_state(
        instance_id,
        desired_state,
        unix_ts.max(instance.updated_at()),
    )? {
        return Err(InstanceCommandError::ConcurrentUpdate(instance_id.0));
    }
    get_from_store(&store, instance_id)
}

pub fn delete_instance(
    database: &Path,
    instance_id: InstanceId,
) -> Result<InstanceView, InstanceCommandError> {
    let store = open_database(database)?;
    let instance = require_instance(&store, instance_id)?;
    require_stopped(&instance)?;
    if !store.instances().delete(instance_id)? {
        return Err(InstanceCommandError::ConcurrentUpdate(instance_id.0));
    }
    Ok(InstanceView::from(&instance))
}

/// Rebinds a stopped instance to a trusted, verified historical installation.
pub fn rollback_instance(
    database: &Path,
    plugin_store: &Path,
    instance_id: InstanceId,
    target_version: &str,
    reason: &str,
    unix_ts: u64,
) -> Result<InstanceView, InstanceCommandError> {
    if reason.is_empty() || reason.len() > 512 {
        return Err(InstanceCommandError::InvalidArguments(
            "rollback 需要 1..=512 字节的 --reason".to_owned(),
        ));
    }
    let store = open_database(database)?;
    let instance = require_instance(&store, instance_id)?;
    require_stopped(&instance)?;
    let current = load_reference(plugin_store, instance.installation())?.ok_or_else(|| {
        InstanceCommandError::InstallationMissing {
            id: instance.installation().plugin().0.clone(),
            version: instance.installation().version().to_owned(),
        }
    })?;
    let target = load_exact(
        plugin_store,
        &instance.installation().plugin().0,
        target_version,
    )?
    .ok_or_else(|| InstanceCommandError::InstallationMissing {
        id: instance.installation().plugin().0.clone(),
        version: target_version.to_owned(),
    })?;
    plan_rollback(&current.manifest, &target.manifest)?;
    validate_config(&target, instance.config())?;
    crate::install::verify_trusted_installation(&target, &store)?;
    let target_ref = target.reference()?;
    if !store.instances().rollback_installation(
        instance_id,
        instance.installation(),
        &target_ref,
        reason,
        unix_ts.max(instance.updated_at()),
    )? {
        return Err(InstanceCommandError::ConcurrentUpdate(instance_id.0));
    }
    get_from_store(&store, instance_id)
}

fn validate_config(
    installation: &InstalledInstallation,
    config: &InstanceConfig,
) -> Result<(), InstanceCommandError> {
    installation
        .validate_config(config)
        .map_err(|error| match error {
            ConfigValidationError::NotDeclared => InstanceCommandError::ConfigNotDeclared,
            ConfigValidationError::SchemaMissing => InstanceCommandError::ConfigSchemaMissing,
            ConfigValidationError::SchemaInvalid => InstanceCommandError::ConfigSchemaInvalid,
            ConfigValidationError::Config(error) => InstanceCommandError::Config(error),
        })
}

fn open_database(database: &Path) -> Result<floatile_store::Store, InstanceCommandError> {
    if let Some(parent) = database.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| InstanceCommandError::DatabaseDirectory(error.to_string()))?;
    }
    floatile_store::open(database).map_err(Into::into)
}

fn require_instance(
    store: &floatile_store::Store,
    instance_id: InstanceId,
) -> Result<PluginInstance, InstanceCommandError> {
    store
        .instances()
        .get(instance_id)?
        .ok_or(InstanceCommandError::NotFound(instance_id.0))
}

fn require_stopped(instance: &PluginInstance) -> Result<(), InstanceCommandError> {
    if instance.desired_state() == InstanceDesiredState::Stopped {
        Ok(())
    } else {
        Err(InstanceCommandError::MustBeStopped(instance.id().0))
    }
}

fn get_from_store(
    store: &floatile_store::Store,
    instance_id: InstanceId,
) -> Result<InstanceView, InstanceCommandError> {
    let instance = require_instance(store, instance_id)?;
    Ok(InstanceView::from(&instance))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use floatile_core::install::{InstallMeta, content_digest, file_digest, hex_encode};
    use serde_json::json;

    use super::*;

    struct Fixture {
        root: PathBuf,
        database: PathBuf,
        plugin_store: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str, with_config: bool) -> Self {
            let root = std::env::temp_dir().join(format!(
                "floatile-instance-cli-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let plugin_store = root.join("plugins");
            let database = root.join("layout.db");
            let dir = plugin_store.join("dev.floatile.clock").join("1.0.0");
            std::fs::create_dir_all(dir.join("ui")).unwrap();
            std::fs::create_dir_all(dir.join("logic")).unwrap();

            let mut manifest = json!({
                "manifestVersion": 1,
                "id": "dev.floatile.clock",
                "name": "Clock",
                "version": "1.0.0",
                "publisher": { "id": "dev.floatile", "name": "Floatile" },
                "engineApiVersion": "1.0.0",
                "uiApiVersion": "1.0.0",
                "type": "widget",
                "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
                "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 100, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
                "permissions": []
            });
            let mut files = BTreeMap::from([
                ("logic/plugin.wasm".to_owned(), b"wasm".to_vec()),
                ("ui/widget.ftui".to_owned(), b"{}".to_vec()),
            ]);
            if with_config {
                manifest["config"] = json!({"schema": "config.schema.json"});
                files.insert(
                    "config.schema.json".to_owned(),
                    serde_json::to_vec(&json!({
                        "type": "object",
                        "required": ["timezone"],
                        "additionalProperties": false,
                        "properties": {
                            "timezone": { "type": "string", "maxLength": 32 }
                        }
                    }))
                    .unwrap(),
                );
            }
            files.insert(
                "manifest.json".to_owned(),
                serde_json::to_vec(&manifest).unwrap(),
            );
            for (name, bytes) in &files {
                std::fs::write(dir.join(name), bytes).unwrap();
            }
            let meta = InstallMeta {
                manifest_version: 1,
                id: "dev.floatile.clock".to_owned(),
                version: "1.0.0".to_owned(),
                engine_api_version: "1.0.0".to_owned(),
                ui_api_version: "1.0.0".to_owned(),
                installed_at: 1,
                source: "test".to_owned(),
                trust: floatile_core::install::InstallationTrust::Unsigned,
                files: files
                    .iter()
                    .map(|(name, bytes)| (name.clone(), hex_encode(&file_digest(bytes))))
                    .collect(),
                digest: hex_encode(&content_digest(&files)),
            };
            std::fs::write(dir.join("install.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
            Self {
                root,
                database,
                plugin_store,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn lifecycle_crud_is_persistent_and_uses_exact_installation() {
        let fixture = Fixture::new("lifecycle", true);
        let created = create_instance(
            &fixture.database,
            &fixture.plugin_store,
            "dev.floatile.clock",
            "1.0.0",
            InstanceConfig::new(json!({"timezone": "UTC"})).unwrap(),
            InstanceDesiredState::Stopped,
            10,
        )
        .unwrap();
        assert_eq!(created.instance_id, 2);
        assert_eq!(created.desired_state, InstanceDesiredState::Stopped);

        let running = set_instance_desired_state(
            &fixture.database,
            InstanceId(created.instance_id),
            InstanceDesiredState::Running,
            11,
        )
        .unwrap();
        assert_eq!(running.desired_state, InstanceDesiredState::Running);
        assert!(matches!(
            configure_instance(
                &fixture.database,
                &fixture.plugin_store,
                InstanceId(created.instance_id),
                InstanceConfig::new(json!({"timezone": "Asia/Shanghai"})).unwrap(),
                12,
            ),
            Err(InstanceCommandError::MustBeStopped(2))
        ));

        set_instance_desired_state(
            &fixture.database,
            InstanceId(created.instance_id),
            InstanceDesiredState::Stopped,
            12,
        )
        .unwrap();
        let configured = configure_instance(
            &fixture.database,
            &fixture.plugin_store,
            InstanceId(created.instance_id),
            InstanceConfig::new(json!({"timezone": "Asia/Shanghai"})).unwrap(),
            13,
        )
        .unwrap();
        assert_eq!(configured.config, json!({"timezone": "Asia/Shanghai"}));
        assert_eq!(list_instances(&fixture.database).unwrap().len(), 1);

        let deleted = delete_instance(&fixture.database, InstanceId(created.instance_id)).unwrap();
        assert_eq!(deleted.instance_id, 2);
        assert!(list_instances(&fixture.database).unwrap().is_empty());
        assert!(
            fixture
                .plugin_store
                .join("dev.floatile.clock/1.0.0")
                .is_dir()
        );
    }

    #[test]
    fn config_schema_and_absent_schema_are_enforced() {
        let fixture = Fixture::new("schema", true);
        let error = create_instance(
            &fixture.database,
            &fixture.plugin_store,
            "dev.floatile.clock",
            "1.0.0",
            InstanceConfig::new(json!({"timezone": 7})).unwrap(),
            InstanceDesiredState::Stopped,
            10,
        )
        .unwrap_err();
        assert_eq!(error.code(), "FINSTANCE_CONFIG_INVALID");

        let no_schema = Fixture::new("no-schema", false);
        assert!(matches!(
            create_instance(
                &no_schema.database,
                &no_schema.plugin_store,
                "dev.floatile.clock",
                "1.0.0",
                InstanceConfig::new(json!({"timezone": "UTC"})).unwrap(),
                InstanceDesiredState::Stopped,
                10,
            ),
            Err(InstanceCommandError::ConfigNotDeclared)
        ));
    }

    #[test]
    fn missing_and_tampered_installations_do_not_create_records() {
        let fixture = Fixture::new("tamper", true);
        let missing = create_instance(
            &fixture.database,
            &fixture.plugin_store,
            "dev.floatile.clock",
            "9.9.9",
            InstanceConfig::empty(),
            InstanceDesiredState::Stopped,
            10,
        )
        .unwrap_err();
        assert_eq!(missing.code(), "FINSTANCE_INSTALLATION_MISSING");

        std::fs::write(
            fixture
                .plugin_store
                .join("dev.floatile.clock/1.0.0/logic/plugin.wasm"),
            b"tampered",
        )
        .unwrap();
        let tampered = create_instance(
            &fixture.database,
            &fixture.plugin_store,
            "dev.floatile.clock",
            "1.0.0",
            InstanceConfig::new(json!({"timezone": "UTC"})).unwrap(),
            InstanceDesiredState::Stopped,
            10,
        )
        .unwrap_err();
        assert_eq!(tampered.code(), "FCAT_DIGEST_MISMATCH");
        assert!(list_instances(&fixture.database).unwrap().is_empty());
    }
}
