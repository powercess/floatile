//! 宿主托管异步 Operation 的有界执行与 typed result registry（PP-M2 spike）。
//!
//! `OperationRegistry` 只向同 crate 的 `Broker` 暴露提交/取消/领取原语，避免出现绕过
//! PermissionBroker 的公开执行入口。结果在宿主内以 Rust 类型暂存；正式 WIT adapter 必须为每个
//! capability 提供 typed `take-result`，不得把这里的类型擦除实现泄漏为通用 JSON-RPC。

use std::any::Any;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use floatile_core::{
    CapabilityId, DenyReason, OperationCompletion, OperationFailure, OperationId, OperationOwner,
    OperationTerminal,
};
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::time::Instant;

static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

type ErasedValue = Box<dyn Any + Send>;
type ErasedFuture =
    Pin<Box<dyn Future<Output = Result<ErasedValue, OperationFailure>> + Send + 'static>>;

/// 每实例 Operation 预算。所有容量必须非零，deadline 不得超过 `max_timeout`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationLimits {
    pub queue_capacity: usize,
    pub completion_capacity: usize,
    pub max_in_flight: usize,
    pub max_retained_results: usize,
    pub max_timeout: Duration,
}

impl Default for OperationLimits {
    fn default() -> Self {
        Self {
            queue_capacity: 16,
            completion_capacity: 16,
            max_in_flight: 4,
            max_retained_results: 16,
            max_timeout: Duration::from_secs(30),
        }
    }
}

impl OperationLimits {
    fn validate(self) -> Result<Self, OperationServiceError> {
        if self.queue_capacity == 0
            || self.completion_capacity == 0
            || self.max_in_flight == 0
            || self.max_retained_results == 0
            || self.max_timeout.is_zero()
        {
            return Err(OperationServiceError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperationServiceError {
    #[error("operation limits must be non-zero")]
    InvalidLimits,
    #[error("operation service requires a Tokio runtime")]
    RuntimeUnavailable,
    #[error("operation registry owner does not match its broker instance")]
    OwnerMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperationSubmitError {
    #[error("operation permission denied: {0:?}")]
    PermissionDenied(DenyReason),
    #[error("operation queue is full")]
    QueueFull,
    #[error("operation deadline is invalid")]
    InvalidDeadline,
    #[error("operation service is unavailable")]
    Unavailable,
    #[error("operation id space is exhausted")]
    IdExhausted,
}

impl OperationSubmitError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::PermissionDenied(_) => "permission-denied",
            Self::QueueFull => "queue-full",
            Self::InvalidDeadline => "invalid-deadline",
            Self::Unavailable => "unavailable",
            Self::IdExhausted => "id-exhausted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperationCancelError {
    #[error("operation is not active for this capability")]
    NotActive,
    #[error("operation permission denied: {0:?}")]
    PermissionDenied(DenyReason),
    #[error("operation service is unavailable")]
    Unavailable,
}

impl OperationCancelError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotActive => "not-active",
            Self::PermissionDenied(_) => "permission-denied",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperationTakeError {
    #[error("operation result is not available")]
    NotAvailable,
    #[error("operation result belongs to another capability")]
    CapabilityMismatch,
    #[error("operation result type does not match its capability adapter")]
    TypeMismatch,
    #[error("operation permission denied: {0:?}")]
    PermissionDenied(DenyReason),
    #[error("operation service is unavailable")]
    Unavailable,
}

impl OperationTakeError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotAvailable => "not-available",
            Self::CapabilityMismatch => "capability-mismatch",
            Self::TypeMismatch => "type-mismatch",
            Self::PermissionDenied(_) => "permission-denied",
            Self::Unavailable => "unavailable",
        }
    }
}

struct Cancellation {
    cancelled: AtomicBool,
    notify: Notify,
}

impl Cancellation {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn cancelled(&self) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        let notified = self.notify.notified();
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

struct ActiveOperation {
    capability: CapabilityId,
    cancellation: Arc<Cancellation>,
}

struct StoredResult {
    capability: CapabilityId,
    value: ErasedValue,
}

struct SharedState {
    active: Mutex<BTreeMap<OperationId, ActiveOperation>>,
    results: Mutex<BTreeMap<OperationId, StoredResult>>,
}

impl SharedState {
    fn new() -> Self {
        Self {
            active: Mutex::new(BTreeMap::new()),
            results: Mutex::new(BTreeMap::new()),
        }
    }
}

struct OperationJob {
    id: OperationId,
    capability: CapabilityId,
    deadline: Instant,
    cancellation: Arc<Cancellation>,
    work: ErasedFuture,
}

/// 克隆后可由单实例 Broker 持有的 Operation registry。
#[derive(Clone)]
pub struct OperationRegistry {
    owner: OperationOwner,
    limits: OperationLimits,
    submit: mpsc::Sender<OperationJob>,
    shared: Arc<SharedState>,
}

impl std::fmt::Debug for OperationRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperationRegistry")
            .field("owner", &self.owner)
            .field("limits", &self.limits)
            .field("active", &self.active_len())
            .field("retained_results", &self.retained_result_len())
            .finish()
    }
}

/// runtime actor 消费的唯一终态接收端。
pub struct OperationCompletionReceiver {
    receiver: mpsc::Receiver<OperationCompletion>,
}

impl OperationCompletionReceiver {
    pub async fn recv(&mut self) -> Option<OperationCompletion> {
        self.receiver.recv().await
    }

    pub fn try_recv(&mut self) -> Result<OperationCompletion, mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl OperationRegistry {
    pub fn new(
        owner: OperationOwner,
        limits: OperationLimits,
    ) -> Result<(Self, OperationCompletionReceiver), OperationServiceError> {
        let limits = limits.validate()?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| OperationServiceError::RuntimeUnavailable)?;
        let (submit, jobs) = mpsc::channel(limits.queue_capacity);
        let (completion_tx, completion_rx) = mpsc::channel(limits.completion_capacity);
        let shared = Arc::new(SharedState::new());
        runtime.spawn(dispatch_operations(
            owner.clone(),
            limits,
            Arc::clone(&shared),
            jobs,
            completion_tx,
        ));
        Ok((
            Self {
                owner,
                limits,
                submit,
                shared,
            },
            OperationCompletionReceiver {
                receiver: completion_rx,
            },
        ))
    }

    pub fn owner(&self) -> &OperationOwner {
        &self.owner
    }

    pub fn active_len(&self) -> usize {
        lock(&self.shared.active).len()
    }

    pub fn retained_result_len(&self) -> usize {
        lock(&self.shared.results).len()
    }

    pub fn discard_result(&self, id: OperationId) -> bool {
        lock(&self.shared.results).remove(&id).is_some()
    }

    pub fn cancel_all(&self) -> usize {
        let active = lock(&self.shared.active);
        for operation in active.values() {
            operation.cancellation.cancel();
        }
        active.len()
    }

    pub(crate) fn submit<T, F>(
        &self,
        capability: CapabilityId,
        timeout: Duration,
        work: F,
    ) -> Result<OperationId, OperationSubmitError>
    where
        T: Any + Send + 'static,
        F: Future<Output = Result<T, OperationFailure>> + Send + 'static,
    {
        if timeout.is_zero() || timeout > self.limits.max_timeout {
            return Err(OperationSubmitError::InvalidDeadline);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(OperationSubmitError::InvalidDeadline)?;
        let id = next_operation_id()?;
        let cancellation = Arc::new(Cancellation::new());
        lock(&self.shared.active).insert(
            id,
            ActiveOperation {
                capability,
                cancellation: Arc::clone(&cancellation),
            },
        );
        let work = Box::pin(async move {
            work.await
                .map(|value| Box::new(value) as Box<dyn Any + Send>)
        });
        let job = OperationJob {
            id,
            capability,
            deadline,
            cancellation,
            work,
        };
        match self.submit.try_send(job) {
            Ok(()) => Ok(id),
            Err(mpsc::error::TrySendError::Full(_)) => {
                lock(&self.shared.active).remove(&id);
                Err(OperationSubmitError::QueueFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                lock(&self.shared.active).remove(&id);
                Err(OperationSubmitError::Unavailable)
            }
        }
    }

    pub(crate) fn cancel(
        &self,
        capability: CapabilityId,
        id: OperationId,
    ) -> Result<(), OperationCancelError> {
        let active = lock(&self.shared.active);
        let Some(operation) = active.get(&id) else {
            return Err(OperationCancelError::NotActive);
        };
        if operation.capability != capability {
            return Err(OperationCancelError::NotActive);
        }
        operation.cancellation.cancel();
        Ok(())
    }

    pub(crate) fn take<T: Any + Send + 'static>(
        &self,
        capability: CapabilityId,
        id: OperationId,
    ) -> Result<T, OperationTakeError> {
        let mut results = lock(&self.shared.results);
        let Some(result) = results.remove(&id) else {
            return Err(OperationTakeError::NotAvailable);
        };
        if result.capability != capability {
            results.insert(id, result);
            return Err(OperationTakeError::CapabilityMismatch);
        }
        match result.value.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(value) => {
                results.insert(
                    id,
                    StoredResult {
                        capability: result.capability,
                        value,
                    },
                );
                Err(OperationTakeError::TypeMismatch)
            }
        }
    }
}

async fn dispatch_operations(
    owner: OperationOwner,
    limits: OperationLimits,
    shared: Arc<SharedState>,
    mut jobs: mpsc::Receiver<OperationJob>,
    completions: mpsc::Sender<OperationCompletion>,
) {
    let permits = Arc::new(Semaphore::new(limits.max_in_flight));
    loop {
        let permit = match Arc::clone(&permits).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => break,
        };
        let Some(job) = jobs.recv().await else {
            break;
        };
        let job_owner = owner.clone();
        let job_shared = Arc::clone(&shared);
        let job_completions = completions.clone();
        tokio::spawn(async move {
            let _permit = permit;
            run_operation(
                job_owner,
                limits.max_retained_results,
                job_shared,
                job_completions,
                job,
            )
            .await;
        });
    }
}

async fn run_operation(
    owner: OperationOwner,
    max_retained_results: usize,
    shared: Arc<SharedState>,
    completions: mpsc::Sender<OperationCompletion>,
    job: OperationJob,
) {
    let result = if Instant::now() >= job.deadline {
        Err(OperationFailure::Timeout)
    } else {
        tokio::select! {
            biased;
            () = job.cancellation.cancelled() => Err(OperationFailure::Cancelled),
            timed = tokio::time::timeout_at(job.deadline, job.work) => {
                match timed {
                    Ok(result) => result,
                    Err(_) => Err(OperationFailure::Timeout),
                }
            }
        }
    };
    let terminal = match result {
        Ok(value) => {
            let mut results = lock(&shared.results);
            if results.len() >= max_retained_results {
                OperationTerminal::Failed(OperationFailure::ResultDropped)
            } else {
                results.insert(
                    job.id,
                    StoredResult {
                        capability: job.capability,
                        value,
                    },
                );
                OperationTerminal::Succeeded
            }
        }
        Err(error) => OperationTerminal::Failed(error),
    };
    lock(&shared.active).remove(&job.id);
    let completion = OperationCompletion {
        id: job.id,
        owner,
        capability: job.capability,
        terminal,
    };
    if completions.send(completion).await.is_err() {
        lock(&shared.results).remove(&job.id);
    }
}

fn next_operation_id() -> Result<OperationId, OperationSubmitError> {
    let value = NEXT_OPERATION_ID
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current != u64::MAX).then_some(current + 1)
        })
        .map_err(|_| OperationSubmitError::IdExhausted)?;
    OperationId::new(value).ok_or(OperationSubmitError::IdExhausted)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
