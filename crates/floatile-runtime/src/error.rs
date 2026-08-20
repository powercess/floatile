//! 插件实例运行时错误。

use thiserror::Error;

/// 运行时（管理器/actor）错误。
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("wasmtime: {0}")]
    Wasmtime(#[from] wasmtime::Error),
    #[error("组件加载或世界不匹配: {0}")]
    Component(String),
    #[error("实例已终止: {0}")]
    InstanceFailed(String),
    #[error("实例命令通道已关闭")]
    Closed,
}

/// 单次调用（start/handle-event/stop）的结果错误。
#[derive(Debug, Error)]
pub enum InstanceError {
    /// guest 业务拒绝（`widget-error`）。
    #[error("插件拒绝: {0}")]
    Rejected(String),
    /// trap / fuel / 超时 / 内存等导致实例终止。
    #[error("实例终止: {0}")]
    Failed(String),
}
