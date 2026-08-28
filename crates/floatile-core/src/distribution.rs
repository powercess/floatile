//! PP-M8 package signature verification and publisher trust binding.
//!
//! The package remains untrusted input throughout this module. A valid signature proves that the
//! signable package bytes were authenticated by a host-trusted publisher key; it never grants a
//! capability and it does not replace install-time path or resource validation.

use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::install::{content_digest, hex_encode};
use crate::{CapabilityId, CapabilityParams, Manifest, parse_capability_params};

pub const SIGNATURE_FILE: &str = "signature.json";
pub const PACKAGE_DIGEST_PAYLOAD_TYPE: &str = "application/vnd.floatile.package-digest.v1";
pub const MAX_SIGNATURE_ENVELOPE_BYTES: usize = 16 * 1024;
pub const MAX_PACKAGE_SIGNATURES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustedPublisherState {
    Active,
    Revoked,
}

/// Host-owned trust binding. Keys are raw Ed25519 public-key bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedPublisher {
    pub publisher_id: String,
    pub state: TrustedPublisherState,
    pub keys: Vec<[u8; 32]>,
    /// Host-revoked key identifiers retained to distinguish revocation from an unknown key.
    pub revoked_key_ids: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignatureVerificationError {
    #[error("signature envelope exceeds the size limit")]
    EnvelopeTooLarge,
    #[error("signature envelope is malformed")]
    MalformedEnvelope,
    #[error("signature payload type is unsupported")]
    UnsupportedPayloadType,
    #[error("signature envelope contains an invalid signature count")]
    InvalidSignatureCount,
    #[error("signed package digest does not match package contents")]
    DigestMismatch,
    #[error("package publisher does not match the trust binding")]
    PublisherMismatch,
    #[error("package publisher is revoked")]
    PublisherRevoked,
    #[error("package signing key is revoked")]
    KeyRevoked,
    #[error("signature envelope does not reference a trusted publisher key")]
    UnknownKey,
    #[error("package signature is invalid")]
    InvalidSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChangeKind {
    Added,
    Removed,
    Expanded,
    Reduced,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionChange {
    pub capability: CapabilityId,
    pub kind: PermissionChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradePlan {
    pub current_version: semver::Version,
    pub candidate_version: semver::Version,
    pub permissions: Vec<PermissionChange>,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackPlan {
    pub current_version: semver::Version,
    pub target_version: semver::Version,
    pub permissions: Vec<PermissionChange>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UpgradePlanError {
    #[error("upgrade candidate changes plugin id")]
    PluginMismatch,
    #[error("upgrade candidate changes publisher")]
    PublisherMismatch,
    #[error("upgrade candidate version must be newer")]
    VersionNotNewer,
    #[error("upgrade candidate decreases storage migration version")]
    StorageMigrationRollback,
    #[error("upgrade manifest contains invalid permission: {0}")]
    InvalidPermission(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RollbackPlanError {
    #[error("rollback target changes plugin id")]
    PluginMismatch,
    #[error("rollback target changes publisher")]
    PublisherMismatch,
    #[error("rollback target version must be older")]
    VersionNotOlder,
    #[error("rollback target cannot read the current storage migration")]
    StorageMigrationIncompatible,
    #[error("rollback would restore previously removed or reduced permissions")]
    PermissionExpansion,
    #[error("rollback manifest contains invalid permission: {0}")]
    InvalidPermission(String),
}

/// Builds a deterministic permission and compatibility plan for an in-place upgrade.
pub fn plan_upgrade(
    current: &Manifest,
    candidate: &Manifest,
) -> Result<UpgradePlan, UpgradePlanError> {
    if current.id != candidate.id {
        return Err(UpgradePlanError::PluginMismatch);
    }
    if current.publisher.id != candidate.publisher.id {
        return Err(UpgradePlanError::PublisherMismatch);
    }
    let current_version = semver::Version::parse(&current.version)
        .map_err(|error| UpgradePlanError::InvalidPermission(error.to_string()))?;
    let candidate_version = semver::Version::parse(&candidate.version)
        .map_err(|error| UpgradePlanError::InvalidPermission(error.to_string()))?;
    if candidate_version <= current_version {
        return Err(UpgradePlanError::VersionNotNewer);
    }
    let current_migration = current
        .storage
        .as_ref()
        .map_or(0, |storage| storage.migration_version);
    let candidate_migration = candidate
        .storage
        .as_ref()
        .map_or(0, |storage| storage.migration_version);
    if candidate_migration < current_migration {
        return Err(UpgradePlanError::StorageMigrationRollback);
    }

    let permissions =
        permission_changes(current, candidate).map_err(UpgradePlanError::InvalidPermission)?;
    let requires_confirmation = permissions.iter().any(|change| {
        matches!(
            change.kind,
            PermissionChangeKind::Added | PermissionChangeKind::Expanded
        )
    });
    Ok(UpgradePlan {
        current_version,
        candidate_version,
        permissions,
        requires_confirmation,
    })
}

/// Plans an explicit rollback without lowering anti-rollback state or restoring old authority.
pub fn plan_rollback(
    current: &Manifest,
    target: &Manifest,
) -> Result<RollbackPlan, RollbackPlanError> {
    if current.id != target.id {
        return Err(RollbackPlanError::PluginMismatch);
    }
    if current.publisher.id != target.publisher.id {
        return Err(RollbackPlanError::PublisherMismatch);
    }
    let current_version = semver::Version::parse(&current.version)
        .map_err(|error| RollbackPlanError::InvalidPermission(error.to_string()))?;
    let target_version = semver::Version::parse(&target.version)
        .map_err(|error| RollbackPlanError::InvalidPermission(error.to_string()))?;
    if target_version >= current_version {
        return Err(RollbackPlanError::VersionNotOlder);
    }
    let current_migration = current
        .storage
        .as_ref()
        .map_or(0, |storage| storage.migration_version);
    let target_migration = target
        .storage
        .as_ref()
        .map_or(0, |storage| storage.migration_version);
    if target_migration != current_migration {
        return Err(RollbackPlanError::StorageMigrationIncompatible);
    }
    let permissions =
        permission_changes(current, target).map_err(RollbackPlanError::InvalidPermission)?;
    if permissions.iter().any(|change| {
        matches!(
            change.kind,
            PermissionChangeKind::Added | PermissionChangeKind::Expanded
        )
    }) {
        return Err(RollbackPlanError::PermissionExpansion);
    }
    Ok(RollbackPlan {
        current_version,
        target_version,
        permissions,
    })
}

fn permission_changes(
    current: &Manifest,
    candidate: &Manifest,
) -> Result<Vec<PermissionChange>, String> {
    let current_permissions = parsed_permissions(current).map_err(|error| error.to_string())?;
    let candidate_permissions = parsed_permissions(candidate).map_err(|error| error.to_string())?;
    let capabilities: BTreeSet<_> = current_permissions
        .keys()
        .chain(candidate_permissions.keys())
        .copied()
        .collect();
    let mut permissions = Vec::with_capacity(capabilities.len());
    for capability in capabilities {
        let kind = match (
            current_permissions.get(&capability),
            candidate_permissions.get(&capability),
        ) {
            (None, Some(_)) => PermissionChangeKind::Added,
            (Some(_), None) => PermissionChangeKind::Removed,
            (Some(current), Some(candidate)) if current == candidate => {
                PermissionChangeKind::Unchanged
            }
            (Some(current), Some(candidate)) if params_within(candidate, current) => {
                PermissionChangeKind::Reduced
            }
            (Some(_), Some(_)) => PermissionChangeKind::Expanded,
            (None, None) => continue,
        };
        permissions.push(PermissionChange { capability, kind });
    }
    Ok(permissions)
}

fn parsed_permissions(
    manifest: &Manifest,
) -> Result<BTreeMap<CapabilityId, Option<CapabilityParams>>, UpgradePlanError> {
    manifest
        .permissions
        .iter()
        .map(|permission| {
            let capability = CapabilityId::from_name(&permission.capability).ok_or_else(|| {
                UpgradePlanError::InvalidPermission(permission.capability.clone())
            })?;
            let params = parse_capability_params(capability, permission.params.as_ref())
                .map_err(|error| UpgradePlanError::InvalidPermission(error.to_string()))?;
            Ok((capability, params))
        })
        .collect()
}

fn params_within(candidate: &Option<CapabilityParams>, current: &Option<CapabilityParams>) -> bool {
    match (candidate, current) {
        (None, None) => true,
        (
            Some(CapabilityParams::Storage {
                keys: candidate_keys,
                max_bytes: candidate_bytes,
            }),
            Some(CapabilityParams::Storage {
                keys: current_keys,
                max_bytes: current_bytes,
            }),
        ) => key_scope_within(candidate_keys, current_keys) && candidate_bytes <= current_bytes,
        (
            Some(CapabilityParams::Timer {
                max_per_minute: candidate_rate,
                max_active: candidate_active,
            }),
            Some(CapabilityParams::Timer {
                max_per_minute: current_rate,
                max_active: current_active,
            }),
        ) => candidate_rate <= current_rate && candidate_active <= current_active,
        (
            Some(CapabilityParams::Metrics {
                sample_rate_hz: candidate_rate,
            }),
            Some(CapabilityParams::Metrics {
                sample_rate_hz: current_rate,
            }),
        ) => candidate_rate <= current_rate,
        (
            Some(CapabilityParams::Network {
                origins: candidate_origins,
                max_requests_per_minute: candidate_requests,
                max_response_bytes: candidate_bytes,
                max_timeout_ms: candidate_timeout,
            }),
            Some(CapabilityParams::Network {
                origins: current_origins,
                max_requests_per_minute: current_requests,
                max_response_bytes: current_bytes,
                max_timeout_ms: current_timeout,
            }),
        ) => {
            candidate_origins
                .iter()
                .all(|origin| current_origins.contains(origin))
                && candidate_requests <= current_requests
                && candidate_bytes <= current_bytes
                && candidate_timeout <= current_timeout
        }
        _ => false,
    }
}

fn key_scope_within(candidate: &[String], current: &[String]) -> bool {
    if current.is_empty() {
        return true;
    }
    !candidate.is_empty() && candidate.iter().all(|key| current.contains(key))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureEnvelope {
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    signatures: Vec<EnvelopeSignature>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeSignature {
    keyid: String,
    sig: String,
}

/// Computes the digest authenticated by `signature.json`.
///
/// The detached envelope is the sole excluded root file. Install integrity must separately use
/// `content_digest`, which includes every installed ordinary file.
pub fn signable_content_digest(files: &BTreeMap<String, Vec<u8>>) -> [u8; 32] {
    let signable = files
        .iter()
        .filter(|(path, _)| path.as_str() != SIGNATURE_FILE)
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect();
    content_digest(&signable)
}

/// Returns the lowercase SHA-256 key identifier for raw Ed25519 public-key bytes.
pub fn publisher_key_id(public_key: &[u8; 32]) -> String {
    hex_encode(&Sha256::digest(public_key))
}

/// DSSE v1 pre-authentication encoding.
pub fn dsse_pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    format!(
        "DSSEv1 {} {payload_type} {} ",
        payload_type.len(),
        payload.len()
    )
    .into_bytes()
    .into_iter()
    .chain(payload.iter().copied())
    .collect()
}

/// Verifies a detached package signature against a host-owned publisher trust binding.
pub fn verify_signature_envelope(
    envelope_json: &[u8],
    files: &BTreeMap<String, Vec<u8>>,
    manifest_publisher_id: &str,
    trusted_publisher: &TrustedPublisher,
) -> Result<(), SignatureVerificationError> {
    if envelope_json.len() > MAX_SIGNATURE_ENVELOPE_BYTES {
        return Err(SignatureVerificationError::EnvelopeTooLarge);
    }
    if manifest_publisher_id != trusted_publisher.publisher_id {
        return Err(SignatureVerificationError::PublisherMismatch);
    }
    if trusted_publisher.state == TrustedPublisherState::Revoked {
        return Err(SignatureVerificationError::PublisherRevoked);
    }

    let envelope: SignatureEnvelope = serde_json::from_slice(envelope_json)
        .map_err(|_| SignatureVerificationError::MalformedEnvelope)?;
    if envelope.payload_type != PACKAGE_DIGEST_PAYLOAD_TYPE {
        return Err(SignatureVerificationError::UnsupportedPayloadType);
    }
    if envelope.signatures.is_empty() || envelope.signatures.len() > MAX_PACKAGE_SIGNATURES {
        return Err(SignatureVerificationError::InvalidSignatureCount);
    }

    let payload = decode_canonical_base64(&envelope.payload)?;
    if payload.len() != 32 {
        return Err(SignatureVerificationError::MalformedEnvelope);
    }
    if payload.as_slice() != signable_content_digest(files) {
        return Err(SignatureVerificationError::DigestMismatch);
    }
    let pae = dsse_pae(&envelope.payload_type, &payload);
    if envelope
        .signatures
        .iter()
        .any(|signature| trusted_publisher.revoked_key_ids.contains(&signature.keyid))
    {
        return Err(SignatureVerificationError::KeyRevoked);
    }
    let mut matched_trusted_key = false;

    for signature in &envelope.signatures {
        for public_key in &trusted_publisher.keys {
            if signature.keyid != publisher_key_id(public_key) {
                continue;
            }
            matched_trusted_key = true;
            let signature_bytes = decode_canonical_base64(&signature.sig)?;
            let signature_bytes: [u8; 64] = signature_bytes
                .try_into()
                .map_err(|_| SignatureVerificationError::MalformedEnvelope)?;
            let verifying_key = VerifyingKey::from_bytes(public_key)
                .map_err(|_| SignatureVerificationError::MalformedEnvelope)?;
            if verifying_key
                .verify_strict(&pae, &Signature::from_bytes(&signature_bytes))
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    if matched_trusted_key {
        Err(SignatureVerificationError::InvalidSignature)
    } else {
        Err(SignatureVerificationError::UnknownKey)
    }
}

fn decode_canonical_base64(value: &str) -> Result<Vec<u8>, SignatureVerificationError> {
    let decoded = STANDARD
        .decode(value)
        .map_err(|_| SignatureVerificationError::MalformedEnvelope)?;
    if STANDARD.encode(&decoded) != value {
        return Err(SignatureVerificationError::MalformedEnvelope);
    }
    Ok(decoded)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    use super::*;
    use crate::install::hex_decode;

    fn package_files() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            ("logic/plugin.wasm".to_owned(), vec![0, 97, 115, 109]),
            (
                "manifest.json".to_owned(),
                br#"{"id":"dev.floatile.clock"}"#.to_vec(),
            ),
            (SIGNATURE_FILE.to_owned(), b"detached envelope".to_vec()),
        ])
    }

    fn signed_envelope(files: &BTreeMap<String, Vec<u8>>, signing_key: &SigningKey) -> Vec<u8> {
        let payload = signable_content_digest(files);
        let signature = signing_key.sign(&dsse_pae(PACKAGE_DIGEST_PAYLOAD_TYPE, &payload));
        serde_json::to_vec(&json!({
            "payloadType": PACKAGE_DIGEST_PAYLOAD_TYPE,
            "payload": STANDARD.encode(payload),
            "signatures": [{
                "keyid": publisher_key_id(signing_key.verifying_key().as_bytes()),
                "sig": STANDARD.encode(signature.to_bytes()),
            }]
        }))
        .unwrap()
    }

    fn trusted(signing_key: &SigningKey) -> TrustedPublisher {
        TrustedPublisher {
            publisher_id: "dev.floatile".to_owned(),
            state: TrustedPublisherState::Active,
            keys: vec![signing_key.verifying_key().to_bytes()],
            revoked_key_ids: Vec::new(),
        }
    }

    #[test]
    fn verifies_trusted_dsse_signature() {
        let files = package_files();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let envelope = signed_envelope(&files, &signing_key);

        assert_eq!(
            verify_signature_envelope(&envelope, &files, "dev.floatile", &trusted(&signing_key)),
            Ok(())
        );
    }

    #[test]
    fn signature_file_is_detached_but_other_files_are_authenticated() {
        let files = package_files();
        let signing_key = SigningKey::from_bytes(&[8; 32]);
        let envelope = signed_envelope(&files, &signing_key);

        let mut changed_envelope_file = files.clone();
        changed_envelope_file.insert(SIGNATURE_FILE.to_owned(), b"replacement".to_vec());
        assert_eq!(
            verify_signature_envelope(
                &envelope,
                &changed_envelope_file,
                "dev.floatile",
                &trusted(&signing_key)
            ),
            Ok(())
        );

        let mut changed_wasm = files.clone();
        changed_wasm.insert("logic/plugin.wasm".to_owned(), vec![1]);
        assert_eq!(
            verify_signature_envelope(
                &envelope,
                &changed_wasm,
                "dev.floatile",
                &trusted(&signing_key)
            ),
            Err(SignatureVerificationError::DigestMismatch)
        );
    }

    #[test]
    fn rejects_unknown_key_invalid_signature_and_publisher_states() {
        let files = package_files();
        let signing_key = SigningKey::from_bytes(&[9; 32]);
        let envelope = signed_envelope(&files, &signing_key);
        let other_key = SigningKey::from_bytes(&[10; 32]);

        assert_eq!(
            verify_signature_envelope(&envelope, &files, "dev.floatile", &trusted(&other_key)),
            Err(SignatureVerificationError::UnknownKey)
        );
        assert_eq!(
            verify_signature_envelope(&envelope, &files, "other.publisher", &trusted(&signing_key)),
            Err(SignatureVerificationError::PublisherMismatch)
        );

        let mut revoked = trusted(&signing_key);
        revoked.state = TrustedPublisherState::Revoked;
        assert_eq!(
            verify_signature_envelope(&envelope, &files, "dev.floatile", &revoked),
            Err(SignatureVerificationError::PublisherRevoked)
        );

        let mut revoked_key = trusted(&signing_key);
        revoked_key.keys.clear();
        revoked_key
            .revoked_key_ids
            .push(publisher_key_id(signing_key.verifying_key().as_bytes()));
        assert_eq!(
            verify_signature_envelope(&envelope, &files, "dev.floatile", &revoked_key),
            Err(SignatureVerificationError::KeyRevoked)
        );

        let mut value: serde_json::Value = serde_json::from_slice(&envelope).unwrap();
        value["signatures"][0]["sig"] = json!(STANDARD.encode([0; 64]));
        assert_eq!(
            verify_signature_envelope(
                &serde_json::to_vec(&value).unwrap(),
                &files,
                "dev.floatile",
                &trusted(&signing_key)
            ),
            Err(SignatureVerificationError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_domain_changes_and_unbounded_or_noncanonical_envelopes() {
        let files = package_files();
        let signing_key = SigningKey::from_bytes(&[11; 32]);
        let envelope = signed_envelope(&files, &signing_key);
        let mut value: serde_json::Value = serde_json::from_slice(&envelope).unwrap();
        value["payloadType"] = json!("application/octet-stream");
        assert_eq!(
            verify_signature_envelope(
                &serde_json::to_vec(&value).unwrap(),
                &files,
                "dev.floatile",
                &trusted(&signing_key)
            ),
            Err(SignatureVerificationError::UnsupportedPayloadType)
        );

        assert_eq!(
            verify_signature_envelope(
                &vec![b' '; MAX_SIGNATURE_ENVELOPE_BYTES + 1],
                &files,
                "dev.floatile",
                &trusted(&signing_key)
            ),
            Err(SignatureVerificationError::EnvelopeTooLarge)
        );

        let mut value: serde_json::Value = serde_json::from_slice(&envelope).unwrap();
        value["unexpected"] = json!(true);
        assert_eq!(
            verify_signature_envelope(
                &serde_json::to_vec(&value).unwrap(),
                &files,
                "dev.floatile",
                &trusted(&signing_key)
            ),
            Err(SignatureVerificationError::MalformedEnvelope)
        );
    }

    #[test]
    fn accepts_rfc8032_ed25519_test_vector() {
        // RFC 8032 section 7.1, test 1: empty message.
        let public_key: [u8; 32] = hex_decode(
            "d75a980182b10ab7d54bfed3c964073a\
             0ee172f3daa62325af021a68f707511a",
        )
        .unwrap()
        .try_into()
        .unwrap();
        let signature: [u8; 64] = hex_decode(
            "e5564300c360ac729086e2cc806e828a\
             84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46b\
             d25bf5f0595bbe24655141438e7a100b",
        )
        .unwrap()
        .try_into()
        .unwrap();

        assert!(
            VerifyingKey::from_bytes(&public_key)
                .unwrap()
                .verify_strict(b"", &Signature::from_bytes(&signature))
                .is_ok()
        );
    }

    #[test]
    fn pae_matches_dsse_v1_encoding() {
        assert_eq!(
            dsse_pae("text/plain", b"hello"),
            b"DSSEv1 10 text/plain 5 hello"
        );
    }

    fn upgrade_manifest(
        version: &str,
        permissions: serde_json::Value,
        migration_version: u64,
    ) -> Manifest {
        serde_json::from_value(json!({
            "manifestVersion": 1,
            "id": "dev.floatile.clock",
            "name": "Clock",
            "version": version,
            "publisher": { "id": "dev.floatile", "name": "Floatile" },
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "sizes": {
                "default": { "width": 240.0, "height": 120.0 },
                "min": { "width": 100.0, "height": 60.0 },
                "max": { "width": 800.0, "height": 600.0 },
                "resizable": true
            },
            "permissions": permissions,
            "storage": { "migration_version": migration_version }
        }))
        .unwrap()
    }

    #[test]
    fn upgrade_plan_requires_confirmation_for_added_or_expanded_permissions() {
        let current = upgrade_manifest(
            "1.0.0",
            json!([{
                "capability": "timer:schedule",
                "params": { "maxPerMinute": 10, "maxActive": 2 }
            }]),
            1,
        );
        let candidate = upgrade_manifest(
            "2.0.0",
            json!([
                {
                    "capability": "timer:schedule",
                    "params": { "maxPerMinute": 20, "maxActive": 2 }
                },
                { "capability": "theme:subscribe" }
            ]),
            2,
        );
        let plan = plan_upgrade(&current, &candidate).unwrap();
        assert!(plan.requires_confirmation);
        assert_eq!(
            plan.permissions,
            vec![
                PermissionChange {
                    capability: CapabilityId::TimerSchedule,
                    kind: PermissionChangeKind::Expanded,
                },
                PermissionChange {
                    capability: CapabilityId::ThemeSubscribe,
                    kind: PermissionChangeKind::Added,
                }
            ]
        );
    }

    #[test]
    fn upgrade_plan_allows_removed_and_strictly_reduced_permissions() {
        let current = upgrade_manifest(
            "1.0.0",
            json!([
                {
                    "capability": "storage:read",
                    "params": { "keys": [], "maxBytes": 4096 }
                },
                { "capability": "theme:subscribe" }
            ]),
            2,
        );
        let candidate = upgrade_manifest(
            "1.1.0",
            json!([{
                "capability": "storage:read",
                "params": { "keys": ["clock"], "maxBytes": 1024 }
            }]),
            2,
        );
        let plan = plan_upgrade(&current, &candidate).unwrap();
        assert!(!plan.requires_confirmation);
        assert!(plan.permissions.iter().any(|change| {
            change.capability == CapabilityId::StorageRead
                && change.kind == PermissionChangeKind::Reduced
        }));
        assert!(plan.permissions.iter().any(|change| {
            change.capability == CapabilityId::ThemeSubscribe
                && change.kind == PermissionChangeKind::Removed
        }));
    }

    #[test]
    fn upgrade_plan_rejects_identity_version_and_migration_regressions() {
        let current = upgrade_manifest("2.0.0", json!([]), 2);
        let old = upgrade_manifest("1.0.0", json!([]), 2);
        assert_eq!(
            plan_upgrade(&current, &old),
            Err(UpgradePlanError::VersionNotNewer)
        );
        let migration = upgrade_manifest("3.0.0", json!([]), 1);
        assert_eq!(
            plan_upgrade(&current, &migration),
            Err(UpgradePlanError::StorageMigrationRollback)
        );
        let mut publisher = upgrade_manifest("3.0.0", json!([]), 2);
        publisher.publisher.id = "other.publisher".to_owned();
        assert_eq!(
            plan_upgrade(&current, &publisher),
            Err(UpgradePlanError::PublisherMismatch)
        );
    }

    #[test]
    fn rollback_plan_requires_older_compatible_non_expanding_target() {
        let current = upgrade_manifest(
            "2.0.0",
            json!([{
                "capability": "timer:schedule",
                "params": { "maxPerMinute": 10, "maxActive": 1 }
            }]),
            2,
        );
        let safe_target = upgrade_manifest("1.0.0", json!([]), 2);
        let plan = plan_rollback(&current, &safe_target).unwrap();
        assert_eq!(
            plan.target_version,
            semver::Version::parse("1.0.0").unwrap()
        );

        let expanded_target = upgrade_manifest(
            "1.0.0",
            json!([{
                "capability": "timer:schedule",
                "params": { "maxPerMinute": 60, "maxActive": 2 }
            }]),
            2,
        );
        assert_eq!(
            plan_rollback(&current, &expanded_target),
            Err(RollbackPlanError::PermissionExpansion)
        );
        let incompatible_storage = upgrade_manifest("1.0.0", json!([]), 1);
        assert_eq!(
            plan_rollback(&current, &incompatible_storage),
            Err(RollbackPlanError::StorageMigrationIncompatible)
        );
    }
}
