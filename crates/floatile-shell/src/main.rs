//! Floatile Host 入口（S2：编辑/展示模式、点击穿透、拖拽与缩放）。
//!
//! P0 验收点 F3/F4/F5/F6 的载体：
//! - Edit 模式显示边框/手柄/设置/删除控件并关闭点击穿透，支持拖拽与缩放；
//! - Show 模式隐藏全部宿主控件并按平台能力开启点击穿透；
//! - Windows 与 X11 全局热键（Ctrl+Shift+E）在展示模式下切回编辑模式。
//!
//! Windows 上以 GUI 子系统运行，不创建控制台窗口。

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use floatile_core::capability::{
    CapabilityId, CapabilityParams, EffectiveGrant, Grant, Grants, TrustLevel, narrow_instance,
};
use floatile_core::layout::recover_layout;
use floatile_core::{
    InstanceId, LogicalPosition, LogicalRect, LogicalSize, MonitorLayout, PhysicalSize, PluginId,
    ScaleFactor, SizeConstraints, WidgetMode,
};
use floatile_platform::capability::probe;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use floatile_platform::listen_hotkey;
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
use floatile_platform::{Hotkey, HotkeyModifiers};
use floatile_platform::{
    PlatformError, PlatformKind, WindowOptions, apply_window_options, data_dir, enumerate_monitors,
    process_metrics, resize_window, set_always_on_top, set_click_through, set_window_position,
    start_window_drag, to_monitor_layout,
};
#[cfg(windows)]
use floatile_platform::{
    install_hotkey_message_hook, register_hotkey, remove_window_decorations, unregister_hotkey,
};
use floatile_runtime::{WidgetConfig, WidgetManager};
use floatile_shell::{
    BUILTIN_CLOCK_PLUGIN, CLOCK_INSTANCE_ID, PluginProjection, layout_from_window,
    project_plugin_ui, resolve_plugin_view_state,
};
use floatile_ui_schema::schema::JsonSchema;
use floatile_ui_schema::{UiDocument, validate_document};
use slint::Timer;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};

slint::slint! {
    export component Clock inherits Window {
        width: 260px;
        height: 120px;
        background: transparent;
        no-frame: true;

        callback show-mode;
        callback resize-down(pos-x: float, pos-y: float);
        callback resize-move(pos-x: float, pos-y: float);
        callback resize-up;
        callback settings-clicked;
        callback delete-clicked;

        in-out property <bool> edit-mode: true;
        in property <string> time-text: "00:00:00";

        if edit-mode: Rectangle {
            border-radius: 16px;
            border-width: 1px;
            border-color: #4a90e2;
            background: transparent;
            x: -1px;
            y: -1px;
            width: parent.width + 2px;
            height: parent.height + 2px;
        }

        Rectangle {
            border-radius: 16px;
            background: #1c1f26;
            opacity: 0.92;
            border-width: 1px;
            border-color: #3a3f4b;

            Text {
                text: "Floatile";
                font-size: 11px;
                color: #8b93a7;
                horizontal-alignment: center;
                y: 18px;
            }

            Text {
                text: root.time-text;
                font-size: 34px;
                font-weight: 700;
                color: white;
                horizontal-alignment: center;
                vertical-alignment: center;
                y: 30px;
                height: 60px;
            }

        }

        if edit-mode: TouchArea {
            x: 8px;
            y: 8px;
            width: 52px;
            height: 24px;

            Rectangle {
                width: 52px;
                height: 24px;
                background: #2a2f3a;
                border-radius: 4px;
                border-width: 1px;
                border-color: #4a90e2;

                Text {
                    text: "设置";
                    font-size: 11px;
                    color: white;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
            }
            pointer-event(event) => {
                if (event.kind == PointerEventKind.down) {
                    root.settings-clicked();
                }
            }
        }

        if edit-mode: TouchArea {
            x: 66px;
            y: 8px;
            width: 52px;
            height: 24px;

            Rectangle {
                width: 52px;
                height: 24px;
                background: #2a2f3a;
                border-radius: 4px;
                border-width: 1px;
                border-color: #4a90e2;

                Text {
                    text: "展示";
                    font-size: 11px;
                    color: white;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
            }
            pointer-event(event) => {
                if (event.kind == PointerEventKind.down) {
                    root.show-mode();
                }
            }
        }

        if edit-mode: TouchArea {
            x: 124px;
            y: 8px;
            width: 52px;
            height: 24px;

            Rectangle {
                width: 52px;
                height: 24px;
                background: #2a2f3a;
                border-radius: 4px;
                border-width: 1px;
                border-color: #4a90e2;

                Text {
                    text: "删除";
                    font-size: 11px;
                    color: white;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
            }
            pointer-event(event) => {
                if (event.kind == PointerEventKind.down) {
                    root.delete-clicked();
                }
            }
        }

        if edit-mode: touch := TouchArea {
            width: 24px;
            height: 24px;
            x: parent.width - 24px;
            y: parent.height - 24px;

            Rectangle {
                width: 24px;
                height: 24px;
                background: #2a2f3a;
                border-radius: 4px;
                border-width: 1px;
                border-color: #4a90e2;

                Text {
                    text: "⤢";
                    font-size: 14px;
                    color: white;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
            }

            pointer-event(event) => {
                if (event.kind == PointerEventKind.down) {
                    root.resize-down(touch.mouse-x / 1px, touch.mouse-y / 1px);
                } else if (event.kind == PointerEventKind.move) {
                    root.resize-move(touch.mouse-x / 1px, touch.mouse-y / 1px);
                } else if (event.kind == PointerEventKind.up) {
                    root.resize-up();
                }
            }
        }
    }
}

/// 缩放手柄拖动状态（逻辑像素，由 Rust 侧跟踪起点）。
struct ResizeState {
    active: bool,
    start_size: LogicalSize,
    start_pos: (f32, f32),
}
struct PerfSampler {
    stop: SyncSender<()>,
    worker: JoinHandle<()>,
}

struct RuntimeSession {
    stop: SyncSender<()>,
    worker: JoinHandle<()>,
}

impl PerfSampler {
    fn start() -> std::io::Result<Self> {
        let (stop, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("floatile-perf".into())
            .spawn(move || {
                let mut previous = match process_metrics() {
                    Ok(metrics) => metrics,
                    Err(error) => {
                        tracing::warn!(target: "floatile::perf", %error, "process metrics unavailable");
                        return;
                    }
                };
                let mut sampled_at = Instant::now();

                loop {
                    match receiver.recv_timeout(Duration::from_secs(1)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let current = match process_metrics() {
                                Ok(metrics) => metrics,
                                Err(error) => {
                                    tracing::warn!(
                                        target: "floatile::perf",
                                        %error,
                                        "process metrics sample failed"
                                    );
                                    continue;
                                }
                            };
                            let now = Instant::now();
                            let elapsed = now.duration_since(sampled_at);
                            let cpu_time = current.cpu_time.saturating_sub(previous.cpu_time);
                            let cpu_percent = if elapsed.is_zero() {
                                0.0
                            } else {
                                cpu_time.as_secs_f64() / elapsed.as_secs_f64() * 100.0
                            };
                            let rss_mib = current.rss_bytes as f64 / (1024.0 * 1024.0);
                            tracing::info!(
                                target: "floatile::perf",
                                cpu_percent,
                                rss_bytes = current.rss_bytes,
                                rss_mib,
                                "process metrics sample"
                            );
                            previous = current;
                            sampled_at = now;
                        }
                    }
                }
            })?;

        Ok(Self { stop, worker })
    }

    fn stop(self) {
        let _ = self.stop.try_send(());
        if self.worker.join().is_err() {
            tracing::warn!(target: "floatile::perf", "process metrics worker panicked");
        }
    }
}

/// 构建期由 `floatile_sdk::build::build_ftui` 生成的参考时钟 `widget.ftui`
/// （`build.rs` 固化，单一事实源，不手写第二份 JSON）。
const CLOCK_FTUI_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/clock_ftui.json"));

/// 参考时钟的投影 + 宿主下发的 canonical initial State。
struct ProjectedClock {
    projection: PluginProjection,
    initial_state: serde_json::Value,
}

fn load_clock_projection() -> Option<ProjectedClock> {
    let doc: UiDocument = match serde_json::from_str(CLOCK_FTUI_JSON) {
        Ok(doc) => doc,
        Err(error) => {
            tracing::warn!(%error, "embedded widget.ftui is invalid JSON; falling back to builtin clock");
            return None;
        }
    };
    if let Err(error) = validate_document(&doc) {
        tracing::warn!(%error, "embedded widget.ftui failed validation; falling back to builtin clock");
        return None;
    }
    match project_plugin_ui(&doc) {
        Ok(projection) => Some(ProjectedClock {
            projection,
            initial_state: doc.state.initial.clone(),
        }),
        Err(error) => {
            tracing::warn!(%error, "embedded widget.ftui unsupported by shell projection; falling back to builtin clock");
            None
        }
    }
}

fn clock_wasm_bytes() -> Option<Vec<u8>> {
    let wasm_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target/wasm32-wasip2/debug/floatile_clock_wasm.wasm");
    match std::fs::read(&wasm_path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::warn!(path = %wasm_path.display(), %error, "clock wasm missing; falling back to builtin timer");
            None
        }
    }
}

fn clock_state_schema() -> JsonSchema {
    JsonSchema::Object {
        required: vec![],
        properties: BTreeMap::from([
            (
                "time".into(),
                JsonSchema::String {
                    max_length: Some(32),
                },
            ),
            ("running".into(), JsonSchema::Boolean),
        ]),
        additional_properties: false,
    }
}

fn clock_grants() -> Result<floatile_core::InstanceGrant, floatile_core::CapabilityError> {
    let plugin = Grants {
        plugin: PluginId("dev.floatile.clock".into()),
        trust: TrustLevel::Dev,
        caps: vec![Grant {
            capability: CapabilityId::TimerSchedule,
            params: Some(CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 4,
            }),
            effective: EffectiveGrant::DerivedFromInstall,
        }],
    };
    narrow_instance(
        &plugin,
        InstanceId(1),
        vec![Grant {
            capability: CapabilityId::TimerSchedule,
            params: Some(CapabilityParams::Timer {
                max_per_minute: 60,
                max_active: 4,
            }),
            effective: EffectiveGrant::DerivedFromInstall,
        }],
    )
}

fn spawn_clock_runtime(
    app: slint::Weak<Clock>,
    projection: PluginProjection,
    initial_state: serde_json::Value,
) -> Option<RuntimeSession> {
    let wasm = clock_wasm_bytes()?;
    let plugin = PluginId("dev.floatile.clock".into());
    let (stop, stop_rx) = mpsc::sync_channel::<()>(1);
    let worker = std::thread::Builder::new()
        .name("floatile-runtime-clock".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::warn!(%error, "failed to build tokio runtime; falling back to builtin timer");
                    return;
                }
            };
            runtime.block_on(async move {
                let manager = match WidgetManager::new() {
                    Ok(manager) => manager,
                    Err(error) => {
                        tracing::warn!(%error, "failed to create widget manager; falling back to builtin timer");
                        return;
                    }
                };
                let grants = match clock_grants() {
                    Ok(grants) => grants,
                    Err(error) => {
                        tracing::warn!(%error, "failed to narrow clock grants; falling back to builtin timer");
                        return;
                    }
                };
                let config = WidgetConfig {
                    plugin: plugin.clone(),
                    instance: CLOCK_INSTANCE_ID,
                    wasm,
                    initial_state,
                    state_schema: clock_state_schema(),
                    config_json: "{}".into(),
                    grants,
                };
                let mut handle = match manager.spawn(config) {
                    Ok(handle) => handle,
                    Err(error) => {
                        tracing::warn!(%error, "failed to spawn runtime clock; falling back to builtin timer");
                        return;
                    }
                };
                if let Err(error) = handle.start().await {
                    tracing::warn!(%error, "runtime clock start failed; falling back to builtin timer");
                    let _ = handle.shutdown().await;
                    return;
                }
                tracing::info!(plugin_id = %plugin.0, "runtime clock started");

                loop {
                    match stop_rx.try_recv() {
                        Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    let next = tokio::time::timeout(
                        Duration::from_millis(200),
                        handle.ui_updates().recv(),
                    )
                    .await;
                    let Some(update) = (match next {
                        Ok(update) => update,
                        Err(_) => continue,
                    }) else {
                        break;
                    };
                    match resolve_plugin_view_state(&projection, &update.state) {
                        Ok(view) => {
                            let text = view.time_text;
                            if let Err(error) = app.upgrade_in_event_loop(move |app| {
                                app.set_time_text(text.into());
                            }) {
                                tracing::debug!(%error, "event loop delivery failed; stopping runtime clock bridge");
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(seq = update.seq, %error, "runtime state rejected by shell projection");
                        }
                    }
                }

                if let Err(error) = handle.shutdown().await {
                    tracing::warn!(%error, "runtime clock shutdown failed");
                }
            });
        })
        .ok()?;

    Some(RuntimeSession { stop, worker })
}

fn now_hhmmss() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = now % 60;
    let m = (now / 60) % 60;
    let h = (now / 3600) % 24;
    format!("{h:02}:{m:02}:{s:02}")
}

/// 将 Slint 组件读到的逻辑像素尺寸应用到窗口。
fn apply_size(app: &Clock, size: LogicalSize) {
    use slint::winit_030::winit::window::Window;
    let _ = app
        .window()
        .with_winit_window(|w: &Window| resize_window(w, size))
        .unwrap_or(Err(PlatformError::WindowNotReady));
}
fn schedule_always_on_top(app: slint::Weak<Clock>, delay: Duration) {
    Timer::single_shot(delay, move || {
        let Some(app) = app.upgrade() else { return };
        use slint::winit_030::winit::window::Window;
        match app
            .window()
            .with_winit_window(|window: &Window| set_always_on_top(window, true))
        {
            Some(Ok(())) => tracing::info!("always-on-top applied"),
            Some(Err(error)) => tracing::warn!(%error, "always-on-top failed"),
            None => schedule_always_on_top(app.as_weak(), Duration::from_millis(50)),
        }
    });
}

/// 布局持久化共享状态：SQLite store 与归一后的活动显示器快照。
///
/// `store` 打开失败（数据库损坏/路径不可写）时为 `None`，持久化整体禁用但宿主
/// 继续运行；`monitors` 为空时保存被跳过、恢复使用默认位置。
struct PersistedState {
    store: Option<floatile_store::Store>,
    monitors: Vec<MonitorLayout>,
    scale_factor: f64,
    /// 恢复流程主动移动窗口后会紧跟一个 `Moved` 事件；置位后跳过紧随的一次
    /// Moved 保存，防止 openbox/WM 的初始放置位置污染持久化布局。
    suppress_next_moved_save: bool,
}

impl PersistedState {
    /// 重新枚举显示器并归一为逻辑布局快照。
    ///
    /// 平台不支持（Wayland/Windows/macOS）或查询失败时清空快照，调用方据此
    /// 跳过保存并记录降级，而不是使用伪造的单显示器拓扑。
    fn refresh_monitors(&mut self) {
        match enumerate_monitors() {
            Ok(infos) => {
                let scale_factor = match ScaleFactor::new(self.scale_factor) {
                    Ok(sf) => sf,
                    Err(_) => {
                        // 窗口报告非法 scale factor（不应发生）；回退恒等并留痕。
                        tracing::warn!(
                            scale = self.scale_factor,
                            "invalid window scale factor; assuming 1.0"
                        );
                        ScaleFactor::one()
                    }
                };
                self.monitors = infos
                    .iter()
                    .map(|info| to_monitor_layout(info, scale_factor))
                    .collect();
                tracing::info!(count = self.monitors.len(), "monitor snapshot refreshed");
            }
            Err(error) => {
                self.monitors.clear();
                tracing::warn!(%error, "monitor enumeration failed; layout persistence disabled");
            }
        }
    }
}

/// 当前时间（Unix 秒），用作布局记录时间戳。
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 将窗口移动到恢复目标位置；窗口已在该位置时不发送移动请求。
///
/// 返回是否实际发出了移动请求。调用方应在移动后置位
/// `suppress_next_moved_save`，避免恢复产生的 `Moved` 事件覆盖持久化布局；
/// 位置一致时不置位，防止抑制标志残留吞掉用户下一次拖拽保存。
fn apply_restored_position(
    window: &winit::window::Window,
    position: LogicalPosition,
) -> Result<bool, PlatformError> {
    let scale = window.scale_factor();
    let target = winit::dpi::PhysicalPosition::new(
        (f64::from(position.x) * scale).round() as i32,
        (f64::from(position.y) * scale).round() as i32,
    );
    let needs_move = match window.outer_position() {
        Ok(current) => current.x != target.x || current.y != target.y,
        Err(_) => true,
    };
    if needs_move {
        set_window_position(window, position)?;
    }
    Ok(needs_move)
}

/// 将当前窗口几何与模式写入布局数据库。
///
/// 失败只记录不中断（持久化是尽力而为；显示拓扑不可用、位置由合成器控制时
/// 跳过保存并留痕）。
fn save_layout(window: &winit::window::Window, state: &PersistedState, mode: WidgetMode) {
    let Some(store) = &state.store else {
        tracing::debug!("layout store unavailable; skip save");
        return;
    };
    let scale = window.scale_factor();
    let Ok(position) = window.outer_position() else {
        tracing::warn!("window position unavailable (compositor-controlled); layout not saved");
        return;
    };
    let size = window.inner_size();
    let window_rect = LogicalRect {
        position: LogicalPosition {
            x: position.x as f32 / scale as f32,
            y: position.y as f32 / scale as f32,
        },
        size: LogicalSize {
            width: size.width as f32 / scale as f32,
            height: size.height as f32 / scale as f32,
        },
    };
    let scale_factor = match ScaleFactor::new(scale) {
        Ok(sf) => sf,
        Err(_) => {
            tracing::warn!(scale, "invalid window scale factor; layout not saved");
            return;
        }
    };
    let layout = match layout_from_window(
        CLOCK_INSTANCE_ID,
        PluginId(BUILTIN_CLOCK_PLUGIN.into()),
        floatile_shell::WindowSnapshot {
            rect: window_rect,
            physical_size: PhysicalSize {
                width: size.width,
                height: size.height,
            },
            scale_factor,
            mode,
        },
        &state.monitors,
        unix_now(),
    ) {
        Ok(Some(layout)) => layout,
        Ok(None) => {
            tracing::warn!("no active monitor contains the window; layout not saved");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "invalid layout; not saved");
            return;
        }
    };
    match store.layout().save(&layout) {
        Ok(()) => tracing::info!(
            instance = layout.instance_id.0,
            monitor = ?layout.monitor_key,
            x = layout.rect.position.x,
            y = layout.rect.position.y,
            w = layout.rect.size.width,
            h = layout.rect.size.height,
            mode = ?layout.mode,
            "layout saved"
        ),
        Err(error) => tracing::warn!(%error, "layout save failed"),
    }
}

/// 计算持久化布局在当前显示器拓扑下的落点。
///
/// 返回 `(虚拟桌面逻辑矩形, 保存的模式, 是否降级到主屏)`；无记录、记录无效或
/// 恢复失败时返回 `None`（调用方使用默认位置与当前模式）。
fn restored_placement(
    window: &winit::window::Window,
    state: &PersistedState,
) -> Option<(LogicalRect, WidgetMode, bool)> {
    let store = state.store.as_ref()?;
    let layouts = match store.layout().list() {
        Ok(layouts) => layouts,
        Err(error) => {
            tracing::warn!(%error, "layout load failed");
            return None;
        }
    };
    let layout = layouts
        .iter()
        .find(|layout| layout.instance_id == CLOCK_INSTANCE_ID)?;
    if let Err(error) = layout.validate() {
        tracing::warn!(%error, "persisted layout invalid; using defaults");
        return None;
    }
    match recover_layout(layout, &state.monitors) {
        Ok(recovered) => {
            if recovered.lost_monitor {
                tracing::warn!(
                    key = ?layout.monitor_key,
                    "expected monitor missing; placed on primary (lost_monitor)"
                );
            }
            let _ = window;
            Some((recovered.rect, layout.mode, recovered.lost_monitor))
        }
        Err(error) => {
            tracing::warn!(%error, "layout recovery failed; using defaults");
            None
        }
    }
}

/// 窗口就绪后调度一次布局恢复；窗口尚未创建时按固定间隔重试。
fn schedule_layout_restore(
    app: slint::Weak<Clock>,
    state: Arc<Mutex<PersistedState>>,
    controller: Arc<Mutex<floatile_shell::ShellController>>,
    delay: Duration,
) {
    Timer::single_shot(delay, move || {
        let Some(app) = app.upgrade() else { return };
        use slint::winit_030::winit::window::Window;
        let result = app.window().with_winit_window(|window: &Window| {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.scale_factor = window.scale_factor();
            state.refresh_monitors();
            let placement: Result<Option<(WidgetMode, bool)>, PlatformError> =
                match restored_placement(window, &state) {
                    Some((rect, mode, lost)) => {
                        let moved = apply_restored_position(window, rect.position)?;
                        resize_window(window, rect.size)?;
                        if moved {
                            state.suppress_next_moved_save = true;
                        }
                        Ok(Some((mode, lost)))
                    }
                    None => Ok(None),
                };
            placement
        });
        match result {
            Some(Ok(Some((mode, lost)))) => {
                tracing::info!(mode = ?mode, lost_monitor = lost, "layout restored");
                let mut ctrl = controller
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if ctrl.mode != mode {
                    ctrl.mode = mode;
                    let effect = ctrl.current_effect();
                    drop(ctrl);
                    apply_mode_effect(&app, effect);
                }
            }
            Some(Ok(None)) => {}
            Some(Err(error)) => tracing::warn!(%error, "layout restore failed"),
            None => {
                schedule_layout_restore(app.as_weak(), state, controller, Duration::from_millis(50))
            }
        }
    });
}

#[cfg(any(windows, target_os = "linux"))]
const HOTKEY_ID: u32 = 0x0001;
#[cfg(any(windows, target_os = "linux"))]
const KEY_E: u32 = 0x45;
#[cfg(target_os = "macos")]
const HOTKEY_ID: u32 = 0x0001;
/// Carbon 虚拟键码 `kVK_ANSI_E`（物理 E 键位；区别于 Windows/X11 的 ASCII 0x45）。
#[cfg(target_os = "macos")]
const KEY_E: u32 = 0x0E;
#[cfg(windows)]
const KEY_F12: u32 = 0x7B;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let process_started = Instant::now();
    let perf_enabled = std::env::args_os().any(|arg| arg == "--perf");
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::level_filters::LevelFilter::INFO.into()),
        )
        .init();
    let perf_sampler = if perf_enabled {
        Some(PerfSampler::start()?)
    } else {
        None
    };

    let caps = probe();
    let compositing_available = caps.compositing.is_available();
    let click_through_capable = caps.click_through.is_available();
    let always_on_top_available = caps.always_on_top.is_available();
    tracing::info!(
        kind = ?caps.kind,
        compositing = compositing_available,
        compositing_reason = ?caps.compositing.unavailable_reason(),
        click_through = click_through_capable,
        click_through_reason = ?caps.click_through.unavailable_reason(),
        always_on_top = always_on_top_available,
        always_on_top_reason = ?caps.always_on_top.unavailable_reason(),
        "platform capability probe"
    );
    if !compositing_available {
        tracing::warn!(
            kind = ?caps.kind,
            reason = ?caps.compositing.unavailable_reason(),
            "transparent window disabled"
        );
    }
    if matches!(caps.kind, PlatformKind::X11 | PlatformKind::MacOS) {
        match enumerate_monitors() {
            Ok(monitors) => {
                for monitor in monitors {
                    tracing::info!(
                        key = %monitor.key,
                        key_source = ?monitor.key_source,
                        name = %monitor.name,
                        x = monitor.position.x,
                        y = monitor.position.y,
                        width = monitor.size.width,
                        height = monitor.size.height,
                        physical_width_mm = monitor.physical_size_mm.map(|size| size.width),
                        physical_height_mm = monitor.physical_size_mm.map(|size| size.height),
                        primary = monitor.primary,
                        "platform monitor detected"
                    );
                }
            }
            Err(error) => tracing::warn!(%error, "platform monitor enumeration failed"),
        }
    }

    let window_options = WindowOptions {
        transparent: compositing_available,
        always_on_top: always_on_top_available,
        ..WindowOptions::default()
    };

    // 只有底层能力与恢复热键都成功后，模式控制器才允许启用点击穿透。
    let controller = Arc::new(Mutex::new(floatile_shell::ShellController::new(false)));

    // 布局持久化：数据目录 + SQLite（数据库不可用时降级为无持久化运行）。
    let store = data_dir()
        .and_then(|dir| {
            std::fs::create_dir_all(&dir)
                .map_err(|e| PlatformError::Platform(format!("创建数据目录失败: {e}")))?;
            Ok(dir.join("layout.db"))
        })
        .and_then(|path| {
            floatile_store::open(&path)
                .map_err(|e| PlatformError::Platform(format!("打开布局数据库失败: {e}")))
        });
    let store = match store {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::warn!(%error, "layout store unavailable; persistence disabled");
            None
        }
    };
    let persisted = Arc::new(Mutex::new(PersistedState {
        store,
        monitors: Vec::new(),
        scale_factor: 1.0,
        suppress_next_moved_save: false,
    }));
    #[cfg(windows)]
    let hotkey_app = Rc::new(RefCell::new(None::<Clock>));

    #[cfg(windows)]
    let mut event_loop_builder =
        winit::event_loop::EventLoop::<slint::winit_030::SlintEvent>::with_user_event();
    #[cfg(not(windows))]
    let event_loop_builder =
        winit::event_loop::EventLoop::<slint::winit_030::SlintEvent>::with_user_event();
    #[cfg(windows)]
    {
        let controller_for_hotkey = Arc::clone(&controller);
        let app_for_hotkey = Rc::clone(&hotkey_app);
        let persisted_for_hotkey = Arc::clone(&persisted);
        if click_through_capable {
            install_hotkey_message_hook(&mut event_loop_builder, move |hotkey_id| {
                if hotkey_id != HOTKEY_ID {
                    return false;
                }
                tracing::info!("global hotkey pressed");
                let effect = controller_for_hotkey
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .restore_edit_mode();
                if let Some(app) = app_for_hotkey.borrow().as_ref() {
                    apply_mode_effect(app, effect);
                    let _ = app
                        .window()
                        .with_winit_window(|window: &winit::window::Window| {
                            let state = persisted_for_hotkey
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            save_layout(window, &state, WidgetMode::Edit);
                        });
                }
                true
            });
        }
    }

    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(move |attrs| {
            apply_window_options(&window_options, attrs)
        })
        .with_winit_event_loop_builder(event_loop_builder)
        .select()?;

    let app = Clock::new()?;
    let plugin_projection = load_clock_projection();
    app.set_time_text(now_hhmmss().into());
    if let Some(clock) = &plugin_projection
        && let Ok(view) = resolve_plugin_view_state(&clock.projection, &clock.initial_state)
        && !view.time_text.is_empty()
    {
        app.set_time_text(view.time_text.into());
    }
    let runtime_clock = plugin_projection.and_then(|clock| {
        spawn_clock_runtime(app.as_weak(), clock.projection, clock.initial_state)
    });
    if always_on_top_available {
        schedule_always_on_top(app.as_weak(), Duration::ZERO);
    }
    // 窗口就绪后恢复持久化布局（位置/尺寸/模式），并刷新显示器快照。
    schedule_layout_restore(
        app.as_weak(),
        Arc::clone(&persisted),
        Arc::clone(&controller),
        Duration::ZERO,
    );
    #[cfg(windows)]
    {
        *hotkey_app.borrow_mut() = Some(app.clone_strong());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let hotkey_listener =
        if matches!(caps.kind, PlatformKind::X11 | PlatformKind::MacOS) && click_through_capable {
            let hotkey = Hotkey {
                id: HOTKEY_ID,
                modifiers: HotkeyModifiers {
                    control: true,
                    shift: true,
                    ..HotkeyModifiers::none()
                },
                virtual_key: KEY_E,
            };
            let weak = app.as_weak();
            let controller_for_hotkey = Arc::clone(&controller);
            let persisted_for_hotkey = Arc::clone(&persisted);
            match listen_hotkey(hotkey, move || {
                let controller = Arc::clone(&controller_for_hotkey);
                let persisted = Arc::clone(&persisted_for_hotkey);
                if let Err(error) = weak.upgrade_in_event_loop(move |app| {
                    tracing::info!("global hotkey pressed");
                    let effect = controller
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .restore_edit_mode();
                    apply_mode_effect(&app, effect);
                    let _ = app
                        .window()
                        .with_winit_window(|window: &winit::window::Window| {
                            let state = persisted
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            save_layout(window, &state, WidgetMode::Edit);
                        });
                }) {
                    tracing::debug!(%error, "global hotkey event loop delivery failed");
                }
            }) {
                Ok(listener) => {
                    controller
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .click_through_supported = true;
                    tracing::info!("global hotkey registered (Ctrl+Shift+E)");
                    Some(listener)
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "click-through disabled because recovery hotkey registration failed"
                    );
                    None
                }
            }
        } else {
            None
        };

    let weak = app.as_weak();
    let controller_for_window_events = Arc::clone(&controller);
    let persisted_for_window_events = Arc::clone(&persisted);
    let mut cursor_position = None;
    app.window()
        .on_winit_window_event(move |slint_window, event| {
            use slint::winit_030::winit::window::Window;

            if let winit::event::WindowEvent::CursorMoved { position, .. } = event {
                cursor_position = Some(*position);
            }

            // 窗口移动/关闭后持久化布局。恢复流程主动移动窗口产生的 Moved 事件
            // 由 `suppress_next_moved_save` 跳过，避免 WM 初始放置覆盖用户布局。
            if matches!(
                event,
                winit::event::WindowEvent::Moved(_) | winit::event::WindowEvent::CloseRequested
            ) {
                let _ = slint_window.with_winit_window(|window: &Window| {
                    let mut state = persisted_for_window_events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if matches!(event, winit::event::WindowEvent::Moved(_))
                        && state.suppress_next_moved_save
                    {
                        state.suppress_next_moved_save = false;
                        return;
                    }
                    let mode = controller_for_window_events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .mode;
                    save_layout(window, &state, mode);
                });
            }

            if matches!(
                event,
                winit::event::WindowEvent::MouseInput {
                    state: winit::event::ElementState::Pressed,
                    button: winit::event::MouseButton::Left,
                    ..
                }
            ) {
                let mode = controller_for_window_events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .mode;
                if let Some(position) = cursor_position {
                    let scale_factor = slint_window.scale_factor();
                    let physical_size = slint_window.size();
                    let logical_position = LogicalPosition {
                        x: position.x as f32 / scale_factor,
                        y: position.y as f32 / scale_factor,
                    };
                    let logical_size = LogicalSize {
                        width: physical_size.width as f32 / scale_factor,
                        height: physical_size.height as f32 / scale_factor,
                    };

                    if floatile_shell::is_window_drag_region(logical_position, logical_size, mode) {
                        match slint_window.with_winit_window(start_window_drag) {
                            Some(Ok(())) => {
                                tracing::debug!(
                                    x = logical_position.x,
                                    y = logical_position.y,
                                    "window drag started before Slint pointer grab"
                                );
                                return EventResult::PreventDefault;
                            }
                            Some(Err(error)) => tracing::warn!(%error, "drag_window failed"),
                            None => tracing::warn!("winit window not ready"),
                        }
                    }
                }
            }

            // Show 模式退出：穿透开启时靠全局热键；降级态（热键候选全部注册失败、
            // 未开启穿透）时窗口仍可获得键盘焦点，按 Esc 退出展示模式。
            // 鼠标点击只做正常的窗口激活，绝不作为模式切换入口。
            if let winit::event::WindowEvent::KeyboardInput {
                event: key_event, ..
            } = event
                && key_event.state == winit::event::ElementState::Pressed
                && key_event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                let mut controller = controller_for_window_events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if controller.mode == WidgetMode::Show {
                    let effect = controller.restore_edit_mode();
                    drop(controller);
                    if let Some(app) = weak.upgrade() {
                        apply_mode_effect(&app, effect);
                    }
                    tracing::info!("show mode exited by Escape");
                    return EventResult::PreventDefault;
                }
            }

            if matches!(
                event,
                winit::event::WindowEvent::Focused(true)
                    | winit::event::WindowEvent::Occluded(false)
            ) {
                // 显示器拓扑可能变化（插拔/分辨率变更）：刷新快照并重新应用恢复结果。
                let _ = slint_window.with_winit_window(|window: &Window| {
                    let mut state = persisted_for_window_events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.scale_factor = window.scale_factor();
                    state.refresh_monitors();
                    if let Some((rect, mode, lost)) = restored_placement(window, &state) {
                        let moved = match apply_restored_position(window, rect.position) {
                            Ok(moved) => moved,
                            Err(error) => {
                                tracing::warn!(%error, "layout position re-apply failed");
                                return;
                            }
                        };
                        let size_result = resize_window(window, rect.size);
                        if moved {
                            state.suppress_next_moved_save = true;
                        }
                        if size_result.is_ok() {
                            tracing::info!(
                                mode = ?mode,
                                lost_monitor = lost,
                                "layout re-applied after focus/topology change"
                            );
                        } else {
                            tracing::warn!(
                                size_error = ?size_result.err(),
                                "layout size re-apply failed after focus/topology change"
                            );
                        }
                    }
                });
                let effect = controller_for_window_events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .current_effect();
                if let Some(app) = weak.upgrade() {
                    apply_mode_effect(&app, effect);
                }
            }
            EventResult::Propagate
        });

    if perf_enabled {
        let first_frame_logged = Cell::new(false);
        let frame_count = Cell::new(0_u64);
        let sample_started = Cell::new(process_started);
        app.window().set_rendering_notifier(move |state, _| {
            if !matches!(state, slint::RenderingState::AfterRendering) {
                return;
            }
            if !first_frame_logged.replace(true) {
                tracing::info!(
                    target: "floatile::perf",
                    first_frame_ms = process_started.elapsed().as_secs_f64() * 1000.0,
                    "first frame rendered"
                );
                frame_count.set(0);
                sample_started.set(Instant::now());
                return;
            }

            let frames = frame_count.get() + 1;
            frame_count.set(frames);
            let elapsed = sample_started.get().elapsed();
            if elapsed >= Duration::from_secs(1) {
                tracing::info!(
                    target: "floatile::perf",
                    fps = frames as f64 / elapsed.as_secs_f64(),
                    "render rate sample"
                );
                frame_count.set(0);
                sample_started.set(Instant::now());
            }
        })?;
    }

    let constraints = SizeConstraints::default();
    let resize_state = Rc::new(RefCell::new(ResizeState {
        active: false,
        start_size: LogicalSize {
            width: 260.0,
            height: 120.0,
        },
        start_pos: (0.0, 0.0),
    }));

    // 展示模式按钮：切换模式并持久化。
    let weak = app.as_weak();
    let ctrl = Arc::clone(&controller);
    let persisted_for_mode = Arc::clone(&persisted);
    app.on_show_mode(move || {
        let Some(app) = weak.upgrade() else { return };
        let effect = ctrl
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .toggle_mode();
        let mode = match effect {
            floatile_shell::ModeEffect::Edit => WidgetMode::Edit,
            floatile_shell::ModeEffect::Show { .. } => WidgetMode::Show,
        };
        tracing::info!(mode = ?mode, "show-mode button");
        apply_mode_effect(&app, effect);
        let _ = app
            .window()
            .with_winit_window(|window: &winit::window::Window| {
                let state = persisted_for_mode
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                save_layout(window, &state, mode);
            });
    });

    // 设置：S2 占位，记录事件
    app.on_settings_clicked(|| {
        tracing::info!("settings clicked (placeholder)");
    });

    // 删除：移除持久化记录并关闭窗口（单窗口宿主语义）。
    let weak = app.as_weak();
    let persisted_for_delete = Arc::clone(&persisted);
    app.on_delete_clicked(move || {
        tracing::info!("delete clicked; removing clock layout and closing window");
        if let Some(store) = &persisted_for_delete
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .store
        {
            match store.layout().delete(CLOCK_INSTANCE_ID.0) {
                Ok(()) => tracing::info!("layout deleted"),
                Err(error) => tracing::warn!(%error, "layout delete failed"),
            }
        }
        if let Some(app) = weak.upgrade() {
            let _ = app.window().hide();
        }
    });

    // 缩放手柄：down 记录起点，move 按指针位移精确缩放，clamp 到约束。
    let weak = app.as_weak();
    let st = Rc::clone(&resize_state);
    app.on_resize_down(move |x, y| {
        let Some(app) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        s.active = true;
        s.start_size = current_size(&app);
        s.start_pos = (x, y);
    });
    let weak = app.as_weak();
    let st = Rc::clone(&resize_state);
    app.on_resize_move(move |x, y| {
        let Some(app) = weak.upgrade() else { return };
        let s = st.borrow_mut();
        if !s.active {
            return;
        }
        let delta = ((x - s.start_pos.0).max(0.0), (y - s.start_pos.1).max(0.0));
        let requested = LogicalSize {
            width: s.start_size.width + delta.0,
            height: s.start_size.height + delta.1,
        };
        let clamped = constraints.clamp(requested);
        apply_size(&app, clamped);
    });
    let st = Rc::clone(&resize_state);
    let weak = app.as_weak();
    let ctrl = Arc::clone(&controller);
    let persisted_for_resize = Arc::clone(&persisted);
    app.on_resize_up(move || {
        st.borrow_mut().active = false;
        let Some(app) = weak.upgrade() else { return };
        let _ = app
            .window()
            .with_winit_window(|window: &winit::window::Window| {
                let state = persisted_for_resize
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mode = ctrl
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .mode;
                save_layout(window, &state, mode);
            });
    });

    // 回退路径：只有 runtime clock 未启动时才使用内建时钟文本。
    let fallback_builtin_timer = runtime_clock.is_none();
    let weak = app.as_weak();
    let timer = Timer::default();
    if fallback_builtin_timer {
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(1),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.set_time_text(now_hhmmss().into());
                }
            },
        );
    }

    // 注册全局热键（Windows 展示模式下切回编辑）。winit 窗口在事件循环首次迭代后
    // 才创建，因此用 Repeated Timer 重试直到注册成功。装饰移除与热键注册各自
    // 一次性收敛：成功后不再重复执行，避免每次 tick 重改窗口样式
    // （SetWindowLongPtrW + ShowWindow 隐藏/重现）造成持续频闪。
    #[cfg(windows)]
    let register_timer = {
        // 200 ms × 25 = 5 s 的窗口就绪与热键注册预算；耗尽后按降级模型停止重试。
        const MAX_RETRIES: u32 = 25;
        // 恢复热键候选组合，按顺序尝试。默认 Ctrl+Shift+E 被其他程序全局占用
        // （RegisterHotKey 返回 ERROR_HOTKEY_ALREADY_REGISTERED）时自动换下一组，
        // 保证 Show 模式总有可用的恢复热键。
        let hotkey_candidates: [Hotkey; 3] = [
            Hotkey {
                id: HOTKEY_ID,
                modifiers: HotkeyModifiers {
                    control: true,
                    shift: true,
                    ..HotkeyModifiers::none()
                },
                virtual_key: KEY_E,
            },
            Hotkey {
                id: HOTKEY_ID,
                modifiers: HotkeyModifiers {
                    control: true,
                    alt: true,
                    ..HotkeyModifiers::none()
                },
                virtual_key: KEY_E,
            },
            Hotkey {
                id: HOTKEY_ID,
                modifiers: HotkeyModifiers {
                    control: true,
                    shift: true,
                    ..HotkeyModifiers::none()
                },
                virtual_key: KEY_F12,
            },
        ];
        let weak = app.as_weak();
        let controller_for_registration = Arc::clone(&controller);
        let register_timer = Rc::new(Timer::default());
        let timer_for_callback = Rc::clone(&register_timer);
        let decorations_done = Rc::new(RefCell::new(false));
        let hotkey_done = Rc::new(RefCell::new(false));
        let candidate_index = Rc::new(RefCell::new(0usize));
        let retries = Rc::new(RefCell::new(0u32));
        register_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(200),
            move || {
                let Some(app) = weak.upgrade() else { return };
                use slint::winit_030::winit::window::Window;

                let attempts = *retries.borrow();
                if attempts >= MAX_RETRIES {
                    tracing::error!(
                        "window decorations/hotkey setup failed after {attempts} retries; \
                         click-through stays disabled, Show mode exits by pressing Escape"
                    );
                    timer_for_callback.stop();
                    return;
                }

                let mut pending = false;
                if !*decorations_done.borrow() {
                    // winit 0.30 顶层窗口的 with_decorations(false) 不生效，需创建后强制移除；
                    // 首次成功后不再重放，防止隐藏/重现窗口导致频闪。
                    let deco_result = app
                        .window()
                        .with_winit_window(|w: &Window| remove_window_decorations(w))
                        .unwrap_or(Err(PlatformError::WindowNotReady));
                    match deco_result {
                        Ok(()) => {
                            tracing::info!("window decorations removed");
                            *decorations_done.borrow_mut() = true;
                        }
                        Err(error) => {
                            pending = true;
                            tracing::debug!("remove_window_decorations retry: {error}");
                        }
                    }
                }

                if !*hotkey_done.borrow() {
                    let index = *candidate_index.borrow();
                    if index >= hotkey_candidates.len() {
                        tracing::error!(
                            "no global hotkey candidate registered \
                             (Ctrl+Shift+E, Ctrl+Alt+E, Ctrl+Shift+F12 all unavailable); \
                             click-through stays disabled, Show mode exits by pressing Escape"
                        );
                        *hotkey_done.borrow_mut() = true;
                    } else {
                        let hotkey_result = app
                            .window()
                            .with_winit_window(|w: &Window| {
                                register_hotkey(w, hotkey_candidates[index])
                            })
                            .unwrap_or(Err(PlatformError::WindowNotReady));
                        match hotkey_result {
                            Ok(()) => {
                                controller_for_registration
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .click_through_supported = true;
                                tracing::info!(
                                    "global hotkey registered ({})",
                                    hotkey_label(index)
                                );
                                *hotkey_done.borrow_mut() = true;
                            }
                            Err(error) => {
                                *candidate_index.borrow_mut() = index + 1;
                                pending = true;
                                tracing::debug!(
                                    "hotkey {} registration retry {attempts}: {error}",
                                    hotkey_label(index)
                                );
                            }
                        }
                    }
                }

                if pending {
                    *retries.borrow_mut() = attempts + 1;
                } else {
                    timer_for_callback.stop();
                }
            },
        );
        register_timer
    };

    tracing::info!("floatile-shell running");
    let run_result = app.run();

    if let Some(perf_sampler) = perf_sampler {
        perf_sampler.stop();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(listener) = hotkey_listener {
        listener.stop();
    }

    #[cfg(windows)]
    register_timer.stop();

    if let Some(runtime_clock) = runtime_clock {
        let _ = runtime_clock.stop.try_send(());
        if runtime_clock.worker.join().is_err() {
            tracing::warn!("runtime clock worker panicked");
        }
    }

    // Windows 退出时注销热键。
    #[cfg(windows)]
    let _ = {
        use slint::winit_030::winit::window::Window;
        app.window()
            .with_winit_window(|w: &Window| unregister_hotkey(w, HOTKEY_ID))
    };

    run_result?;
    Ok(())
}

fn current_size(app: &Clock) -> LogicalSize {
    let size = app.window().size();
    LogicalSize {
        width: size.width as f32,
        height: size.height as f32,
    }
}

/// 恢复热键候选的人类可读标签，与 `register_timer` 中候选数组下标一一对应。
#[cfg(windows)]
fn hotkey_label(index: usize) -> &'static str {
    match index {
        0 => "Ctrl+Shift+E",
        1 => "Ctrl+Alt+E",
        2 => "Ctrl+Shift+F12",
        _ => "unknown",
    }
}

fn apply_mode_effect(app: &Clock, effect: floatile_shell::ModeEffect) {
    use slint::winit_030::winit::window::Window;

    let (mode, click_through) = match effect {
        floatile_shell::ModeEffect::Edit => (WidgetMode::Edit, false),
        floatile_shell::ModeEffect::Show { click_through } => (WidgetMode::Show, click_through),
    };
    let edit_mode = mode == WidgetMode::Edit;
    let result = app
        .window()
        .with_winit_window(|window: &Window| set_click_through(window, click_through))
        .unwrap_or(Err(PlatformError::WindowNotReady));
    match result {
        Ok(()) => {
            app.set_edit_mode(edit_mode);
            tracing::debug!(mode = ?mode, click_through, "mode applied");
        }
        Err(error) => tracing::warn!(%error, mode = ?mode, "set_click_through failed"),
    }
}
