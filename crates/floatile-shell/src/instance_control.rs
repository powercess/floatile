//! 插件安装与实例控制面（PP-M1）。
//!
//! Slint 线程只读取已准备的快照、编辑有界字符串并 `try_send` 命令；SQLite、安装目录、
//! JSON 解析和 Config Schema 求值都在专用 worker 完成。observed lifecycle 来自
//! `DynamicInstanceSupervisor`，不把运行结果写回 desired-state 持久化记录。

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use floatile_core::{InstanceConfig, InstanceDesiredState, InstanceId, PluginInstance};
use floatile_store::installation::{
    InstalledInstallation, list_highest, load_exact, load_reference,
};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::instance_supervisor::{InstanceSupervisorHandle, ObservedInstanceState};

const CONTROL_QUEUE_CAPACITY: usize = 16;
const SNAPSHOT_QUEUE_CAPACITY: usize = 2;
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const UI_DRAIN_INTERVAL: Duration = Duration::from_millis(100);

slint::slint! {
    import { Button, CheckBox, LineEdit, ScrollView } from "std-widgets.slint";

    export struct InstallationListItem {
        title: string,
        subtitle: string,
    }

    export struct InstanceListItem {
        title: string,
        subtitle: string,
        status: string,
        error-code: string,
    }

    export struct ConfigFieldItem {
        key: string,
        label: string,
        value: string,
        kind: string,
        required: bool,
        present: bool,
    }

    component SectionTitle inherits Text {
        color: #dbe4f3;
        font-size: 15px;
        font-weight: 700;
    }

    export component PluginControlWindow inherits Window {
        title: "Floatile 插件与实例";
        width: 900px;
        height: 620px;
        background: #151922;
        no-frame: true;

        in property <[InstallationListItem]> installations;
        in property <[InstanceListItem]> instances;
        in property <[ConfigFieldItem]> config-fields;
        in property <string> selection-title: "请选择插件或实例";
        in property <string> selection-subtitle: "";
        in property <string> notice: "";
        in property <bool> notice-ok: false;
        in property <bool> selected-instance: false;
        in property <bool> selected-installation: false;
        in property <bool> can-start: false;
        in property <bool> can-stop: false;
        in property <bool> can-retry: false;
        in property <bool> can-configure: false;

        callback select-installation(int);
        callback select-instance(int);
        callback field-edited(int, string, bool);
        callback create-instance;
        callback save-config;
        callback start-instance;
        callback stop-instance;
        callback retry-instance;
        callback delete-instance;

        Rectangle {
            x: 0px; y: 0px; width: parent.width; height: 46px;
            background: #202735;
            Text { x: 18px; y: 12px; text: "Floatile 插件与实例"; color: white; font-size: 17px; font-weight: 700; }
            close := TouchArea { x: parent.width - 46px; width: 46px; height: 46px; clicked => { root.hide(); } }
            Text { x: parent.width - 31px; y: 11px; text: "×"; color: #bdc8da; font-size: 20px; }
        }

        Rectangle {
            x: 0px; y: 46px; width: 310px; height: parent.height - 46px;
            background: #1a202b;
            SectionTitle { x: 16px; y: 14px; text: "已安装插件"; }
            ScrollView {
                x: 10px; y: 42px; width: 290px; height: 160px;
                viewport-height: Math.max(160px, root.installations.length * 56px);
                for item[index] in root.installations: Rectangle {
                    y: index * 56px; width: 278px; height: 52px;
                    background: install-touch.has-hover ? #2a3445 : #222a38;
                    border-radius: 6px;
                    install-touch := TouchArea { clicked => { root.select-installation(index); } }
                    Text { x: 10px; y: 7px; width: 258px; text: item.title; color: #e8eef8; font-size: 13px; overflow: elide; }
                    Text { x: 10px; y: 28px; width: 258px; text: item.subtitle; color: #8fa0ba; font-size: 11px; overflow: elide; }
                }
            }
            SectionTitle { x: 16px; y: 218px; text: "插件实例"; }
            ScrollView {
                x: 10px; y: 246px; width: 290px; height: parent.height - 256px;
                viewport-height: Math.max(200px, root.instances.length * 76px);
                for item[index] in root.instances: Rectangle {
                    y: index * 76px; width: 278px; height: 72px;
                    background: instance-touch.has-hover ? #2a3445 : #222a38;
                    border-radius: 6px;
                    instance-touch := TouchArea { clicked => { root.select-instance(index); } }
                    Text { x: 10px; y: 7px; width: 188px; text: item.title; color: #e8eef8; font-size: 13px; overflow: elide; }
                    Text { x: 202px; y: 7px; width: 66px; text: item.status; color: item.status == "failed" ? #ff8080 : item.status == "running" ? #65d899 : #a9b6ca; font-size: 11px; horizontal-alignment: right; }
                    Text { x: 10px; y: 29px; width: 258px; text: item.subtitle; color: #8fa0ba; font-size: 11px; overflow: elide; }
                    Text { x: 10px; y: 49px; width: 258px; text: item.error-code; color: #ff9a9a; font-size: 10px; overflow: elide; }
                }
            }
        }

        Rectangle {
            x: 310px; y: 46px; width: parent.width - 310px; height: parent.height - 46px;
            background: #151922;
            Text { x: 22px; y: 18px; width: parent.width - 44px; text: root.selection-title; color: #f0f4fa; font-size: 18px; font-weight: 700; overflow: elide; }
            Text { x: 22px; y: 48px; width: parent.width - 44px; text: root.selection-subtitle; color: #93a2b9; font-size: 12px; overflow: elide; }
            Text { x: 22px; y: 76px; width: parent.width - 44px; text: root.notice; color: root.notice-ok ? #65d899 : #ff9a9a; font-size: 11px; overflow: elide; }

            ScrollView {
                x: 18px; y: 106px; width: parent.width - 36px; height: parent.height - 174px;
                viewport-height: Math.max(250px, root.config-fields.length * 82px);
                for field[index] in root.config-fields: Rectangle {
                    y: index * 82px; width: parent.width - 12px; height: 76px;
                    background: #1d2430; border-radius: 6px;
                    present := CheckBox {
                        x: 8px; y: 8px; width: 22px; height: 22px;
                        checked: field.present;
                        enabled: !field.required;
                        toggled => { root.field-edited(index, editor.text, self.checked); }
                    }
                    Text { x: 36px; y: 8px; width: parent.width - 48px; text: field.label + (field.required ? " *" : ""); color: #dce5f3; font-size: 12px; overflow: elide; }
                    editor := LineEdit {
                        x: 36px; y: 32px; width: parent.width - 48px; height: 34px;
                        text: field.value;
                        enabled: field.present || field.required;
                        placeholder-text: field.kind;
                        edited(text) => { root.field-edited(index, text, true); }
                    }
                }
            }

            HorizontalLayout {
                x: 18px; y: parent.height - 58px; width: parent.width - 36px; height: 40px;
                spacing: 8px;
                if root.selected-installation: Button { text: "创建 stopped 实例"; clicked => { root.create-instance(); } }
                if root.selected-instance && root.can-configure: Button { text: "保存配置"; clicked => { root.save-config(); } }
                if root.selected-instance && root.can-start: Button { text: "启动"; clicked => { root.start-instance(); } }
                if root.selected-instance && root.can-stop: Button { text: "停止"; clicked => { root.stop-instance(); } }
                if root.selected-instance && root.can-retry: Button { text: "重试"; clicked => { root.retry-instance(); } }
                if root.selected-instance && root.can-configure: Button { text: "删除"; clicked => { root.delete-instance(); } }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct InstallationRecord {
    plugin_id: String,
    name: String,
    version: String,
    fields: Vec<ConfigField>,
}

#[derive(Debug, Clone)]
struct InstanceRecord {
    instance: PluginInstance,
    fields: Vec<ConfigField>,
}

#[derive(Debug, Clone, Default)]
struct ControlSnapshot {
    installations: Vec<InstallationRecord>,
    instances: Vec<InstanceRecord>,
    notice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFieldKind {
    String,
    Integer,
    Number,
    Boolean,
    Json,
    RootJson,
}

impl ConfigFieldKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean (true/false)",
            Self::Json => "JSON",
            Self::RootJson => "完整 JSON object",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ConfigField {
    key: String,
    label: String,
    kind: ConfigFieldKind,
    value: String,
    required: bool,
    present: bool,
}

#[derive(Debug)]
enum ControlCommand {
    Create {
        plugin_id: String,
        version: String,
        fields: Vec<ConfigField>,
    },
    Configure {
        instance_id: InstanceId,
        fields: Vec<ConfigField>,
    },
    SetDesired {
        instance_id: InstanceId,
        desired: InstanceDesiredState,
    },
    Delete(InstanceId),
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    None,
    Installation { plugin_id: String, version: String },
    Instance(InstanceId),
}

/// 持有插件管理窗口、后台 worker 与快照 timer。
pub struct InstanceControlSurface {
    window: PluginControlWindow,
    timer: Timer,
    commands: SyncSender<ControlCommand>,
    worker: Option<thread::JoinHandle<()>>,
    _snapshot: Rc<RefCell<ControlSnapshot>>,
    _fields: Rc<RefCell<Vec<ConfigField>>>,
    _selection: Rc<RefCell<Selection>>,
}

impl InstanceControlSurface {
    pub fn start(
        database: PathBuf,
        plugin_store: PathBuf,
        supervisor: InstanceSupervisorHandle,
    ) -> Result<Self, slint::PlatformError> {
        let window = PluginControlWindow::new()?;
        let (commands, command_rx) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        let (snapshot_tx, snapshot_rx) = mpsc::sync_channel(SNAPSHOT_QUEUE_CAPACITY);
        let worker_database = database.clone();
        let worker_plugin_store = plugin_store.clone();
        let worker = thread::Builder::new()
            .name("floatile-instance-control".to_owned())
            .spawn(move || {
                control_worker(
                    worker_database,
                    worker_plugin_store,
                    command_rx,
                    snapshot_tx,
                );
            })
            .map_err(|error| slint::PlatformError::Other(error.to_string()))?;

        let snapshot = Rc::new(RefCell::new(ControlSnapshot::default()));
        let fields = Rc::new(RefCell::new(Vec::new()));
        let selection = Rc::new(RefCell::new(Selection::None));
        wire_callbacks(
            &window,
            &commands,
            &supervisor,
            Rc::clone(&snapshot),
            Rc::clone(&fields),
            Rc::clone(&selection),
        );

        let timer = Timer::default();
        let weak = window.as_weak();
        let timer_snapshot = Rc::clone(&snapshot);
        let timer_fields = Rc::clone(&fields);
        let timer_selection = Rc::clone(&selection);
        timer.start(TimerMode::Repeated, UI_DRAIN_INTERVAL, move || {
            let mut newest = None;
            while let Ok(value) = snapshot_rx.try_recv() {
                newest = Some(value);
            }
            if let Some(value) = newest {
                *timer_snapshot.borrow_mut() = value;
                normalize_selection(&timer_snapshot, &timer_selection);
            }
            if let Some(window) = weak.upgrade() {
                render_window(
                    &window,
                    &timer_snapshot.borrow(),
                    &timer_fields,
                    timer_selection.borrow().clone(),
                    &supervisor,
                );
            }
        });

        Ok(Self {
            window,
            timer,
            commands,
            worker: Some(worker),
            _snapshot: snapshot,
            _fields: fields,
            _selection: selection,
        })
    }

    pub fn weak(&self) -> slint::Weak<PluginControlWindow> {
        self.window.as_weak()
    }
}

impl Drop for InstanceControlSurface {
    fn drop(&mut self) {
        self.timer.stop();
        let _ = self.commands.try_send(ControlCommand::Stop);
        let Some(worker) = self.worker.take() else {
            return;
        };
        if let Err(error) = thread::Builder::new()
            .name("floatile-instance-control-reaper".to_owned())
            .spawn(move || {
                let _ = worker.join();
            })
        {
            tracing::warn!(%error, "failed to spawn instance control reaper");
        }
    }
}

fn wire_callbacks(
    window: &PluginControlWindow,
    commands: &SyncSender<ControlCommand>,
    supervisor: &InstanceSupervisorHandle,
    snapshot: Rc<RefCell<ControlSnapshot>>,
    fields: Rc<RefCell<Vec<ConfigField>>>,
    selection: Rc<RefCell<Selection>>,
) {
    let weak = window.as_weak();
    let select_snapshot = Rc::clone(&snapshot);
    let select_fields = Rc::clone(&fields);
    let select_selection = Rc::clone(&selection);
    let select_supervisor = supervisor.clone();
    window.on_select_installation(move |index| {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let snapshot = select_snapshot.borrow();
        let Some(installation) = snapshot.installations.get(index) else {
            return;
        };
        let selected = Selection::Installation {
            plugin_id: installation.plugin_id.clone(),
            version: installation.version.clone(),
        };
        *select_fields.borrow_mut() = installation.fields.clone();
        *select_selection.borrow_mut() = selected.clone();
        if let Some(window) = weak.upgrade() {
            render_window(
                &window,
                &snapshot,
                &select_fields,
                selected,
                &select_supervisor,
            );
        }
    });

    let weak = window.as_weak();
    let select_snapshot = Rc::clone(&snapshot);
    let select_fields = Rc::clone(&fields);
    let select_selection = Rc::clone(&selection);
    let select_supervisor = supervisor.clone();
    window.on_select_instance(move |index| {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let snapshot = select_snapshot.borrow();
        let Some(record) = snapshot.instances.get(index) else {
            return;
        };
        *select_fields.borrow_mut() = record.fields.clone();
        *select_selection.borrow_mut() = Selection::Instance(record.instance.id());
        if let Some(window) = weak.upgrade() {
            render_window(
                &window,
                &snapshot,
                &select_fields,
                Selection::Instance(record.instance.id()),
                &select_supervisor,
            );
        }
    });

    let field_model = Rc::clone(&fields);
    let weak = window.as_weak();
    window.on_field_edited(move |index, value, present| {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let mut fields = field_model.borrow_mut();
        let Some(field) = fields.get_mut(index) else {
            return;
        };
        field.value = value.to_string();
        field.present = field.required || present;
        if let Some(window) = weak.upgrade() {
            window.set_config_fields(config_model(&fields));
        }
    });

    let create_commands = commands.clone();
    let create_snapshot = Rc::clone(&snapshot);
    let create_fields = Rc::clone(&fields);
    let create_selection = Rc::clone(&selection);
    window.on_create_instance(move || {
        let Selection::Installation { plugin_id, version } = create_selection.borrow().clone()
        else {
            return;
        };
        let snapshot = create_snapshot.borrow();
        let Some(installation) = selected_installation(&snapshot, &plugin_id, &version) else {
            return;
        };
        try_command(
            &create_commands,
            ControlCommand::Create {
                plugin_id: installation.plugin_id.clone(),
                version: installation.version.clone(),
                fields: create_fields.borrow().clone(),
            },
        );
    });

    let save_commands = commands.clone();
    let save_snapshot = Rc::clone(&snapshot);
    let save_fields = Rc::clone(&fields);
    let save_selection = Rc::clone(&selection);
    window.on_save_config(move || {
        let Some(instance_id) =
            selected_instance_id(&save_snapshot, save_selection.borrow().clone())
        else {
            return;
        };
        try_command(
            &save_commands,
            ControlCommand::Configure {
                instance_id,
                fields: save_fields.borrow().clone(),
            },
        );
    });

    wire_instance_command(
        window,
        commands,
        &snapshot,
        &selection,
        InstanceDesiredState::Running,
    );
    wire_instance_command(
        window,
        commands,
        &snapshot,
        &selection,
        InstanceDesiredState::Stopped,
    );

    let retry_supervisor = supervisor.clone();
    let retry_snapshot = Rc::clone(&snapshot);
    let retry_selection = Rc::clone(&selection);
    window.on_retry_instance(move || {
        let Some(instance_id) =
            selected_instance_id(&retry_snapshot, retry_selection.borrow().clone())
        else {
            return;
        };
        if let Err(error) = retry_supervisor.retry(instance_id) {
            tracing::warn!(%error, instance_id = instance_id.0, "manual retry enqueue failed");
        }
    });

    let delete_commands = commands.clone();
    let delete_snapshot = Rc::clone(&snapshot);
    let delete_selection = Rc::clone(&selection);
    window.on_delete_instance(move || {
        let Some(instance_id) =
            selected_instance_id(&delete_snapshot, delete_selection.borrow().clone())
        else {
            return;
        };
        try_command(&delete_commands, ControlCommand::Delete(instance_id));
    });
}

fn wire_instance_command(
    window: &PluginControlWindow,
    commands: &SyncSender<ControlCommand>,
    snapshot: &Rc<RefCell<ControlSnapshot>>,
    selection: &Rc<RefCell<Selection>>,
    desired: InstanceDesiredState,
) {
    let commands = commands.clone();
    let snapshot = Rc::clone(snapshot);
    let selection = Rc::clone(selection);
    let callback = move || {
        let Some(instance_id) = selected_instance_id(&snapshot, selection.borrow().clone()) else {
            return;
        };
        try_command(
            &commands,
            ControlCommand::SetDesired {
                instance_id,
                desired,
            },
        );
    };
    match desired {
        InstanceDesiredState::Running => window.on_start_instance(callback),
        InstanceDesiredState::Stopped => window.on_stop_instance(callback),
    }
}

fn try_command(commands: &SyncSender<ControlCommand>, command: ControlCommand) {
    if let Err(error) = commands.try_send(command) {
        tracing::warn!(%error, "instance control command enqueue failed");
    }
}

fn selected_instance_id(
    snapshot: &Rc<RefCell<ControlSnapshot>>,
    selection: Selection,
) -> Option<InstanceId> {
    let Selection::Instance(instance_id) = selection else {
        return None;
    };
    snapshot
        .borrow()
        .instances
        .iter()
        .any(|record| record.instance.id() == instance_id)
        .then_some(instance_id)
}

fn selected_installation<'a>(
    snapshot: &'a ControlSnapshot,
    plugin_id: &str,
    version: &str,
) -> Option<&'a InstallationRecord> {
    snapshot
        .installations
        .iter()
        .find(|record| record.plugin_id == plugin_id && record.version == version)
}

fn normalize_selection(
    snapshot: &Rc<RefCell<ControlSnapshot>>,
    selection: &Rc<RefCell<Selection>>,
) {
    let valid = match &*selection.borrow() {
        Selection::None => true,
        Selection::Installation { plugin_id, version } => {
            selected_installation(&snapshot.borrow(), plugin_id, version).is_some()
        }
        Selection::Instance(instance_id) => snapshot
            .borrow()
            .instances
            .iter()
            .any(|record| record.instance.id() == *instance_id),
    };
    if !valid {
        *selection.borrow_mut() = Selection::None;
    }
}

fn render_window(
    window: &PluginControlWindow,
    snapshot: &ControlSnapshot,
    fields: &Rc<RefCell<Vec<ConfigField>>>,
    selection: Selection,
    supervisor: &InstanceSupervisorHandle,
) {
    window.set_installations(ModelRc::new(VecModel::from(
        snapshot
            .installations
            .iter()
            .map(|record| InstallationListItem {
                title: SharedString::from(record.name.as_str()),
                subtitle: SharedString::from(format!("{} @ {}", record.plugin_id, record.version)),
            })
            .collect::<Vec<_>>(),
    )));
    let observed = supervisor.observed_snapshot();
    window.set_instances(ModelRc::new(VecModel::from(
        snapshot
            .instances
            .iter()
            .map(|record| {
                let status = observed
                    .iter()
                    .find(|status| status.instance_id == record.instance.id());
                let state = status.map_or_else(
                    || match record.instance.desired_state() {
                        InstanceDesiredState::Running => ObservedInstanceState::Starting,
                        InstanceDesiredState::Stopped => ObservedInstanceState::Stopped,
                    },
                    |status| status.state,
                );
                InstanceListItem {
                    title: SharedString::from(format!(
                        "#{} {}",
                        record.instance.id().0,
                        record.instance.installation().plugin().0
                    )),
                    subtitle: SharedString::from(format!(
                        "{} · desired {}",
                        record.instance.installation().version(),
                        record.instance.desired_state().as_str()
                    )),
                    status: SharedString::from(state.as_str()),
                    error_code: SharedString::from(
                        status.and_then(|status| status.code).unwrap_or_default(),
                    ),
                }
            })
            .collect::<Vec<_>>(),
    )));
    window.set_config_fields(config_model(&fields.borrow()));
    window.set_notice(SharedString::from(
        snapshot.notice.as_deref().unwrap_or_default(),
    ));
    window.set_notice_ok(
        snapshot
            .notice
            .as_deref()
            .is_some_and(|notice| notice.starts_with("OK")),
    );

    window.set_selected_installation(matches!(&selection, Selection::Installation { .. }));
    window.set_selected_instance(matches!(&selection, Selection::Instance(_)));
    match selection {
        Selection::Installation { plugin_id, version } => {
            if let Some(record) = selected_installation(snapshot, &plugin_id, &version) {
                window.set_selection_title(SharedString::from(record.name.as_str()));
                window.set_selection_subtitle(SharedString::from(format!(
                    "{} @ {} · 新实例默认 stopped",
                    record.plugin_id, record.version
                )));
            }
            set_actions(window, false, false, false, false);
        }
        Selection::Instance(instance_id) => {
            if let Some(record) = snapshot
                .instances
                .iter()
                .find(|record| record.instance.id() == instance_id)
            {
                let observed = observed
                    .iter()
                    .find(|status| status.instance_id == record.instance.id());
                let failed =
                    observed.is_some_and(|status| status.state == ObservedInstanceState::Failed);
                let stopped = record.instance.desired_state() == InstanceDesiredState::Stopped;
                window.set_selection_title(SharedString::from(format!(
                    "实例 #{} · {}",
                    record.instance.id().0,
                    record.instance.installation().plugin().0
                )));
                window.set_selection_subtitle(SharedString::from(format!(
                    "{} · generation {}",
                    record.instance.installation().version(),
                    record.instance.generation()
                )));
                set_actions(window, stopped, !stopped, failed && !stopped, stopped);
            }
        }
        Selection::None => {
            window.set_selection_title("请选择插件或实例".into());
            window.set_selection_subtitle("配置解析与持久化均在后台执行".into());
            set_actions(window, false, false, false, false);
        }
    }
}

fn set_actions(
    window: &PluginControlWindow,
    can_start: bool,
    can_stop: bool,
    can_retry: bool,
    can_configure: bool,
) {
    window.set_can_start(can_start);
    window.set_can_stop(can_stop);
    window.set_can_retry(can_retry);
    window.set_can_configure(can_configure);
}

fn config_model(fields: &[ConfigField]) -> ModelRc<ConfigFieldItem> {
    ModelRc::new(VecModel::from(
        fields
            .iter()
            .map(|field| ConfigFieldItem {
                key: SharedString::from(field.key.as_str()),
                label: SharedString::from(field.label.as_str()),
                value: SharedString::from(field.value.as_str()),
                kind: SharedString::from(field.kind.as_str()),
                required: field.required,
                present: field.present,
            })
            .collect::<Vec<_>>(),
    ))
}

fn control_worker(
    database: PathBuf,
    plugin_store: PathBuf,
    command_rx: Receiver<ControlCommand>,
    snapshot_tx: SyncSender<ControlSnapshot>,
) {
    let store = match floatile_store::open(&database) {
        Ok(store) => store,
        Err(error) => {
            let _ = snapshot_tx.try_send(ControlSnapshot {
                notice: Some(format!("FINSTANCE_STORE: {error}")),
                ..ControlSnapshot::default()
            });
            return;
        }
    };
    let mut notice = None;
    loop {
        let snapshot = load_snapshot(&store, &plugin_store, notice.take());
        let _ = snapshot_tx.try_send(snapshot);
        match command_rx.recv_timeout(REFRESH_INTERVAL) {
            Ok(ControlCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                notice = Some(match apply_command(&store, &plugin_store, command) {
                    Ok(()) => "OK: 操作已保存".to_owned(),
                    Err(error) => format!("{}: {error}", error.code()),
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn load_snapshot(
    store: &floatile_store::Store,
    plugin_store: &std::path::Path,
    notice: Option<String>,
) -> ControlSnapshot {
    let installations = match list_highest(plugin_store) {
        Ok(installations) => match installations
            .iter()
            .map(installation_record)
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(records) => records,
            Err(error) => {
                return ControlSnapshot {
                    notice: Some(format!("{}: {error}", error.code())),
                    ..ControlSnapshot::default()
                };
            }
        },
        Err(error) => {
            return ControlSnapshot {
                notice: Some(format!("{}: {error}", error.code())),
                ..ControlSnapshot::default()
            };
        }
    };
    let mut instance_notice = None;
    let instances = match store.instances().list() {
        Ok(instances) => instances
            .into_iter()
            .map(|instance| {
                let fields = match load_reference(plugin_store, instance.installation()) {
                    Ok(Some(installation)) => match config_schema(&installation) {
                        Ok(schema) => form_fields(schema.as_ref(), Some(instance.config())),
                        Err(error) => {
                            instance_notice
                                .get_or_insert_with(|| format!("{}: {error}", error.code()));
                            Vec::new()
                        }
                    },
                    Ok(None) => {
                        instance_notice.get_or_insert_with(|| {
                            "FINSTANCE_INSTALLATION_MISSING: installation is missing".to_owned()
                        });
                        Vec::new()
                    }
                    Err(error) => {
                        instance_notice.get_or_insert_with(|| format!("{}: {error}", error.code()));
                        Vec::new()
                    }
                };
                InstanceRecord { instance, fields }
            })
            .collect(),
        Err(error) => {
            return ControlSnapshot {
                installations,
                notice: Some(format!("FINSTANCE_STORE: {error}")),
                ..ControlSnapshot::default()
            };
        }
    };
    ControlSnapshot {
        installations,
        instances,
        notice: notice.or(instance_notice),
    }
}

fn installation_record(
    installation: &InstalledInstallation,
) -> Result<InstallationRecord, ControlError> {
    let schema = config_schema(installation)?;
    Ok(InstallationRecord {
        plugin_id: installation.meta.id.clone(),
        name: installation.manifest.name.clone(),
        version: installation.meta.version.clone(),
        fields: form_fields(schema.as_ref(), None),
    })
}

fn config_schema(
    installation: &InstalledInstallation,
) -> Result<Option<serde_json::Value>, ControlError> {
    let Some(config) = &installation.manifest.config else {
        return Ok(None);
    };
    let bytes = installation
        .file(config.schema.as_str())
        .ok_or(ControlError::SchemaInvalid)?;
    serde_json::from_slice(bytes)
        .map(Some)
        .map_err(|_| ControlError::SchemaInvalid)
}

fn apply_command(
    store: &floatile_store::Store,
    plugin_store: &std::path::Path,
    command: ControlCommand,
) -> Result<(), ControlError> {
    match command {
        ControlCommand::Create {
            plugin_id,
            version,
            fields,
        } => {
            let installation = load_exact(plugin_store, &plugin_id, &version)?
                .ok_or(ControlError::InstallationMissing)?;
            let config = config_from_fields(&fields)?;
            installation.validate_config(&config)?;
            store.instances().create(
                &installation.reference()?,
                &config,
                InstanceDesiredState::Stopped,
                unix_now(),
            )?;
        }
        ControlCommand::Configure {
            instance_id,
            fields,
        } => {
            let instance = require_instance(store, instance_id)?;
            require_stopped(&instance)?;
            let installation = load_reference(plugin_store, instance.installation())?
                .ok_or(ControlError::InstallationMissing)?;
            let config = config_from_fields(&fields)?;
            installation.validate_config(&config)?;
            if !store.instances().update_config(
                instance_id,
                &config,
                unix_now().max(instance.updated_at()),
            )? {
                return Err(ControlError::ConcurrentUpdate);
            }
        }
        ControlCommand::SetDesired {
            instance_id,
            desired,
        } => {
            let instance = require_instance(store, instance_id)?;
            if !store.instances().set_desired_state(
                instance_id,
                desired,
                unix_now().max(instance.updated_at()),
            )? {
                return Err(ControlError::ConcurrentUpdate);
            }
        }
        ControlCommand::Delete(instance_id) => {
            let instance = require_instance(store, instance_id)?;
            require_stopped(&instance)?;
            if !store.instances().delete(instance_id)? {
                return Err(ControlError::ConcurrentUpdate);
            }
        }
        ControlCommand::Stop => {}
    }
    Ok(())
}

fn require_instance(
    store: &floatile_store::Store,
    instance_id: InstanceId,
) -> Result<PluginInstance, ControlError> {
    store
        .instances()
        .get(instance_id)?
        .ok_or(ControlError::InstanceMissing)
}

fn require_stopped(instance: &PluginInstance) -> Result<(), ControlError> {
    if instance.desired_state() == InstanceDesiredState::Stopped {
        Ok(())
    } else {
        Err(ControlError::MustBeStopped)
    }
}

#[derive(Debug, thiserror::Error)]
enum ControlError {
    #[error("installation is missing")]
    InstallationMissing,
    #[error("instance is missing")]
    InstanceMissing,
    #[error("instance must be stopped")]
    MustBeStopped,
    #[error("record changed concurrently")]
    ConcurrentUpdate,
    #[error("config schema is invalid")]
    SchemaInvalid,
    #[error("field `{0}` has an invalid value")]
    InvalidField(String),
    #[error(transparent)]
    Catalog(#[from] floatile_store::installation::InstallationCatalogError),
    #[error(transparent)]
    Config(#[from] floatile_core::instance::InstanceModelError),
    #[error(transparent)]
    ConfigValidation(#[from] floatile_store::installation::ConfigValidationError),
    #[error(transparent)]
    Store(#[from] floatile_store::StoreError),
}

impl ControlError {
    fn code(&self) -> &'static str {
        match self {
            Self::InstallationMissing => "FINSTANCE_INSTALLATION_MISSING",
            Self::InstanceMissing => "FINSTANCE_NOT_FOUND",
            Self::MustBeStopped => "FINSTANCE_MUST_BE_STOPPED",
            Self::ConcurrentUpdate => "FINSTANCE_CONCURRENT_UPDATE",
            Self::SchemaInvalid => "FINSTANCE_CONFIG_SCHEMA_INVALID",
            Self::InvalidField(_) | Self::Config(_) | Self::ConfigValidation(_) => {
                "FINSTANCE_CONFIG_INVALID"
            }
            Self::Catalog(error) => error.code(),
            Self::Store(_) => "FINSTANCE_STORE",
        }
    }
}

fn form_fields(
    schema: Option<&serde_json::Value>,
    config: Option<&InstanceConfig>,
) -> Vec<ConfigField> {
    let Some(schema) = schema else {
        return Vec::new();
    };
    let Some(properties) = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        return root_config_field(config);
    };
    let has_composition = [
        "allOf",
        "anyOf",
        "oneOf",
        "if",
        "then",
        "else",
        "patternProperties",
    ]
    .iter()
    .any(|keyword| schema.get(keyword).is_some());
    let has_unrepresented_values = config.is_some_and(|config| {
        config
            .as_object()
            .keys()
            .any(|key| !properties.contains_key(key))
    });
    if has_composition || has_unrepresented_values {
        return root_config_field(config);
    }
    let required: BTreeSet<&str> = schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let config = config.map(InstanceConfig::as_object);
    let mut keys: Vec<&String> = properties.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|key| {
            let field_schema = resolve_schema(schema, &properties[key]);
            let kind = field_kind(field_schema);
            let existing = config.and_then(|config| config.get(key));
            let default = field_schema.get("default");
            let value = existing.or(default);
            ConfigField {
                key: key.clone(),
                label: field_schema
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(key)
                    .to_owned(),
                kind,
                value: value.map_or_else(String::new, |value| field_value(kind, value)),
                required: required.contains(key.as_str()),
                present: existing.is_some() || default.is_some() || required.contains(key.as_str()),
            }
        })
        .collect()
}

fn root_config_field(config: Option<&InstanceConfig>) -> Vec<ConfigField> {
    vec![ConfigField {
        key: String::new(),
        label: "完整配置".to_owned(),
        kind: ConfigFieldKind::RootJson,
        value: config.map(InstanceConfig::to_value).map_or_else(
            || "{}".to_owned(),
            |value| field_value(ConfigFieldKind::RootJson, &value),
        ),
        required: true,
        present: true,
    }]
}

fn resolve_schema<'a>(
    root: &'a serde_json::Value,
    schema: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let Some(reference) = schema.get("$ref").and_then(serde_json::Value::as_str) else {
        return schema;
    };
    let Some(pointer) = reference.strip_prefix('#') else {
        return schema;
    };
    root.pointer(pointer).unwrap_or(schema)
}

fn field_kind(schema: &serde_json::Value) -> ConfigFieldKind {
    match schema.get("type").and_then(serde_json::Value::as_str) {
        Some("string") => ConfigFieldKind::String,
        Some("integer") => ConfigFieldKind::Integer,
        Some("number") => ConfigFieldKind::Number,
        Some("boolean") => ConfigFieldKind::Boolean,
        _ => ConfigFieldKind::Json,
    }
}

fn field_value(kind: ConfigFieldKind, value: &serde_json::Value) -> String {
    match (kind, value) {
        (ConfigFieldKind::String, serde_json::Value::String(value)) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn config_from_fields(fields: &[ConfigField]) -> Result<InstanceConfig, ControlError> {
    if let Some(root) = fields
        .iter()
        .find(|field| field.kind == ConfigFieldKind::RootJson)
    {
        let value = serde_json::from_str(&root.value)
            .map_err(|_| ControlError::InvalidField("<root>".to_owned()))?;
        return InstanceConfig::new(value).map_err(Into::into);
    }
    let mut object = serde_json::Map::new();
    for field in fields
        .iter()
        .filter(|field| field.present || field.required)
    {
        let value = match field.kind {
            ConfigFieldKind::String => serde_json::Value::String(field.value.clone()),
            ConfigFieldKind::Integer => field
                .value
                .parse::<i64>()
                .map(serde_json::Value::from)
                .map_err(|_| ControlError::InvalidField(field.key.clone()))?,
            ConfigFieldKind::Number => field
                .value
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number)
                .ok_or_else(|| ControlError::InvalidField(field.key.clone()))?,
            ConfigFieldKind::Boolean => field
                .value
                .parse::<bool>()
                .map(serde_json::Value::Bool)
                .map_err(|_| ControlError::InvalidField(field.key.clone()))?,
            ConfigFieldKind::Json | ConfigFieldKind::RootJson => serde_json::from_str(&field.value)
                .map_err(|_| ControlError::InvalidField(field.key.clone()))?,
        };
        object.insert(field.key.clone(), value);
    }
    InstanceConfig::new(serde_json::Value::Object(object)).map_err(Into::into)
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
    use super::*;
    use std::collections::BTreeMap;

    use floatile_core::install::{InstallMeta, content_digest, file_digest, hex_encode};

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "floatile-instance-control-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_install(root: &std::path::Path) {
        let dir = root.join("dev.floatile.clock").join("1.0.0");
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::create_dir_all(dir.join("logic")).unwrap();
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "id": "dev.floatile.clock",
            "name": "Clock",
            "version": "1.0.0",
            "publisher": { "id": "dev.floatile", "name": "Floatile" },
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "config": { "schema": "config.schema.json" },
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 100, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": []
        })
        .to_string()
        .into_bytes();
        let mut files = BTreeMap::from([
            (
                "config.schema.json".to_owned(),
                serde_json::to_vec(&serde_json::json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["zone"],
                    "properties": { "zone": { "type": "string", "minLength": 1 } }
                }))
                .unwrap(),
            ),
            ("logic/plugin.wasm".to_owned(), b"wasm".to_vec()),
            ("manifest.json".to_owned(), manifest),
            ("ui/widget.ftui".to_owned(), b"{}".to_vec()),
        ]);
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let meta = InstallMeta {
            manifest_version: 1,
            id: "dev.floatile.clock".to_owned(),
            version: "1.0.0".to_owned(),
            engine_api_version: "1.0.0".to_owned(),
            ui_api_version: "1.0.0".to_owned(),
            installed_at: 1,
            source: "test".to_owned(),
            trust: floatile_core::install::InstallationTrust::Unsigned,
            files: files
                .iter()
                .map(|(name, bytes)| (name.clone(), hex_encode(&file_digest(bytes))))
                .collect(),
            digest: hex_encode(&content_digest(&files)),
        };
        std::fs::write(dir.join("install.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        files.clear();
    }

    fn zone_field(value: &str) -> Vec<ConfigField> {
        vec![ConfigField {
            key: "zone".to_owned(),
            label: "zone".to_owned(),
            kind: ConfigFieldKind::String,
            value: value.to_owned(),
            required: true,
            present: true,
        }]
    }

    #[test]
    fn schema_form_round_trips_common_and_json_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["zone", "enabled"],
            "properties": {
                "zone": { "type": "string", "title": "Time zone" },
                "enabled": { "type": "boolean", "default": true },
                "retries": { "type": "integer" },
                "labels": { "type": "array", "items": { "type": "string" } }
            }
        });
        let config = InstanceConfig::new(serde_json::json!({
            "zone": "UTC",
            "enabled": false,
            "labels": ["desk"]
        }))
        .unwrap();
        let fields = form_fields(Some(&schema), Some(&config));
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[3].label, "Time zone");
        assert_eq!(config_from_fields(&fields).unwrap(), config);
    }

    #[test]
    fn schema_form_rejects_invalid_typed_input_without_exposing_value() {
        let fields = vec![ConfigField {
            key: "retries".to_owned(),
            label: "Retries".to_owned(),
            kind: ConfigFieldKind::Integer,
            value: "secret-not-a-number".to_owned(),
            required: true,
            present: true,
        }];
        let error = config_from_fields(&fields).unwrap_err();
        assert_eq!(error.code(), "FINSTANCE_CONFIG_INVALID");
        assert!(!error.to_string().contains("secret-not-a-number"));
    }

    #[test]
    fn schema_form_resolves_local_fragment_references() {
        let schema = serde_json::json!({
            "type": "object",
            "$defs": { "zone": { "type": "string", "title": "Zone" } },
            "properties": { "zone": { "$ref": "#/$defs/zone" } }
        });
        let fields = form_fields(Some(&schema), None);
        assert_eq!(fields[0].kind, ConfigFieldKind::String);
        assert_eq!(fields[0].label, "Zone");
    }

    #[test]
    fn composed_schema_uses_lossless_root_json_fallback() {
        let schema = serde_json::json!({
            "type": "object",
            "allOf": [{ "properties": { "zone": { "type": "string" } } }]
        });
        let config = InstanceConfig::new(serde_json::json!({ "zone": "UTC" })).unwrap();
        let fields = form_fields(Some(&schema), Some(&config));
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].kind, ConfigFieldKind::RootJson);
        assert_eq!(config_from_fields(&fields).unwrap(), config);
    }

    #[test]
    fn installation_selection_tracks_exact_identity_across_list_changes() {
        let record = |plugin_id: &str, version: &str| InstallationRecord {
            plugin_id: plugin_id.to_owned(),
            name: plugin_id.to_owned(),
            version: version.to_owned(),
            fields: Vec::new(),
        };
        let snapshot = ControlSnapshot {
            installations: vec![
                record("dev.floatile.alpha", "1.0.0"),
                record("dev.floatile.clock", "1.0.0"),
            ],
            ..ControlSnapshot::default()
        };
        let selection = Rc::new(RefCell::new(Selection::Installation {
            plugin_id: "dev.floatile.clock".to_owned(),
            version: "1.0.0".to_owned(),
        }));
        let reordered = Rc::new(RefCell::new(ControlSnapshot {
            installations: snapshot.installations.into_iter().rev().collect(),
            ..ControlSnapshot::default()
        }));

        normalize_selection(&reordered, &selection);
        assert!(matches!(
            &*selection.borrow(),
            Selection::Installation { plugin_id, version }
                if plugin_id == "dev.floatile.clock" && version == "1.0.0"
        ));

        reordered
            .borrow_mut()
            .installations
            .retain(|record| record.plugin_id != "dev.floatile.clock");
        normalize_selection(&reordered, &selection);
        assert_eq!(*selection.borrow(), Selection::None);
    }

    #[test]
    fn control_commands_cover_create_configure_start_stop_and_delete() {
        let root = temp_root("commands");
        let plugin_store = root.join("plugins");
        write_install(&plugin_store);
        let store = floatile_store::open(root.join("layout.db")).unwrap();

        apply_command(
            &store,
            &plugin_store,
            ControlCommand::Create {
                plugin_id: "dev.floatile.clock".to_owned(),
                version: "1.0.0".to_owned(),
                fields: zone_field("UTC"),
            },
        )
        .unwrap();
        let instance = store.instances().list().unwrap().remove(0);
        assert_eq!(instance.desired_state(), InstanceDesiredState::Stopped);

        apply_command(
            &store,
            &plugin_store,
            ControlCommand::Configure {
                instance_id: instance.id(),
                fields: zone_field("CET"),
            },
        )
        .unwrap();
        apply_command(
            &store,
            &plugin_store,
            ControlCommand::SetDesired {
                instance_id: instance.id(),
                desired: InstanceDesiredState::Running,
            },
        )
        .unwrap();
        assert!(matches!(
            apply_command(
                &store,
                &plugin_store,
                ControlCommand::Configure {
                    instance_id: instance.id(),
                    fields: zone_field("EST"),
                },
            ),
            Err(ControlError::MustBeStopped)
        ));
        apply_command(
            &store,
            &plugin_store,
            ControlCommand::SetDesired {
                instance_id: instance.id(),
                desired: InstanceDesiredState::Stopped,
            },
        )
        .unwrap();
        apply_command(&store, &plugin_store, ControlCommand::Delete(instance.id())).unwrap();
        assert!(store.instances().list().unwrap().is_empty());
    }
}
