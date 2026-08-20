pub mod capability;
pub mod constants;
pub mod layout;
pub mod manifest;
pub mod types;

pub use capability::{
    CapabilityError, CapabilityId, CapabilityParams, DenyReason, EffectiveGrant, Grant, Grants,
    InstanceGrant, PermissionDecision, TrustLevel, decide, narrow_instance,
    parse_capability_params,
};
pub use layout::{
    LAYOUT_RECORD_VERSION, LayoutRecoveryError, LayoutValidationError, MonitorLayout,
    RecoveredLayout, WidgetLayout, recover_layout,
};
pub use manifest::{
    BuildMeta, ConfigRef, Entrypoints, Manifest, ManifestError, PackagePath, PermissionDecl,
    PluginKind, Publisher, Sizes, StorageDecl, validate_manifest,
};
pub use types::{
    InstanceId, LogicalPosition, LogicalRect, LogicalSize, MonitorKey, PhysicalPosition,
    PhysicalRect, PhysicalSize, PluginId, ScaleFactor, ScaleFactorError, SizeConstraints,
    WidgetMode,
};
