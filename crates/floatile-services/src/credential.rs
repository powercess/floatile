//! 宿主 Credential Vault 边界。
//!
//! Secret 不实现 `Debug`、`Clone` 或序列化，只能在宿主闭包内短暂借用。当前进程内实现用于
//! Broker 组合与确定性测试；平台持久 Keyring 接入前，宿主重启后必须把连接报告为 unavailable。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use floatile_core::CredentialRef;

pub const MAX_CREDENTIAL_BYTES: usize = 8 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CredentialError {
    #[error("credential secret 为空")]
    Empty,
    #[error("credential secret 超过宿主预算")]
    TooLarge,
    #[error("credential 不存在")]
    NotFound,
    #[error("credential vault 不可用")]
    Unavailable,
}

struct SecretBytes(Vec<u8>);

impl SecretBytes {
    fn new(value: &[u8]) -> Result<Self, CredentialError> {
        if value.is_empty() {
            return Err(CredentialError::Empty);
        }
        if value.len() > MAX_CREDENTIAL_BYTES {
            return Err(CredentialError::TooLarge);
        }
        Ok(Self(value.to_vec()))
    }

    fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&mut self.0);
    }
}

/// 最小凭证库能力。调用方不能取得拥有所有权的 secret，也不能枚举凭证。
pub trait CredentialVault: Send + Sync {
    fn put(&self, reference: &CredentialRef, secret: &[u8]) -> Result<(), CredentialError>;
    fn delete(&self, reference: &CredentialRef) -> Result<bool, CredentialError>;
    fn with_secret(
        &self,
        reference: &CredentialRef,
        use_secret: &mut dyn FnMut(&[u8]),
    ) -> Result<(), CredentialError>;
}

/// 不落盘的宿主凭证库。用于开发、测试和平台 Keyring 不可用时的显式会话模式。
#[derive(Default)]
pub struct MemoryCredentialVault {
    entries: Mutex<BTreeMap<String, SecretBytes>>,
}

impl CredentialVault for MemoryCredentialVault {
    fn put(&self, reference: &CredentialRef, secret: &[u8]) -> Result<(), CredentialError> {
        let secret = SecretBytes::new(secret)?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        entries.insert(reference.as_str().to_owned(), secret);
        Ok(())
    }

    fn delete(&self, reference: &CredentialRef) -> Result<bool, CredentialError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        Ok(entries.remove(reference.as_str()).is_some())
    }

    fn with_secret(
        &self,
        reference: &CredentialRef,
        use_secret: &mut dyn FnMut(&[u8]),
    ) -> Result<(), CredentialError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        let secret = entries
            .get(reference.as_str())
            .ok_or(CredentialError::NotFound)?;
        use_secret(secret.expose());
        Ok(())
    }
}

/// OS-backed credential vault. On Windows this uses the current user's Credential Manager;
/// unsupported platforms fail closed instead of silently falling back to process memory.
pub struct PlatformCredentialVault {
    fallback_root: PathBuf,
}

impl PlatformCredentialVault {
    pub fn new(fallback_root: PathBuf) -> Self {
        Self { fallback_root }
    }

    fn fallback_path(&self, reference: &CredentialRef) -> PathBuf {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(reference.as_str().as_bytes());
        let name = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.fallback_root.join(format!("{name}.credential"))
    }

    fn put_fallback(
        &self,
        reference: &CredentialRef,
        secret: &[u8],
    ) -> Result<(), CredentialError> {
        let protected =
            floatile_platform::credential_protect(secret).map_err(map_platform_error)?;
        std::fs::create_dir_all(&self.fallback_root).map_err(|_| CredentialError::Unavailable)?;
        let destination = self.fallback_path(reference);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let temporary =
            destination.with_extension(format!("credential.tmp-{}-{nonce}", std::process::id()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| CredentialError::Unavailable)?;
            file.write_all(&protected)
                .and_then(|()| file.sync_all())
                .map_err(|_| CredentialError::Unavailable)?;
            std::fs::rename(&temporary, &destination).map_err(|_| CredentialError::Unavailable)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }

    fn read_fallback(&self, reference: &CredentialRef) -> Result<Vec<u8>, CredentialError> {
        let protected = std::fs::read(self.fallback_path(reference)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CredentialError::NotFound
            } else {
                CredentialError::Unavailable
            }
        })?;
        if protected.len() > MAX_CREDENTIAL_BYTES.saturating_mul(4) {
            return Err(CredentialError::Unavailable);
        }
        floatile_platform::credential_unprotect(&protected).map_err(map_platform_error)
    }
}

impl CredentialVault for PlatformCredentialVault {
    fn put(&self, reference: &CredentialRef, secret: &[u8]) -> Result<(), CredentialError> {
        if secret.is_empty() {
            return Err(CredentialError::Empty);
        }
        if secret.len() > MAX_CREDENTIAL_BYTES {
            return Err(CredentialError::TooLarge);
        }
        match floatile_platform::credential_put(reference.as_str(), secret) {
            Ok(()) => Ok(()),
            Err(_) => self.put_fallback(reference, secret),
        }
    }

    fn delete(&self, reference: &CredentialRef) -> Result<bool, CredentialError> {
        let manager_deleted =
            floatile_platform::credential_delete(reference.as_str()).unwrap_or(false);
        let fallback = self.fallback_path(reference);
        let fallback_deleted = match std::fs::remove_file(fallback) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(CredentialError::Unavailable),
        };
        Ok(manager_deleted || fallback_deleted)
    }

    fn with_secret(
        &self,
        reference: &CredentialRef,
        use_secret: &mut dyn FnMut(&[u8]),
    ) -> Result<(), CredentialError> {
        match floatile_platform::credential_with_secret(reference.as_str(), use_secret) {
            Ok(()) => Ok(()),
            Err(_) => {
                let mut secret = self.read_fallback(reference)?;
                use_secret(&secret);
                secret.fill(0);
                std::hint::black_box(&mut secret);
                Ok(())
            }
        }
    }
}

fn map_platform_error(error: floatile_platform::PlatformCredentialError) -> CredentialError {
    match error {
        floatile_platform::PlatformCredentialError::NotFound => CredentialError::NotFound,
        floatile_platform::PlatformCredentialError::InvalidInput
        | floatile_platform::PlatformCredentialError::Unavailable
        | floatile_platform::PlatformCredentialError::Platform(_) => CredentialError::Unavailable,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn reference() -> CredentialRef {
        CredentialRef::new("cred://provider/account").unwrap()
    }

    #[test]
    fn secret_is_only_borrowed_and_replacement_is_visible() {
        let vault = MemoryCredentialVault::default();
        vault.put(&reference(), b"first-secret").unwrap();
        vault.put(&reference(), b"second-secret").unwrap();
        let mut observed = Vec::new();
        vault
            .with_secret(&reference(), &mut |secret| {
                observed.extend_from_slice(secret)
            })
            .unwrap();
        assert_eq!(observed, b"second-secret");
    }

    #[test]
    fn missing_deleted_and_oversized_secrets_fail_without_fallback() {
        let vault = MemoryCredentialVault::default();
        assert_eq!(
            vault.with_secret(&reference(), &mut |_| {}),
            Err(CredentialError::NotFound)
        );
        assert_eq!(vault.put(&reference(), &[]), Err(CredentialError::Empty));
        assert_eq!(
            vault.put(&reference(), &vec![0; MAX_CREDENTIAL_BYTES + 1]),
            Err(CredentialError::TooLarge)
        );
        vault.put(&reference(), b"secret").unwrap();
        assert!(vault.delete(&reference()).unwrap());
        assert!(!vault.delete(&reference()).unwrap());
    }

    #[test]
    fn vault_debug_surface_never_contains_secret() {
        assert!(!std::any::type_name::<MemoryCredentialVault>().contains("secret"));
        assert!(!std::any::type_name::<CredentialRef>().contains("secret"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_platform_vault_round_trips_and_deletes_current_user_credential() {
        let reference = CredentialRef::new(format!(
            "cred://floatile-test/process-{}",
            std::process::id()
        ))
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "floatile-platform-vault-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let vault = PlatformCredentialVault::new(root.clone());
        let _ = vault.delete(&reference);
        vault.put(&reference, b"windows-vault-test").unwrap();
        let restored_vault = PlatformCredentialVault::new(root.clone());
        let mut observed = Vec::new();
        restored_vault
            .with_secret(&reference, &mut |secret| observed.extend_from_slice(secret))
            .unwrap();
        assert_eq!(observed, b"windows-vault-test");
        assert!(restored_vault.delete(&reference).unwrap());
        assert_eq!(
            restored_vault.with_secret(&reference, &mut |_| {}),
            Err(CredentialError::NotFound)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
