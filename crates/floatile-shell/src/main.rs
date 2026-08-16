//! Floatile Host 入口（S2：编辑/展示模式、点击穿透、拖拽与缩放）。
//!
//! P0 验收点 F3/F4/F5/F6 的载体：
//! - Edit 模式显示边框/手柄/设置/删除控件并关闭点击穿透，支持拖拽与缩放；
//! - Show 模式隐藏全部宿主控件并按平台能力开启点击穿透；
//! - Windows 全局热键（Ctrl+Shift+E）在展示模式下切回编辑模式。
//!
//! Windows 上以 GUI 子系统运行，不创建控制台窗口。

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use floatile_core::{LogicalSize, SizeConstraints, WidgetMode};
use floatile_platform::capability::probe;
#[cfg(windows)]
use floatile_platform::{
    Hotkey, HotkeyModifiers, extract_hotkey_id, register_hotkey, remove_window_decorations,
    unregister_hotkey,
};
use floatile_platform::{
    PlatformError, WindowOptions, apply_window_options, process_metrics, resize_window,
    set_always_on_top, set_click_through, start_window_drag,
};
use slint::Timer;
use slint::winit_030::{WinitWindowAccessor, winit};
#[cfg(windows)]
use winit::platform::windows::EventLoopBuilderExtWindows;

slint::slint! {
    export component Clock inherits Window {
        width: 260px;
        height: 120px;
        background: transparent;
        no-frame: true;

        callback drag-start;
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

            TouchArea {
                enabled: root.edit-mode;
                pointer-event(event) => {
                    if (event.kind == PointerEventKind.down) {
                        root.drag-start();
                    }
                }
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
            clicked => { root.settings-clicked(); }
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
            clicked => { root.show-mode(); }
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
            clicked => { root.delete-clicked(); }
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

#[cfg(windows)]
const HOTKEY_ID: u32 = 0x0001;
#[cfg(windows)]
const VK_E: u32 = 0x45; // E 键

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
    tracing::info!(
        kind = ?caps.kind,
        compositing = caps.compositing,
        click_through = caps.click_through,
        always_on_top = caps.always_on_top,
        "platform capability probe"
    );
    if !caps.compositing {
        tracing::warn!(
            kind = ?caps.kind,
            reason = "compositor_not_detected",
            "transparent window disabled"
        );
    }

    let window_options = WindowOptions {
        transparent: caps.compositing,
        always_on_top: caps.always_on_top,
        ..WindowOptions::default()
    };
    let click_through_supported = caps.click_through;

    // 模式控制器（可测试纯逻辑）与热键回调共享。
    let controller = Rc::new(RefCell::new(floatile_shell::ShellController::new(
        click_through_supported,
    )));
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
        let controller_for_hotkey = Rc::clone(&controller);
        let app_for_hotkey = Rc::clone(&hotkey_app);
        if caps.click_through {
            #[allow(unsafe_code)]
            event_loop_builder.with_msg_hook(move |msg| {
                // SAFETY: winit 在派发消息时传入有效 MSG 指针。
                let hotkey = unsafe { extract_hotkey_id(msg) };
                if hotkey != Some(HOTKEY_ID) {
                    return false;
                }
                tracing::info!("global hotkey pressed");
                let mut ctrl = controller_for_hotkey.borrow_mut();
                let effect = ctrl.toggle_mode();
                let mode = match effect {
                    floatile_shell::ModeEffect::Edit => WidgetMode::Edit,
                    floatile_shell::ModeEffect::Show { .. } => WidgetMode::Show,
                };
                if let Some(app) = app_for_hotkey.borrow().as_ref() {
                    apply_mode(app, mode, click_through_supported);
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
    app.set_time_text(now_hhmmss().into());
    if caps.always_on_top {
        schedule_always_on_top(app.as_weak(), Duration::ZERO);
    }
    #[cfg(windows)]
    {
        *hotkey_app.borrow_mut() = Some(app.clone_strong());
    }

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

    // 拖拽（仅编辑模式，由 TouchArea enabled 控制）
    let weak = app.as_weak();
    app.on_drag_start(move || {
        let Some(app) = weak.upgrade() else { return };
        use slint::winit_030::winit::window::Window;
        let started = app
            .window()
            .with_winit_window(|w: &Window| start_window_drag(w));
        match started {
            Some(Ok(())) => tracing::debug!("window drag started"),
            Some(Err(e)) => tracing::warn!("drag_window failed: {e}"),
            None => tracing::warn!("winit window not ready"),
        }
    });

    // 展示模式按钮
    let weak = app.as_weak();
    let ctrl = Rc::clone(&controller);
    app.on_show_mode(move || {
        let Some(app) = weak.upgrade() else { return };
        let effect = ctrl.borrow_mut().toggle_mode();
        let mode = match effect {
            floatile_shell::ModeEffect::Edit => WidgetMode::Edit,
            floatile_shell::ModeEffect::Show { .. } => WidgetMode::Show,
        };
        tracing::info!(mode = ?mode, "show-mode button");
        apply_mode(&app, mode, click_through_supported);
    });

    // 设置/删除：S2 占位，记录事件
    app.on_settings_clicked(|| {
        tracing::info!("settings clicked (placeholder)");
    });
    app.on_delete_clicked(|| {
        tracing::info!("delete clicked (placeholder)");
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
    app.on_resize_up(move || {
        st.borrow_mut().active = false;
    });

    // 时钟定时器
    let weak = app.as_weak();
    let timer = Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            if let Some(app) = weak.upgrade() {
                app.set_time_text(now_hhmmss().into());
            }
        },
    );

    // 注册全局热键（Windows 展示模式下切回编辑）。winit 窗口在事件循环首次迭代后
    // 才创建，因此用 Repeated Timer 重试直到注册成功（成功后置标志跳过）。
    #[cfg(windows)]
    {
        let hotkey = Hotkey {
            id: HOTKEY_ID,
            modifiers: HotkeyModifiers {
                control: true,
                shift: true,
                ..HotkeyModifiers::none()
            },
            virtual_key: VK_E,
        };
        let registered = Rc::new(RefCell::new(false));
        let weak = app.as_weak();
        let reg_flag = Rc::clone(&registered);
        let register_timer = Timer::default();
        register_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(200),
            move || {
                if *reg_flag.borrow() {
                    return;
                }
                let Some(app) = weak.upgrade() else { return };
                use slint::winit_030::winit::window::Window;
                // winit 0.30 顶层窗口的 with_decorations(false) 不生效，需创建后强制移除。
                let deco_result = app
                    .window()
                    .with_winit_window(|w: &Window| remove_window_decorations(w))
                    .unwrap_or(Err(PlatformError::WindowNotReady));
                match deco_result {
                    Ok(()) => tracing::info!("window decorations removed"),
                    Err(e) => tracing::debug!("remove_window_decorations retry: {e}"),
                }
                let hotkey_result = app
                    .window()
                    .with_winit_window(|w: &Window| register_hotkey(w, hotkey))
                    .unwrap_or(Err(PlatformError::WindowNotReady));
                match hotkey_result {
                    Ok(()) => {
                        tracing::info!("global hotkey registered (Ctrl+Shift+E)");
                        *reg_flag.borrow_mut() = true;
                    }
                    Err(e) => tracing::debug!("hotkey registration retry: {e}"),
                }
            },
        );
    }

    tracing::info!("floatile-shell running");
    let run_result = app.run();

    if let Some(perf_sampler) = perf_sampler {
        perf_sampler.stop();
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

fn apply_mode(app: &Clock, mode: WidgetMode, click_through_supported: bool) {
    use slint::winit_030::winit::window::Window;
    let click_through = match mode {
        WidgetMode::Edit => false,
        WidgetMode::Show => click_through_supported,
    };
    app.set_edit_mode(mode == WidgetMode::Edit);
    let result = app
        .window()
        .with_winit_window(|w: &Window| set_click_through(w, click_through))
        .unwrap_or(Err(PlatformError::WindowNotReady));
    match result {
        Ok(()) => tracing::debug!(mode = ?mode, click_through, "mode applied"),
        Err(e) => tracing::warn!("set_click_through failed: {e}"),
    }
}
