//! 持久插件实例的动态监督器（PP-M1）。
//!
//! SQLite 和安装目录 I/O 全部在专用后台线程执行；Slint 主线程只从
//! 有界 channel 接收已备好的启停动作。记录的 `desired_state` 是持久意图，窗口
//! 会话是 observed runtime 状态，不反向写入数据库。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use floatile_core::{InstanceDesiredState, InstanceId, PluginInstance};
use floatile_platform::PlatformCapabilities;
use floatile_services::AuditListener;
use slint::{Timer, TimerMode};

use crate::plugin_manager::{RunnableInstance, load_runnable_instance};
use crate::runtime_ui::{RuntimeUiLifecycleEvent, RuntimeUiSession, spawn_runtime_ui};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const UI_DRAIN_INTERVAL: Duration = Duration::from_millis(50);
const ACTION_QUEUE_CAPACITY: usize = 16;
const COMMAND_QUEUE_CAPACITY: usize = 16;
const UI_BATCH_LIMIT: usize = 8;

/// 实例的宿主观测状态。它只存在于当前 shell 进程，不写回 desired-state 数据库。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedInstanceState {
    Starting,
    Running,
    Failed,
    Stopped,
}

impl ObservedInstanceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// 控制面可读取的脱敏 observed 状态；错误只暴露稳定 code，不携带 Config 等值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedInstanceStatus {
    pub instance_id: InstanceId,
    pub state: ObservedInstanceState,
    pub code: Option<&'static str>,
}

#[derive(Debug)]
enum SupervisorCommand {
    Retry(InstanceId),
    Stop,
}

/// Slint 控制面使用的非阻塞 supervisor 句柄。
#[derive(Clone)]
pub struct InstanceSupervisorHandle {
    commands: SyncSender<SupervisorCommand>,
    observed: Rc<RefCell<BTreeMap<u64, ObservedInstanceStatus>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SupervisorCommandError {
    #[error("supervisor command queue is full")]
    QueueFull,
    #[error("supervisor is unavailable")]
    Closed,
}

impl InstanceSupervisorHandle {
    /// 清除某实例已隔离的 fingerprint，让后台 worker 用最新持久快照再尝试一次。
    pub fn retry(&self, instance_id: InstanceId) -> Result<(), SupervisorCommandError> {
        self.commands
            .try_send(SupervisorCommand::Retry(instance_id))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => SupervisorCommandError::QueueFull,
                mpsc::TrySendError::Disconnected(_) => SupervisorCommandError::Closed,
            })
    }

    pub fn observed_snapshot(&self) -> Vec<ObservedInstanceStatus> {
        self.observed.borrow().values().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstanceFingerprint {
    plugin_id: String,
    version: String,
    digest: String,
    config: serde_json::Value,
}

impl From<&PluginInstance> for InstanceFingerprint {
    fn from(instance: &PluginInstance) -> Self {
        Self {
            plugin_id: instance.installation().plugin().0.clone(),
            version: instance.installation().version().to_owned(),
            digest: instance.installation().digest().to_string(),
            config: instance.config().to_value(),
        }
    }
}

#[derive(Debug)]
enum DesiredAction {
    Stop(InstanceId),
    Start(PluginInstance),
}

enum SupervisorAction {
    Stop(InstanceId),
    Start(Box<RunnableInstance>),
    Failure {
        instance_id: Option<InstanceId>,
        plugin_id: Option<String>,
        code: &'static str,
        detail: String,
    },
}

/// 对快照做幂等 reconcile。`known` 同时表示已启动或已隔离的 fingerprint，
/// 因此安装缺失/篡改不会每 500ms 重试和刷屏；记录变更后才再试。
fn plan_snapshot(
    instances: &[PluginInstance],
    known: &BTreeMap<u64, InstanceFingerprint>,
) -> (Vec<DesiredAction>, BTreeMap<u64, InstanceFingerprint>) {
    let mut actions = Vec::new();
    let mut next = BTreeMap::new();
    let snapshot_ids: BTreeSet<u64> = instances.iter().map(|instance| instance.id().0).collect();

    for id in known.keys().filter(|id| !snapshot_ids.contains(id)) {
        actions.push(DesiredAction::Stop(InstanceId(*id)));
    }
    for instance in instances {
        let id = instance.id().0;
        if instance.desired_state() == InstanceDesiredState::Stopped {
            if known.contains_key(&id) {
                actions.push(DesiredAction::Stop(InstanceId(id)));
            }
            continue;
        }
        let fingerprint = InstanceFingerprint::from(instance);
        if known.get(&id) != Some(&fingerprint) {
            if known.contains_key(&id) {
                actions.push(DesiredAction::Stop(InstanceId(id)));
            }
            actions.push(DesiredAction::Start(instance.clone()));
        }
        next.insert(id, fingerprint);
    }
    (actions, next)
}

/// 持有动态实例窗口、Slint 排程器和后台 reconcile worker。
pub struct DynamicInstanceSupervisor {
    timer: Timer,
    sessions: Rc<RefCell<BTreeMap<u64, RuntimeUiSession>>>,
    desired_generations: Rc<RefCell<BTreeMap<u64, u64>>>,
    tasks: Rc<RefCell<Vec<slint::JoinHandle<()>>>>,
    observed: Rc<RefCell<BTreeMap<u64, ObservedInstanceStatus>>>,
    commands: SyncSender<SupervisorCommand>,
    worker: Option<thread::JoinHandle<()>>,
}

impl DynamicInstanceSupervisor {
    pub fn start(
        database: PathBuf,
        plugin_store: PathBuf,
        caps: PlatformCapabilities,
        audit_listener: Option<AuditListener>,
    ) -> Result<Self, std::io::Error> {
        let (action_tx, action_rx) = mpsc::sync_channel(ACTION_QUEUE_CAPACITY);
        let (commands, command_rx) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name("floatile-instance-supervisor".to_owned())
            .spawn(move || supervisor_worker(database, plugin_store, action_tx, command_rx))?;

        let sessions = Rc::new(RefCell::new(BTreeMap::new()));
        let desired_generations = Rc::new(RefCell::new(BTreeMap::new()));
        let observed = Rc::new(RefCell::new(BTreeMap::new()));
        let tasks: Rc<RefCell<Vec<slint::JoinHandle<()>>>> = Rc::new(RefCell::new(Vec::new()));
        let timer = Timer::default();
        let timer_sessions = Rc::clone(&sessions);
        let timer_desired = Rc::clone(&desired_generations);
        let timer_tasks = Rc::clone(&tasks);
        let timer_observed = Rc::clone(&observed);
        timer.start(TimerMode::Repeated, UI_DRAIN_INTERVAL, move || {
            timer_tasks.borrow_mut().retain(|task| !task.is_finished());
            for _ in 0..UI_BATCH_LIMIT {
                let actions = match action_rx.try_recv() {
                    Ok(actions) => actions,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break,
                };
                apply_actions(
                    actions,
                    caps,
                    audit_listener.clone(),
                    &timer_sessions,
                    &timer_desired,
                    &timer_tasks,
                    &timer_observed,
                );
            }
            poll_session_lifecycle(&timer_sessions, &timer_observed);
        });

        Ok(Self {
            timer,
            sessions,
            desired_generations,
            tasks,
            observed,
            commands,
            worker: Some(worker),
        })
    }

    pub fn handle(&self) -> InstanceSupervisorHandle {
        InstanceSupervisorHandle {
            commands: self.commands.clone(),
            observed: Rc::clone(&self.observed),
        }
    }
}

impl Drop for DynamicInstanceSupervisor {
    fn drop(&mut self) {
        self.timer.stop();
        self.desired_generations.borrow_mut().clear();
        self.observed.borrow_mut().clear();
        self.sessions.borrow_mut().clear();
        for task in self.tasks.borrow_mut().drain(..) {
            if !task.is_finished() {
                task.abort();
            }
        }
        let _ = self.commands.try_send(SupervisorCommand::Stop);
        let Some(worker) = self.worker.take() else {
            return;
        };
        if let Err(error) = thread::Builder::new()
            .name("floatile-instance-supervisor-reaper".to_owned())
            .spawn(move || {
                let _ = worker.join();
            })
        {
            tracing::warn!(%error, "failed to spawn instance supervisor reaper");
        }
    }
}

fn supervisor_worker(
    database: PathBuf,
    plugin_store: PathBuf,
    action_tx: SyncSender<Vec<SupervisorAction>>,
    command_rx: Receiver<SupervisorCommand>,
) {
    let store = match floatile_store::open(&database) {
        Ok(store) => store,
        Err(error) => {
            let _ = action_tx.send(vec![SupervisorAction::Failure {
                instance_id: None,
                plugin_id: None,
                code: "FINSTANCE_STORE",
                detail: error.to_string(),
            }]);
            return;
        }
    };
    let mut known = BTreeMap::new();
    loop {
        let snapshot = match store.instances().list() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if action_tx
                    .send(vec![SupervisorAction::Failure {
                        instance_id: None,
                        plugin_id: None,
                        code: "FINSTANCE_STORE",
                        detail: error.to_string(),
                    }])
                    .is_err()
                {
                    break;
                }
                if wait_for_command(&command_rx, &mut known) {
                    break;
                }
                continue;
            }
        };
        let (desired, next_known) = plan_snapshot(&snapshot, &known);
        if !desired.is_empty() {
            let actions = prepare_actions(&store, &plugin_store, desired);
            if action_tx.send(actions).is_err() {
                break;
            }
        }
        known = next_known;
        if wait_for_command(&command_rx, &mut known) {
            break;
        }
    }
}

/// 等待下一轮 reconcile，并消费一个控制命令。返回 true 表示 worker 应停止。
fn wait_for_command(
    command_rx: &Receiver<SupervisorCommand>,
    known: &mut BTreeMap<u64, InstanceFingerprint>,
) -> bool {
    match command_rx.recv_timeout(POLL_INTERVAL) {
        Ok(SupervisorCommand::Retry(id)) => {
            known.remove(&id.0);
            false
        }
        Ok(SupervisorCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
        Err(mpsc::RecvTimeoutError::Timeout) => false,
    }
}

fn prepare_actions(
    store: &floatile_store::Store,
    plugin_store: &std::path::Path,
    desired: Vec<DesiredAction>,
) -> Vec<SupervisorAction> {
    let mut prepared = Vec::with_capacity(desired.len());
    for action in desired {
        match action {
            DesiredAction::Stop(id) => prepared.push(SupervisorAction::Stop(id)),
            DesiredAction::Start(instance) => {
                let id = instance.id();
                let plugin_id = instance.installation().plugin().0.clone();
                let updated_at = unix_now().max(instance.updated_at());
                let advanced = match store.instances().advance_generation(id, updated_at) {
                    Ok(Some(_)) => store.instances().get(id),
                    Ok(None) => {
                        prepared.push(SupervisorAction::Failure {
                            instance_id: Some(id),
                            plugin_id: Some(plugin_id),
                            code: "FINSTANCE_GENERATION",
                            detail: "实例不存在、时间戳过期或 generation 已耗尽".to_owned(),
                        });
                        continue;
                    }
                    Err(error) => Err(error),
                };
                let instance = match advanced {
                    Ok(Some(instance)) => instance,
                    Ok(None) => {
                        prepared.push(SupervisorAction::Failure {
                            instance_id: Some(id),
                            plugin_id: Some(plugin_id),
                            code: "FINSTANCE_MISSING",
                            detail: "推进 generation 后实例记录消失".to_owned(),
                        });
                        continue;
                    }
                    Err(error) => {
                        prepared.push(SupervisorAction::Failure {
                            instance_id: Some(id),
                            plugin_id: Some(plugin_id),
                            code: "FINSTANCE_STORE",
                            detail: error.to_string(),
                        });
                        continue;
                    }
                };
                let version = instance.installation().version().to_owned();
                match load_runnable_instance(plugin_store, instance) {
                    Ok(Some(runnable)) => {
                        prepared.push(SupervisorAction::Start(Box::new(runnable)))
                    }
                    Ok(None) => prepared.push(SupervisorAction::Failure {
                        instance_id: Some(id),
                        plugin_id: Some(plugin_id),
                        code: "FLOAD_INSTALLATION_MISSING",
                        detail: format!("安装版本 {version} 不存在"),
                    }),
                    Err(error) => prepared.push(SupervisorAction::Failure {
                        instance_id: Some(id),
                        plugin_id: Some(plugin_id),
                        code: error.code(),
                        detail: error.to_string(),
                    }),
                }
            }
        }
    }
    prepared
}

fn apply_actions(
    actions: Vec<SupervisorAction>,
    caps: PlatformCapabilities,
    audit_listener: Option<AuditListener>,
    sessions: &Rc<RefCell<BTreeMap<u64, RuntimeUiSession>>>,
    desired_generations: &Rc<RefCell<BTreeMap<u64, u64>>>,
    tasks: &Rc<RefCell<Vec<slint::JoinHandle<()>>>>,
    observed: &Rc<RefCell<BTreeMap<u64, ObservedInstanceStatus>>>,
) {
    for action in actions {
        match action {
            SupervisorAction::Stop(id) => {
                desired_generations.borrow_mut().remove(&id.0);
                sessions.borrow_mut().remove(&id.0);
                set_observed(observed, id, ObservedInstanceState::Stopped, None);
                tracing::info!(instance_id = id.0, "persistent plugin instance stopped");
            }
            SupervisorAction::Failure {
                instance_id,
                plugin_id,
                code,
                detail,
            } => {
                if let Some(id) = instance_id {
                    desired_generations.borrow_mut().remove(&id.0);
                    sessions.borrow_mut().remove(&id.0);
                    set_observed(observed, id, ObservedInstanceState::Failed, Some(code));
                }
                tracing::warn!(
                    instance_id = instance_id.map(|id| id.0),
                    plugin_id,
                    code,
                    detail,
                    "persistent plugin instance reconcile failed (isolated; host continues)"
                );
            }
            SupervisorAction::Start(runnable) => {
                let id = runnable.instance.id();
                let generation = runnable.instance.generation();
                let plugin_id = runnable.plugin.manifest.id.0.clone();
                desired_generations.borrow_mut().insert(id.0, generation);
                set_observed(observed, id, ObservedInstanceState::Starting, None);
                let task_sessions = Rc::clone(sessions);
                let task_desired = Rc::clone(desired_generations);
                let task_observed = Rc::clone(observed);
                let task_audit = audit_listener.clone();
                match slint::spawn_local(async move {
                    match spawn_runtime_ui(runnable.plugin, runnable.instance, caps, task_audit)
                        .await
                    {
                        Ok(session) => {
                            if task_desired.borrow().get(&id.0) == Some(&generation) {
                                task_sessions.borrow_mut().insert(id.0, session);
                                tracing::info!(
                                    instance_id = id.0,
                                    plugin_id = %plugin_id,
                                    generation,
                                    "persistent plugin instance started"
                                );
                            }
                        }
                        Err(error) => {
                            if task_desired.borrow().get(&id.0) == Some(&generation) {
                                set_observed(
                                    &task_observed,
                                    id,
                                    ObservedInstanceState::Failed,
                                    Some(error.code()),
                                );
                            }
                            tracing::warn!(
                                instance_id = id.0,
                                plugin_id = %plugin_id,
                                generation,
                                code = error.code(),
                                %error,
                                "persistent plugin instance failed to start (isolated; host continues)"
                            );
                        }
                    }
                }) {
                    Ok(task) => tasks.borrow_mut().push(task),
                    Err(error) => {
                        set_observed(
                            observed,
                            id,
                            ObservedInstanceState::Failed,
                            Some("FINSTANCE_SCHEDULE"),
                        );
                        tracing::warn!(
                            instance_id = id.0,
                            %error,
                            "failed to schedule persistent plugin instance launch"
                        );
                    }
                }
            }
        }
    }
}

fn set_observed(
    observed: &Rc<RefCell<BTreeMap<u64, ObservedInstanceStatus>>>,
    instance_id: InstanceId,
    state: ObservedInstanceState,
    code: Option<&'static str>,
) {
    observed.borrow_mut().insert(
        instance_id.0,
        ObservedInstanceStatus {
            instance_id,
            state,
            code,
        },
    );
}

fn poll_session_lifecycle(
    sessions: &Rc<RefCell<BTreeMap<u64, RuntimeUiSession>>>,
    observed: &Rc<RefCell<BTreeMap<u64, ObservedInstanceStatus>>>,
) {
    let events: Vec<(u64, RuntimeUiLifecycleEvent)> = sessions
        .borrow()
        .iter()
        .filter_map(|(id, session)| session.try_lifecycle_event().map(|event| (*id, event)))
        .collect();
    for (id, event) in events {
        match event {
            RuntimeUiLifecycleEvent::Running => set_observed(
                observed,
                InstanceId(id),
                ObservedInstanceState::Running,
                None,
            ),
            RuntimeUiLifecycleEvent::Failed { code, detail } => {
                set_observed(
                    observed,
                    InstanceId(id),
                    ObservedInstanceState::Failed,
                    Some(code),
                );
                sessions.borrow_mut().remove(&id);
                tracing::warn!(
                    instance_id = id,
                    code,
                    detail,
                    "persistent plugin runtime exited (isolated; host continues)"
                );
            }
            RuntimeUiLifecycleEvent::Stopped => {
                set_observed(
                    observed,
                    InstanceId(id),
                    ObservedInstanceState::Stopped,
                    None,
                );
                sessions.borrow_mut().remove(&id);
            }
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use floatile_core::install::InstallMeta;
    use floatile_core::{InstallationDigest, InstallationRef, InstanceConfig, PluginInstance};

    use super::*;

    fn instance(
        id: u64,
        desired: InstanceDesiredState,
        config: serde_json::Value,
    ) -> PluginInstance {
        let reference = InstallationRef::from_install_meta(&InstallMeta {
            manifest_version: 1,
            id: "dev.floatile.clock".to_owned(),
            version: "1.0.0".to_owned(),
            engine_api_version: "0.1.0".to_owned(),
            ui_api_version: "0.1.0".to_owned(),
            digest: InstallationDigest::from_bytes([id as u8; 32]).to_string(),
            source: "test.floatile".to_owned(),
            installed_at: 1,
            files: Default::default(),
        })
        .unwrap();
        PluginInstance::restore(
            InstanceId(id),
            reference,
            InstanceConfig::new(config).unwrap(),
            desired,
            0,
            1,
            1,
        )
        .unwrap()
    }

    #[test]
    fn reconcile_is_independent_idempotent_and_handles_stop_delete_and_config_change() {
        let first = instance(
            1,
            InstanceDesiredState::Running,
            serde_json::json!({"zone": "UTC"}),
        );
        let second = instance(
            2,
            InstanceDesiredState::Running,
            serde_json::json!({"zone": "CET"}),
        );
        let (initial, known) = plan_snapshot(&[first.clone(), second.clone()], &BTreeMap::new());
        assert_eq!(initial.len(), 2);
        assert!(matches!(initial[0], DesiredAction::Start(_)));
        assert!(matches!(initial[1], DesiredAction::Start(_)));

        let (unchanged, known) = plan_snapshot(&[first.clone(), second.clone()], &known);
        assert!(unchanged.is_empty());

        let changed = instance(
            1,
            InstanceDesiredState::Running,
            serde_json::json!({"zone": "EST"}),
        );
        let stopped = instance(
            2,
            InstanceDesiredState::Stopped,
            serde_json::json!({"zone": "CET"}),
        );
        let (updated, known) = plan_snapshot(&[changed, stopped], &known);
        assert_eq!(updated.len(), 3);
        assert!(matches!(updated[0], DesiredAction::Stop(InstanceId(1))));
        assert!(matches!(updated[1], DesiredAction::Start(_)));
        assert!(matches!(updated[2], DesiredAction::Stop(InstanceId(2))));

        let (deleted, known) = plan_snapshot(&[], &known);
        assert_eq!(deleted.len(), 1);
        assert!(matches!(deleted[0], DesiredAction::Stop(InstanceId(1))));
        assert!(known.is_empty());
    }

    #[test]
    fn manual_retry_forgets_only_the_selected_fingerprint() {
        let first = instance(
            1,
            InstanceDesiredState::Running,
            serde_json::json!({"zone": "UTC"}),
        );
        let second = instance(
            2,
            InstanceDesiredState::Running,
            serde_json::json!({"zone": "CET"}),
        );
        let (_, mut known) = plan_snapshot(&[first, second], &BTreeMap::new());
        let (commands, command_rx) = mpsc::sync_channel(1);
        commands
            .try_send(SupervisorCommand::Retry(InstanceId(1)))
            .unwrap();

        assert!(!wait_for_command(&command_rx, &mut known));
        assert!(!known.contains_key(&1));
        assert!(known.contains_key(&2));
    }

    #[test]
    fn observed_state_names_are_stable_control_contract() {
        assert_eq!(ObservedInstanceState::Starting.as_str(), "starting");
        assert_eq!(ObservedInstanceState::Running.as_str(), "running");
        assert_eq!(ObservedInstanceState::Failed.as_str(), "failed");
        assert_eq!(ObservedInstanceState::Stopped.as_str(), "stopped");
    }
}
