//! Floatile UI schema 单源 crate。
//!
//! 定义 `widget.ftui` v1 的 IR 类型、组件 registry、State/Event schema 与结构/
//! 预算校验。Rust/TypeScript SDK、CLI、runtime 与 shell renderer 都以本 crate 为
//! 单一语义源；不得在别处手写平行 UI 类型。
//!
//! 本 crate 是纯、guest-safe 的领域层：不依赖 Slint、Wasmtime、Tokio、SQLite、
//! 平台 API 或宿主服务，可同时在 host 与 `wasm32-wasip2` 编译。

pub mod error;
pub mod ir;
pub mod patch;
pub mod path;
pub mod registry;
pub mod schema;
pub mod theme;
pub mod validate;

pub use error::UiSchemaError;
pub use ir::{Binding, Component, EmittedEvent, EventSchema, PropValue, StateSchema, UiDocument};
pub use patch::{MAX_PATCH_BYTES, MAX_STATE_BYTES, MAX_UPDATE_RATE_PER_SEC, merge_patch};
pub use registry::{
    ChildrenPolicy, ComponentKind, ComponentSpec, JsonType, PropSchema, RegistryContract,
    contract as component_registry_contract,
};
pub use schema::{JsonSchema, validate_value};
pub use validate::validate_document;

/// 当前受支持的 UI IR 版本（独立于 WIT/manifest/SDK/plugin version）。
pub const UI_API_VERSION: &str = "1.6.0";

// ---- P0 初始预算（ui-ir-v1 §10；evil/clock/10-instance 数据后才能冻结）----

/// IR 文件最大字节数。
pub const MAX_IR_BYTES: usize = 256 * 1024;
/// 组件树节点上限。
pub const MAX_NODES: usize = 256;
/// 组件树最大深度。
pub const MAX_TREE_DEPTH: usize = 32;
/// 绑定总数上限。
pub const MAX_BINDINGS: usize = 512;
/// 事件声明数量上限。
pub const MAX_EVENT_DECLS: usize = 128;
/// asset 引用数量上限。
pub const MAX_ASSET_REFS: usize = 64;
/// 单个动态 List 的字符串项上限。
pub const MAX_LIST_ITEMS: usize = 256;
/// 单个 Sparkline 的数值采样点上限。
pub const MAX_CHART_POINTS: usize = 128;
