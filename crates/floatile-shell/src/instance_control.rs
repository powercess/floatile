//! 插件安装与实例控制面（PP-M1）。
//!
//! Slint 线程只读取已准备的快照、编辑有界字符串并 `try_send` 命令；SQLite、安装目录、
//! JSON 解析和 Config Schema 求值都在专用 worker 完成。observed lifecycle 来自
//! `DynamicInstanceSupervisor`，不把运行结果写回 desired-state 持久化记录。

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Take};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use floatile_core::{
    ConnectionHealth, ConnectionId, CredentialRef, InstallationRef, InstanceConfig,
    InstanceDesiredState, InstanceId, PluginInstance,
};
use floatile_platform::raise_always_on_top;
use floatile_platform::{AutostartState, autostart_state, set_autostart};
use floatile_services::{CredentialVault, MemoryCredentialVault};
use floatile_store::installation::{InstalledInstallation, list_all, load_exact, load_reference};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::instance_supervisor::{
    InstanceSupervisorHandle, ObservedInstanceState, ObservedInstanceStatus,
};
use crate::runtime_ui::RuntimeSettingsHandler;

const CONTROL_QUEUE_CAPACITY: usize = 16;
const SNAPSHOT_QUEUE_CAPACITY: usize = 2;
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const UI_DRAIN_INTERVAL: Duration = Duration::from_millis(100);

fn host_runtime_guidance() -> (&'static str, &'static str, &'static str) {
    #[cfg(target_os = "windows")]
    {
        (
            "Windows · 单实例宿主运行中",
            "关闭管理中心后，Widget 与通知区域托盘继续运行。",
            "恢复编辑：Ctrl+Shift+E · 完全退出：通知区域托盘菜单",
        )
    }
    #[cfg(target_os = "macos")]
    {
        (
            "macOS · Floatile 宿主运行中",
            "关闭管理中心不会删除实例；运行中的 Widget 继续由宿主管理。",
            "恢复编辑：Ctrl+Shift+E · 完全退出：应用菜单",
        )
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        (
            "Linux · Floatile 宿主运行中",
            "关闭管理中心不会删除实例；运行中的 Widget 继续由宿主管理。",
            "恢复编辑：Ctrl+Shift+E · 完全退出：桌面托盘或进程管理器",
        )
    }
}

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
        status-kind: string,
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

    component HostAction inherits Rectangle {
        in property <string> label;
        in property <bool> primary: false;
        in property <bool> danger: false;
        in property <bool> default-focus: false;
        in property <bool> enabled: true;
        callback clicked;
        init => { if root.default-focus { action-focus.focus(); } }
        preferred-width: 112px;
        min-width: 92px;
        max-width: 132px;
        border-radius: 7px;
        border-width: action-focus.has-focus ? 2px : 1px;
        border-color: !root.enabled ? #394253 : action-focus.has-focus ? #ffffff : root.danger ? #a94a55 : root.primary ? #3d85d8 : #46536a;
        background: !root.enabled ? #242b38 : action-touch.has-hover
            ? (root.danger ? #a83d4c : root.primary ? #397dcc : #354158)
            : (root.danger ? #82313d : root.primary ? #2f6fba : #2a3344);
        forward-focus: action-focus;
        accessible-role: button;
        accessible-label: root.label;
        accessible-enabled: root.enabled;
        accessible-action-default => { if root.enabled { root.clicked(); } }
        Text {
            text: root.label;
            color: root.enabled ? #f4f7fc : #778399;
            font-size: 12px;
            horizontal-alignment: center;
            vertical-alignment: center;
        }
        action-focus := FocusScope {
            key-pressed(event) => {
                if root.enabled && (event.text == " " || event.text == "\n") {
                    root.clicked();
                    return accept;
                }
                return reject;
            }
        }
        action-touch := TouchArea { enabled: root.enabled; clicked => { action-focus.focus(); root.clicked(); } }
    }

    component HostToggle inherits Rectangle {
        in property <string> label;
        in property <bool> enabled: true;
        in-out property <bool> checked: false;
        callback toggled;
        accessible-role: checkbox;
        accessible-checkable: true;
        accessible-enabled: root.enabled;
        accessible-label: root.label;
        accessible-checked: root.checked;
        accessible-action-default => {
            if root.enabled { root.checked = !root.checked; root.toggled(); }
        }
        forward-focus: toggle-focus;
        border-width: toggle-focus.has-focus ? 1px : 0px;
        border-color: #ffffff;
        border-radius: 5px;
        Rectangle {
            x: 2px; y: (parent.height - 18px) / 2; width: 18px; height: 18px;
            background: !root.enabled ? #242b38 : root.checked ? #2f6fba : #202735;
            border-width: 1px;
            border-color: !root.enabled ? #394253 : root.checked ? #4c9cf4 : #596778;
            border-radius: 4px;
            Text { text: root.checked ? "✓" : ""; color: #ffffff; font-size: 13px; horizontal-alignment: center; vertical-alignment: center; }
        }
        Text {
            x: 30px; width: parent.width - 32px; height: parent.height;
            text: root.label; color: root.enabled ? #c3cede : #778399; font-size: 11px;
            vertical-alignment: center; overflow: elide;
        }
        toggle-focus := FocusScope {
            key-pressed(event) => {
                if root.enabled && (event.text == " " || event.text == "\n") {
                    root.checked = !root.checked; root.toggled(); return accept;
                }
                return reject;
            }
        }
        toggle-touch := TouchArea {
            enabled: root.enabled;
            clicked => { toggle-focus.focus(); root.checked = !root.checked; root.toggled(); }
        }
    }

    export component PluginControlWindow inherits Window {
        title: "Floatile 插件与实例";
        preferred-width: 900px;
        preferred-height: 620px;
        min-width: 700px;
        min-height: 480px;
        background: #151922;
        no-frame: false;
        always-on-top: true;

        in property <[InstallationListItem]> installations;
        in property <[InstanceListItem]> instances;
        in property <[ConfigFieldItem]> config-fields;
        in property <string> selection-title: "请选择插件或实例";
        in property <string> selection-subtitle: "";
        in property <string> host-platform-summary: "";
        in property <string> host-lifecycle-summary: "";
        in property <string> host-recovery-summary: "";
        in property <bool> autostart-enabled: false;
        in property <bool> autostart-available: false;
        in property <string> autostart-label: "开机启动不可用";
        in property <string> diagnostic-summary: "";
        in-out property <bool> diagnostics-open: false;
        in property <string> installation-publisher: "";
        in property <string> installation-trust: "";
        in property <string> installation-source: "";
        in property <string> installation-permissions: "";
        in property <string> installation-permission-risk: "";
        in property <string> notice: "";
        in property <bool> notice-ok: false;
        in property <bool> selected-instance: false;
        in property <bool> selected-installation: false;
        in property <bool> can-start: false;
        in property <bool> can-stop: false;
        in property <bool> can-retry: false;
        in property <string> retry-label: "重试";
        in property <bool> can-configure: false;
        in property <bool> can-uninstall: false;
        in property <bool> can-rebind: false;
        in property <string> rebind-label: "切换版本";
        in property <string> rebind-warning: "确认切换实例版本？";
        in property <string> instance-state: "";
        in property <string> instance-error-code: "";
        in property <string> instance-error-title: "";
        in property <string> instance-error-help: "";
        in property <bool> instance-requires-connection: false;
        in property <string> instance-connection-title: "";
        in property <string> instance-connection-help: "";
        in property <string> instance-connection-status-kind: "missing";
        in-out property <string> connection-provider: "";
        in-out property <string> connection-account: "";
        in-out property <string> connection-secret: "";
        in-out property <bool> editing-connection-secret: false;
        in-out property <bool> confirm-delete: false;
        in-out property <bool> confirm-uninstall: false;
        in-out property <bool> confirm-rebind: false;
        in-out property <bool> confirm-create: false;
        in-out property <bool> confirm-revoke-connection: false;
        in property <int> selected-installation-index: -1;
        in property <int> selected-instance-index: -1;
        in-out property <string> package-path: "";
        in-out property <bool> picker-busy: false;

        callback install-package;
        callback choose-package;
        callback show-overview;
        callback select-installation(int);
        callback select-instance(int);
        callback field-edited(int, string, bool);
        callback create-instance;
        callback save-config;
        callback start-instance;
        callback stop-instance;
        callback retry-instance;
        callback delete-instance;
        callback uninstall-package;
        callback rebind-instance;
        callback add-connection;
        callback rotate-connection-credential;
        callback revoke-connection;
        callback set-autostart(bool);

        forward-focus: window-focus;
        window-focus := FocusScope {
            key-pressed(event) => {
                if event.text == Key.Escape && (root.confirm-delete || root.confirm-uninstall || root.confirm-rebind || root.confirm-create || root.confirm-revoke-connection || root.editing-connection-secret) {
                    root.confirm-delete = false;
                    root.confirm-uninstall = false;
                    root.confirm-rebind = false;
                    root.confirm-create = false;
                    root.confirm-revoke-connection = false;
                    root.connection-secret = "";
                    root.editing-connection-secret = false;
                    return accept;
                }
                return reject;
            }

        Rectangle {
            x: 0px; y: 0px; width: 310px; height: parent.height;
            background: #1a202b;
            SectionTitle { x: 16px; y: 14px; text: "安装本地开发包"; }
            Button {
                x: 174px; y: 6px; width: 60px; height: 28px;
                text: "诊断";
                clicked => { root.diagnostics-open = true; root.show-overview(); }
            }
            Button {
                x: 240px; y: 6px; width: 60px; height: 28px;
                text: "概览";
                clicked => { root.diagnostics-open = false; root.show-overview(); }
            }
            LineEdit {
                x: 10px; y: 40px; width: 160px; height: 34px;
                text <=> root.package-path;
                placeholder-text: "C:\\path\\plugin.floatile";
            }
            Button {
                x: 176px; y: 40px; width: 58px; height: 34px;
                text: root.picker-busy ? "等待" : "浏览";
                enabled: !root.picker-busy;
                clicked => { root.choose-package(); }
            }
            Button {
                x: 240px; y: 40px; width: 60px; height: 34px;
                text: "安装";
                enabled: root.package-path != "" && !root.picker-busy;
                clicked => { root.install-package(); }
            }
            SectionTitle { x: 16px; y: 84px; text: "已安装插件"; }
            ScrollView {
                x: 10px; y: 108px; width: 290px; height: 94px;
                viewport-height: Math.max(94px, root.installations.length * 56px);
                for item[index] in root.installations: Rectangle {
                    y: index * 56px; width: 278px; height: 52px;
                    background: root.selected-installation-index == index ? #2d4566 : install-touch.has-hover ? #2a3445 : #222a38;
                    border-width: install-focus.has-focus ? 2px : root.selected-installation-index == index ? 1px : 0px;
                    border-color: install-focus.has-focus ? #ffffff : #4c8bd4;
                    border-radius: 6px;
                    forward-focus: install-focus;
                    accessible-role: list-item;
                    accessible-label: (root.selected-installation-index == index ? "已选择，" : "") + item.title + ", " + item.subtitle;
                    accessible-action-default => { install-touch.clicked(); }
                    install-focus := FocusScope {
                        key-pressed(event) => {
                            if event.text == " " || event.text == "\n" {
                                root.confirm-delete = false;
                                root.confirm-uninstall = false;
                                root.confirm-rebind = false;
                                root.confirm-create = false;
                                root.select-installation(index);
                                return accept;
                            }
                            return reject;
                        }
                    }
                    install-touch := TouchArea { clicked => { install-focus.focus(); root.confirm-delete = false; root.confirm-uninstall = false; root.confirm-rebind = false; root.confirm-create = false; root.select-installation(index); } }
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
                    background: root.selected-instance-index == index ? #2d4566 : instance-touch.has-hover ? #2a3445 : #222a38;
                    border-width: instance-focus.has-focus ? 2px : root.selected-instance-index == index ? 1px : 0px;
                    border-color: instance-focus.has-focus ? #ffffff : #4c8bd4;
                    border-radius: 6px;
                    forward-focus: instance-focus;
                    accessible-role: list-item;
                    accessible-label: (root.selected-instance-index == index ? "已选择，" : "") + item.title + ", " + item.subtitle + ", " + item.status;
                    accessible-action-default => { instance-touch.clicked(); }
                    instance-focus := FocusScope {
                        key-pressed(event) => {
                            if event.text == " " || event.text == "\n" {
                                root.confirm-delete = false;
                                root.confirm-uninstall = false;
                                root.confirm-rebind = false;
                                root.confirm-create = false;
                                root.select-instance(index);
                                return accept;
                            }
                            return reject;
                        }
                    }
                    instance-touch := TouchArea { clicked => { instance-focus.focus(); root.confirm-delete = false; root.confirm-uninstall = false; root.confirm-rebind = false; root.confirm-create = false; root.select-instance(index); } }
                    Text { x: 10px; y: 7px; width: 188px; text: item.title; color: #e8eef8; font-size: 13px; overflow: elide; }
                    Text { x: 202px; y: 7px; width: 66px; text: item.status; color: item.status-kind == "failed" ? #ff8080 : item.status-kind == "running" ? #65d899 : item.status-kind == "starting" ? #78b7ff : #a9b6ca; font-size: 11px; horizontal-alignment: right; }
                    Text { x: 10px; y: 29px; width: 258px; text: item.subtitle; color: #8fa0ba; font-size: 11px; overflow: elide; }
                    Text { x: 10px; y: 49px; width: 258px; text: item.error-code; color: #ff9a9a; font-size: 10px; overflow: elide; }
                }
            }
        }

        Rectangle {
            x: 310px; y: 0px; width: parent.width - 310px; height: parent.height;
            background: #151922;
            Text { x: 22px; y: 18px; width: parent.width - 44px; text: root.selection-title; color: #f0f4fa; font-size: 18px; font-weight: 700; overflow: elide; }
            Text { x: 22px; y: 48px; width: parent.width - 44px; text: root.selection-subtitle; color: #93a2b9; font-size: 12px; overflow: elide; }
            Text { x: 22px; y: 76px; width: parent.width - 44px; text: root.notice; color: root.notice-ok ? #65d899 : #ff9a9a; font-size: 11px; overflow: elide; }

            if !root.selected-installation && !root.selected-instance && !root.diagnostics-open: Rectangle {
                x: 18px; y: 106px; width: parent.width - 36px; height: 156px;
                background: #1d2430;
                border-width: 1px;
                border-color: #2b3546;
                border-radius: 9px;
                Rectangle { x: 16px; y: 18px; width: 4px; height: 42px; background: #4c8bd4; border-radius: 2px; }
                Text { x: 32px; y: 16px; width: parent.width - 48px; text: "从左侧开始"; color: #e8eef8; font-size: 15px; font-weight: 700; }
                Text { x: 32px; y: 43px; width: parent.width - 48px; text: "选择一个已安装版本以查看来源、信任和权限，并创建实例。"; color: #a9b6ca; font-size: 12px; wrap: word-wrap; }
                Rectangle { x: 16px; y: 82px; width: parent.width - 32px; height: 1px; background: #2b3546; }
                Text { x: 32px; y: 98px; width: parent.width - 48px; text: "选择一个实例以编辑配置、控制运行状态或安全切换版本。"; color: #a9b6ca; font-size: 12px; wrap: word-wrap; }
            }

            if !root.selected-installation && !root.selected-instance && !root.diagnostics-open: Rectangle {
                x: 18px; y: 278px; width: parent.width - 36px; height: 174px;
                background: #192431;
                border-width: 1px;
                border-color: #29445f;
                border-radius: 9px;
                Text { x: 16px; y: 14px; width: parent.width - 32px; text: root.host-platform-summary; color: #78b7ff; font-size: 13px; font-weight: 700; overflow: elide; }
                Text { x: 16px; y: 43px; width: parent.width - 32px; text: root.host-lifecycle-summary; color: #c3cede; font-size: 12px; wrap: word-wrap; }
                Rectangle { x: 16px; y: 78px; width: parent.width - 32px; height: 1px; background: #29445f; }
                Text { x: 16px; y: 92px; width: parent.width - 32px; text: root.host-recovery-summary; color: #9fb4ce; font-size: 11px; wrap: word-wrap; }
                Rectangle { x: 16px; y: 122px; width: parent.width - 32px; height: 1px; background: #29445f; }
                HostToggle {
                    x: 16px; y: 134px; width: parent.width - 32px; height: 28px;
                    label: root.autostart-label;
                    checked: root.autostart-enabled;
                    enabled: root.autostart-available;
                    toggled => { root.set-autostart(self.checked); }
                }
            }

            if !root.selected-installation && !root.selected-instance && root.diagnostics-open: diagnostic-card := Rectangle {
                property <int> copy-stage: 0;
                property <bool> copy-feedback: false;
                x: 18px; y: 106px; width: parent.width - 36px; height: parent.height - 124px;
                background: #192431;
                border-width: 1px;
                border-color: #29445f;
                border-radius: 9px;
                Text { x: 16px; y: 14px; width: parent.width - 150px; text: "脱敏诊断摘要"; color: #78b7ff; font-size: 13px; font-weight: 700; }
                Button {
                    x: parent.width - 128px; y: 10px; width: 112px; height: 28px;
                    text: diagnostic-card.copy-feedback ? "已复制" : "复制摘要";
                    clicked => {
                        diagnostic-card.copy-feedback = false;
                        diagnostic-input.focus();
                        diagnostic-card.copy-stage = 1;
                    }
                }
                Text { x: 16px; y: 44px; width: parent.width - 32px; text: "仅包含版本、实例状态和稳定错误码；不包含配置、账户、凭证或本地路径。"; color: #9fb4ce; font-size: 11px; wrap: word-wrap; }
                Rectangle {
                    x: 16px; y: 80px; width: parent.width - 32px; height: parent.height - 96px;
                    background: #111720; border-width: 1px; border-color: #2b3c52; border-radius: 6px;
                    diagnostic-input := TextInput {
                        x: 10px; y: 8px; width: parent.width - 20px; height: parent.height - 16px;
                        text: root.diagnostic-summary; read-only: true; single-line: false; wrap: word-wrap;
                        color: #c3cede; selection-background-color: #2f6fba; selection-foreground-color: #ffffff;
                        font-size: 11px;
                    }
                }
                Timer {
                    interval: 1ms;
                    running: diagnostic-card.copy-stage > 0;
                    triggered => {
                        if diagnostic-card.copy-stage == 1 {
                            diagnostic-input.select-all();
                            diagnostic-card.copy-stage = 2;
                        } else {
                            diagnostic-input.copy();
                            diagnostic-card.copy-stage = 0;
                            diagnostic-card.copy-feedback = true;
                        }
                    }
                }
                Timer {
                    interval: 2s;
                    running: diagnostic-card.copy-feedback;
                    triggered => { diagnostic-card.copy-feedback = false; }
                }
            }

            if root.selected-instance && root.instance-state == "failed": Rectangle {
                x: 18px; y: 106px; width: parent.width - 36px; height: 148px;
                background: #291e24;
                border-width: 1px;
                border-color: #6f3742;
                border-radius: 9px;
                Rectangle { x: 16px; y: 18px; width: 4px; height: 42px; background: #ff8080; border-radius: 2px; }
                Text { x: 32px; y: 15px; width: parent.width - 48px; text: root.instance-error-title; color: #ffd7db; font-size: 15px; font-weight: 700; overflow: elide; }
                Text { x: 32px; y: 43px; width: parent.width - 48px; text: root.instance-error-code; color: #ff9a9a; font-size: 11px; overflow: elide; }
                Rectangle { x: 16px; y: 76px; width: parent.width - 32px; height: 1px; background: #55303a; }
                Text { x: 32px; y: 91px; width: parent.width - 48px; text: root.instance-error-help; color: #d8b8bd; font-size: 12px; wrap: word-wrap; }
            }

            if root.selected-instance && root.instance-state == "starting": Rectangle {
                x: 18px; y: 106px; width: parent.width - 36px; height: 116px;
                background: #1b2635;
                border-width: 1px;
                border-color: #345d89;
                border-radius: 9px;
                Rectangle { x: 16px; y: 18px; width: 4px; height: 42px; background: #62a9f5; border-radius: 2px; }
                Text { x: 32px; y: 16px; width: parent.width - 48px; text: "正在启动插件"; color: #dcebff; font-size: 15px; font-weight: 700; }
                Text { x: 32px; y: 43px; width: parent.width - 48px; text: "宿主正在重新验证安装、配置与运行环境。完成前无需重复操作。"; color: #a9bed8; font-size: 12px; wrap: word-wrap; }
            }

            if root.selected-instance && root.instance-requires-connection && root.instance-state != "failed" && root.instance-state != "starting": Rectangle {
                x: 18px; y: 106px; width: parent.width - 36px; height: root.instance-connection-title == "尚未授权连接" ? 210px : root.editing-connection-secret ? 162px : 116px;
                background: root.instance-connection-status-kind == "unavailable" ? #291e24 : root.instance-connection-status-kind == "healthy" ? #192831 : root.instance-connection-status-kind == "missing" || root.instance-connection-status-kind == "degraded" ? #29231b : #1b2635;
                border-width: 1px;
                border-color: root.instance-connection-status-kind == "unavailable" ? #6f3742 : root.instance-connection-status-kind == "healthy" ? #315a58 : root.instance-connection-status-kind == "missing" || root.instance-connection-status-kind == "degraded" ? #735b35 : #345d89;
                border-radius: 9px;
                Rectangle { x: 16px; y: 18px; width: 4px; height: 42px; background: root.instance-connection-status-kind == "unavailable" ? #ff8080 : root.instance-connection-status-kind == "healthy" ? #65d899 : root.instance-connection-status-kind == "missing" || root.instance-connection-status-kind == "degraded" ? #f0b35f : #62a9f5; border-radius: 2px; }
                Text { x: 32px; y: 16px; width: parent.width - 48px; text: root.instance-connection-title; color: #e8eef8; font-size: 15px; font-weight: 700; }
                Text { x: 32px; y: 43px; width: parent.width - 48px; text: root.instance-connection-help; color: #a9b6ca; font-size: 12px; wrap: word-wrap; }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state == "stopped" && !root.confirm-revoke-connection && !root.editing-connection-secret: HostAction {
                    x: 32px; y: 72px; width: 112px; height: 30px;
                    label: "更新凭证";
                    clicked => { root.connection-secret = ""; root.editing-connection-secret = true; }
                }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state == "stopped" && !root.confirm-revoke-connection && !root.editing-connection-secret: HostAction {
                    x: 152px; y: 72px; width: 112px; height: 30px;
                    label: "撤销连接"; danger: true;
                    clicked => { root.confirm-revoke-connection = true; }
                }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state == "stopped" && root.confirm-revoke-connection: HostAction {
                    x: 32px; y: 72px; width: 112px; height: 30px;
                    label: "确认撤销"; danger: true;
                    clicked => { root.confirm-revoke-connection = false; root.revoke-connection(); }
                }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state == "stopped" && root.confirm-revoke-connection: HostAction {
                    x: 152px; y: 72px; width: 92px; height: 30px;
                    label: "取消"; default-focus: true;
                    clicked => { root.confirm-revoke-connection = false; }
                }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state != "stopped": Text {
                    x: 32px; y: 75px; width: parent.width - 64px;
                    text: "停止实例后可撤销或更换连接。";
                    color: #8fa0b8; font-size: 11px;
                }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state == "stopped" && root.editing-connection-secret: LineEdit {
                    x: 32px; y: 72px; width: parent.width - 64px; height: 34px;
                    text <=> root.connection-secret;
                    placeholder-text: "输入新的 API Key / Token";
                    input-type: password;
                }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state == "stopped" && root.editing-connection-secret: HostAction {
                    x: 32px; y: 116px; width: 112px; height: 30px;
                    label: "保存新凭证"; primary: true;
                    enabled: root.connection-secret != "";
                    clicked => { root.rotate-connection-credential(); }
                }
                if root.instance-connection-title != "尚未授权连接" && root.instance-state == "stopped" && root.editing-connection-secret: HostAction {
                    x: 152px; y: 116px; width: 92px; height: 30px;
                    label: "取消"; default-focus: true;
                    clicked => { root.connection-secret = ""; root.editing-connection-secret = false; }
                }
                if root.instance-connection-title == "尚未授权连接": LineEdit {
                    x: 32px; y: 94px; width: 142px; height: 34px;
                    text <=> root.connection-provider;
                    placeholder-text: "provider（如 openai）";
                }
                if root.instance-connection-title == "尚未授权连接": LineEdit {
                    x: 182px; y: 94px; width: parent.width - 214px; height: 34px;
                    text <=> root.connection-account;
                    placeholder-text: "账户标识（非秘密）";
                }
                if root.instance-connection-title == "尚未授权连接": LineEdit {
                    x: 32px; y: 140px; width: parent.width - 190px; height: 34px;
                    text <=> root.connection-secret;
                    placeholder-text: "API Key / Token";
                    input-type: password;
                }
                if root.instance-connection-title == "尚未授权连接": Button {
                    x: parent.width - 150px; y: 140px; width: 118px; height: 34px;
                    text: "保存并授权";
                    enabled: root.instance-state == "stopped" && root.connection-provider != "" && root.connection-account != "" && root.connection-secret != "";
                    clicked => { root.add-connection(); }
                }
            }

            if root.selected-instance && root.config-fields.length == 0 && !root.instance-requires-connection && root.instance-state != "failed" && root.instance-state != "starting": Rectangle {
                x: 18px; y: 106px; width: parent.width - 36px; height: 116px;
                background: #1d2430;
                border-width: 1px;
                border-color: #2b3546;
                border-radius: 9px;
                Rectangle { x: 16px; y: 18px; width: 4px; height: 42px; background: #65d899; border-radius: 2px; }
                Text { x: 32px; y: 16px; width: parent.width - 48px; text: "无需配置"; color: #e8eef8; font-size: 15px; font-weight: 700; }
                Text { x: 32px; y: 43px; width: parent.width - 48px; text: "此插件没有可编辑设置。仍可使用底部操作启动、停止、切换版本或删除实例。"; color: #a9b6ca; font-size: 12px; wrap: word-wrap; }
            }

            if root.selected-installation: Rectangle {
                x: 18px; y: 106px; width: parent.width - 36px; height: 118px;
                background: #1d2430;
                border-radius: 7px;
                Text { x: 14px; y: 12px; width: parent.width - 28px; text: "发布者  " + root.installation-publisher; color: #dce5f3; font-size: 12px; overflow: elide; }
                Text { x: 14px; y: 37px; width: parent.width - 28px; text: "信任状态  " + root.installation-trust; color: root.installation-trust == "已验证签名" ? #65d899 : #f0b35f; font-size: 12px; overflow: elide; }
                Text { x: 14px; y: 62px; width: parent.width - 28px; text: "来源  " + root.installation-source; color: #a9b6ca; font-size: 11px; overflow: elide; }
                Text { x: 14px; y: 87px; width: parent.width - 28px; text: "权限  " + root.installation-permissions; color: #a9b6ca; font-size: 11px; overflow: elide; }
            }

            if root.selected-installation && root.confirm-create: Rectangle {
                x: 18px; y: 242px; width: parent.width - 36px; height: 142px;
                background: #29231b;
                border-width: 1px;
                border-color: #735b35;
                border-radius: 9px;
                Text { x: 16px; y: 14px; width: parent.width - 32px; text: "确认创建使用以下宿主能力的实例？"; color: #ffd89b; font-size: 14px; font-weight: 700; overflow: elide; }
                Text { x: 16px; y: 43px; width: parent.width - 32px; text: "信任状态：" + root.installation-trust; color: root.installation-trust == "已验证签名" ? #65d899 : #f0b35f; font-size: 11px; overflow: elide; }
                Text { x: 16px; y: 65px; width: parent.width - 32px; text: root.installation-permission-risk; color: #e6bd79; font-size: 10px; overflow: elide; }
                Text { x: 16px; y: 85px; width: parent.width - 32px; text: "能力：" + root.installation-permissions; color: #d9c7aa; font-size: 11px; wrap: word-wrap; }
                Text { x: 16px; y: 116px; width: parent.width - 32px; text: "只创建停止状态实例；启动时 Broker 仍会复核范围与配额。"; color: #aa9a82; font-size: 10px; wrap: word-wrap; }
            }

            ScrollView {
                x: 18px; y: root.selected-installation ? 238px : 106px; width: parent.width - 36px; height: parent.height - self.y - 68px;
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

            Text {
                visible: root.confirm-delete;
                x: 18px; y: parent.height - 82px; width: parent.width - 36px;
                text: "确认删除这个实例？窗口和保存的布局会一并移除。";
                color: #ff9a9a;
                font-size: 11px;
                overflow: elide;
            }

            Text {
                visible: root.confirm-uninstall;
                x: 18px; y: parent.height - 82px; width: parent.width - 36px;
                text: "确认卸载这个精确版本？其他并存版本不会受影响。";
                color: #ff9a9a;
                font-size: 11px;
                overflow: elide;
            }
            Text {
                visible: root.confirm-rebind;
                x: 18px; y: parent.height - 82px; width: parent.width - 36px;
                text: root.rebind-warning;
                color: #f0b35f;
                font-size: 11px;
                overflow: elide;
            }

            HorizontalLayout {
                x: 18px; y: parent.height - 58px; width: parent.width - 36px; height: 40px;
                spacing: 8px;
                if root.selected-installation && !root.confirm-delete && !root.confirm-uninstall && !root.confirm-create: HostAction { label: "创建实例"; primary: true; clicked => { root.confirm-create = true; } }
                if root.selected-installation && root.confirm-create: HostAction { label: "确认创建"; primary: true; clicked => { root.confirm-create = false; root.create-instance(); } }
                if root.selected-installation && root.confirm-create: HostAction { label: "取消"; default-focus: true; clicked => { root.confirm-create = false; } }
                if root.selected-installation && root.can-uninstall && !root.confirm-uninstall && !root.confirm-create: HostAction { label: "卸载此版本"; danger: true; clicked => { root.confirm-uninstall = true; } }
                if root.selected-installation && root.can-uninstall && root.confirm-uninstall: HostAction { label: "确认卸载"; danger: true; clicked => { root.confirm-uninstall = false; root.uninstall-package(); } }
                if root.selected-installation && root.can-uninstall && root.confirm-uninstall: HostAction { label: "取消"; default-focus: true; clicked => { root.confirm-uninstall = false; } }
                if root.selected-instance && root.can-configure && !root.confirm-delete && !root.confirm-rebind: HostAction { label: "保存配置"; primary: true; clicked => { root.save-config(); } }
                if root.selected-instance && root.can-start && !root.confirm-delete && !root.confirm-rebind: HostAction { label: "启动"; primary: true; clicked => { root.start-instance(); } }
                if root.selected-instance && root.can-stop && !root.confirm-delete && !root.confirm-rebind: HostAction { label: "停止"; clicked => { root.stop-instance(); } }
                if root.selected-instance && root.can-retry && !root.confirm-delete && !root.confirm-rebind: HostAction { label: root.retry-label; primary: true; clicked => { root.retry-instance(); } }
                if root.selected-instance && root.can-rebind && !root.confirm-delete && !root.confirm-rebind: HostAction { label: root.rebind-label; clicked => { root.confirm-rebind = true; } }
                if root.selected-instance && root.can-rebind && root.confirm-rebind: HostAction { label: "确认切换"; primary: true; clicked => { root.confirm-rebind = false; root.rebind-instance(); } }
                if root.selected-instance && root.can-rebind && root.confirm-rebind: HostAction { label: "取消"; default-focus: true; clicked => { root.confirm-rebind = false; } }
                if root.selected-instance && root.can-configure && !root.confirm-delete && !root.confirm-rebind: HostAction { label: "删除"; danger: true; clicked => { root.confirm-delete = true; } }
                if root.selected-instance && root.can-configure && root.confirm-delete: HostAction { label: "确认删除"; danger: true; clicked => { root.confirm-delete = false; root.delete-instance(); } }
                if root.selected-instance && root.can-configure && root.confirm-delete: HostAction { label: "取消"; default-focus: true; clicked => { root.confirm-delete = false; } }
            }
        }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct InstallationRecord {
    plugin_id: String,
    name: String,
    version: String,
    referenced_by: Vec<InstanceId>,
    publisher: String,
    trust: String,
    source: String,
    permissions: String,
    permission_risk: String,
    fields: Vec<ConfigField>,
}

#[derive(Debug, Clone, PartialEq)]
struct InstanceRecord {
    instance: PluginInstance,
    fields: Vec<ConfigField>,
    requires_connection: bool,
    granted_connections: usize,
    granted_connection_records: Vec<ConnectionRecord>,
    migration_target: Option<InstallationRef>,
    migration_permission_diff: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConnectionRecord {
    id: ConnectionId,
    provider: String,
    account: String,
    health: ConnectionHealth,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ControlSnapshot {
    installations: Vec<InstallationRecord>,
    instances: Vec<InstanceRecord>,
    notice: Option<String>,
    autostart: AutostartState,
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
    Install(PathBuf),
    Uninstall {
        plugin_id: String,
        version: String,
    },
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
    Rebind {
        instance_id: InstanceId,
        target_plugin_id: String,
        target_version: String,
    },
    AddConnection {
        instance_id: InstanceId,
        provider: String,
        account: String,
        secret: SecretInput,
    },
    RotateConnectionCredential {
        instance_id: InstanceId,
        connection_id: ConnectionId,
        secret: SecretInput,
    },
    RevokeConnection {
        instance_id: InstanceId,
        connection_id: ConnectionId,
    },
    SetAutostart(bool),
    Delete(InstanceId),
    Stop,
}

struct SecretInput(Vec<u8>);

impl std::fmt::Debug for SecretInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretInput([REDACTED])")
    }
}

impl Drop for SecretInput {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&mut self.0);
    }
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
    supervisor: InstanceSupervisorHandle,
}

impl InstanceControlSurface {
    pub fn start(
        database: PathBuf,
        plugin_store: PathBuf,
        supervisor: InstanceSupervisorHandle,
    ) -> Result<Self, slint::PlatformError> {
        Self::start_with_vault(
            database,
            plugin_store,
            supervisor,
            Arc::new(MemoryCredentialVault::default()),
        )
    }

    pub fn start_with_vault(
        database: PathBuf,
        plugin_store: PathBuf,
        supervisor: InstanceSupervisorHandle,
        vault: Arc<dyn CredentialVault>,
    ) -> Result<Self, slint::PlatformError> {
        let window = PluginControlWindow::new()?;
        let (platform, lifecycle, recovery) = host_runtime_guidance();
        window.set_host_platform_summary(platform.into());
        window.set_host_lifecycle_summary(lifecycle.into());
        window.set_host_recovery_summary(recovery.into());
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
                    vault,
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
        let timer_supervisor = supervisor.clone();
        let mut last_observed = Vec::new();
        let mut rendered_once = false;
        timer.start(TimerMode::Repeated, UI_DRAIN_INTERVAL, move || {
            let mut newest = None;
            while let Ok(value) = snapshot_rx.try_recv() {
                newest = Some(value);
            }
            let mut should_render = !rendered_once;
            if let Some(value) = newest
                && *timer_snapshot.borrow() != value
            {
                *timer_snapshot.borrow_mut() = value;
                normalize_selection(&timer_snapshot, &timer_selection);
                should_render = true;
            }
            let observed = timer_supervisor.observed_snapshot();
            if observed != last_observed {
                last_observed = observed;
                should_render = true;
            }
            if should_render && let Some(window) = weak.upgrade() {
                render_window(
                    &window,
                    &timer_snapshot.borrow(),
                    &timer_fields,
                    timer_selection.borrow().clone(),
                    &timer_supervisor,
                );
                let result = window.window().with_winit_window(raise_always_on_top);
                if let Some(Err(error)) = result {
                    tracing::debug!(%error, "management window z-order refresh skipped");
                }
                rendered_once = true;
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
            supervisor,
        })
    }

    pub fn weak(&self) -> slint::Weak<PluginControlWindow> {
        self.window.as_weak()
    }

    /// 供运行时 Widget 的宿主管理按钮使用：打开控制面并选中来源实例。
    pub fn settings_handler(&self) -> RuntimeSettingsHandler {
        let weak = self.window.as_weak();
        let snapshot = Rc::clone(&self._snapshot);
        let fields = Rc::clone(&self._fields);
        let selection = Rc::clone(&self._selection);
        let supervisor = self.supervisor.clone();
        Rc::new(move |instance_id| {
            let current = snapshot.borrow();
            if let Some(record) = current
                .instances
                .iter()
                .find(|record| record.instance.id() == instance_id)
            {
                *fields.borrow_mut() = record.fields.clone();
                *selection.borrow_mut() = Selection::Instance(instance_id);
            }
            if let Some(window) = weak.upgrade() {
                window.set_confirm_delete(false);
                window.set_confirm_uninstall(false);
                window.set_confirm_rebind(false);
                window.set_confirm_revoke_connection(false);
                window.set_connection_secret("".into());
                window.set_editing_connection_secret(false);
                render_window(
                    &window,
                    &current,
                    &fields,
                    selection.borrow().clone(),
                    &supervisor,
                );
                if let Err(error) = window.show() {
                    tracing::warn!(%error, instance_id = instance_id.0, "management window show failed");
                }
            }
        })
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
    let overview_window = window.as_weak();
    let overview_snapshot = Rc::clone(&snapshot);
    let overview_fields = Rc::clone(&fields);
    let overview_selection = Rc::clone(&selection);
    let overview_supervisor = supervisor.clone();
    window.on_show_overview(move || {
        *overview_selection.borrow_mut() = Selection::None;
        overview_fields.borrow_mut().clear();
        if let Some(window) = overview_window.upgrade() {
            window.set_confirm_delete(false);
            window.set_confirm_uninstall(false);
            window.set_confirm_rebind(false);
            window.set_confirm_create(false);
            window.set_confirm_revoke_connection(false);
            window.set_connection_secret("".into());
            window.set_editing_connection_secret(false);
            render_window(
                &window,
                &overview_snapshot.borrow(),
                &overview_fields,
                Selection::None,
                &overview_supervisor,
            );
        }
    });

    let picker_window = window.as_weak();
    window.on_choose_package(move || {
        let Some(window) = picker_window.upgrade() else {
            return;
        };
        if window.get_picker_busy() {
            return;
        }
        let owner = match window
            .window()
            .with_winit_window(floatile_platform::file_dialog_owner)
        {
            Some(Ok(owner)) => owner,
            Some(Err(error)) => {
                window.set_notice(SharedString::from(format!("FPICKER_OWNER: {error}")));
                window.set_notice_ok(false);
                return;
            }
            None => {
                window.set_notice("FPICKER_OWNER: native window is not ready".into());
                window.set_notice_ok(false);
                return;
            }
        };
        window.set_picker_busy(true);
        let delivery = picker_window.clone();
        let spawn = thread::Builder::new()
            .name("floatile-package-picker".to_owned())
            .spawn(move || {
                let result = floatile_platform::pick_floatile_package(owner);
                if let Err(error) = delivery.upgrade_in_event_loop(move |window| {
                    window.set_picker_busy(false);
                    match result {
                        Ok(Some(path)) => {
                            window.set_package_path(SharedString::from(
                                path.to_string_lossy().as_ref(),
                            ));
                            window.set_notice("已选择插件包，点击安装继续".into());
                            window.set_notice_ok(true);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            window.set_notice(SharedString::from(format!(
                                "FPICKER_PLATFORM: {error}"
                            )));
                            window.set_notice_ok(false);
                        }
                    }
                }) {
                    tracing::debug!(%error, "package picker UI delivery skipped");
                }
            });
        if let Err(error) = spawn {
            window.set_picker_busy(false);
            window.set_notice(SharedString::from(format!("FPICKER_SPAWN: {error}")));
            window.set_notice_ok(false);
        }
    });

    let weak = window.as_weak();
    let install_commands = commands.clone();
    window.on_install_package(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let path = window.get_package_path().trim().to_owned();
        if path.is_empty() {
            return;
        }
        try_command(
            &install_commands,
            ControlCommand::Install(PathBuf::from(path)),
        );
    });

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
            window.set_confirm_revoke_connection(false);
            window.set_connection_secret("".into());
            window.set_editing_connection_secret(false);
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
            window.set_confirm_revoke_connection(false);
            window.set_connection_secret("".into());
            window.set_editing_connection_secret(false);
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

    let connection_commands = commands.clone();
    let connection_snapshot = Rc::clone(&snapshot);
    let connection_selection = Rc::clone(&selection);
    let connection_window = window.as_weak();
    window.on_add_connection(move || {
        let Some(window) = connection_window.upgrade() else {
            return;
        };
        let Some(instance_id) =
            selected_instance_id(&connection_snapshot, connection_selection.borrow().clone())
        else {
            return;
        };
        let provider = window.get_connection_provider().trim().to_ascii_lowercase();
        let account = window.get_connection_account().trim().to_owned();
        let secret = window.get_connection_secret().as_bytes().to_vec();
        window.set_connection_secret("".into());
        try_command(
            &connection_commands,
            ControlCommand::AddConnection {
                instance_id,
                provider,
                account,
                secret: SecretInput(secret),
            },
        );
    });

    let rotate_commands = commands.clone();
    let rotate_snapshot = Rc::clone(&snapshot);
    let rotate_selection = Rc::clone(&selection);
    let rotate_window = window.as_weak();
    window.on_rotate_connection_credential(move || {
        let Some(window) = rotate_window.upgrade() else {
            return;
        };
        let Some(instance_id) =
            selected_instance_id(&rotate_snapshot, rotate_selection.borrow().clone())
        else {
            return;
        };
        let snapshot = rotate_snapshot.borrow();
        let Some(connection_id) = snapshot
            .instances
            .iter()
            .find(|record| record.instance.id() == instance_id)
            .and_then(|record| record.granted_connection_records.first())
            .map(|record| record.id)
        else {
            return;
        };
        let secret = window.get_connection_secret().as_bytes().to_vec();
        window.set_connection_secret("".into());
        window.set_editing_connection_secret(false);
        try_command(
            &rotate_commands,
            ControlCommand::RotateConnectionCredential {
                instance_id,
                connection_id,
                secret: SecretInput(secret),
            },
        );
    });

    let revoke_commands = commands.clone();
    let revoke_snapshot = Rc::clone(&snapshot);
    let revoke_selection = Rc::clone(&selection);
    window.on_revoke_connection(move || {
        let Some(instance_id) =
            selected_instance_id(&revoke_snapshot, revoke_selection.borrow().clone())
        else {
            return;
        };
        let snapshot = revoke_snapshot.borrow();
        let Some(connection_id) = snapshot
            .instances
            .iter()
            .find(|record| record.instance.id() == instance_id)
            .and_then(|record| record.granted_connection_records.first())
            .map(|record| record.id)
        else {
            return;
        };
        try_command(
            &revoke_commands,
            ControlCommand::RevokeConnection {
                instance_id,
                connection_id,
            },
        );
    });

    let autostart_commands = commands.clone();
    window.on_set_autostart(move |enabled| {
        try_command(&autostart_commands, ControlCommand::SetAutostart(enabled));
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
        let requires_authorization = retry_supervisor.observed_snapshot().iter().any(|status| {
            status.instance_id == instance_id && status.code == Some("FPERM_SESSION_REQUIRED")
        });
        let result = if requires_authorization {
            retry_supervisor.authorize_sensitive(instance_id)
        } else {
            retry_supervisor.retry(instance_id)
        };
        if let Err(error) = result {
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

    let uninstall_commands = commands.clone();
    let uninstall_selection = Rc::clone(&selection);
    window.on_uninstall_package(move || {
        let Selection::Installation { plugin_id, version } = uninstall_selection.borrow().clone()
        else {
            return;
        };
        try_command(
            &uninstall_commands,
            ControlCommand::Uninstall { plugin_id, version },
        );
    });

    let rebind_commands = commands.clone();
    let rebind_snapshot = Rc::clone(&snapshot);
    let rebind_selection = Rc::clone(&selection);
    window.on_rebind_instance(move || {
        let Some(instance_id) =
            selected_instance_id(&rebind_snapshot, rebind_selection.borrow().clone())
        else {
            return;
        };
        let snapshot = rebind_snapshot.borrow();
        let Some(record) = snapshot
            .instances
            .iter()
            .find(|record| record.instance.id() == instance_id)
        else {
            return;
        };
        let Some(target) = &record.migration_target else {
            return;
        };
        try_command(
            &rebind_commands,
            ControlCommand::Rebind {
                instance_id,
                target_plugin_id: target.plugin().0.clone(),
                target_version: target.version().to_owned(),
            },
        );
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
                subtitle: SharedString::from(format!(
                    "{} @ {} · {} 个实例",
                    record.plugin_id,
                    record.version,
                    record.referenced_by.len()
                )),
            })
            .collect::<Vec<_>>(),
    )));
    let observed = supervisor.observed_snapshot();
    window.set_diagnostic_summary(SharedString::from(diagnostic_summary(snapshot, &observed)));
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
                        "{} · 期望状态：{}",
                        record.instance.installation().version(),
                        desired_state_label(record.instance.desired_state())
                    )),
                    status: SharedString::from(observed_state_label(state)),
                    status_kind: SharedString::from(state.as_str()),
                    error_code: SharedString::from(
                        status.and_then(|status| status.code).unwrap_or_default(),
                    ),
                }
            })
            .collect::<Vec<_>>(),
    )));
    window.set_config_fields(config_model(&fields.borrow()));
    window.set_notice(SharedString::from(
        snapshot
            .notice
            .as_deref()
            .map(display_notice)
            .unwrap_or_default(),
    ));
    window.set_notice_ok(
        snapshot
            .notice
            .as_deref()
            .is_some_and(|notice| notice.starts_with("OK")),
    );
    window.set_autostart_enabled(snapshot.autostart == AutostartState::Enabled);
    window.set_autostart_available(matches!(
        snapshot.autostart,
        AutostartState::Disabled | AutostartState::Enabled | AutostartState::Stale
    ));
    window.set_autostart_label(
        match snapshot.autostart {
            AutostartState::Disabled | AutostartState::Enabled => {
                "登录 Windows 后自动运行 Floatile"
            }
            AutostartState::Stale => "修复 Windows 开机启动",
            AutostartState::Unavailable => "Windows 开机启动状态暂不可用",
            AutostartState::Unsupported => "此平台暂不支持开机启动",
        }
        .into(),
    );

    window.set_selected_installation(matches!(&selection, Selection::Installation { .. }));
    window.set_selected_instance(matches!(&selection, Selection::Instance(_)));
    window.set_instance_state("".into());
    window.set_instance_error_code("".into());
    window.set_instance_error_title("".into());
    window.set_instance_error_help("".into());
    window.set_instance_requires_connection(false);
    window.set_instance_connection_title("".into());
    window.set_instance_connection_help("".into());
    window.set_instance_connection_status_kind("missing".into());
    let selected_installation_index = match &selection {
        Selection::Installation { plugin_id, version } => snapshot
            .installations
            .iter()
            .position(|record| record.plugin_id == *plugin_id && record.version == *version)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1),
        _ => -1,
    };
    let selected_instance_index = match &selection {
        Selection::Instance(instance_id) => snapshot
            .instances
            .iter()
            .position(|record| record.instance.id() == *instance_id)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1),
        _ => -1,
    };
    window.set_selected_installation_index(selected_installation_index);
    window.set_selected_instance_index(selected_instance_index);
    window.set_can_rebind(false);
    window.set_retry_label("重试".into());
    window.set_rebind_warning("确认切换实例版本？".into());
    match selection {
        Selection::Installation { plugin_id, version } => {
            if let Some(record) = selected_installation(snapshot, &plugin_id, &version) {
                window.set_selection_title(SharedString::from(record.name.as_str()));
                window.set_selection_subtitle(SharedString::from(format!(
                    "{} @ {} · {}",
                    record.plugin_id,
                    record.version,
                    if record.referenced_by.is_empty() {
                        "未被实例使用，可安全卸载".to_owned()
                    } else {
                        format!(
                            "被实例 {} 使用，需先删除或迁移实例",
                            record
                                .referenced_by
                                .iter()
                                .map(|instance| format!("#{}", instance.0))
                                .collect::<Vec<_>>()
                                .join("、")
                        )
                    }
                )));
                window.set_can_uninstall(record.referenced_by.is_empty());
                window.set_installation_publisher(SharedString::from(record.publisher.as_str()));
                window.set_installation_trust(SharedString::from(record.trust.as_str()));
                window.set_installation_source(SharedString::from(record.source.as_str()));
                window
                    .set_installation_permissions(SharedString::from(record.permissions.as_str()));
                window.set_installation_permission_risk(SharedString::from(
                    record.permission_risk.as_str(),
                ));
            }
            set_actions(window, false, false, false, false);
        }
        Selection::Instance(instance_id) => {
            window.set_can_uninstall(false);
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
                let observed_state = observed.map_or_else(
                    || match record.instance.desired_state() {
                        InstanceDesiredState::Running => ObservedInstanceState::Starting,
                        InstanceDesiredState::Stopped => ObservedInstanceState::Stopped,
                    },
                    |status| status.state,
                );
                window.set_instance_state(SharedString::from(observed_state.as_str()));
                window.set_instance_requires_connection(record.requires_connection);
                if record.requires_connection {
                    let has_connection = record.granted_connections > 0;
                    if let Some(connection) = record.granted_connection_records.first() {
                        window.set_instance_connection_title(SharedString::from(format!(
                            "已连接 · {}",
                            connection.provider
                        )));
                        window.set_instance_connection_help(SharedString::from(format!(
                            "账户：{} · 健康状态：{}。凭证仅由宿主使用，不进入插件 State 或界面。",
                            connection.account,
                            connection_health_label(connection.health)
                        )));
                        window.set_instance_connection_status_kind(SharedString::from(
                            connection_health_kind(connection.health),
                        ));
                    } else if has_connection {
                        window.set_instance_connection_title(SharedString::from(format!(
                            "已授权 {} 个连接",
                            record.granted_connections
                        )));
                        window.set_instance_connection_help(
                            "连接详情暂不可用；宿主不会把未验证的 grant 提供给插件。".into(),
                        );
                        window.set_instance_connection_status_kind("unavailable".into());
                    } else {
                        window.set_instance_connection_title("尚未授权连接".into());
                        window.set_instance_connection_help("此插件需要外部数据连接。请在下方填写 provider、非秘密账户标识和凭证；宿主会安全保存并只授权给此实例。".into());
                        window.set_instance_connection_status_kind("missing".into());
                    }
                }
                if let Some(code) = observed.and_then(|status| status.code) {
                    let explanation = explain_failure(code);
                    window.set_instance_error_code(SharedString::from(code));
                    window.set_instance_error_title(SharedString::from(explanation.title));
                    window.set_instance_error_help(SharedString::from(explanation.guidance));
                    if code == "FPERM_SESSION_REQUIRED" {
                        window.set_retry_label("授权并启动".into());
                    }
                }
                let stopped = record.instance.desired_state() == InstanceDesiredState::Stopped;
                window.set_selection_title(SharedString::from(format!(
                    "实例 #{} · {}",
                    record.instance.id().0,
                    record.instance.installation().plugin().0
                )));
                window.set_selection_subtitle(SharedString::from(format!(
                    "{} · generation {}{}",
                    record.instance.installation().version(),
                    record.instance.generation(),
                    record
                        .migration_target
                        .as_ref()
                        .map_or_else(String::new, |target| {
                            format!(" · 可切换到 {}", target.version())
                        })
                )));
                if stopped && let Some(target) = &record.migration_target {
                    window.set_can_rebind(true);
                    window.set_rebind_label(SharedString::from(format!(
                        "切换到 {}",
                        target.version()
                    )));
                    window.set_rebind_warning(SharedString::from(format!(
                        "确认切换到 {}？{}；配置会先按目标版本校验。",
                        target.version(),
                        record
                            .migration_permission_diff
                            .as_deref()
                            .unwrap_or("权限变化未知")
                    )));
                }
                set_actions(window, stopped, !stopped, failed && !stopped, stopped);
            }
        }
        Selection::None => {
            window.set_can_uninstall(false);
            window.set_selection_title("请选择插件或实例".into());
            window.set_selection_subtitle("配置解析与持久化均在后台执行".into());
            set_actions(window, false, false, false, false);
        }
    }
}

fn desired_state_label(state: InstanceDesiredState) -> &'static str {
    match state {
        InstanceDesiredState::Running => "运行",
        InstanceDesiredState::Stopped => "停止",
    }
}

const MAX_DIAGNOSTIC_INSTANCES: usize = 32;

fn diagnostic_summary(snapshot: &ControlSnapshot, observed: &[ObservedInstanceStatus]) -> String {
    let mut lines = vec![
        format!("Floatile {}", env!("CARGO_PKG_VERSION")),
        format!(
            "installations={} instances={}",
            snapshot.installations.len(),
            snapshot.instances.len()
        ),
    ];
    for record in snapshot.instances.iter().take(MAX_DIAGNOSTIC_INSTANCES) {
        let status = observed
            .iter()
            .find(|status| status.instance_id == record.instance.id());
        let observed_state = status.map_or_else(
            || match record.instance.desired_state() {
                InstanceDesiredState::Running => ObservedInstanceState::Starting,
                InstanceDesiredState::Stopped => ObservedInstanceState::Stopped,
            },
            |status| status.state,
        );
        let code = status.and_then(|status| status.code).unwrap_or("none");
        let connection = if !record.requires_connection {
            "not-required"
        } else if let Some(connection) = record.granted_connection_records.first() {
            connection_health_kind(connection.health)
        } else {
            "missing"
        };
        lines.push(format!(
            "#{} {}@{} desired={} observed={} code={} connection={}",
            record.instance.id().0,
            record.instance.installation().plugin().0,
            record.instance.installation().version(),
            record.instance.desired_state().as_str(),
            observed_state.as_str(),
            code,
            connection
        ));
    }
    if snapshot.instances.len() > MAX_DIAGNOSTIC_INSTANCES {
        lines.push(format!(
            "omitted_instances={}",
            snapshot.instances.len() - MAX_DIAGNOSTIC_INSTANCES
        ));
    }
    lines.join("\n")
}

fn observed_state_label(state: ObservedInstanceState) -> &'static str {
    match state {
        ObservedInstanceState::Starting => "启动中",
        ObservedInstanceState::Running => "运行中",
        ObservedInstanceState::Stopped => "已停止",
        ObservedInstanceState::Failed => "启动失败",
    }
}

fn connection_health_label(health: ConnectionHealth) -> &'static str {
    match health {
        ConnectionHealth::Unknown => "尚未检测",
        ConnectionHealth::Healthy => "正常",
        ConnectionHealth::Degraded => "暂时异常",
        ConnectionHealth::Unavailable => "凭证不可用",
        ConnectionHealth::Revoked => "已撤销",
    }
}

fn connection_health_kind(health: ConnectionHealth) -> &'static str {
    match health {
        ConnectionHealth::Unknown => "unknown",
        ConnectionHealth::Healthy => "healthy",
        ConnectionHealth::Degraded => "degraded",
        ConnectionHealth::Unavailable | ConnectionHealth::Revoked => "unavailable",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureExplanation {
    title: &'static str,
    guidance: &'static str,
}

fn explain_failure(code: &str) -> FailureExplanation {
    if code == "FPERM_SESSION_REQUIRED" {
        FailureExplanation {
            title: "需要敏感能力会话授权",
            guidance: "此插件声明 L2 敏感能力。确认授权只对当前宿主会话的本次激活有效；退出或再次激活时会重新询问。",
        }
    } else if code.starts_with("FLOAD_") {
        FailureExplanation {
            title: "插件安装无法加载",
            guidance: "安装内容缺失、损坏或信任校验失败。请检查对应精确版本；修复或重新安装后再重试。",
        }
    } else if code.starts_with("FCONFIG_") {
        FailureExplanation {
            title: "插件配置不兼容",
            guidance: "当前配置未通过插件约束。请修正配置并保存，然后重新启动实例。",
        }
    } else if code == "FINSTANCE_STORE" {
        FailureExplanation {
            title: "实例存储暂时不可用",
            guidance: "Floatile 无法读取持久状态。请确认数据目录可访问后重试；重复失败时重新启动宿主。",
        }
    } else if matches!(code, "FINSTANCE_MISSING" | "FINSTANCE_GENERATION") {
        FailureExplanation {
            title: "实例状态已经变化",
            guidance: "实例在后台刷新期间被修改或移除。请重新选择实例；仍存在时可再次启动。",
        }
    } else if code == "FINSTANCE_SCHEDULE" {
        FailureExplanation {
            title: "启动任务暂时繁忙",
            guidance: "宿主未能安排本次启动。稍后点击重试；若持续出现，请重新启动 Floatile。",
        }
    } else {
        FailureExplanation {
            title: "插件运行失败",
            guidance: "点击重试可重新创建运行会话。若错误码保持不变，请检查插件版本、权限和宿主日志。",
        }
    }
}

fn display_notice(notice: &str) -> String {
    if notice.starts_with("OK") || !notice.contains(':') {
        return notice.to_owned();
    }
    let code = notice.split_once(':').map_or(notice, |(code, _)| code);
    if !code.is_empty()
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        format!("{code} · 操作未完成，请查看状态说明或重试")
    } else {
        "操作未完成，请查看状态说明或重试".to_owned()
    }
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
    vault: Arc<dyn CredentialVault>,
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
        let snapshot = load_snapshot(&store, &plugin_store, notice.clone());
        let _ = snapshot_tx.try_send(snapshot);
        match command_rx.recv_timeout(REFRESH_INTERVAL) {
            Ok(ControlCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                notice = Some(
                    match apply_command_with_vault(&store, &plugin_store, vault.as_ref(), command) {
                        Ok(message) => message.to_owned(),
                        Err(error) => format!("{}: {error}", error.code()),
                    },
                );
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
    let verified_installations = match list_all(plugin_store) {
        Ok(installations) => installations,
        Err(error) => {
            return ControlSnapshot {
                notice: Some(format!("{}: {error}", error.code())),
                ..ControlSnapshot::default()
            };
        }
    };
    let installations = match verified_installations
        .iter()
        .map(|installation| installation_record(store, installation))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(records) => records,
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
                let (fields, requires_connection) =
                    match load_reference(plugin_store, instance.installation()) {
                        Ok(Some(installation)) => {
                            let requires_connection =
                                !installation.manifest.http_templates.is_empty();
                            let fields = match config_schema(&installation) {
                                Ok(schema) => form_fields(schema.as_ref(), Some(instance.config())),
                                Err(error) => {
                                    instance_notice.get_or_insert_with(|| {
                                        format!("{}: {error}", error.code())
                                    });
                                    Vec::new()
                                }
                            };
                            (fields, requires_connection)
                        }
                        Ok(None) => {
                            instance_notice.get_or_insert_with(|| {
                                "FINSTANCE_INSTALLATION_MISSING: installation is missing".to_owned()
                            });
                            (Vec::new(), false)
                        }
                        Err(error) => {
                            instance_notice
                                .get_or_insert_with(|| format!("{}: {error}", error.code()));
                            (Vec::new(), false)
                        }
                    };
                let granted_connection_records =
                    load_connection_records(store, instance.id(), &mut instance_notice);
                let granted_connections = granted_connection_records.len();
                let target_installation = verified_installations.iter().find(|installation| {
                    installation.meta.id == instance.installation().plugin().0
                        && installation.reference().is_ok_and(|reference| {
                            reference != *instance.installation()
                                && installation.validate_config(instance.config()).is_ok()
                        })
                });
                let migration_target =
                    target_installation.and_then(|installation| installation.reference().ok());
                let migration_permission_diff = target_installation.map(|target| {
                    verified_installations
                        .iter()
                        .find(|current| {
                            current
                                .reference()
                                .is_ok_and(|reference| reference == *instance.installation())
                        })
                        .map_or_else(
                            || "无法读取当前版本权限声明".to_owned(),
                            |current| {
                                describe_permission_diff(
                                    &current.manifest.permissions,
                                    &target.manifest.permissions,
                                )
                            },
                        )
                });
                InstanceRecord {
                    instance,
                    fields,
                    requires_connection,
                    granted_connections,
                    granted_connection_records,
                    migration_target,
                    migration_permission_diff,
                }
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
        autostart: current_autostart_state(),
    }
}

fn current_autostart_state() -> AutostartState {
    let Ok(executable) = std::env::current_exe() else {
        return AutostartState::Unavailable;
    };
    autostart_state(&executable).unwrap_or(AutostartState::Unavailable)
}

fn installation_record(
    store: &floatile_store::Store,
    installation: &InstalledInstallation,
) -> Result<InstallationRecord, ControlError> {
    let schema = config_schema(installation)?;
    let referenced_by = store
        .instances()
        .referencing_installation(&installation.reference()?)?;
    Ok(InstallationRecord {
        plugin_id: installation.meta.id.clone(),
        name: installation.manifest.name.clone(),
        version: installation.meta.version.clone(),
        referenced_by,
        publisher: format!(
            "{} ({})",
            installation.manifest.publisher.name, installation.manifest.publisher.id
        ),
        trust: match installation.meta.trust {
            floatile_core::install::InstallationTrust::Trusted => "已验证签名".to_owned(),
            floatile_core::install::InstallationTrust::Unsigned => {
                "未签名 · 仅限本地开发".to_owned()
            }
        },
        source: installation.meta.source.clone(),
        permissions: if installation.manifest.permissions.is_empty() {
            "无宿主能力".to_owned()
        } else {
            installation
                .manifest
                .permissions
                .iter()
                .map(|permission| permission.capability.as_str())
                .collect::<Vec<_>>()
                .join("、")
        },
        permission_risk: permission_risk_summary(&installation.manifest.permissions),
        fields: form_fields(schema.as_ref(), None),
    })
}

fn load_connection_records(
    store: &floatile_store::Store,
    instance_id: InstanceId,
    notice: &mut Option<String>,
) -> Vec<ConnectionRecord> {
    let grants = match store.connections().grants_for_instance(instance_id) {
        Ok(grants) => grants,
        Err(error) => {
            notice.get_or_insert_with(|| format!("FINSTANCE_CONNECTION: {error}"));
            return Vec::new();
        }
    };
    let mut records = Vec::with_capacity(grants.len());
    for grant in grants {
        match store.connections().get(grant.connection_id) {
            Ok(Some(connection)) => records.push(ConnectionRecord {
                id: connection.id(),
                provider: connection.provider().to_owned(),
                account: connection.account_identity().to_owned(),
                health: connection.health(),
            }),
            Ok(None) => {
                notice.get_or_insert_with(|| {
                    "FINSTANCE_CONNECTION: grant references a missing Connection".to_owned()
                });
                return Vec::new();
            }
            Err(error) => {
                notice.get_or_insert_with(|| format!("FINSTANCE_CONNECTION: {error}"));
                return Vec::new();
            }
        }
    }
    records
}

fn permission_risk_summary(permissions: &[floatile_core::manifest::PermissionDecl]) -> String {
    if permissions.is_empty() {
        return "无声明能力 · 仅使用宿主固有的 UI、日志与时钟".to_owned();
    }
    let has_sensitive = permissions.iter().any(|permission| {
        floatile_core::CapabilityId::from_name(&permission.capability).is_some_and(|capability| {
            capability.definition().risk == floatile_core::capability::CapabilityRisk::L2
        })
    });
    if has_sensitive {
        "L2 敏感能力 · 激活前需要独立的会话授权".to_owned()
    } else {
        "L0 低风险能力 · 仍受实例范围、参数与配额限制".to_owned()
    }
}

fn describe_permission_diff(
    current: &[floatile_core::manifest::PermissionDecl],
    target: &[floatile_core::manifest::PermissionDecl],
) -> String {
    let normalize = |permissions: &[floatile_core::manifest::PermissionDecl]| {
        permissions
            .iter()
            .map(|permission| {
                (
                    permission.capability.clone(),
                    permission
                        .params
                        .as_ref()
                        .map_or_else(|| "null".to_owned(), serde_json::Value::to_string),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let current = normalize(current);
    let target = normalize(target);
    let added = target
        .keys()
        .filter(|capability| !current.contains_key(*capability))
        .cloned()
        .collect::<Vec<_>>();
    let removed = current
        .keys()
        .filter(|capability| !target.contains_key(*capability))
        .cloned()
        .collect::<Vec<_>>();
    let changed = target
        .iter()
        .filter_map(|(capability, params)| {
            current
                .get(capability)
                .is_some_and(|current_params| current_params != params)
                .then_some(capability.clone())
        })
        .collect::<Vec<_>>();
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("新增权限 {}", added.join("、")));
    }
    if !removed.is_empty() {
        parts.push(format!("移除权限 {}", removed.join("、")));
    }
    if !changed.is_empty() {
        parts.push(format!("参数变化 {}", changed.join("、")));
    }
    if parts.is_empty() {
        "权限声明无变化".to_owned()
    } else {
        parts.join("；")
    }
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

fn apply_command_with_vault(
    store: &floatile_store::Store,
    plugin_store: &std::path::Path,
    vault: &dyn CredentialVault,
    command: ControlCommand,
) -> Result<&'static str, ControlError> {
    match command {
        ControlCommand::Install(path) => {
            let limits = floatile_cli::PackageLimits::default();
            let bytes = read_package_bounded(&path, limits.max_archive_bytes)?;
            let source = path.to_string_lossy();
            floatile_cli::install_package(&bytes, plugin_store, &source, &limits)?;
            return Ok("OK: 插件安装完成");
        }
        ControlCommand::Uninstall { plugin_id, version } => {
            let report =
                floatile_cli::uninstall_package(plugin_store, store, &plugin_id, &version)?;
            return if report.cleanup_pending.is_some() {
                Ok("OK: 版本已卸载，后台文件清理待重试")
            } else {
                Ok("OK: 插件版本已安全卸载")
            };
        }
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
        ControlCommand::Rebind {
            instance_id,
            target_plugin_id,
            target_version,
        } => {
            let instance = require_instance(store, instance_id)?;
            require_stopped(&instance)?;
            if instance.installation().plugin().0 != target_plugin_id {
                return Err(ControlError::InstallationIdentityMismatch);
            }
            let target = load_exact(plugin_store, &target_plugin_id, &target_version)?
                .ok_or(ControlError::InstallationMissing)?;
            if target.meta.trust == floatile_core::install::InstallationTrust::Trusted {
                floatile_cli::verify_trusted_installation(&target, store)?;
            }
            target.validate_config(instance.config())?;
            let target_reference = target.reference()?;
            if !store.instances().rebind_installation(
                instance_id,
                instance.installation(),
                &target_reference,
                "explicit Windows management action",
                unix_now().max(instance.updated_at()),
            )? {
                return Err(ControlError::ConcurrentUpdate);
            }
            return Ok("OK: 实例版本已切换，启动后生效");
        }
        ControlCommand::AddConnection {
            instance_id,
            provider,
            account,
            secret,
        } => {
            let instance = require_instance(store, instance_id)?;
            require_stopped(&instance)?;
            let installation = load_reference(plugin_store, instance.installation())?
                .ok_or(ControlError::InstallationMissing)?;
            if installation.manifest.http_templates.is_empty() {
                return Err(ControlError::ConnectionNotRequired);
            }
            if !store
                .connections()
                .grants_for_instance(instance_id)?
                .is_empty()
            {
                return Err(ControlError::ConnectionAlreadyGranted);
            }
            let reference = CredentialRef::new(format!(
                "cred://{provider}/instance-{}-{}",
                instance.id().0,
                unix_now()
            ))
            .map_err(|_| ControlError::ConnectionInput)?;
            vault
                .put(&reference, &secret.0)
                .map_err(|_| ControlError::CredentialStore)?;
            let connection =
                match store
                    .connections()
                    .create(&provider, &account, &reference, unix_now())
                {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = vault.delete(&reference);
                        return Err(ControlError::Store(error));
                    }
                };
            if !store
                .connections()
                .grant(instance_id, connection.id(), unix_now())?
            {
                let _ = store.connections().delete_unreferenced(connection.id());
                let _ = vault.delete(&reference);
                return Err(ControlError::ConcurrentUpdate);
            }
            return Ok("OK: Connection 已安全保存并授权给此实例");
        }
        ControlCommand::RotateConnectionCredential {
            instance_id,
            connection_id,
            secret,
        } => {
            let instance = require_instance(store, instance_id)?;
            require_stopped(&instance)?;
            if !store
                .connections()
                .grants_for_instance(instance_id)?
                .iter()
                .any(|grant| grant.connection_id == connection_id)
            {
                return Err(ControlError::ConnectionMissing);
            }
            let connection = store
                .connections()
                .get(connection_id)?
                .ok_or(ControlError::ConnectionMissing)?;
            let next_generation = connection
                .credential_generation()
                .checked_add(1)
                .ok_or(ControlError::CredentialGenerationExhausted)?;
            let reference = CredentialRef::new(format!(
                "cred://{}/connection-{}-generation-{next_generation}-{}",
                connection.provider(),
                connection.id().0,
                unix_now()
            ))
            .map_err(|_| ControlError::ConnectionInput)?;
            vault
                .put(&reference, &secret.0)
                .map_err(|_| ControlError::CredentialStore)?;
            let updated_at = unix_now().max(connection.updated_at());
            match store
                .connections()
                .rotate_credential(connection_id, &reference, updated_at)
            {
                Ok(true) => {}
                Ok(false) => {
                    let _ = vault.delete(&reference);
                    return Err(ControlError::ConcurrentUpdate);
                }
                Err(error) => {
                    let _ = vault.delete(&reference);
                    return Err(ControlError::Store(error));
                }
            }
            return if vault.delete(connection.credential()).is_ok() {
                Ok("OK: Connection 凭证已更新，健康状态将在下次启动后重新检测")
            } else {
                Ok("OK: 新凭证已生效，旧凭证清理待重试")
            };
        }
        ControlCommand::RevokeConnection {
            instance_id,
            connection_id,
        } => {
            let instance = require_instance(store, instance_id)?;
            require_stopped(&instance)?;
            let connection = store
                .connections()
                .get(connection_id)?
                .ok_or(ControlError::ConnectionMissing)?;
            if !store.connections().revoke(instance_id, connection_id)? {
                return Err(ControlError::ConcurrentUpdate);
            }
            if store.connections().delete_unreferenced(connection_id)? {
                return if vault.delete(connection.credential()).is_ok() {
                    Ok("OK: Connection 授权与安全凭证已删除")
                } else {
                    Ok("OK: Connection 已撤销，孤立凭证清理待重试")
                };
            }
            return Ok("OK: 已撤销此实例授权，共享 Connection 保持可用");
        }
        ControlCommand::SetAutostart(enabled) => {
            let executable = std::env::current_exe().map_err(|_| ControlError::Autostart)?;
            set_autostart(&executable, enabled).map_err(|_| ControlError::Autostart)?;
            return Ok(if enabled {
                "OK: 已启用 Windows 开机启动；登录后将在后台运行"
            } else {
                "OK: 已关闭 Windows 开机启动"
            });
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
    Ok("OK: 操作已保存")
}

#[cfg(test)]
fn apply_command(
    store: &floatile_store::Store,
    plugin_store: &std::path::Path,
    command: ControlCommand,
) -> Result<&'static str, ControlError> {
    apply_command_with_vault(
        store,
        plugin_store,
        &MemoryCredentialVault::default(),
        command,
    )
}

fn read_package_bounded(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, ControlError> {
    let file = File::open(path).map_err(|_| ControlError::PackageRead)?;
    let read_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut reader: Take<File> = file.take(read_limit);
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| ControlError::PackageRead)?;
    if bytes.len() > limit {
        return Err(ControlError::PackageTooLarge);
    }
    Ok(bytes)
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
    #[error("package file could not be read")]
    PackageRead,
    #[error("package file exceeds the archive byte limit")]
    PackageTooLarge,
    #[error(transparent)]
    Install(#[from] floatile_cli::InstallError),
    #[error(transparent)]
    Uninstall(#[from] floatile_cli::UninstallError),
    #[error("installation is missing")]
    InstallationMissing,
    #[error("target installation belongs to another plugin")]
    InstallationIdentityMismatch,
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
    #[error("connection input is invalid")]
    ConnectionInput,
    #[error("plugin does not declare an external connection")]
    ConnectionNotRequired,
    #[error("connection is missing")]
    ConnectionMissing,
    #[error("instance already has a connection grant")]
    ConnectionAlreadyGranted,
    #[error("credential store is unavailable")]
    CredentialStore,
    #[error("credential generation is exhausted")]
    CredentialGenerationExhausted,
    #[error("desktop autostart setting is unavailable")]
    Autostart,
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
            Self::PackageRead => "FINSTALL_READ",
            Self::PackageTooLarge => "FPKG_ARCHIVE_TOO_LARGE",
            Self::Install(error) => error.code(),
            Self::Uninstall(error) => error.code(),
            Self::InstallationMissing => "FINSTANCE_INSTALLATION_MISSING",
            Self::InstallationIdentityMismatch => "FINSTANCE_INSTALLATION_IDENTITY",
            Self::InstanceMissing => "FINSTANCE_NOT_FOUND",
            Self::MustBeStopped => "FINSTANCE_MUST_BE_STOPPED",
            Self::ConcurrentUpdate => "FINSTANCE_CONCURRENT_UPDATE",
            Self::SchemaInvalid => "FINSTANCE_CONFIG_SCHEMA_INVALID",
            Self::InvalidField(_) | Self::Config(_) | Self::ConfigValidation(_) => {
                "FINSTANCE_CONFIG_INVALID"
            }
            Self::ConnectionInput => "FCONNECTION_INPUT",
            Self::ConnectionNotRequired => "FCONNECTION_NOT_REQUIRED",
            Self::ConnectionMissing => "FCONNECTION_NOT_FOUND",
            Self::ConnectionAlreadyGranted => "FCONNECTION_ALREADY_GRANTED",
            Self::CredentialStore => "FCONNECTION_CREDENTIAL_STORE",
            Self::CredentialGenerationExhausted => "FCONNECTION_GENERATION_EXHAUSTED",
            Self::Autostart => "FAUTOSTART_UNAVAILABLE",
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

    #[test]
    fn instance_state_labels_are_clear_for_windows_users() {
        assert_eq!(desired_state_label(InstanceDesiredState::Running), "运行");
        assert_eq!(desired_state_label(InstanceDesiredState::Stopped), "停止");
        assert_eq!(
            observed_state_label(ObservedInstanceState::Starting),
            "启动中"
        );
        assert_eq!(
            observed_state_label(ObservedInstanceState::Running),
            "运行中"
        );
        assert_eq!(
            observed_state_label(ObservedInstanceState::Stopped),
            "已停止"
        );
        assert_eq!(
            observed_state_label(ObservedInstanceState::Failed),
            "启动失败"
        );
    }
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
        write_install_version(root, "1.0.0");
    }

    fn write_install_version(root: &std::path::Path, version: &str) {
        write_install_version_with_http(root, version, false);
    }

    fn write_install_version_with_http(root: &std::path::Path, version: &str, with_http: bool) {
        let dir = root.join("dev.floatile.clock").join(version);
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::create_dir_all(dir.join("logic")).unwrap();
        let mut manifest = serde_json::json!({
            "manifestVersion": 1,
            "id": "dev.floatile.clock",
            "name": "Clock",
            "version": version,
            "publisher": { "id": "dev.floatile", "name": "Floatile" },
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "config": { "schema": "config.schema.json" },
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 100, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": []
        });
        if with_http {
            manifest["permissions"] = serde_json::json!([{
                "capability": "network:https",
                "params": { "origins": ["https://api.example.com"] }
            }]);
            manifest["httpTemplates"] = serde_json::json!([{
                "id": "balance",
                "method": "GET",
                "url": "https://api.example.com/v1/balance",
                "queryParams": [],
                "credentialHeader": "authorization",
                "allowedStatuses": [200],
                "maxResponseBytes": 16384,
                "timeoutMs": 5000,
                "cacheTtlMs": 60000,
                "staleIfErrorMs": 300000,
                "maxRetries": 0,
                "retryBaseDelayMs": 250
            }]);
        }
        let manifest = manifest.to_string().into_bytes();
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
            version: version.to_owned(),
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
    fn failure_explanations_are_actionable_and_do_not_expose_details() {
        let load = explain_failure("FLOAD_DIGEST_MISMATCH");
        assert_eq!(load.title, "插件安装无法加载");
        assert!(load.guidance.contains("重新安装"));

        let config = explain_failure("FCONFIG_VALUE_INVALID");
        assert_eq!(config.title, "插件配置不兼容");
        assert!(config.guidance.contains("修正配置"));

        let unknown = explain_failure("FRUNTIME_UNKNOWN");
        assert_eq!(unknown.title, "插件运行失败");
        assert!(unknown.guidance.contains("重试"));
        assert!(!unknown.guidance.contains("FRUNTIME_UNKNOWN"));

        let notice = display_notice("FLOAD_READ: secret local path");
        assert_eq!(notice, "FLOAD_READ · 操作未完成，请查看状态说明或重试");
        assert!(!notice.contains("secret local path"));
        assert_eq!(display_notice("OK: saved"), "OK: saved");
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
    fn package_reader_rejects_missing_and_oversized_input_without_unbounded_read() {
        let root = temp_root("package-reader");
        let missing = read_package_bounded(&root.join("missing.floatile"), 4).unwrap_err();
        assert_eq!(missing.code(), "FINSTALL_READ");

        let package = root.join("large.floatile");
        std::fs::write(&package, b"12345").unwrap();
        let oversized = read_package_bounded(&package, 4).unwrap_err();
        assert_eq!(oversized.code(), "FPKG_ARCHIVE_TOO_LARGE");
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
    fn permission_diff_explains_added_removed_and_changed_capabilities() {
        let permission = |capability: &str, params: Option<serde_json::Value>| {
            floatile_core::manifest::PermissionDecl {
                capability: capability.to_owned(),
                params,
            }
        };
        let current = vec![
            permission("system:cpu", Some(serde_json::json!({ "sampleRateHz": 1 }))),
            permission("system:memory", None),
        ];
        let target = vec![
            permission("system:cpu", Some(serde_json::json!({ "sampleRateHz": 2 }))),
            permission("timer:schedule", None),
        ];
        let description = describe_permission_diff(&current, &target);
        assert_eq!(
            description,
            "新增权限 timer:schedule；移除权限 system:memory；参数变化 system:cpu"
        );
        assert_eq!(
            describe_permission_diff(&current, &current),
            "权限声明无变化"
        );
    }

    #[test]
    fn permission_risk_summary_distinguishes_low_and_sensitive_capabilities() {
        let permission = |capability: &str| floatile_core::manifest::PermissionDecl {
            capability: capability.to_owned(),
            params: None,
        };
        assert!(permission_risk_summary(&[]).contains("无声明能力"));
        assert!(permission_risk_summary(&[permission("system:cpu")]).contains("L0 低风险"));
        assert!(permission_risk_summary(&[permission("network:https")]).contains("L2 敏感能力"));
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
            referenced_by: Vec::new(),
            publisher: String::new(),
            trust: String::new(),
            source: String::new(),
            permissions: String::new(),
            permission_risk: String::new(),
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

    #[test]
    fn connection_command_persists_only_reference_and_grants_stopped_instance() {
        let root = temp_root("connection-command");
        let plugin_store = root.join("plugins");
        write_install_version_with_http(&plugin_store, "1.0.0", true);
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
        let vault = MemoryCredentialVault::default();
        let secret = b"test-secret-never-in-sqlite";

        apply_command_with_vault(
            &store,
            &plugin_store,
            &vault,
            ControlCommand::AddConnection {
                instance_id: instance.id(),
                provider: "example".to_owned(),
                account: "desktop-test".to_owned(),
                secret: SecretInput(secret.to_vec()),
            },
        )
        .unwrap();

        let connection = store.connections().list().unwrap().remove(0);
        assert_eq!(connection.provider(), "example");
        assert_eq!(connection.account_identity(), "desktop-test");
        assert_eq!(
            store
                .connections()
                .grants_for_instance(instance.id())
                .unwrap()
                .len(),
            1
        );
        let mut recovered = Vec::new();
        vault
            .with_secret(connection.credential(), &mut |value| {
                recovered.extend_from_slice(value)
            })
            .unwrap();
        assert_eq!(recovered, secret);
        assert!(matches!(
            apply_command_with_vault(
                &store,
                &plugin_store,
                &vault,
                ControlCommand::AddConnection {
                    instance_id: instance.id(),
                    provider: "example".to_owned(),
                    account: "duplicate".to_owned(),
                    secret: SecretInput(b"duplicate-secret".to_vec()),
                },
            ),
            Err(ControlError::ConnectionAlreadyGranted)
        ));
        assert_eq!(store.connections().list().unwrap().len(), 1);
        let database = std::fs::read(root.join("layout.db")).unwrap();
        assert!(
            !database
                .windows(secret.len())
                .any(|window| window == secret)
        );
        assert_eq!(
            format!("{:?}", SecretInput(secret.to_vec())),
            "SecretInput([REDACTED])"
        );
        store
            .connections()
            .set_health(connection.id(), ConnectionHealth::Degraded, unix_now())
            .unwrap();
        let old_reference = connection.credential().clone();
        let replacement = b"replacement-secret-never-in-sqlite";
        apply_command_with_vault(
            &store,
            &plugin_store,
            &vault,
            ControlCommand::RotateConnectionCredential {
                instance_id: instance.id(),
                connection_id: connection.id(),
                secret: SecretInput(replacement.to_vec()),
            },
        )
        .unwrap();
        let rotated = store.connections().get(connection.id()).unwrap().unwrap();
        assert_eq!(rotated.credential_generation(), 1);
        assert_eq!(rotated.health(), ConnectionHealth::Unknown);
        assert_ne!(rotated.credential(), &old_reference);
        assert_eq!(
            vault.with_secret(&old_reference, &mut |_| {}),
            Err(floatile_services::CredentialError::NotFound)
        );
        let mut recovered_replacement = Vec::new();
        vault
            .with_secret(rotated.credential(), &mut |value| {
                recovered_replacement.extend_from_slice(value)
            })
            .unwrap();
        assert_eq!(recovered_replacement, replacement);
        apply_command_with_vault(
            &store,
            &plugin_store,
            &vault,
            ControlCommand::RevokeConnection {
                instance_id: instance.id(),
                connection_id: connection.id(),
            },
        )
        .unwrap();
        assert!(store.connections().list().unwrap().is_empty());
        assert_eq!(
            vault.with_secret(rotated.credential(), &mut |_| {}),
            Err(floatile_services::CredentialError::NotFound)
        );
    }

    #[test]
    fn diagnostic_summary_is_bounded_and_omits_config_account_and_credentials() {
        let root = temp_root("diagnostic-summary");
        let plugin_store = root.join("plugins");
        write_install_version_with_http(&plugin_store, "1.0.0", true);
        let store = floatile_store::open(root.join("layout.db")).unwrap();
        apply_command(
            &store,
            &plugin_store,
            ControlCommand::Create {
                plugin_id: "dev.floatile.clock".to_owned(),
                version: "1.0.0".to_owned(),
                fields: zone_field("private-zone-value"),
            },
        )
        .unwrap();
        let instance = store.instances().list().unwrap().remove(0);
        apply_command_with_vault(
            &store,
            &plugin_store,
            &MemoryCredentialVault::default(),
            ControlCommand::AddConnection {
                instance_id: instance.id(),
                provider: "private-provider".to_owned(),
                account: "private-account".to_owned(),
                secret: SecretInput(b"private-secret".to_vec()),
            },
        )
        .unwrap();

        let snapshot = load_snapshot(&store, &plugin_store, None);
        let summary = diagnostic_summary(&snapshot, &[]);
        assert!(summary.contains("dev.floatile.clock@1.0.0"));
        assert!(summary.contains("connection=unknown"));
        for forbidden in [
            "private-zone-value",
            "private-provider",
            "private-account",
            "private-secret",
            "cred://",
        ] {
            assert!(!summary.contains(forbidden));
        }
        assert!(summary.len() < 8 * 1024);
    }

    #[test]
    fn stopped_instance_rebinds_only_to_verified_config_compatible_version() {
        let root = temp_root("rebind");
        let plugin_store = root.join("plugins");
        write_install_version(&plugin_store, "1.0.0");
        write_install_version(&plugin_store, "2.0.0");
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
        apply_command(
            &store,
            &plugin_store,
            ControlCommand::Rebind {
                instance_id: instance.id(),
                target_plugin_id: "dev.floatile.clock".to_owned(),
                target_version: "2.0.0".to_owned(),
            },
        )
        .unwrap();
        let rebound = store.instances().get(instance.id()).unwrap().unwrap();
        assert_eq!(rebound.installation().version(), "2.0.0");

        store
            .instances()
            .set_desired_state(instance.id(), InstanceDesiredState::Running, unix_now())
            .unwrap();
        assert!(matches!(
            apply_command(
                &store,
                &plugin_store,
                ControlCommand::Rebind {
                    instance_id: instance.id(),
                    target_plugin_id: "dev.floatile.clock".to_owned(),
                    target_version: "1.0.0".to_owned(),
                },
            ),
            Err(ControlError::MustBeStopped)
        ));
    }
}
