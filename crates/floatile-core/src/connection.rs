//! 宿主拥有的外部数据连接领域模型（PP-M5）。
//!
//! Connection 只保存外部账户的非敏感身份与不透明凭证引用。明文 secret 由
//! `floatile-services` 的凭证库持有，不得进入本模型、SQLite、guest Config 或 State。

use serde::{Deserialize, Serialize};

use crate::InstanceId;

pub const MAX_CONNECTION_PROVIDER_BYTES: usize = 64;
pub const MAX_CONNECTION_ACCOUNT_BYTES: usize = 256;
pub const MAX_CREDENTIAL_REF_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CredentialRef(String);

impl CredentialRef {
    pub fn new(value: impl Into<String>) -> Result<Self, ConnectionModelError> {
        let value = value.into();
        if value.len() > MAX_CREDENTIAL_REF_BYTES || !valid_credential_ref(&value) {
            return Err(ConnectionModelError::InvalidCredentialRef);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CredentialRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionHealth {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
    Revoked,
}

impl ConnectionHealth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ConnectionModelError> {
        match value {
            "unknown" => Ok(Self::Unknown),
            "healthy" => Ok(Self::Healthy),
            "degraded" => Ok(Self::Degraded),
            "unavailable" => Ok(Self::Unavailable),
            "revoked" => Ok(Self::Revoked),
            _ => Err(ConnectionModelError::InvalidHealth),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Connection {
    id: ConnectionId,
    provider: String,
    account_identity: String,
    credential: CredentialRef,
    health: ConnectionHealth,
    credential_generation: u64,
    created_at: u64,
    updated_at: u64,
}

impl Connection {
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: ConnectionId,
        provider: impl Into<String>,
        account_identity: impl Into<String>,
        credential: CredentialRef,
        health: ConnectionHealth,
        credential_generation: u64,
        created_at: u64,
        updated_at: u64,
    ) -> Result<Self, ConnectionModelError> {
        let provider = provider.into();
        let account_identity = account_identity.into();
        validate_token(&provider, MAX_CONNECTION_PROVIDER_BYTES)
            .map_err(|_| ConnectionModelError::InvalidProvider)?;
        validate_text(&account_identity, MAX_CONNECTION_ACCOUNT_BYTES)
            .map_err(|_| ConnectionModelError::InvalidAccountIdentity)?;
        if id.0 == 0 {
            return Err(ConnectionModelError::InvalidId);
        }
        if updated_at < created_at {
            return Err(ConnectionModelError::InvalidTimestamps);
        }
        Ok(Self {
            id,
            provider,
            account_identity,
            credential,
            health,
            credential_generation,
            created_at,
            updated_at,
        })
    }

    pub const fn id(&self) -> ConnectionId {
        self.id
    }
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn account_identity(&self) -> &str {
        &self.account_identity
    }
    pub fn credential(&self) -> &CredentialRef {
        &self.credential
    }
    pub const fn health(&self) -> ConnectionHealth {
        self.health
    }
    pub const fn credential_generation(&self) -> u64 {
        self.credential_generation
    }
    pub const fn created_at(&self) -> u64 {
        self.created_at
    }
    pub const fn updated_at(&self) -> u64 {
        self.updated_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionGrant {
    pub instance_id: InstanceId,
    pub connection_id: ConnectionId,
    pub granted_at: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConnectionModelError {
    #[error("connection id 无效")]
    InvalidId,
    #[error("provider 无效")]
    InvalidProvider,
    #[error("account identity 无效")]
    InvalidAccountIdentity,
    #[error("credential reference 无效")]
    InvalidCredentialRef,
    #[error("connection health 无效")]
    InvalidHealth,
    #[error("connection 时间戳无效")]
    InvalidTimestamps,
}

fn valid_credential_ref(value: &str) -> bool {
    let Some(path) = value.strip_prefix("cred://") else {
        return false;
    };
    !path.is_empty()
        && !path.starts_with('/')
        && !path.ends_with('/')
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn validate_token(value: &str, maximum: usize) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > maximum
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
    {
        return Err(());
    }
    Ok(())
}

fn validate_text(value: &str, maximum: usize) -> Result<(), ()> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn credential_reference_is_opaque_bounded_and_canonical() {
        assert!(CredentialRef::new("cred://openai/account-1").is_ok());
        for invalid in [
            "secret",
            "cred://",
            "cred:///root",
            "cred://a/../b",
            "cred://a//b",
        ] {
            assert_eq!(
                CredentialRef::new(invalid),
                Err(ConnectionModelError::InvalidCredentialRef)
            );
        }
    }

    #[test]
    fn connection_rejects_sensitive_or_corrupt_identity_fields() {
        let credential = CredentialRef::new("cred://openai/default").unwrap();
        assert!(
            Connection::restore(
                ConnectionId(1),
                "openai",
                "user@example.com",
                credential.clone(),
                ConnectionHealth::Unknown,
                0,
                1,
                1
            )
            .is_ok()
        );
        assert!(
            Connection::restore(
                ConnectionId(1),
                "Open AI",
                "account",
                credential.clone(),
                ConnectionHealth::Unknown,
                0,
                1,
                1
            )
            .is_err()
        );
        assert!(
            Connection::restore(
                ConnectionId(1),
                "openai",
                "token\nvalue",
                credential,
                ConnectionHealth::Unknown,
                0,
                2,
                1
            )
            .is_err()
        );
    }
}
