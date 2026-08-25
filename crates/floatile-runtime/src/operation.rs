//! Operation completion 到实例 actor 的有界桥（PP-M2 spike）。
//!
//! 桥只接收 `floatile-services` 已产生的唯一终态，按 `plugin + instance + generation`
//! 过滤后非阻塞投递。旧 generation、队列满或 actor 关闭都会立即丢弃宿主暂存结果。

use floatile_core::{OperationCompletion, OperationCompletionDisposition, OperationOwner};
use floatile_services::Broker;
use tokio::sync::mpsc;

pub use floatile_core::OperationCompletionDisposition as OperationDelivery;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperationBridgeError {
    #[error("operation completion queue capacity must be non-zero")]
    InvalidCapacity,
}

pub struct OperationCompletionBridge {
    owner: OperationOwner,
    sender: mpsc::Sender<OperationCompletion>,
}

pub struct RuntimeOperationEvents {
    receiver: mpsc::Receiver<OperationCompletion>,
}

impl OperationCompletionBridge {
    pub fn new(
        owner: OperationOwner,
        capacity: usize,
    ) -> Result<(Self, RuntimeOperationEvents), OperationBridgeError> {
        if capacity == 0 {
            return Err(OperationBridgeError::InvalidCapacity);
        }
        let (sender, receiver) = mpsc::channel(capacity);
        Ok((Self { owner, sender }, RuntimeOperationEvents { receiver }))
    }

    /// 仅执行短时身份比较、审计和 `try_send`；可安全用于 runtime actor 的 completion 分支。
    pub fn try_route(
        &self,
        source_broker: &Broker,
        completion: OperationCompletion,
    ) -> OperationDelivery {
        if !completion.is_current_for(&self.owner) {
            let _ = source_broker.audit_operation_completion(
                &completion,
                OperationCompletionDisposition::StaleGeneration,
            );
            source_broker.discard_operation_result(completion.id);
            return OperationDelivery::StaleGeneration;
        }
        let audit_completion = completion.clone();
        match self.sender.try_send(completion) {
            Ok(()) => {
                let _ = source_broker.audit_operation_completion(
                    &audit_completion,
                    OperationCompletionDisposition::Delivered,
                );
                OperationDelivery::Delivered
            }
            Err(mpsc::error::TrySendError::Full(completion)) => {
                let _ = source_broker.audit_operation_completion(
                    &completion,
                    OperationCompletionDisposition::QueueFull,
                );
                source_broker.discard_operation_result(completion.id);
                OperationDelivery::QueueFull
            }
            Err(mpsc::error::TrySendError::Closed(completion)) => {
                let _ = source_broker.audit_operation_completion(
                    &completion,
                    OperationCompletionDisposition::ActorClosed,
                );
                source_broker.discard_operation_result(completion.id);
                OperationDelivery::ActorClosed
            }
        }
    }
}

impl RuntimeOperationEvents {
    pub async fn recv(&mut self) -> Option<OperationCompletion> {
        self.receiver.recv().await
    }

    pub fn try_recv(&mut self) -> Result<OperationCompletion, mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}
