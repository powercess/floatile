//! 已安装插件元数据与内容摘要（S6 安装/加载的单一事实源）。
//!
//! `InstallMeta` 由 `floatile-cli` 在原子安装时写入
//! `<插件存储>/<id>/<version>/install.json`；`floatile-shell` 的 PluginManager 据此做
//! 完整性校验后加载。摘要算法与文件集合的规范化定义在此收敛，CLI 写入与宿主校验
//! 共用同一实现，避免两处重复。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallationTrust {
    #[default]
    Unsigned,
    Trusted,
}

/// 一个已安装插件的可验证元数据（manifest 与摘要快照）。
///
/// 不冗余存放插件内容，只存每条文件的 SHA-256 与覆盖全部规范文件集合的内容摘要，
/// 供宿主在加载前检测是否被篡改。摘要字段本身不参与授权。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallMeta {
    #[serde(rename = "manifestVersion")]
    pub manifest_version: u32,
    pub id: String,
    pub version: String,
    #[serde(rename = "engineApiVersion")]
    pub engine_api_version: String,
    #[serde(rename = "uiApiVersion")]
    pub ui_api_version: String,
    /// 安装时间（UNIX 秒）。
    pub installed_at: u64,
    /// 来源包文件名（仅诊断，不参与信任）。
    pub source: String,
    /// Installation policy used at commit time; runtime re-verifies trusted signatures.
    #[serde(default)]
    pub trust: InstallationTrust,
    /// 每条允许文件相对路径 → SHA-256 hex。
    pub files: BTreeMap<String, String>,
    /// 覆盖全部规范文件集合的内容摘要（hex）。
    pub digest: String,
}

/// 单文件 SHA-256。
pub fn file_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// 覆盖全部规范文件集合的内容摘要。
///
/// 规范化定义：按相对路径字典序遍历（`BTreeMap` 保证），对每个条目依次混入
/// 路径字节、NUL、长度（u64 little-endian）与其内容字节。路径加入摘要，因此
/// 增删或重命名任意文件都会改变结果，能捕获安装后文件集合的结构篡改。
pub fn content_digest(files: &BTreeMap<String, Vec<u8>>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for (name, bytes) in files {
        hasher.update(name.as_bytes());
        hasher.update([0u8]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes.as_slice());
    }
    hasher.finalize().into()
}

/// 小端 hex 编码（不引入 hex crate）。
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// 从 hex 解码；非法输入返回 `None`。
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic_and_path_sensitive() {
        let mut a = BTreeMap::new();
        a.insert("logic/plugin.wasm".to_owned(), vec![1, 2, 3]);
        a.insert("manifest.json".to_owned(), vec![9]);
        let d1 = content_digest(&a);

        // 相同内容、不同插入顺序 → 摘要不变（BTreeMap 排序）。
        let mut b = BTreeMap::new();
        b.insert("manifest.json".to_owned(), vec![9]);
        b.insert("logic/plugin.wasm".to_owned(), vec![1, 2, 3]);
        assert_eq!(d1, content_digest(&b));

        // 内容变化 → 摘要变化。
        let mut c = b.clone();
        c.insert("logic/plugin.wasm".to_owned(), vec![1, 2, 4]);
        assert_ne!(d1, content_digest(&c));

        // 结构变化（增删文件）→ 摘要变化。
        let mut d = b.clone();
        d.insert("assets/hi".to_owned(), vec![0]);
        assert_ne!(d1, content_digest(&d));

        let mut e = b.clone();
        e.remove("manifest.json");
        assert_ne!(d1, content_digest(&e));
    }

    #[test]
    fn file_digest_is_sha256_and_differs() {
        let a = file_digest(b"hello");
        let b = file_digest(b"world");
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        // 已知向量：sha256("hello")。
        assert_eq!(
            hex_encode(&a),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn hex_round_trip() {
        let bytes = [0u8, 1, 15, 16, 255];
        let enc = hex_encode(&bytes);
        assert_eq!(enc, "00010f10ff");
        assert_eq!(hex_decode(&enc).as_deref(), Some(bytes.as_slice()));
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("0z"), None);
    }

    #[test]
    fn install_meta_serde_round_trip() {
        let mut files = BTreeMap::new();
        files.insert("logic/plugin.wasm".to_owned(), hex_encode(&[1, 2, 3]));
        let meta = InstallMeta {
            manifest_version: 1,
            id: "dev.floatile.clock".to_owned(),
            version: "0.1.0".to_owned(),
            engine_api_version: "1.0.0".to_owned(),
            ui_api_version: "1.0.0".to_owned(),
            installed_at: 42,
            source: "clock.floatile".to_owned(),
            trust: InstallationTrust::Unsigned,
            files,
            digest: hex_encode(&[9; 32]),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: InstallMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back, meta);

        let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
        legacy.as_object_mut().unwrap().remove("trust");
        let legacy: InstallMeta = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.trust, InstallationTrust::Unsigned);
    }
}
