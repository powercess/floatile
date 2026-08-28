//! 原子安装：校验 → staging → digest → 原子 rename 进插件存储 + install.json。
//!
//! 对应 manifest-v1 §6 的安全校验顺序第 9/10 步：为全部允许文件计算 digest，全部
//! 检查通过后原子移动进插件存储；任何失败清理暂存目录，绝不留下半安装状态。
//!
//! 安装目录约定：`<store>/<id>/<version>/`，其中 `<store>` 由调用方指定（CLI 默认
//! 取 `--store`，否则 `$FLOATTILE_PLUGIN_DIR`）；`install.json` 记录 id/version、
//! 每文件 SHA-256 与覆盖全部规范文件集合的聚合 digest，供宿主 PluginManager 在
//! 加载前做完整性校验。

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use floatile_core::distribution::{
    SIGNATURE_FILE, SignatureVerificationError, UpgradePlan, UpgradePlanError, plan_upgrade,
    signable_content_digest, verify_signature_envelope,
};
use floatile_core::install::{InstallMeta, content_digest, file_digest, hex_encode};
use floatile_core::manifest::Manifest;
use floatile_store::Store;
use floatile_store::installation::InstalledInstallation;
use floatile_store::installation::{InstallationCatalogError, load_highest};
use floatile_store::trust::{PendingInstallation, TrustPolicyError};
use semver::Version;
use thiserror::Error;

use crate::package::{PackageError, PackageLimits, ValidatedPackage, validate_package};

/// 安装错误（稳定 code `FINST_*`）。
#[derive(Debug, Error)]
pub enum InstallError {
    #[error("包校验失败: {0}")]
    Package(#[from] PackageError),
    #[error("插件存储根目录不可用: {0}")]
    StoreUnavailable(String),
    #[error("插件 {id} {version} 已安装")]
    AlreadyInstalled { id: String, version: String },
    #[error("写入暂存目录失败: {0}")]
    StagingWrite(String),
    #[error("提交安装失败: {0}")]
    Commit(String),
    #[error("I/O 失败: {0}")]
    Io(String),
    #[error("包缺少 detached signature.json")]
    MissingSignature,
    #[error("找不到 manifest publisher 的宿主信任记录")]
    UnknownPublisher,
    #[error("签名验证失败: {0}")]
    Signature(#[from] SignatureVerificationError),
    #[error("安装信任策略拒绝: {0}")]
    TrustPolicy(#[from] TrustPolicyError),
    #[error("安装已落盘但信任状态尚待恢复: transaction={0}")]
    RecoveryRequired(String),
    #[error("恢复安装事务失败: {0}")]
    Recovery(String),
    #[error("升级兼容性检查失败: {0}")]
    Upgrade(#[from] UpgradePlanError),
    #[error("升级扩大权限，必须显式确认")]
    PermissionConfirmationRequired(UpgradePlan),
    #[error("读取当前安装失败: {0}")]
    InstallationCatalog(#[from] InstallationCatalogError),
}

impl InstallError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Package(_) => "FINST_PACKAGE",
            Self::StoreUnavailable(_) => "FINST_STORE_UNAVAILABLE",
            Self::AlreadyInstalled { .. } => "FINST_ALREADY_INSTALLED",
            Self::StagingWrite(_) => "FINST_STAGING_WRITE",
            Self::Commit(_) => "FINST_COMMIT",
            Self::Io(_) => "FINST_IO",
            Self::MissingSignature => "FINST_SIGNATURE_MISSING",
            Self::UnknownPublisher => "FINST_PUBLISHER_UNKNOWN",
            Self::Signature(error) => match error {
                SignatureVerificationError::PublisherRevoked => "FINST_PUBLISHER_REVOKED",
                SignatureVerificationError::KeyRevoked => "FINST_KEY_REVOKED",
                SignatureVerificationError::UnknownKey => "FINST_KEY_UNKNOWN",
                SignatureVerificationError::DigestMismatch => "FINST_SIGNATURE_DIGEST",
                SignatureVerificationError::InvalidSignature => "FINST_SIGNATURE_INVALID",
                _ => "FINST_SIGNATURE_MALFORMED",
            },
            Self::TrustPolicy(error) => match error {
                TrustPolicyError::UnknownPublisher => "FINST_PUBLISHER_UNKNOWN",
                TrustPolicyError::RevokedPublisher => "FINST_PUBLISHER_REVOKED",
                TrustPolicyError::Rollback { .. } => "FINST_ROLLBACK",
                TrustPolicyError::SameVersionDifferentDigest => "FINST_VERSION_REPLACED",
                TrustPolicyError::Store(_) => "FINST_TRUST_STORE",
            },
            Self::RecoveryRequired(_) => "FINST_RECOVERY_REQUIRED",
            Self::Recovery(_) => "FINST_RECOVERY_FAILED",
            Self::Upgrade(_) => "FINST_UPGRADE_INCOMPATIBLE",
            Self::PermissionConfirmationRequired(_) => "FINST_PERMISSION_CONFIRMATION",
            Self::InstallationCatalog(_) => "FINST_INSTALLED_INVALID",
        }
    }

    pub fn public_detail(&self) -> Cow<'static, str> {
        match self {
            Self::Package(_) => Cow::Borrowed("插件包未通过安全校验"),
            Self::StoreUnavailable(_) => Cow::Borrowed("插件存储不可用"),
            Self::AlreadyInstalled { id, version } => {
                Cow::Owned(format!("插件 {id} {version} 已安装"))
            }
            Self::StagingWrite(_) => Cow::Borrowed("无法写入安装暂存目录"),
            Self::Commit(_) => Cow::Borrowed("无法原子提交安装"),
            Self::Io(_) => Cow::Borrowed("插件包读取失败"),
            Self::MissingSignature => Cow::Borrowed("插件包缺少签名"),
            Self::UnknownPublisher => Cow::Borrowed("插件发布者不受信任"),
            Self::Signature(_) => Cow::Borrowed("插件签名未通过验证"),
            Self::TrustPolicy(_) => Cow::Borrowed("插件安装被信任或版本策略拒绝"),
            Self::RecoveryRequired(_) => Cow::Borrowed("安装需要在下次运行时恢复"),
            Self::Recovery(_) => Cow::Borrowed("无法安全恢复上次安装事务"),
            Self::Upgrade(_) => Cow::Borrowed("候选包与当前安装不兼容"),
            Self::PermissionConfirmationRequired(_) => {
                Cow::Borrowed("升级新增或扩大权限，需要显式确认")
            }
            Self::InstallationCatalog(_) => Cow::Borrowed("当前安装未通过完整性校验"),
        }
    }
}

/// 安装结果（供 CLI 输出与宿主后续读取）。
#[derive(Debug)]
pub struct InstalledPackage {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub meta: InstallMeta,
    pub upgrade: Option<UpgradePlan>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub finalized: usize,
    pub aborted: usize,
}

/// 插件存储根目录下某插件的安装目录：`<store>/<id>/<version>`。
///
/// `id` 与 `version` 都经过 manifest 校验（反向域名 + 严格 semver），因此路径安全；
/// 但仍由本函数集中定义布局，CLI 写入与宿主读取共享同一约定。
pub fn install_dir(store: &Path, id: &str, version: &str) -> PathBuf {
    store.join(id).join(version)
}

/// 原子安装一个已通过完整校验的 `.floatile` 包。
///
/// 语义：
/// 1. 校验（复用 `validate_package`，含路径/预算/zip-bomb/manifest/UI/WASM 拒绝）；
/// 2. 在 `<store>` 下创建同文件系统的 `.staging-*` 兄弟目录并写入全部校验通过的条目
///    与 `install.json`（含每文件 digest 与聚合 digest），逐文件 fsync；
/// 3. `rename(staging, final)` 原子提交；任何失败删除 staging，绝不产生半安装目录。
pub fn install_package(
    bytes: &[u8],
    store: &Path,
    source: &str,
    limits: &PackageLimits,
) -> Result<InstalledPackage, InstallError> {
    let validated = validate_package(bytes, limits)?;
    install_validated(&validated, store, source)
}

/// Installs a package only after a detached signature verifies against host-owned publisher trust.
pub fn install_trusted_package(
    bytes: &[u8],
    plugin_store: &Path,
    source: &str,
    limits: &PackageLimits,
    trust_store: &Store,
    accept_permission_expansion: bool,
) -> Result<InstalledPackage, InstallError> {
    let validated = validate_package(bytes, limits)?;
    let publisher_id = validated.manifest.publisher.id.as_str();
    let trust = trust_store
        .trust()
        .get(publisher_id)
        .map_err(TrustPolicyError::Store)?
        .ok_or(InstallError::UnknownPublisher)?;
    let envelope = validated
        .files
        .get(SIGNATURE_FILE)
        .ok_or(InstallError::MissingSignature)?;
    verify_signature_envelope(
        envelope,
        &validated.files,
        publisher_id,
        &trust.verifier_binding(),
    )?;

    let transaction_id = nonce();
    let staging_name = format!(".staging-{transaction_id}");
    let staging = plugin_store.join(&staging_name);
    let manifest = validated.manifest.clone();
    let final_dir = install_dir(plugin_store, &manifest.id.0, &manifest.version);
    if final_dir.exists() {
        return Err(InstallError::AlreadyInstalled {
            id: manifest.id.0,
            version: manifest.version,
        });
    }
    let upgrade = match load_highest(plugin_store, &manifest.id.0)? {
        Some(current)
            if Version::parse(&manifest.version)
                .map_err(|error| InstallError::Commit(format!("validated version: {error}")))?
                > Version::parse(&current.manifest.version).map_err(|error| {
                    InstallError::Commit(format!("installed version passed validation: {error}"))
                })? =>
        {
            let plan = plan_upgrade(&current.manifest, &manifest)?;
            if plan.requires_confirmation && !accept_permission_expansion {
                return Err(InstallError::PermissionConfirmationRequired(plan));
            }
            Some(plan)
        }
        _ => None,
    };
    let parent = final_dir.parent().unwrap_or(plugin_store);
    fs::create_dir_all(parent)
        .map_err(|error| InstallError::StoreUnavailable(error.to_string()))?;
    let meta = match write_staging(&staging, &validated, &manifest, source) {
        Ok(meta) => meta,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let version = Version::parse(&manifest.version)
        .map_err(|error| InstallError::Commit(format!("validated version: {error}")))?;
    let pending = PendingInstallation {
        transaction_id: transaction_id.clone(),
        publisher_id: publisher_id.to_owned(),
        plugin_id: manifest.id.0.clone(),
        version,
        signed_digest: signable_content_digest(&validated.files),
        install_digest: content_digest(&validated.files),
        staging_name,
        final_relative: format!("{}/{}", manifest.id.0, manifest.version),
        created_at: meta.installed_at,
    };
    if let Err(error) = trust_store.trust().prepare_install(&pending) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&staging, &final_dir) {
        let _ = fs::remove_dir_all(&staging);
        let _ = trust_store.trust().abort_install(&transaction_id);
        return Err(InstallError::Commit(error.to_string()));
    }
    if trust_store
        .trust()
        .finalize_install(&transaction_id, meta.installed_at)
        .is_err()
    {
        return Err(InstallError::RecoveryRequired(transaction_id));
    }

    Ok(InstalledPackage {
        dir: final_dir,
        manifest,
        meta,
        upgrade,
    })
}

/// Re-verifies an installed package against current host trust before rollback or execution.
pub fn verify_trusted_installation(
    installed: &InstalledInstallation,
    trust_store: &Store,
) -> Result<(), InstallError> {
    let publisher_id = installed.manifest.publisher.id.as_str();
    let trust = trust_store
        .trust()
        .get(publisher_id)
        .map_err(TrustPolicyError::Store)?
        .ok_or(InstallError::UnknownPublisher)?;
    let mut files = BTreeMap::new();
    for name in installed.meta.files.keys() {
        let bytes = installed
            .file(name)
            .ok_or_else(|| InstallError::Commit(format!("installed file missing: {name}")))?;
        files.insert(name.clone(), bytes.to_vec());
    }
    let envelope = files
        .get(SIGNATURE_FILE)
        .ok_or(InstallError::MissingSignature)?;
    verify_signature_envelope(envelope, &files, publisher_id, &trust.verifier_binding())?;
    Ok(())
}

/// Reconciles crash-interrupted trusted installs before a new trusted install begins.
pub fn recover_trusted_installs(
    plugin_store: &Path,
    trust_store: &Store,
) -> Result<RecoveryReport, InstallError> {
    let mut report = RecoveryReport::default();
    for pending in trust_store
        .trust()
        .pending_installs()
        .map_err(TrustPolicyError::Store)?
    {
        let staging = plugin_store.join(&pending.staging_name);
        let final_dir = plugin_store.join(&pending.final_relative);
        match (staging.exists(), final_dir.exists()) {
            (true, true) => {
                return Err(InstallError::Recovery(format!(
                    "transaction {} 同时存在 staging 与 final",
                    pending.transaction_id
                )));
            }
            (true, false) => {
                fs::remove_dir_all(&staging)
                    .map_err(|error| InstallError::Recovery(error.to_string()))?;
                trust_store
                    .trust()
                    .abort_install(&pending.transaction_id)
                    .map_err(TrustPolicyError::Store)?;
                report.aborted += 1;
            }
            (false, false) => {
                trust_store
                    .trust()
                    .abort_install(&pending.transaction_id)
                    .map_err(TrustPolicyError::Store)?;
                report.aborted += 1;
            }
            (false, true) => {
                let installed = floatile_store::installation::load_exact(
                    plugin_store,
                    &pending.plugin_id,
                    &pending.version.to_string(),
                )
                .map_err(|error| InstallError::Recovery(error.to_string()))?
                .ok_or_else(|| InstallError::Recovery("final installation 消失".to_owned()))?;
                let mut files = BTreeMap::new();
                for name in installed.meta.files.keys() {
                    let bytes = installed
                        .file(name)
                        .ok_or_else(|| InstallError::Recovery(format!("安装文件缺失: {name}")))?;
                    files.insert(name.clone(), bytes.to_vec());
                }
                if content_digest(&files) != pending.install_digest
                    || signable_content_digest(&files) != pending.signed_digest
                {
                    return Err(InstallError::Recovery(
                        "恢复安装摘要与 journal 不一致".to_owned(),
                    ));
                }
                let trust = trust_store
                    .trust()
                    .get(&pending.publisher_id)
                    .map_err(TrustPolicyError::Store)?
                    .ok_or(InstallError::UnknownPublisher)?;
                let envelope = files
                    .get(SIGNATURE_FILE)
                    .ok_or(InstallError::MissingSignature)?;
                verify_signature_envelope(
                    envelope,
                    &files,
                    &installed.manifest.publisher.id,
                    &trust.verifier_binding(),
                )?;
                trust_store
                    .trust()
                    .finalize_install(&pending.transaction_id, now_secs())?;
                report.finalized += 1;
            }
        }
    }
    Ok(report)
}

fn install_validated(
    validated: &ValidatedPackage,
    store: &Path,
    source: &str,
) -> Result<InstalledPackage, InstallError> {
    let manifest = validated.manifest.clone();
    let id = manifest.id.0.clone();
    let version = manifest.version.clone();
    let final_dir = install_dir(store, &id, &version);

    if final_dir.exists() {
        return Err(InstallError::AlreadyInstalled { id, version });
    }
    // 父目录 `<store>/<id>` 必须在 rename 前存在，否则 rename 会失败。
    let parent = final_dir.parent().unwrap_or(store); // install_dir 的产物至少含一层 `id`，parent 恒存在
    fs::create_dir_all(parent).map_err(|e| InstallError::StoreUnavailable(e.to_string()))?;

    let staging = store.join(format!(".staging-{}", nonce()));

    let meta = match write_staging(&staging, validated, &manifest, source) {
        Ok(meta) => meta,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    if let Err(e) = fs::rename(&staging, &final_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(InstallError::Commit(e.to_string()));
    }

    Ok(InstalledPackage {
        dir: final_dir,
        manifest,
        meta,
        upgrade: None,
    })
}

/// 向暂存目录写入全部校验通过的文件 + install.json，并返回安装元数据。
fn write_staging(
    staging: &Path,
    validated: &ValidatedPackage,
    manifest: &Manifest,
    source: &str,
) -> Result<InstallMeta, InstallError> {
    let installed_at = now_secs();

    let mut file_digests = BTreeMap::new();
    for (name, bytes) in &validated.files {
        file_digests.insert(name.clone(), hex_encode(&file_digest(bytes)));
    }
    let aggregate = hex_encode(&content_digest(&validated.files));

    let meta = InstallMeta {
        manifest_version: manifest.manifest_version,
        id: manifest.id.0.clone(),
        version: manifest.version.clone(),
        engine_api_version: manifest.engine_api_version.clone(),
        ui_api_version: manifest.ui_api_version.clone(),
        installed_at,
        source: source.to_owned(),
        files: file_digests,
        digest: aggregate,
    };

    for (name, bytes) in &validated.files {
        let path = staging.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", parent.display())))?;
        }
        let mut file = fs::File::create(&path)
            .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", path.display())))?;
        file.write_all(bytes)
            .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", path.display())))?;
        file.sync_all()
            .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", path.display())))?;
    }

    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| InstallError::StagingWrite(format!("install.json: {e}")))?;
    let meta_path = staging.join("install.json");
    let mut meta_file = fs::File::create(&meta_path)
        .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", meta_path.display())))?;
    meta_file
        .write_all(&meta_json)
        .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", meta_path.display())))?;
    meta_file
        .sync_all()
        .map_err(|e| InstallError::StagingWrite(format!("{}: {e}", meta_path.display())))?;

    // fsync 目录本身（unix 上使 rename 更持久；Windows 不支持打开目录，尽力而为）。
    #[cfg(unix)]
    {
        if let Ok(dir) = fs::File::open(staging) {
            let _ = dir.sync_all();
        }
    }

    Ok(meta)
}

/// 当前 UNIX 秒（用于 install.json 的时间戳）。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 暂存目录去重：进程 id + 纳秒时间戳（同文件系统内 rename 需唯一且在同一目录树下）。
fn nonce() -> String {
    let pid = std::process::id();
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{pid}-{ns}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer, SigningKey};
    use floatile_core::distribution::{PACKAGE_DIGEST_PAYLOAD_TYPE, dsse_pae, publisher_key_id};
    use floatile_store::trust::TrustState;
    use std::io::Cursor;

    fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, data) in files {
                w.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    /// 与 package.rs 测试一致的合法 manifest/UI；wasm 用真实 clock-wasm。
    fn valid_manifest_json() -> String {
        serde_json::json!({
            "manifestVersion": 1,
            "id": "dev.floatile.clock",
            "name": "World Clock",
            "version": "0.1.0",
            "publisher": { "id": "dev.floatile", "name": "Floatile Labs" },
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": [ { "capability": "timer:schedule", "params": { "maxPerMinute": 60, "maxActive": 2 } } ]
        })
        .to_string()
    }

    fn valid_ui_ir() -> String {
        serde_json::json!({
            "uiApiVersion": "1.0.0",
            "state": { "initial": {}, "schema": { "type": "object", "additionalProperties": false, "properties": {} } },
            "events": {},
            "root": { "type": "Column", "props": {}, "children": [] }
        })
        .to_string()
    }

    fn real_wasm() -> Vec<u8> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let path = root.join("target/wasm32-wasip2/debug/floatile_clock_wasm.wasm");
        if !path.exists() {
            let status = std::process::Command::new("cargo")
                .current_dir(&root)
                .args([
                    "build",
                    "-p",
                    "floatile-clock-wasm",
                    "--target",
                    "wasm32-wasip2",
                ])
                .status()
                .expect("failed to run cargo build for clock-wasm");
            assert!(status.success(), "clock-wasm 构建失败");
        }
        fs::read(&path).unwrap_or_default()
    }

    fn valid_pkg_bytes() -> Vec<u8> {
        let wasm = real_wasm();
        assert!(!wasm.is_empty(), "需要先构建 clock-wasm");
        build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ])
    }

    fn signed_pkg_bytes(version: &str, signing_key: &SigningKey) -> Vec<u8> {
        signed_pkg_with_manifest_versions(version, version, signing_key)
    }

    fn signed_pkg_with_permissions(
        version: &str,
        permissions: serde_json::Value,
        signing_key: &SigningKey,
    ) -> Vec<u8> {
        let wasm = real_wasm();
        let mut manifest: serde_json::Value = serde_json::from_str(&valid_manifest_json()).unwrap();
        manifest["version"] = serde_json::json!(version);
        manifest["permissions"] = permissions;
        let manifest = manifest.to_string().into_bytes();
        let ui = valid_ui_ir().into_bytes();
        let mut files = BTreeMap::from([
            ("manifest.json".to_owned(), manifest),
            ("ui/widget.ftui".to_owned(), ui),
            ("logic/plugin.wasm".to_owned(), wasm),
        ]);
        let digest = content_digest(&files);
        let signature = signing_key.sign(&dsse_pae(PACKAGE_DIGEST_PAYLOAD_TYPE, &digest));
        let envelope = serde_json::to_vec(&serde_json::json!({
            "payloadType": PACKAGE_DIGEST_PAYLOAD_TYPE,
            "payload": STANDARD.encode(digest),
            "signatures": [{
                "keyid": publisher_key_id(signing_key.verifying_key().as_bytes()),
                "sig": STANDARD.encode(signature.to_bytes())
            }]
        }))
        .unwrap();
        files.insert(SIGNATURE_FILE.to_owned(), envelope);
        let entries: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        build_zip(&entries)
    }

    fn signed_pkg_with_manifest_versions(
        signed_version: &str,
        packaged_version: &str,
        signing_key: &SigningKey,
    ) -> Vec<u8> {
        let wasm = real_wasm();
        let mut manifest: serde_json::Value = serde_json::from_str(&valid_manifest_json()).unwrap();
        manifest["version"] = serde_json::json!(signed_version);
        let signed_manifest = manifest.to_string().into_bytes();
        let ui = valid_ui_ir().into_bytes();
        let files = BTreeMap::from([
            ("manifest.json".to_owned(), signed_manifest),
            ("ui/widget.ftui".to_owned(), ui.clone()),
            ("logic/plugin.wasm".to_owned(), wasm.clone()),
        ]);
        let digest = content_digest(&files);
        let signature = signing_key.sign(&dsse_pae(PACKAGE_DIGEST_PAYLOAD_TYPE, &digest));
        let envelope = serde_json::to_vec(&serde_json::json!({
            "payloadType": PACKAGE_DIGEST_PAYLOAD_TYPE,
            "payload": STANDARD.encode(digest),
            "signatures": [{
                "keyid": publisher_key_id(signing_key.verifying_key().as_bytes()),
                "sig": STANDARD.encode(signature.to_bytes())
            }]
        }))
        .unwrap();
        manifest["version"] = serde_json::json!(packaged_version);
        let packaged_manifest = manifest.to_string().into_bytes();
        build_zip(&[
            ("manifest.json", packaged_manifest.as_slice()),
            ("ui/widget.ftui", ui.as_slice()),
            ("logic/plugin.wasm", wasm.as_slice()),
            (SIGNATURE_FILE, envelope.as_slice()),
        ])
    }

    fn temp_store(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("floatile-install-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn installs_valid_package_atomically() {
        let store = temp_store("valid");
        let installed = install_package(
            &valid_pkg_bytes(),
            &store,
            "clock.floatile",
            &Default::default(),
        )
        .unwrap();
        let version_dir = install_dir(&store, "dev.floatile.clock", "0.1.0");
        assert_eq!(installed.dir, version_dir);

        // 所有校验通过的条目都落盘。
        for name in ["manifest.json", "ui/widget.ftui", "logic/plugin.wasm"] {
            assert!(version_dir.join(name).exists(), "缺失 {name}");
        }
        // install.json 存在且可解析，digest 与按实际文件重新计算的 digest 一致。
        let meta: InstallMeta =
            serde_json::from_slice(&fs::read(version_dir.join("install.json")).unwrap()).unwrap();
        assert_eq!(meta.id, "dev.floatile.clock");
        assert_eq!(meta.version, "0.1.0");
        let actual: BTreeMap<String, Vec<u8>> = meta
            .files
            .keys()
            .map(|k| (k.clone(), fs::read(version_dir.join(k)).unwrap()))
            .collect();
        assert_eq!(meta.digest, hex_encode(&content_digest(&actual)));
        assert_eq!(meta.digest.len(), 64);
        // 无残留暂存目录。
        let leftovers: Vec<_> = fs::read_dir(&store)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
            .collect();
        assert!(leftovers.is_empty(), "存在未清理的暂存目录");
    }

    #[test]
    fn refuses_reinstall_same_version() {
        let store = temp_store("reinstall");
        install_package(
            &valid_pkg_bytes(),
            &store,
            "a.floatile",
            &Default::default(),
        )
        .unwrap();
        let err = install_package(
            &valid_pkg_bytes(),
            &store,
            "b.floatile",
            &Default::default(),
        )
        .unwrap_err();
        assert!(matches!(err, InstallError::AlreadyInstalled { .. }));
        assert_eq!(err.code(), "FINST_ALREADY_INSTALLED");
    }

    #[test]
    fn installs_different_versions_side_by_side() {
        let store = temp_store("versions");
        install_package(
            &valid_pkg_bytes(),
            &store,
            "a.floatile",
            &Default::default(),
        )
        .unwrap();
        // 构造 0.2.0 版本的合法包（仅版本不同）。
        let wasm = real_wasm();
        let mut manifest: serde_json::Value = serde_json::from_str(&valid_manifest_json()).unwrap();
        manifest["version"] = serde_json::json!("0.2.0");
        let bytes = build_zip(&[
            ("manifest.json", manifest.to_string().as_bytes()),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        install_package(&bytes, &store, "v2.floatile", &Default::default()).unwrap();
        assert!(install_dir(&store, "dev.floatile.clock", "0.1.0").exists());
        assert!(install_dir(&store, "dev.floatile.clock", "0.2.0").exists());
    }

    #[test]
    fn rejects_evil_package_with_no_partial_install() {
        let store = temp_store("evil");
        let wasm = real_wasm();

        // 路径穿越条目 → 校验拒绝，且不留下任何安装产物/半安装目录。
        let traversal = build_zip(&[
            ("manifest.json", valid_manifest_json().as_bytes()),
            ("../evil", b"x"),
            ("ui/widget.ftui", valid_ui_ir().as_bytes()),
            ("logic/plugin.wasm", wasm.as_slice()),
        ]);
        let err =
            install_package(&traversal, &store, "evil.floatile", &Default::default()).unwrap_err();
        assert!(matches!(
            err,
            InstallError::Package(PackageError::InvalidPath(_))
        ));
        assert!(!install_dir(&store, "dev.floatile.clock", "0.1.0").exists());
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0, "安装失败应无残留");

        // 解压超预算（zip bomb 类）→ 校验拒绝，同样无残留。
        let mut blast: Vec<u8> = vec![];
        {
            // 用低上限模拟：单条目超过 max_single_entry。
            let big = vec![b'x'; 1024 * 1024 * 20];
            blast = build_zip(&[
                ("manifest.json", valid_manifest_json().as_bytes()),
                ("ui/widget.ftui", valid_ui_ir().as_bytes()),
                ("logic/plugin.wasm", wasm.as_slice()),
                ("assets/big", big.as_slice()),
            ]);
        }
        let limits = PackageLimits {
            max_single_entry: 1024 * 1024,
            ..PackageLimits::default()
        };
        let err2 = install_package(&blast, &store, "evil2.floatile", &limits).unwrap_err();
        assert!(matches!(
            err2,
            InstallError::Package(PackageError::EntryTooLarge { .. })
        ));
        assert_eq!(fs::read_dir(&store).unwrap().count(), 0, "安装失败应无残留");
    }

    #[test]
    fn trusted_install_verifies_signature_and_advances_watermark() {
        let plugin_store = temp_store("trusted");
        let trust_store = floatile_store::open(":memory:").unwrap();
        let signing_key = SigningKey::from_bytes(&[21; 32]);
        trust_store
            .trust()
            .upsert_key(
                "dev.floatile",
                signing_key.verifying_key().to_bytes(),
                TrustState::Active,
                1,
            )
            .unwrap();

        let installed = install_trusted_package(
            &signed_pkg_bytes("1.0.0", &signing_key),
            &plugin_store,
            "signed.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap();
        assert!(installed.dir.join(SIGNATURE_FILE).is_file());
        let accepted = trust_store
            .trust()
            .accepted_package("dev.floatile", "dev.floatile.clock")
            .unwrap()
            .unwrap();
        assert_eq!(accepted.version, Version::parse("1.0.0").unwrap());
        assert!(trust_store.trust().pending_installs().unwrap().is_empty());
    }

    #[test]
    fn trusted_install_rejects_unsigned_tampered_and_downgrade_without_residue() {
        let plugin_store = temp_store("trusted-reject");
        let trust_store = floatile_store::open(":memory:").unwrap();
        let signing_key = SigningKey::from_bytes(&[22; 32]);
        trust_store
            .trust()
            .upsert_key(
                "dev.floatile",
                signing_key.verifying_key().to_bytes(),
                TrustState::Active,
                1,
            )
            .unwrap();
        assert!(matches!(
            install_trusted_package(
                &valid_pkg_bytes(),
                &plugin_store,
                "unsigned.floatile",
                &Default::default(),
                &trust_store,
                false
            ),
            Err(InstallError::MissingSignature)
        ));

        let tampered = signed_pkg_with_manifest_versions("1.0.0", "1.0.1", &signing_key);
        let tampered_error = install_trusted_package(
            &tampered,
            &plugin_store,
            "tampered.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap_err();
        assert_eq!(tampered_error.code(), "FINST_SIGNATURE_DIGEST");

        install_trusted_package(
            &signed_pkg_bytes("2.0.0", &signing_key),
            &plugin_store,
            "v2.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap();
        let downgrade = install_trusted_package(
            &signed_pkg_bytes("1.0.0", &signing_key),
            &plugin_store,
            "v1.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap_err();
        assert_eq!(downgrade.code(), "FINST_ROLLBACK");
        assert!(!install_dir(&plugin_store, "dev.floatile.clock", "1.0.0").exists());
        assert!(trust_store.trust().pending_installs().unwrap().is_empty());
    }

    #[test]
    fn trusted_upgrade_requires_explicit_permission_expansion_confirmation() {
        let plugin_store = temp_store("trusted-permissions");
        let trust_store = floatile_store::open(":memory:").unwrap();
        let signing_key = SigningKey::from_bytes(&[24; 32]);
        trust_store
            .trust()
            .upsert_key(
                "dev.floatile",
                signing_key.verifying_key().to_bytes(),
                TrustState::Active,
                1,
            )
            .unwrap();
        install_trusted_package(
            &signed_pkg_bytes("1.0.0", &signing_key),
            &plugin_store,
            "v1.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap();

        let expanded = signed_pkg_with_permissions(
            "2.0.0",
            serde_json::json!([{
                "capability": "timer:schedule",
                "params": { "maxPerMinute": 120, "maxActive": 4 }
            }]),
            &signing_key,
        );
        let rejected = install_trusted_package(
            &expanded,
            &plugin_store,
            "v2.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap_err();
        assert_eq!(rejected.code(), "FINST_PERMISSION_CONFIRMATION");
        assert!(!install_dir(&plugin_store, "dev.floatile.clock", "2.0.0").exists());
        assert!(trust_store.trust().pending_installs().unwrap().is_empty());

        let installed = install_trusted_package(
            &expanded,
            &plugin_store,
            "v2.floatile",
            &Default::default(),
            &trust_store,
            true,
        )
        .unwrap();
        assert!(installed.upgrade.unwrap().requires_confirmation);

        let reduced = signed_pkg_with_permissions(
            "3.0.0",
            serde_json::json!([{
                "capability": "timer:schedule",
                "params": { "maxPerMinute": 30, "maxActive": 1 }
            }]),
            &signing_key,
        );
        let installed = install_trusted_package(
            &reduced,
            &plugin_store,
            "v3.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap();
        assert!(!installed.upgrade.unwrap().requires_confirmation);
    }

    #[test]
    fn explicit_rollback_rebinds_instance_without_lowering_watermark() {
        let root = temp_store("explicit-rollback");
        let plugin_store = root.join("plugins");
        fs::create_dir_all(&plugin_store).unwrap();
        let database = root.join("floatile.db");
        let trust_store = floatile_store::open(&database).unwrap();
        let signing_key = SigningKey::from_bytes(&[25; 32]);
        trust_store
            .trust()
            .upsert_key(
                "dev.floatile",
                signing_key.verifying_key().to_bytes(),
                TrustState::Active,
                1,
            )
            .unwrap();
        let v1 = install_trusted_package(
            &signed_pkg_bytes("1.0.0", &signing_key),
            &plugin_store,
            "v1.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap();
        let v2 = install_trusted_package(
            &signed_pkg_bytes("2.0.0", &signing_key),
            &plugin_store,
            "v2.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap();
        assert!(v1.upgrade.is_none());
        let instance = trust_store
            .instances()
            .create(
                &floatile_core::InstallationRef::from_install_meta(&v2.meta).unwrap(),
                &floatile_core::InstanceConfig::empty(),
                floatile_core::InstanceDesiredState::Stopped,
                10,
            )
            .unwrap();

        let rolled_back = crate::instance::rollback_instance(
            &database,
            &plugin_store,
            instance.id(),
            "1.0.0",
            "2.0.0 rendering regression",
            11,
        )
        .unwrap();
        assert_eq!(rolled_back.version, "1.0.0");
        let accepted = trust_store
            .trust()
            .accepted_package("dev.floatile", "dev.floatile.clock")
            .unwrap()
            .unwrap();
        assert_eq!(accepted.version, Version::parse("2.0.0").unwrap());
    }

    #[test]
    fn recovery_aborts_staging_and_finalizes_verified_renamed_install() {
        let plugin_store = temp_store("trusted-recovery");
        let trust_store = floatile_store::open(":memory:").unwrap();
        let signing_key = SigningKey::from_bytes(&[23; 32]);
        trust_store
            .trust()
            .upsert_key(
                "dev.floatile",
                signing_key.verifying_key().to_bytes(),
                TrustState::Active,
                1,
            )
            .unwrap();

        let staged = PendingInstallation {
            transaction_id: "staged-1".to_owned(),
            publisher_id: "dev.floatile".to_owned(),
            plugin_id: "dev.floatile.clock".to_owned(),
            version: Version::parse("0.5.0").unwrap(),
            signed_digest: [1; 32],
            install_digest: [2; 32],
            staging_name: ".staging-staged-1".to_owned(),
            final_relative: "dev.floatile.clock/0.5.0".to_owned(),
            created_at: 2,
        };
        fs::create_dir_all(plugin_store.join(&staged.staging_name)).unwrap();
        trust_store.trust().prepare_install(&staged).unwrap();
        let report = recover_trusted_installs(&plugin_store, &trust_store).unwrap();
        assert_eq!(report.aborted, 1);
        assert!(!plugin_store.join(&staged.staging_name).exists());

        let package = signed_pkg_bytes("1.0.0", &signing_key);
        let installed = install_trusted_package(
            &package,
            &plugin_store,
            "signed.floatile",
            &Default::default(),
            &trust_store,
            false,
        )
        .unwrap();
        let validated = validate_package(&package, &Default::default()).unwrap();
        let renamed = PendingInstallation {
            transaction_id: "renamed-1".to_owned(),
            publisher_id: "dev.floatile".to_owned(),
            plugin_id: "dev.floatile.clock".to_owned(),
            version: Version::parse("1.0.0").unwrap(),
            signed_digest: signable_content_digest(&validated.files),
            install_digest: content_digest(&validated.files),
            staging_name: ".staging-renamed-1".to_owned(),
            final_relative: "dev.floatile.clock/1.0.0".to_owned(),
            created_at: installed.meta.installed_at,
        };
        trust_store.trust().prepare_install(&renamed).unwrap();
        let report = recover_trusted_installs(&plugin_store, &trust_store).unwrap();
        assert_eq!(report.finalized, 1);
        assert!(trust_store.trust().pending_installs().unwrap().is_empty());
        assert!(installed.dir.exists());
    }
}
