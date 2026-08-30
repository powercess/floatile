//! 管理中心的真实 Slint 指针事件回归测试。
//!
//! 独立测试二进制避免 Slint event loop 在同一进程被重复初始化；Windows 真实桌面与
//! Linux Xvfb 上向管理窗派发按下/释放事件，验证自绘列表项和操作按钮不是“只聚焦”。

#![allow(clippy::expect_used)] // 集成测试：窗口与回调断言需要明确失败原因。

use std::cell::Cell;
use std::rc::Rc;

use floatile_shell::instance_control::{InstanceListItem, PluginControlWindow};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedString, VecModel};

#[cfg(windows)]
fn has_display() -> bool {
    true
}

#[cfg(not(windows))]
fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn click(window: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

#[test]
fn instance_row_and_retry_action_activate_on_one_pointer_click() {
    if !has_display() {
        eprintln!("SKIP: instance control pointer test needs a display backend");
        return;
    }

    let control = PluginControlWindow::new().expect("管理中心应可实例化");
    control.window().set_size(LogicalSize::new(700.0, 480.0));
    control.set_instances(ModelRc::new(VecModel::from(vec![InstanceListItem {
        title: SharedString::from("#2 dev.floatile.system-monitor"),
        subtitle: SharedString::from("0.2.0 · 期望状态：运行"),
        status: SharedString::from("启动失败"),
        status_kind: SharedString::from("failed"),
        error_code: SharedString::from("FLOAD_INSTALLATION_MISSING"),
    }])));

    let selected = Rc::new(Cell::new(0));
    let selected_callback = Rc::clone(&selected);
    control.on_select_instance(move |index| selected_callback.set(index + 1));

    // 左栏实例区从 y=246 开始，首项高度 72px。
    click(control.window(), 140.0, 282.0);
    assert_eq!(selected.get(), 1, "实例卡第一次单击必须立即触发选择");

    control.set_selected_instance(true);
    control.set_can_retry(true);
    let retried = Rc::new(Cell::new(0));
    let retried_callback = Rc::clone(&retried);
    control.on_retry_instance(move || retried_callback.set(retried_callback.get() + 1));

    // 右栏底部操作区 x=328、y=422；仅显示重试时按钮占首个 preferred slot。
    click(control.window(), 380.0, 442.0);
    assert_eq!(retried.get(), 1, "重试按钮第一次单击必须立即触发操作");

    // 单击会把焦点交给同一操作；触摸层置顶不能破坏既有键盘激活路径。
    control.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Space.into(),
    });
    control.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Space.into(),
    });
    assert_eq!(retried.get(), 2, "聚焦后的 Space 必须继续触发操作");
}
