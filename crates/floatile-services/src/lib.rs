//! 经 PermissionBroker 仲裁的 Floatile 宿主能力实现。
//!
//! `Broker` 是宿主能力的唯一入口：WIT adapter 只持本门面，所有 capability 调用
//! 先经 `decide` 授权（deny-by-default），允许/拒绝都写脱敏审计，再执行对应
//! 服务（clock/log/timer/storage/metrics/theme）。固有能力固定当前实例 scope。

pub mod audit;
pub mod broker;
pub mod clock;
pub mod credential;
pub mod errors;
pub mod http;
pub mod log;
pub mod metrics;
pub mod operation;
pub mod storage;
pub mod theme;
pub mod timer;

pub use audit::{AuditEvent, AuditListener, AuditSink, fnv1a};
pub use broker::Broker;
pub use clock::{Clock, ClockSnapshot};
pub use credential::{
    CredentialError, CredentialVault, MAX_CREDENTIAL_BYTES, MemoryCredentialVault,
    PlatformCredentialVault,
};
pub use errors::{LogError, MetricsError, StorageError, ThemeError, TimerError};
pub use http::{
    ConnectionHealthListener, HttpResponse, HttpServiceError, HttpTransport, HttpTransportRequest,
    HttpsService, QueryParam, ReqwestHttpTransport,
};
pub use log::{LogLevel, LogService};
pub use metrics::{MemorySnapshot, MetricsService};
pub use operation::{
    OperationCancelError, OperationCompletionReceiver, OperationLimits, OperationRegistry,
    OperationResultDiscarder, OperationServiceError, OperationSubmitError, OperationTakeError,
};
pub use storage::StorageService;
pub use theme::ThemeService;
pub use timer::{TimerService, TimerSink};

pub use floatile_core::{
    CapabilityError, CapabilityId, CapabilityParams, DenyReason, EffectiveGrant, Grant,
    InstanceGrant, PermissionDecision, TrustLevel,
};
