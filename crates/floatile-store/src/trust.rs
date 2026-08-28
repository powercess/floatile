//! Host-owned publisher trust persistence for PP-M8.
//!
//! Package-controlled data never creates a trust anchor. Callers must explicitly provision or revoke
//! publisher keys through this store, then pass only active keys to the core signature verifier.

use floatile_core::distribution::{TrustedPublisher, TrustedPublisherState, publisher_key_id};
use rusqlite::{Connection, OptionalExtension};
use semver::Version;

use crate::StoreError;

const MAX_PUBLISHER_ID_BYTES: usize = 256;
const MAX_PLUGIN_ID_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    Active,
    Revoked,
}

impl TrustState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            other => Err(StoreError::Corrupt(format!(
                "publisher trust state 非法: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherKeyRecord {
    pub key_id: String,
    pub public_key: [u8; 32],
    pub state: TrustState,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherTrustRecord {
    pub publisher_id: String,
    pub state: TrustState,
    pub updated_at: u64,
    pub keys: Vec<PublisherKeyRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedPackage {
    pub publisher_id: String,
    pub plugin_id: String,
    pub version: Version,
    pub digest: [u8; 32],
    pub accepted_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceOutcome {
    FirstAccepted,
    NewerAccepted,
    AlreadyAccepted,
}

#[derive(Debug, thiserror::Error)]
pub enum TrustPolicyError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("publisher trust 不存在")]
    UnknownPublisher,
    #[error("publisher trust 已撤销")]
    RevokedPublisher,
    #[error("拒绝低于最高已接受版本的包: {candidate} < {highest}")]
    Rollback {
        candidate: Version,
        highest: Version,
    },
    #[error("同版本包的内容摘要与最高已接受记录不同")]
    SameVersionDifferentDigest,
}

impl From<rusqlite::Error> for TrustPolicyError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::Open(error))
    }
}

impl PublisherTrustRecord {
    /// Produces a verifier binding containing only active host-trusted keys.
    pub fn verifier_binding(&self) -> TrustedPublisher {
        TrustedPublisher {
            publisher_id: self.publisher_id.clone(),
            state: match self.state {
                TrustState::Active => TrustedPublisherState::Active,
                TrustState::Revoked => TrustedPublisherState::Revoked,
            },
            keys: self
                .keys
                .iter()
                .filter(|key| key.state == TrustState::Active)
                .map(|key| key.public_key)
                .collect(),
        }
    }
}

pub struct PublisherTrustStore<'a> {
    conn: &'a Connection,
}

impl<'a> PublisherTrustStore<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Provisions or replaces a publisher key and makes publisher trust explicit.
    pub fn upsert_key(
        &self,
        publisher_id: &str,
        public_key: [u8; 32],
        state: TrustState,
        updated_at: u64,
    ) -> Result<String, StoreError> {
        validate_publisher_id(publisher_id)?;
        let updated_at = sqlite_time(updated_at)?;
        let key_id = publisher_key_id(&public_key);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO publisher_trust (publisher_id, state, updated_at)
             VALUES (?1, 'active', ?2)
             ON CONFLICT(publisher_id) DO UPDATE SET updated_at = excluded.updated_at",
            rusqlite::params![publisher_id, updated_at],
        )?;
        tx.execute(
            "INSERT INTO publisher_keys (publisher_id, key_id, public_key, state, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(publisher_id, key_id) DO UPDATE SET
                 public_key = excluded.public_key,
                 state = excluded.state,
                 updated_at = excluded.updated_at",
            rusqlite::params![
                publisher_id,
                key_id,
                public_key.as_slice(),
                state.as_str(),
                updated_at
            ],
        )?;
        tx.commit()?;
        Ok(key_id)
    }

    /// Revokes or reactivates an existing publisher without modifying its keys.
    pub fn set_publisher_state(
        &self,
        publisher_id: &str,
        state: TrustState,
        updated_at: u64,
    ) -> Result<bool, StoreError> {
        validate_publisher_id(publisher_id)?;
        let changed = self.conn.execute(
            "UPDATE publisher_trust SET state = ?2, updated_at = ?3 WHERE publisher_id = ?1",
            rusqlite::params![publisher_id, state.as_str(), sqlite_time(updated_at)?],
        )?;
        Ok(changed > 0)
    }

    pub fn get(&self, publisher_id: &str) -> Result<Option<PublisherTrustRecord>, StoreError> {
        validate_publisher_id(publisher_id)?;
        let publisher = self
            .conn
            .query_row(
                "SELECT state, updated_at FROM publisher_trust WHERE publisher_id = ?1",
                [publisher_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((state, updated_at)) = publisher else {
            return Ok(None);
        };

        let mut statement = self.conn.prepare(
            "SELECT key_id, public_key, state, updated_at
             FROM publisher_keys WHERE publisher_id = ?1 ORDER BY key_id",
        )?;
        let rows = statement.query_map([publisher_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut keys = Vec::new();
        for row in rows {
            let (key_id, public_key, key_state, key_updated_at) = row?;
            let public_key: [u8; 32] = public_key.try_into().map_err(|bytes: Vec<u8>| {
                StoreError::Corrupt(format!(
                    "publisher public key 必须为 32 字节，实际为 {}",
                    bytes.len()
                ))
            })?;
            if publisher_key_id(&public_key) != key_id {
                return Err(StoreError::Corrupt(
                    "publisher key_id 与 public_key 不匹配".to_owned(),
                ));
            }
            keys.push(PublisherKeyRecord {
                key_id,
                public_key,
                state: TrustState::parse(&key_state)?,
                updated_at: read_time(key_updated_at, "publisher key updated_at")?,
            });
        }

        Ok(Some(PublisherTrustRecord {
            publisher_id: publisher_id.to_owned(),
            state: TrustState::parse(&state)?,
            updated_at: read_time(updated_at, "publisher trust updated_at")?,
            keys,
        }))
    }

    /// Atomically advances the highest accepted version/digest for a trusted publisher/plugin.
    ///
    /// Explicit rollback does not call this method: it references a verified historical installation
    /// while retaining this high-water mark.
    pub fn accept_package(
        &self,
        publisher_id: &str,
        plugin_id: &str,
        version: &Version,
        digest: [u8; 32],
        accepted_at: u64,
    ) -> Result<AcceptanceOutcome, TrustPolicyError> {
        validate_publisher_id(publisher_id)?;
        validate_plugin_id(plugin_id)?;
        let accepted_at = sqlite_time(accepted_at)?;
        let tx = self.conn.unchecked_transaction()?;
        let publisher_state = tx
            .query_row(
                "SELECT state FROM publisher_trust WHERE publisher_id = ?1",
                [publisher_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        match publisher_state.as_deref() {
            None => return Err(TrustPolicyError::UnknownPublisher),
            Some("revoked") => return Err(TrustPolicyError::RevokedPublisher),
            Some("active") => {}
            Some(other) => {
                return Err(
                    StoreError::Corrupt(format!("publisher trust state 非法: {other}")).into(),
                );
            }
        }

        let existing = tx
            .query_row(
                "SELECT version, digest, accepted_at FROM accepted_packages
                 WHERE publisher_id = ?1 AND plugin_id = ?2",
                rusqlite::params![publisher_id, plugin_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((highest_text, highest_digest, _)) = existing else {
            tx.execute(
                "INSERT INTO accepted_packages
                    (publisher_id, plugin_id, version, digest, accepted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    publisher_id,
                    plugin_id,
                    version.to_string(),
                    digest.as_slice(),
                    accepted_at
                ],
            )?;
            tx.commit()?;
            return Ok(AcceptanceOutcome::FirstAccepted);
        };
        let highest = Version::parse(&highest_text).map_err(|error| {
            StoreError::Corrupt(format!("accepted package version 非法: {error}"))
        })?;
        let highest_digest: [u8; 32] = highest_digest.try_into().map_err(|bytes: Vec<u8>| {
            StoreError::Corrupt(format!(
                "accepted package digest 必须为 32 字节，实际为 {}",
                bytes.len()
            ))
        })?;

        if version < &highest {
            return Err(TrustPolicyError::Rollback {
                candidate: version.clone(),
                highest,
            });
        }
        if version == &highest {
            if digest == highest_digest {
                return Ok(AcceptanceOutcome::AlreadyAccepted);
            }
            return Err(TrustPolicyError::SameVersionDifferentDigest);
        }

        tx.execute(
            "UPDATE accepted_packages SET version = ?3, digest = ?4, accepted_at = ?5
             WHERE publisher_id = ?1 AND plugin_id = ?2",
            rusqlite::params![
                publisher_id,
                plugin_id,
                version.to_string(),
                digest.as_slice(),
                accepted_at
            ],
        )?;
        tx.commit()?;
        Ok(AcceptanceOutcome::NewerAccepted)
    }

    pub fn accepted_package(
        &self,
        publisher_id: &str,
        plugin_id: &str,
    ) -> Result<Option<AcceptedPackage>, StoreError> {
        validate_publisher_id(publisher_id)?;
        validate_plugin_id(plugin_id)?;
        let row = self
            .conn
            .query_row(
                "SELECT version, digest, accepted_at FROM accepted_packages
                 WHERE publisher_id = ?1 AND plugin_id = ?2",
                rusqlite::params![publisher_id, plugin_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((version, digest, accepted_at)) = row else {
            return Ok(None);
        };
        Ok(Some(AcceptedPackage {
            publisher_id: publisher_id.to_owned(),
            plugin_id: plugin_id.to_owned(),
            version: Version::parse(&version).map_err(|error| {
                StoreError::Corrupt(format!("accepted package version 非法: {error}"))
            })?,
            digest: digest.try_into().map_err(|bytes: Vec<u8>| {
                StoreError::Corrupt(format!(
                    "accepted package digest 必须为 32 字节，实际为 {}",
                    bytes.len()
                ))
            })?,
            accepted_at: read_time(accepted_at, "accepted_at")?,
        }))
    }
}

fn validate_publisher_id(publisher_id: &str) -> Result<(), StoreError> {
    if publisher_id.is_empty() || publisher_id.len() > MAX_PUBLISHER_ID_BYTES {
        return Err(StoreError::Corrupt(
            "publisher id 为空或超过存储上限".to_owned(),
        ));
    }
    Ok(())
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), StoreError> {
    if plugin_id.is_empty() || plugin_id.len() > MAX_PLUGIN_ID_BYTES {
        return Err(StoreError::Corrupt(
            "plugin id 为空或超过存储上限".to_owned(),
        ));
    }
    Ok(())
}

fn sqlite_time(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Corrupt("publisher trust 时间超出范围".to_owned()))
}

fn read_time(value: i64, field: &str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt(format!("{field} 不能为负数")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::open;

    #[test]
    fn publisher_keys_round_trip_and_revocation_is_enforced_in_binding() {
        let store = open(":memory:").unwrap();
        let first = [7; 32];
        let second = [8; 32];
        let first_id = store
            .trust()
            .upsert_key("dev.floatile", first, TrustState::Active, 10)
            .unwrap();
        store
            .trust()
            .upsert_key("dev.floatile", second, TrustState::Revoked, 11)
            .unwrap();

        let record = store.trust().get("dev.floatile").unwrap().unwrap();
        assert_eq!(record.keys.len(), 2);
        assert!(record.keys.iter().any(|key| key.key_id == first_id));
        assert_eq!(record.verifier_binding().keys, vec![first]);

        assert!(
            store
                .trust()
                .set_publisher_state("dev.floatile", TrustState::Revoked, 12)
                .unwrap()
        );
        assert_eq!(
            store
                .trust()
                .get("dev.floatile")
                .unwrap()
                .unwrap()
                .verifier_binding()
                .state,
            TrustedPublisherState::Revoked
        );
    }

    #[test]
    fn unknown_publisher_is_not_implicitly_created_by_reads_or_state_changes() {
        let store = open(":memory:").unwrap();
        assert!(store.trust().get("unknown.publisher").unwrap().is_none());
        assert!(
            !store
                .trust()
                .set_publisher_state("unknown.publisher", TrustState::Active, 1)
                .unwrap()
        );
        assert!(store.trust().get("unknown.publisher").unwrap().is_none());
    }

    #[test]
    fn corrupt_key_id_is_rejected_on_load() {
        let store = open(":memory:").unwrap();
        store
            .trust()
            .upsert_key("dev.floatile", [9; 32], TrustState::Active, 1)
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE publisher_keys SET key_id = ?1 WHERE publisher_id = ?2",
                rusqlite::params!["00".repeat(32), "dev.floatile"],
            )
            .unwrap();
        assert!(matches!(
            store.trust().get("dev.floatile"),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn anti_rollback_high_water_mark_rejects_downgrade_and_replacement() {
        let store = open(":memory:").unwrap();
        store
            .trust()
            .upsert_key("dev.floatile", [3; 32], TrustState::Active, 1)
            .unwrap();
        let v1 = Version::parse("1.0.0").unwrap();
        let v2 = Version::parse("2.0.0").unwrap();
        assert_eq!(
            store
                .trust()
                .accept_package("dev.floatile", "dev.floatile.clock", &v1, [1; 32], 2)
                .unwrap(),
            AcceptanceOutcome::FirstAccepted
        );
        assert_eq!(
            store
                .trust()
                .accept_package("dev.floatile", "dev.floatile.clock", &v2, [2; 32], 3)
                .unwrap(),
            AcceptanceOutcome::NewerAccepted
        );
        assert_eq!(
            store
                .trust()
                .accept_package("dev.floatile", "dev.floatile.clock", &v2, [2; 32], 4)
                .unwrap(),
            AcceptanceOutcome::AlreadyAccepted
        );
        assert!(matches!(
            store
                .trust()
                .accept_package("dev.floatile", "dev.floatile.clock", &v1, [1; 32], 5),
            Err(TrustPolicyError::Rollback { .. })
        ));
        assert!(matches!(
            store
                .trust()
                .accept_package("dev.floatile", "dev.floatile.clock", &v2, [9; 32], 6),
            Err(TrustPolicyError::SameVersionDifferentDigest)
        ));

        let accepted = store
            .trust()
            .accepted_package("dev.floatile", "dev.floatile.clock")
            .unwrap()
            .unwrap();
        assert_eq!(accepted.version, v2);
        assert_eq!(accepted.digest, [2; 32]);
        assert_eq!(accepted.accepted_at, 3);
    }

    #[test]
    fn anti_rollback_requires_active_explicit_publisher_trust() {
        let store = open(":memory:").unwrap();
        let version = Version::parse("1.0.0").unwrap();
        assert!(matches!(
            store.trust().accept_package(
                "unknown.publisher",
                "unknown.publisher.plugin",
                &version,
                [1; 32],
                1
            ),
            Err(TrustPolicyError::UnknownPublisher)
        ));
        store
            .trust()
            .upsert_key("dev.floatile", [4; 32], TrustState::Active, 1)
            .unwrap();
        store
            .trust()
            .set_publisher_state("dev.floatile", TrustState::Revoked, 2)
            .unwrap();
        assert!(matches!(
            store.trust().accept_package(
                "dev.floatile",
                "dev.floatile.clock",
                &version,
                [1; 32],
                3
            ),
            Err(TrustPolicyError::RevokedPublisher)
        ));
        assert!(
            store
                .trust()
                .accepted_package("dev.floatile", "dev.floatile.clock")
                .unwrap()
                .is_none()
        );
    }
}
