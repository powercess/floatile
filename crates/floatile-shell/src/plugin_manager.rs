//! PluginManager：从插件存储加载已安装插件（S6）。
//!
//! 插件存储中的内容在安装时已由 `floatile-cli` 完整校验，并记录每文件 SHA-256 与
//! 覆盖全部规范文件集合的聚合 digest。PluginManager 加载前按 `install.json` 重算并
//! 校验 digest，确认安装后内容未被篡改，再把可信的 wasm/manifest 交给 runtime。
//!
//! shell 不解析不可信原始 `.floatile` 包——安装期校验与解包由 CLI/安装器完成，这里
//! 只读取本机已安装且已验证的产物。任意 UI 的动态渲染仍受「运行时编译 ADR」门禁；
//! 本模块只提供参考时钟等 dev 包的宿主加载路径。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use floatile_core::install::{InstallMeta, content_digest, file_digest, hex_encode};
use floatile_core::manifest::Manifest;
use semver::Version;
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
    #[error("缺少 manifest.json")]
    MissingManifest,
    #[error("缺少入口 `{0}`")]
    MissingEntrypoint(String),
    #[error("manifest 非法: {0}")]
    InvalidManifest(#[from] floatile_core::manifest::ManifestError),
}

impl LoadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Read(_) => "FLOAD_READ",
            Self::InvalidMeta(_) => "FLOAD_INVALID_META",
            Self::DigestMismatch { .. } => "FLOAD_DIGEST_MISMATCH",
            Self::MissingManifest => "FLOAD_MISSING_MANIFEST",
            Self::MissingEntrypoint(_) => "FLOAD_MISSING_ENTRYPOINT",
            Self::InvalidManifest(_) => "FLOAD_INVALID_MANIFEST",
        }
    }
}

/// 加载某插件 id 的最高已安装版本。
///
/// 无任何已安装版本返回 `Ok(None)`（调用方回退内建实现）；存在多个版本时按 semver
/// 取最高；任意文件 digest 不匹配返回错误并拒绝加载。
pub fn load_installed(store: &Path, id: &str) -> Result<Option<InstalledPlugin>, LoadError> {
    let id_dir = store.join(id);
    if !id_dir.is_dir() {
        return Ok(None);
    }

    let mut best: Option<(Version, PathBuf)> = None;
    let entries = std::fs::read_dir(&id_dir).map_err(|e| LoadError::Read(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| LoadError::Read(e.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(version) = Version::parse(&name) else {
            continue;
        };
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if best.as_ref().is_none_or(|(v, _)| version > *v) {
            best = Some((version, dir));
        }
    }

    let Some((_version, dir)) = best else {
        return Ok(None);
    };
    load_from_dir(&dir).map(Some)
}

/// 从单个已安装版本目录加载，重算并校验 digest 后解析 manifest/入口。
fn load_from_dir(dir: &Path) -> Result<InstalledPlugin, LoadError> {
    let meta_bytes = std::fs::read(dir.join("install.json"))
        .map_err(|e| LoadError::InvalidMeta(e.to_string()))?;
    let meta: InstallMeta =
        serde_json::from_slice(&meta_bytes).map_err(|e| LoadError::InvalidMeta(e.to_string()))?;

    // 重算每文件 digest 与聚合 digest，拦截安装后任何文件被篡改/增删。
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (name, expect_hex) in &meta.files {
        let bytes = std::fs::read(dir.join(name))
            .map_err(|e| LoadError::Read(format!("{}: {e}", dir.join(name).display())))?;
        let actual = hex_encode(&file_digest(&bytes));
        if actual != *expect_hex {
            return Err(LoadError::DigestMismatch {
                id: meta.id.clone(),
                file: name.clone(),
            });
        }
        files.insert(name.clone(), bytes);
    }
    let aggregate = hex_encode(&content_digest(&files));
    if aggregate != meta.digest {
        // 文件集合与安装时不一致（增删/重命名）。取任一文件名用于诊断。
        let probe = files
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "<all>".to_owned());
        return Err(LoadError::DigestMismatch {
            id: meta.id.clone(),
            file: probe,
        });
    }

    let manifest_bytes = files
        .get("manifest.json")
        .ok_or(LoadError::MissingManifest)?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| LoadError::InvalidMeta(format!("manifest.json: {e}")))?;

    let wasm = files
        .get(manifest.entrypoints.logic.as_str())
        .cloned()
        .ok_or_else(|| {
            LoadError::MissingEntrypoint(manifest.entrypoints.logic.as_str().to_owned())
        })?;
    let ui_bytes = files
        .get(manifest.entrypoints.ui.as_str())
        .cloned()
        .ok_or_else(|| LoadError::MissingEntrypoint(manifest.entrypoints.ui.as_str().to_owned()))?;

    Ok(InstalledPlugin {
        manifest,
        meta,
        wasm,
        ui_bytes,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use floatile_core::install::content_digest;

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
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": []
        })
        .to_string()
    }

    fn temp_store(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("floatile-pm-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 安装一个模拟版本目录（直接按 CLI 布列写 install.json + 文件）。
    fn write_install(store: &Path, version: &str, tamper: Option<&str>) {
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        files.insert("manifest.json".into(), manifest_json().into_bytes());
        files.insert("ui/widget.ftui".into(), b"{\"ui\":1}".to_vec());
        files.insert("logic/plugin.wasm".into(), vec![1, 2, 3, 4]);

        let mut file_digests = BTreeMap::new();
        for (name, bytes) in &files {
            file_digests.insert(name.clone(), hex_encode(&file_digest(bytes)));
        }
        // 可选：篡改某文件内容后仍按伪造 digest 写，或改 content 后再校验失败。
        let meta = InstallMeta {
            manifest_version: 1,
            id: "dev.floatile.clock".into(),
            version: version.into(),
            engine_api_version: "1.0.0".into(),
            ui_api_version: "1.0.0".into(),
            installed_at: 0,
            source: "x.floatile".into(),
            files: file_digests.clone(),
            digest: hex_encode(&content_digest(&files)),
        };

        let version_dir = store.join("dev.floatile.clock").join(version);
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

    #[test]
    fn loads_highest_version_and_verifies_digest() {
        let store = temp_store("load");
        write_install(&store, "0.1.0", None);
        write_install(&store, "0.2.0", None);

        let plugin = load_installed(&store, "dev.floatile.clock")
            .unwrap()
            .expect("应加载到已安装插件");
        assert_eq!(plugin.meta.version, "0.2.0");
        assert_eq!(plugin.wasm, vec![1, 2, 3, 4]);
        assert_eq!(plugin.ui_bytes, b"{\"ui\":1}");
        assert_eq!(plugin.manifest.id.0, "dev.floatile.clock");
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
        write_install(&store, "0.1.0", Some("logic/plugin.wasm"));
        let err = load_installed(&store, "dev.floatile.clock").unwrap_err();
        assert!(matches!(err, LoadError::DigestMismatch { .. }));
        assert_eq!(err.code(), "FLOAD_DIGEST_MISMATCH");
    }
}
