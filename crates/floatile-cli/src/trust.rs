//! Host-owned publisher trust administration for PP-M8.

use std::path::Path;

use floatile_core::install::hex_decode;
use floatile_store::trust::{PublisherTrustRecord, TrustState};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum TrustCommandError {
    #[error("publisher id 或 Ed25519 public key 参数非法")]
    InvalidArgument,
    #[error("无法创建数据库目录: {0}")]
    DatabaseDirectory(String),
    #[error("trust store 失败: {0}")]
    Store(#[from] floatile_store::StoreError),
    #[error("publisher trust 不存在")]
    NotFound,
}

impl TrustCommandError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument => "FTRUST_ARGUMENT",
            Self::DatabaseDirectory(_) => "FTRUST_DATABASE",
            Self::Store(_) => "FTRUST_STORE",
            Self::NotFound => "FTRUST_NOT_FOUND",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustView {
    pub publisher_id: String,
    pub state: String,
    pub keys: Vec<TrustKeyView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustKeyView {
    pub key_id: String,
    pub state: String,
}

pub fn add_key(
    database: &Path,
    publisher_id: &str,
    public_key_hex: &str,
    updated_at: u64,
) -> Result<TrustView, TrustCommandError> {
    set_key_state(
        database,
        publisher_id,
        public_key_hex,
        TrustState::Active,
        updated_at,
    )
}

pub fn revoke_key(
    database: &Path,
    publisher_id: &str,
    public_key_hex: &str,
    updated_at: u64,
) -> Result<TrustView, TrustCommandError> {
    set_key_state(
        database,
        publisher_id,
        public_key_hex,
        TrustState::Revoked,
        updated_at,
    )
}

pub fn revoke_publisher(
    database: &Path,
    publisher_id: &str,
    updated_at: u64,
) -> Result<TrustView, TrustCommandError> {
    let store = open_database(database)?;
    if !store
        .trust()
        .set_publisher_state(publisher_id, TrustState::Revoked, updated_at)?
    {
        return Err(TrustCommandError::NotFound);
    }
    view(&store, publisher_id)
}

pub fn show(database: &Path, publisher_id: &str) -> Result<TrustView, TrustCommandError> {
    let store = open_database(database)?;
    view(&store, publisher_id)
}

fn set_key_state(
    database: &Path,
    publisher_id: &str,
    public_key_hex: &str,
    state: TrustState,
    updated_at: u64,
) -> Result<TrustView, TrustCommandError> {
    let public_key: [u8; 32] = hex_decode(public_key_hex)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(TrustCommandError::InvalidArgument)?;
    let store = open_database(database)?;
    store
        .trust()
        .upsert_key(publisher_id, public_key, state, updated_at)?;
    view(&store, publisher_id)
}

fn open_database(database: &Path) -> Result<floatile_store::Store, TrustCommandError> {
    if let Some(parent) = database.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| TrustCommandError::DatabaseDirectory(error.to_string()))?;
    }
    floatile_store::open(database).map_err(Into::into)
}

fn view(store: &floatile_store::Store, publisher_id: &str) -> Result<TrustView, TrustCommandError> {
    store
        .trust()
        .get(publisher_id)?
        .map(TrustView::from)
        .ok_or(TrustCommandError::NotFound)
}

impl From<PublisherTrustRecord> for TrustView {
    fn from(record: PublisherTrustRecord) -> Self {
        Self {
            publisher_id: record.publisher_id,
            state: state_name(record.state).to_owned(),
            keys: record
                .keys
                .into_iter()
                .map(|key| TrustKeyView {
                    key_id: key.key_id,
                    state: state_name(key.state).to_owned(),
                })
                .collect(),
        }
    }
}

fn state_name(state: TrustState) -> &'static str {
    match state {
        TrustState::Active => "active",
        TrustState::Revoked => "revoked",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn database(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("floatile-trust-{tag}-{}.db", std::process::id()))
    }

    #[test]
    fn add_show_and_revoke_never_expose_public_key_bytes() {
        let database = database("lifecycle");
        let _ = std::fs::remove_file(&database);
        let key = "07".repeat(32);
        let added = add_key(&database, "dev.floatile", &key, 1).unwrap();
        assert_eq!(added.state, "active");
        assert_eq!(added.keys[0].state, "active");
        assert!(!serde_json::to_string(&added).unwrap().contains(&key));

        let revoked = revoke_key(&database, "dev.floatile", &key, 2).unwrap();
        assert_eq!(revoked.keys[0].state, "revoked");
        let publisher = revoke_publisher(&database, "dev.floatile", 3).unwrap();
        assert_eq!(publisher.state, "revoked");
        assert_eq!(show(&database, "dev.floatile").unwrap(), publisher);
        let _ = std::fs::remove_file(&database);
    }

    #[test]
    fn rejects_noncanonical_key_length_and_unknown_publisher() {
        let database = database("invalid");
        let _ = std::fs::remove_file(&database);
        assert!(matches!(
            add_key(&database, "dev.floatile", "00", 1),
            Err(TrustCommandError::InvalidArgument)
        ));
        assert!(matches!(
            show(&database, "unknown.publisher"),
            Err(TrustCommandError::NotFound)
        ));
        let _ = std::fs::remove_file(&database);
    }
}
