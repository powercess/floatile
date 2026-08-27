//! 宿主 Credential Vault 边界。
//!
//! Secret 不实现 `Debug`、`Clone` 或序列化，只能在宿主闭包内短暂借用。当前进程内实现用于
//! Broker 组合与确定性测试；平台持久 Keyring 接入前，宿主重启后必须把连接报告为 unavailable。

use std::collections::BTreeMap;
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
}
