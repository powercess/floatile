//! Floatile Host 入口（S1：透明无边框置顶时钟窗口）。
//!
//! P0 验收点 F1/F2/F5 的最小载体。

use std::time::Duration;

use floatile_platform::capability::probe;
use floatile_platform::{WindowOptions, apply_window_options, start_window_drag};
use slint::Timer;
use slint::winit_030::WinitWindowAccessor;

slint::slint! {
    export component Clock inherits Window {
        width: 260px;
        height: 120px;
        background: transparent;

        callback drag-start;

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
                enabled: true;
                pointer-event(event) => {
                    if (event.kind == PointerEventKind.down) {
                        root.drag-start();
                    }
                }
            }
        }

        in property <string> time-text: "00:00:00";
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::level_filters::LevelFilter::INFO.into()),
        )
        .init();

    let caps = probe();
    tracing::info!(kind = ?caps.kind, click_through = caps.click_through, always_on_top = caps.always_on_top, "platform capability probe");

    if !caps.compositing {
        tracing::warn!(kind = ?caps.kind, "transparent window unavailable or unverified; using opaque fallback");
    }
    if !caps.always_on_top {
        tracing::warn!(kind = ?caps.kind, "always-on-top unavailable; using normal window level");
    }

    let window_options = WindowOptions {
        transparent: caps.compositing,
        always_on_top: caps.always_on_top,
        ..WindowOptions::default()
    };
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(move |attrs| {
            apply_window_options(&window_options, attrs)
        })
        .select()?;

    let app = Clock::new()?;
    app.set_time_text(now_hhmmss().into());

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

    tracing::info!("floatile-shell running");
    app.run()?;
    Ok(())
}
