pub mod capability;
pub mod connection;
pub mod constants;
pub mod distribution;
pub mod install;
pub mod instance;
pub mod layout;
pub mod manifest;
pub mod operation;
pub mod types;

pub use capability::{
    CAPABILITY_REGISTRY, CapabilityDefinition, CapabilityError, CapabilityExecution,
    CapabilityExposure, CapabilityId, CapabilityParamKind, CapabilityParams, CapabilityRisk,
    DenyReason, EffectiveGrant, Grant, Grants, InstanceGrant, PermissionDecision, TrustLevel,
    decide, narrow_instance, parse_capability_params,
};
pub use connection::{
    Connection, ConnectionGrant, ConnectionHealth, ConnectionId, ConnectionModelError,
    CredentialRef, MAX_CONNECTION_ACCOUNT_BYTES, MAX_CONNECTION_PROVIDER_BYTES,
    MAX_CREDENTIAL_REF_BYTES,
};
pub use distribution::{
    PACKAGE_DIGEST_PAYLOAD_TYPE, PermissionChange, PermissionChangeKind, SIGNATURE_FILE,
    SignatureVerificationError, TrustedPublisher, TrustedPublisherState, UpgradePlan,
    UpgradePlanError, dsse_pae, plan_upgrade, publisher_key_id, signable_content_digest,
    verify_signature_envelope,
};
pub use instance::{
    InstallationDigest, InstallationRef, InstanceConfig, InstanceDesiredState, InstanceModelError,
    MAX_INSTANCE_CONFIG_BYTES, MAX_INSTANCE_CONFIG_DEPTH, PluginInstance,
};
pub use layout::{
    LAYOUT_RECORD_VERSION, LayoutRecoveryError, LayoutValidationError, MonitorLayout,
    RecoveredLayout, WidgetLayout, recover_layout,
};
pub use manifest::{
    BuildMeta, ConfigRef, Entrypoints, HttpTemplateDecl, Manifest, ManifestError, PackagePath,
    PermissionDecl, PluginKind, Publisher, Sizes, StorageDecl, manifest_json_schema,
    validate_manifest, validate_manifest_json_with_schema,
};
pub use operation::{
    OperationCompletion, OperationCompletionDisposition, OperationFailure, OperationId,
    OperationOwner, OperationTerminal,
};
pub use types::{
    InstanceId, LogicalPosition, LogicalRect, LogicalSize, MonitorKey, PhysicalPosition,
    PhysicalRect, PhysicalSize, PluginId, ScaleFactor, ScaleFactorError, SizeConstraints,
    WidgetMode,
};
