//! PP-M4 作者预览：用正式 renderer、Slint 窗口和 Wasmtime/Broker runtime 运行临时实例。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use floatile_core::PluginInstance;
use floatile_platform::capability::probe;
use floatile_platform::{WindowOptions, apply_window_options};
use floatile_services::AuditListener;
use serde::Serialize;
use slint::{Timer, TimerMode};

use crate::plugin_manager::InstalledPlugin;
use crate::runtime_ui::{RuntimeUiLifecycleEvent, RuntimeUiSession, spawn_runtime_ui};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewOutcome {
    pub running: bool,
    pub code: String,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("无法初始化窗口后端: {0}")]
    Backend(String),
    #[error("无法调度预览: {0}")]
    Schedule(String),
    #[error("预览事件循环失败: {0}")]
    EventLoop(String),
}

impl PreviewError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Backend(_) => "FPREVIEW_BACKEND",
            Self::Schedule(_) => "FPREVIEW_SCHEDULE",
            Self::EventLoop(_) => "FPREVIEW_EVENT_LOOP",
        }
    }

    pub fn public_detail(&self) -> &'static str {
        match self {
            Self::Backend(_) => "当前环境无法初始化预览窗口后端",
            Self::Schedule(_) => "无法在 UI 线程调度插件预览",
            Self::EventLoop(_) => "插件预览事件循环失败",
        }
    }
}

/// 在当前进程运行一个有界真实窗口预览。调用方应在独立 CLI 进程中调用一次。
pub fn run_preview(
    plugin: InstalledPlugin,
    instance: PluginInstance,
    duration: Duration,
    audit_listener: Option<AuditListener>,
) -> Result<PreviewOutcome, PreviewError> {
    let caps = probe();
    let options = WindowOptions {
        transparent: caps.compositing.is_available(),
        always_on_top: caps.always_on_top.is_available(),
        ..WindowOptions::default()
    };
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(move |attributes| {
            apply_window_options(&options, attributes)
        })
        .select()
        .map_err(|error| PreviewError::Backend(error.to_string()))?;

    let session = Rc::new(RefCell::new(None::<RuntimeUiSession>));
    let outcome = Rc::new(RefCell::new(None::<PreviewOutcome>));
    let session_for_spawn = Rc::clone(&session);
    let outcome_for_spawn = Rc::clone(&outcome);
    slint::spawn_local(async move {
        match spawn_runtime_ui(plugin, instance, caps, audit_listener).await {
            Ok(value) => *session_for_spawn.borrow_mut() = Some(value),
            Err(error) => {
                let detail: String = error.to_string().chars().take(2_048).collect();
                *outcome_for_spawn.borrow_mut() = Some(PreviewOutcome {
                    running: false,
                    code: error.code().to_owned(),
                    detail,
                });
                let _ = slint::quit_event_loop();
            }
        }
    })
    .map_err(|error| PreviewError::Schedule(error.to_string()))?;

    let lifecycle_timer = Timer::default();
    let session_for_poll = Rc::clone(&session);
    let outcome_for_poll = Rc::clone(&outcome);
    lifecycle_timer.start(TimerMode::Repeated, Duration::from_millis(20), move || {
        let event = session_for_poll
            .borrow()
            .as_ref()
            .and_then(RuntimeUiSession::try_lifecycle_event);
        match event {
            Some(RuntimeUiLifecycleEvent::Running) => {
                *outcome_for_poll.borrow_mut() = Some(PreviewOutcome {
                    running: true,
                    code: "ok".to_owned(),
                    detail: "真实宿主预览已进入 running".to_owned(),
                });
            }
            Some(RuntimeUiLifecycleEvent::Failed { code, detail }) => {
                *outcome_for_poll.borrow_mut() = Some(PreviewOutcome {
                    running: false,
                    code: code.to_owned(),
                    detail: detail.chars().take(2_048).collect(),
                });
                let _ = slint::quit_event_loop();
            }
            Some(RuntimeUiLifecycleEvent::Stopped) => {
                let mut result = outcome_for_poll.borrow_mut();
                if result.is_none() {
                    *result = Some(PreviewOutcome {
                        running: false,
                        code: "FPREVIEW_STOPPED".to_owned(),
                        detail: "插件预览在进入 running 前停止".to_owned(),
                    });
                }
                let _ = slint::quit_event_loop();
            }
            None => {}
        }
    });

    let timeout = Timer::default();
    timeout.start(TimerMode::SingleShot, duration, || {
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop().map_err(|error| PreviewError::EventLoop(error.to_string()))?;
    lifecycle_timer.stop();
    timeout.stop();
    drop(session.borrow_mut().take());

    let result = outcome.borrow().clone().unwrap_or(PreviewOutcome {
        running: false,
        code: "FPREVIEW_TIMEOUT".to_owned(),
        detail: "插件未在预览时限内进入 running".to_owned(),
    });
    Ok(result)
}
