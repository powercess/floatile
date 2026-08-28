//! PluginManager：从插件存储加载已安装插件（S6）。
//!
//! 插件存储中的内容在安装时已由 `floatile-cli` 完整校验，并记录每文件 SHA-256 与
//! 覆盖全部规范文件集合的聚合 digest。PluginManager 加载前按 `install.json` 重算并
//! 校验 digest，确认安装后内容未被篡改，再把可信的 wasm/manifest 交给 runtime。
//!
//! shell 不解析不可信原始 `.floatile` 包——安装期校验与解包由 CLI/安装器完成，这里
//! 只读取本机已安装且已验证的产物。任意 UI 的动态渲染仍受「运行时编译 ADR」门禁；
//! 本模块只提供参考时钟等 dev 包的宿主加载路径。

use std::path::{Path, PathBuf};

use floatile_core::distribution::{SIGNATURE_FILE, verify_signature_envelope};
use floatile_core::install::{InstallMeta, InstallationTrust};
use floatile_core::instance::InstallationRef;
use floatile_core::manifest::Manifest;
use floatile_core::{InstanceDesiredState, InstanceId, PluginInstance};
use floatile_store::installation::{
    ConfigValidationError, InstallationCatalogError, InstalledInstallation, list_highest,
    load_highest, load_reference,
};
use floatile_store::{Store, StoreError};
use thiserror::Error;

/// 已加载的已安装插件（已通过 digest 完整性校验）。
#[derive(Debug)]
pub struct InstalledPlugin {
    pub manifest: Manifest,
    pub meta: InstallMeta,
    /// `entrypoints.logic` 指向的 WASM 字节。
    pub wasm: Vec<u8>,
    /// `entrypoints.ui` 指向的 widget.ftui 字节。
    pub ui_bytes: Vec<u8>,
}

/// 已按持久实例引用复核、可以交给 runtime UI 启动的单元。
#[derive(Debug)]
pub struct RunnableInstance {
    pub instance: PluginInstance,
    pub plugin: InstalledPlugin,
}

/// 单实例恢复失败；不包含 Config、State 或其他敏感值。
#[derive(Debug)]
pub struct InstanceLoadFailure {
    pub instance_id: InstanceId,
    pub plugin_id: String,
    pub code: &'static str,
    pub detail: String,
}

/// 宿主启动时的实例恢复计划。一个实例失败不清空其他 ready 实例。
#[derive(Debug, Default)]
pub struct RuntimeInstancePlan {
    pub ready: Vec<RunnableInstance>,
    pub failures: Vec<InstanceLoadFailure>,
}

/// 插件存储根目录：优先 `$FLOATTILE_PLUGIN_DIR`，否则平台数据目录下的 `plugins`。
pub fn plugin_store() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FLOATTILE_PLUGIN_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    floatile_platform::data_dir()
        .ok()
        .map(|d| d.join("plugins"))
}

/// 加载错误（稳定 code `FLOAD_*`）。
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("读取安装目录失败: {0}")]
    Read(String),
    #[error("install.json 缺失或损坏: {0}")]
    InvalidMeta(String),
    #[error("插件 {id} 文件 `{file}` digest 不匹配")]
    DigestMismatch { id: String, file: String },
    #[error("安装 {id}@{version} 的内容身份与实例记录不匹配")]
    InstallationMismatch { id: String, version: String },
    #[error("缺少 manifest.json")]
    MissingManifest,
    #[error("缺少入口 `{0}`")]
    MissingEntrypoint(String),
    #[error("manifest 非法: {0}")]
    InvalidManifest(#[from] floatile_core::manifest::ManifestError),
    #[error("实例 Config 契约校验失败: {0}")]
    InvalidConfig(#[from] ConfigValidationError),
    #[error("受信安装缺少 detached signature.json")]
    MissingSignature,
    #[error("受信安装的 publisher 不在宿主 trust store 中")]
    UnknownPublisher,
    #[error("受信安装签名校验失败: {0}")]
    Signature(#[from] floatile_core::SignatureVerificationError),
    #[error("读取 publisher trust 失败: {0}")]
    TrustStore(String),
}

impl LoadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Read(_) => "FLOAD_READ",
            Self::InvalidMeta(_) => "FLOAD_INVALID_META",
            Self::DigestMismatch { .. } => "FLOAD_DIGEST_MISMATCH",
            Self::InstallationMismatch { .. } => "FLOAD_INSTALLATION_MISMATCH",
            Self::MissingManifest => "FLOAD_MISSING_MANIFEST",
            Self::MissingEntrypoint(_) => "FLOAD_MISSING_ENTRYPOINT",
            Self::InvalidManifest(_) => "FLOAD_INVALID_MANIFEST",
            Self::InvalidConfig(error) => error.code(),
            Self::MissingSignature => "FLOAD_SIGNATURE_MISSING",
            Self::UnknownPublisher => "FLOAD_PUBLISHER_UNKNOWN",
            Self::Signature(error) => match error {
                floatile_core::SignatureVerificationError::PublisherRevoked => {
                    "FLOAD_PUBLISHER_REVOKED"
                }
                floatile_core::SignatureVerificationError::KeyRevoked => "FLOAD_KEY_REVOKED",
                floatile_core::SignatureVerificationError::UnknownKey => "FLOAD_KEY_UNKNOWN",
                floatile_core::SignatureVerificationError::DigestMismatch => {
                    "FLOAD_SIGNATURE_DIGEST"
                }
                floatile_core::SignatureVerificationError::InvalidSignature => {
                    "FLOAD_SIGNATURE_INVALID"
                }
                _ => "FLOAD_SIGNATURE_MALFORMED",
            },
            Self::TrustStore(_) => "FLOAD_TRUST_STORE",
        }
    }
}

/// 加载某插件 id 的最高已安装版本。
///
/// 无任何已安装版本返回 `Ok(None)`（调用方回退内建实现）；存在多个版本时按 semver
/// 取最高；任意文件 digest 不匹配返回错误并拒绝加载。
pub fn load_installed(store: &Path, id: &str) -> Result<Option<InstalledPlugin>, LoadError> {
    load_highest(store, id)
        .map_err(map_catalog_error)?
        .map(installed_plugin)
        .transpose()
}

/// 加载实例固定引用的精确 Installation，不静默选择更高版本。
///
/// 目录不存在返回 `Ok(None)`；版本存在但 install metadata 的插件、版本或 digest 与引用不一致时
/// 明确拒绝，避免实例在宿主重启后静默切换内容。
pub fn load_installation(
    store: &Path,
    reference: &InstallationRef,
) -> Result<Option<InstalledPlugin>, LoadError> {
    load_reference(store, reference)
        .map_err(|error| match error {
            InstallationCatalogError::MetadataMismatch => LoadError::InstallationMismatch {
                id: reference.plugin().0.clone(),
                version: reference.version().to_owned(),
            },
            other => map_catalog_error(other),
        })?
        .map(installed_plugin)
        .transpose()
}

/// 按持久实例的精确 Installation 引用加载运行单元，并在交给 runtime
/// 前复验 Config schema。
pub fn load_runnable_instance(
    store: &Path,
    instance: PluginInstance,
) -> Result<Option<RunnableInstance>, LoadError> {
    let reference = instance.installation();
    let installation = load_reference(store, reference).map_err(|error| match error {
        InstallationCatalogError::MetadataMismatch => LoadError::InstallationMismatch {
            id: reference.plugin().0.clone(),
            version: reference.version().to_owned(),
        },
        other => map_catalog_error(other),
    })?;
    let Some(installation) = installation else {
        return Ok(None);
    };
    installation.validate_config(instance.config())?;
    let plugin = installed_plugin(installation)?;
    Ok(Some(RunnableInstance { instance, plugin }))
}

/// Production load path: trusted installations are re-verified against current host trust.
/// Explicit unsigned development installations remain marked and use the sandboxed dev path.
pub fn load_runnable_instance_with_trust(
    plugin_store: &Path,
    trust_store: &Store,
    instance: PluginInstance,
) -> Result<Option<RunnableInstance>, LoadError> {
    let reference = instance.installation();
    let installation = load_reference(plugin_store, reference).map_err(|error| match error {
        InstallationCatalogError::MetadataMismatch => LoadError::InstallationMismatch {
            id: reference.plugin().0.clone(),
            version: reference.version().to_owned(),
        },
        other => map_catalog_error(other),
    })?;
    let Some(installation) = installation else {
        return Ok(None);
    };
    installation.validate_config(instance.config())?;
    verify_runtime_trust(&installation, trust_store)?;
    let plugin = installed_plugin(installation)?;
    Ok(Some(RunnableInstance { instance, plugin }))
}

/// 从持久记录恢复 desired-running 实例，并固定到精确 Installation。
///
/// 该函数执行 SQLite 与文件 I/O，必须在 Slint 事件循环启动前或后台线程调用。每次启动尝试先推进
/// generation，以便后续 Operation 丢弃旧 generation 的迟到结果；单实例失败记录在 plan 中并继续。
pub fn plan_running_instances(
    store: &Store,
    plugin_store: &Path,
    unix_ts: u64,
) -> Result<RuntimeInstancePlan, StoreError> {
    let mut plan = RuntimeInstancePlan::default();
    for instance in store.instances().list()? {
        if instance.desired_state() != InstanceDesiredState::Running {
            continue;
        }
        let instance_id = instance.id();
        let plugin_id = instance.installation().plugin().0.clone();
        let updated_at = unix_ts.max(instance.updated_at());
        match store
            .instances()
            .advance_generation(instance_id, updated_at)
        {
            Ok(Some(_)) => {}
            Ok(None) => {
                plan.failures.push(InstanceLoadFailure {
                    instance_id,
                    plugin_id,
                    code: "FINSTANCE_GENERATION",
                    detail: "实例不存在、时间戳过期或 generation 已耗尽".to_owned(),
                });
                continue;
            }
            Err(error) => {
                plan.failures.push(InstanceLoadFailure {
                    instance_id,
                    plugin_id,
                    code: "FINSTANCE_STORE",
                    detail: error.to_string(),
                });
                continue;
            }
        }

        let instance = match store.instances().get(instance_id) {
            Ok(Some(instance)) => instance,
            Ok(None) => {
                plan.failures.push(InstanceLoadFailure {
                    instance_id,
                    plugin_id,
                    code: "FINSTANCE_MISSING",
                    detail: "推进 generation 后实例记录消失".to_owned(),
                });
                continue;
            }
            Err(error) => {
                plan.failures.push(InstanceLoadFailure {
                    instance_id,
                    plugin_id,
                    code: "FINSTANCE_STORE",
                    detail: error.to_string(),
                });
                continue;
            }
        };

        let version = instance.installation().version().to_owned();
        match load_runnable_instance_with_trust(plugin_store, store, instance) {
            Ok(Some(runnable)) => plan.ready.push(runnable),
            Ok(None) => plan.failures.push(InstanceLoadFailure {
                instance_id,
                plugin_id,
                code: "FLOAD_INSTALLATION_MISSING",
                detail: format!("安装版本 {version} 不存在"),
            }),
            Err(error) => plan.failures.push(InstanceLoadFailure {
                instance_id,
                plugin_id,
                code: error.code(),
                detail: error.to_string(),
            }),
        }
    }
    Ok(plan)
}

fn verify_runtime_trust(
    installation: &InstalledInstallation,
    store: &Store,
) -> Result<(), LoadError> {
    if installation.meta.trust == InstallationTrust::Unsigned {
        return Ok(());
    }
    let publisher_id = installation.manifest.publisher.id.as_str();
    let trust = store
        .trust()
        .get(publisher_id)
        .map_err(|error| LoadError::TrustStore(error.to_string()))?
        .ok_or(LoadError::UnknownPublisher)?;
    let mut files = std::collections::BTreeMap::new();
    for name in installation.meta.files.keys() {
        let bytes = installation
            .file(name)
            .ok_or_else(|| LoadError::Read(format!("安装文件缺失: {name}")))?;
        files.insert(name.clone(), bytes.to_vec());
    }
    let envelope = files
        .get(SIGNATURE_FILE)
        .ok_or(LoadError::MissingSignature)?;
    verify_signature_envelope(envelope, &files, publisher_id, &trust.verifier_binding())?;
    Ok(())
}

/// 枚举插件存储下全部已安装插件，每个 id 取最高已安装版本并逐一做 digest 复核。
///
/// 这是宿主运行多个插件的加载策略：返回稳定的已安装插件集合，供 UI 层按 id 创建
/// 各自实例；任意一个插件 digest 不匹配都会返回错误（拒绝加载，交由调用方决定是
/// 隔离失败还是整体拒绝），绝不会静默跳过被篡改的插件。
pub fn list_installed(store: &Path) -> Result<Vec<InstalledPlugin>, LoadError> {
    list_highest(store)
        .map_err(map_catalog_error)?
        .into_iter()
        .map(installed_plugin)
        .collect()
}

fn installed_plugin(installation: InstalledInstallation) -> Result<InstalledPlugin, LoadError> {
    let wasm = installation
        .file(installation.manifest.entrypoints.logic.as_str())
        .ok_or_else(|| {
            LoadError::MissingEntrypoint(
                installation.manifest.entrypoints.logic.as_str().to_owned(),
            )
        })?
        .to_vec();
    let ui_bytes = installation
        .file(installation.manifest.entrypoints.ui.as_str())
        .ok_or_else(|| {
            LoadError::MissingEntrypoint(installation.manifest.entrypoints.ui.as_str().to_owned())
        })?
        .to_vec();
    Ok(InstalledPlugin {
        manifest: installation.manifest,
        meta: installation.meta,
        wasm,
        ui_bytes,
    })
}

fn map_catalog_error(error: InstallationCatalogError) -> LoadError {
    match error {
        InstallationCatalogError::Read(detail) => LoadError::Read(detail),
        InstallationCatalogError::InvalidMeta(detail) => LoadError::InvalidMeta(detail),
        InstallationCatalogError::DigestMismatch { id, file } => {
            LoadError::DigestMismatch { id, file }
        }
        InstallationCatalogError::MetadataMismatch => {
            LoadError::InvalidMeta("安装元数据与 manifest 身份不一致".to_owned())
        }
        InstallationCatalogError::InvalidIdentity(error) => {
            LoadError::InvalidMeta(error.to_string())
        }
        InstallationCatalogError::InvalidManifest(error) => LoadError::InvalidManifest(error),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use floatile_core::install::{content_digest, file_digest, hex_encode};
    use floatile_store::trust::TrustState;

    fn manifest_json() -> String {
        serde_json::json!({
            "manifestVersion": 1,
            "id": "dev.floatile.clock",
            "name": "World Clock",
            "version": "0.2.0",
            "publisher": { "id": "dev.floatile", "name": "Floatile Labs" },
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "config": { "schema": "config.schema.json" },
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": []
        })
        .to_string()
    }

    fn manifest_json_for(id: &str, version: &str) -> String {
        let mut manifest = serde_json::from_str::<serde_json::Value>(&manifest_json()).unwrap();
        manifest["id"] = serde_json::json!(id);
        manifest["version"] = serde_json::json!(version);
        manifest.to_string()
    }

    fn temp_store(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("floatile-pm-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 安装一个模拟版本目录（直接按 CLI 布列写 install.json + 文件）。
    fn write_install(store: &Path, id: &str, version: &str, tamper: Option<&str>) {
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        files.insert(
            "manifest.json".into(),
            manifest_json_for(id, version).into_bytes(),
        );
        files.insert("ui/widget.ftui".into(), b"{\"ui\":1}".to_vec());
        files.insert("logic/plugin.wasm".into(), vec![1, 2, 3, 4]);
        files.insert(
            "config.schema.json".into(),
            serde_json::to_vec(&serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "timezone": { "type": "string" } }
            }))
            .unwrap(),
        );

        let mut file_digests = BTreeMap::new();
        for (name, bytes) in &files {
            file_digests.insert(name.clone(), hex_encode(&file_digest(bytes)));
        }
        // 可选：篡改某文件内容后仍按伪造 digest 写，或改 content 后再校验失败。
        let meta = InstallMeta {
            manifest_version: 1,
            id: id.into(),
            version: version.into(),
            engine_api_version: "1.0.0".into(),
            ui_api_version: "1.0.0".into(),
            installed_at: 0,
            source: "x.floatile".into(),
            trust: InstallationTrust::Unsigned,
            files: file_digests.clone(),
            digest: hex_encode(&content_digest(&files)),
        };

        let version_dir = store.join(id).join(version);
        std::fs::create_dir_all(&version_dir).unwrap();
        for (name, bytes) in &files {
            let path = version_dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
        }
        // 篡改：改 wasm 文件内容（digest 记录不变 → 加载应拒绝）。
        if let Some(file) = tamper {
            std::fs::write(version_dir.join(file), b"TAMPERED").unwrap();
        }
        std::fs::write(
            version_dir.join("install.json"),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
    }

    fn installation_ref(store: &Path, id: &str, version: &str) -> InstallationRef {
        let bytes = std::fs::read(store.join(id).join(version).join("install.json")).unwrap();
        let meta: InstallMeta = serde_json::from_slice(&bytes).unwrap();
        InstallationRef::from_install_meta(&meta).unwrap()
    }

    #[test]
    fn runtime_refuses_trusted_marker_without_signature() {
        let plugin_store = temp_store("runtime-trust");
        write_install(&plugin_store, "dev.floatile.clock", "0.2.0", None);
        let meta_path = plugin_store.join("dev.floatile.clock/0.2.0/install.json");
        let mut meta: InstallMeta =
            serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
        meta.trust = InstallationTrust::Trusted;
        std::fs::write(&meta_path, serde_json::to_vec(&meta).unwrap()).unwrap();
        let reference = InstallationRef::from_install_meta(&meta).unwrap();
        let store = floatile_store::open(":memory:").unwrap();
        store
            .trust()
            .upsert_key("dev.floatile", [31; 32], TrustState::Active, 1)
            .unwrap();
        let instance = PluginInstance::restore(
            InstanceId(9),
            reference,
            floatile_core::InstanceConfig::empty(),
            InstanceDesiredState::Stopped,
            0,
            1,
            1,
        )
        .unwrap();
        let error = load_runnable_instance_with_trust(&plugin_store, &store, instance).unwrap_err();
        assert!(matches!(error, LoadError::MissingSignature));
    }

    #[test]
    fn loads_highest_version_and_verifies_digest() {
        let store = temp_store("load");
        write_install(&store, "dev.floatile.clock", "0.1.0", None);
        write_install(&store, "dev.floatile.clock", "0.2.0", None);

        let plugin = load_installed(&store, "dev.floatile.clock")
            .unwrap()
            .expect("应加载到已安装插件");
        assert_eq!(plugin.meta.version, "0.2.0");
        assert_eq!(plugin.wasm, vec![1, 2, 3, 4]);
        assert_eq!(plugin.ui_bytes, b"{\"ui\":1}");
        assert_eq!(plugin.manifest.id.0, "dev.floatile.clock");
    }

    #[test]
    fn loads_exact_installation_without_silent_upgrade() {
        let store = temp_store("exact");
        write_install(&store, "dev.floatile.clock", "0.1.0", None);
        write_install(&store, "dev.floatile.clock", "0.2.0", None);
        let reference = installation_ref(&store, "dev.floatile.clock", "0.1.0");

        let plugin = load_installation(&store, &reference)
            .unwrap()
            .expect("精确安装应存在");

        assert_eq!(plugin.meta.version, "0.1.0");
        assert_eq!(
            InstallationRef::from_install_meta(&plugin.meta).unwrap(),
            reference
        );
    }

    #[test]
    fn rejects_installation_with_different_digest() {
        let store = temp_store("exact-digest");
        write_install(&store, "dev.floatile.clock", "0.1.0", None);
        let reference = InstallationRef::new(
            floatile_core::PluginId("dev.floatile.clock".into()),
            "0.1.0",
            floatile_core::InstallationDigest::from_bytes([0xff; 32]),
        )
        .unwrap();

        let error = load_installation(&store, &reference).unwrap_err();
        assert!(matches!(error, LoadError::InstallationMismatch { .. }));
        assert_eq!(error.code(), "FLOAD_INSTALLATION_MISMATCH");
    }

    #[test]
    fn exact_installation_returns_none_when_version_is_absent() {
        let store = temp_store("exact-none");
        let reference = InstallationRef::new(
            floatile_core::PluginId("dev.floatile.clock".into()),
            "9.9.9",
            floatile_core::InstallationDigest::from_bytes([0xff; 32]),
        )
        .unwrap();
        assert!(load_installation(&store, &reference).unwrap().is_none());
    }

    #[test]
    fn returns_none_when_not_installed() {
        let store = temp_store("none");
        assert!(
            load_installed(&store, "dev.floatile.missing")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_tampered_file() {
        let store = temp_store("tamper");
        write_install(
            &store,
            "dev.floatile.clock",
            "0.1.0",
            Some("logic/plugin.wasm"),
        );
        let err = load_installed(&store, "dev.floatile.clock").unwrap_err();
        assert!(matches!(err, LoadError::DigestMismatch { .. }));
        assert_eq!(err.code(), "FLOAD_DIGEST_MISMATCH");
    }

    #[test]
    fn lists_multiple_installed_plugins_sorted() {
        let store = temp_store("list");
        // 两个不同插件，各一版本。
        write_install(&store, "dev.floatile.clock", "1.0.0", None);
        write_install(&store, "dev.floatile.cpu", "0.3.0", None);

        let plugins = list_installed(&store).unwrap();
        assert_eq!(plugins.len(), 2);
        // 按 id 稳定排序。
        assert_eq!(plugins[0].manifest.id.0, "dev.floatile.clock");
        assert_eq!(plugins[1].manifest.id.0, "dev.floatile.cpu");
    }

    #[test]
    fn lists_each_plugin_highest_version() {
        let store = temp_store("list-vers");
        write_install(&store, "dev.floatile.cpu", "0.1.0", None);
        write_install(&store, "dev.floatile.cpu", "0.2.0", None);
        write_install(&store, "dev.floatile.clock", "1.0.0", None);

        let plugins = list_installed(&store).unwrap();
        assert_eq!(plugins.len(), 2);
        let cpu = plugins
            .iter()
            .find(|p| p.manifest.id.0 == "dev.floatile.cpu")
            .unwrap();
        assert_eq!(cpu.meta.version, "0.2.0");
    }

    #[test]
    fn list_skips_non_id_and_returns_error_on_tamper() {
        let store = temp_store("list-tamper");
        // 非 id 目录不应被当作插件。
        let junk = store.join("..junk");
        std::fs::create_dir_all(&junk).unwrap();
        std::fs::write(junk.join("install.json"), b"{}").unwrap();
        // 被篡改的插件 → 整体返回 digest 错误，不清零静默跳过。
        write_install(
            &store,
            "dev.floatile.clock",
            "1.0.0",
            Some("logic/plugin.wasm"),
        );

        assert!(matches!(
            list_installed(&store),
            Err(LoadError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn plans_only_running_instances_and_advances_generation() {
        let plugin_store = temp_store("instance-plan");
        write_install(&plugin_store, "dev.floatile.clock", "1.0.0", None);
        let installation = installation_ref(&plugin_store, "dev.floatile.clock", "1.0.0");
        let store = floatile_store::open(":memory:").unwrap();
        let first = store
            .instances()
            .create(
                &installation,
                &floatile_core::InstanceConfig::new(serde_json::json!({"timezone": "UTC"}))
                    .unwrap(),
                InstanceDesiredState::Running,
                100,
            )
            .unwrap();
        let second = store
            .instances()
            .create(
                &installation,
                &floatile_core::InstanceConfig::new(
                    serde_json::json!({"timezone": "Asia/Shanghai"}),
                )
                .unwrap(),
                InstanceDesiredState::Running,
                100,
            )
            .unwrap();
        let stopped = store
            .instances()
            .create(
                &installation,
                &floatile_core::InstanceConfig::empty(),
                InstanceDesiredState::Stopped,
                100,
            )
            .unwrap();

        // 宿主时钟回拨也不得阻止启动；updated_at 维持单调。
        let plan = plan_running_instances(&store, &plugin_store, 90).unwrap();

        assert!(plan.failures.is_empty());
        assert_eq!(plan.ready.len(), 2);
        assert_eq!(plan.ready[0].instance.id(), first.id());
        assert_eq!(plan.ready[1].instance.id(), second.id());
        assert_eq!(plan.ready[0].instance.generation(), 1);
        assert_eq!(plan.ready[1].instance.generation(), 1);
        assert_eq!(
            plan.ready[0].instance.config().to_value(),
            serde_json::json!({"timezone": "UTC"})
        );
        assert_eq!(
            store
                .instances()
                .get(stopped.id())
                .unwrap()
                .unwrap()
                .generation(),
            0
        );
    }

    #[test]
    fn instance_plan_revalidates_config_before_runtime() {
        let plugin_store = temp_store("instance-config-reject");
        write_install(&plugin_store, "dev.floatile.clock", "1.0.0", None);
        let installation = installation_ref(&plugin_store, "dev.floatile.clock", "1.0.0");
        let store = floatile_store::open(":memory:").unwrap();
        let instance = store
            .instances()
            .create(
                &installation,
                &floatile_core::InstanceConfig::new(serde_json::json!({"secret": "redacted"}))
                    .unwrap(),
                InstanceDesiredState::Running,
                100,
            )
            .unwrap();

        let plan = plan_running_instances(&store, &plugin_store, 101).unwrap();

        assert!(plan.ready.is_empty());
        assert_eq!(plan.failures.len(), 1);
        assert_eq!(plan.failures[0].instance_id, instance.id());
        assert_eq!(plan.failures[0].code, "FCONFIG_VALUE_INVALID");
        assert!(!plan.failures[0].detail.contains("redacted"));
    }

    #[test]
    fn instance_plan_isolates_tampered_installation() {
        let plugin_store = temp_store("instance-isolation");
        write_install(&plugin_store, "dev.floatile.clock", "1.0.0", None);
        write_install(
            &plugin_store,
            "dev.floatile.cpu",
            "1.0.0",
            Some("logic/plugin.wasm"),
        );
        let healthy = installation_ref(&plugin_store, "dev.floatile.clock", "1.0.0");
        let tampered = installation_ref(&plugin_store, "dev.floatile.cpu", "1.0.0");
        let store = floatile_store::open(":memory:").unwrap();
        let healthy_instance = store
            .instances()
            .create(
                &healthy,
                &floatile_core::InstanceConfig::empty(),
                InstanceDesiredState::Running,
                100,
            )
            .unwrap();
        let tampered_instance = store
            .instances()
            .create(
                &tampered,
                &floatile_core::InstanceConfig::empty(),
                InstanceDesiredState::Running,
                100,
            )
            .unwrap();

        let plan = plan_running_instances(&store, &plugin_store, 101).unwrap();

        assert_eq!(plan.ready.len(), 1);
        assert_eq!(plan.ready[0].instance.id(), healthy_instance.id());
        assert_eq!(plan.failures.len(), 1);
        assert_eq!(plan.failures[0].instance_id, tampered_instance.id());
        assert_eq!(plan.failures[0].plugin_id, "dev.floatile.cpu");
        assert_eq!(plan.failures[0].code, "FLOAD_DIGEST_MISMATCH");
        assert_eq!(
            store
                .instances()
                .get(tampered_instance.id())
                .unwrap()
                .unwrap()
                .generation(),
            1
        );
    }
}
