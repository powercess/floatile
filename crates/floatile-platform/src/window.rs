//! 浮动窗口选项与平台窗口操作。
//!
//! 设计约束：所有窗口系统差异（winit / raw window handle / 平台 API）收敛在本 crate；
//! 业务 crate（floatile-shell 等）只能通过 `WindowOptions` 与这里的函数与窗口系统交互。

use winit::window::WindowAttributes;
use winit::window::WindowLevel;

/// 平台窗口操作错误。
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("窗口尚未由 winit 创建（事件循环未就绪）")]
    WindowNotReady,
    #[error("当前平台不支持该操作: {0}")]
    Unsupported(&'static str),
    #[error("平台操作失败: {0}")]
    Platform(String),
}

/// 浮动窗口配置（S1 支持透明/无边框/置顶）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowOptions {
    /// 背景透明（需要合成器；X11 无合成器时降级为不透明）。
    pub transparent: bool,
    /// 无边框。
    pub decorations: bool,
    /// 永远置顶。
    pub always_on_top: bool,
    /// 初始尺寸（逻辑像素）。
    pub size: (f32, f32),
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            transparent: true,
            decorations: false,
            always_on_top: true,
            size: (260.0, 120.0),
        }
    }
}

/// 将 `WindowOptions` 应用到 winit `WindowAttributes`。
///
/// 用法（floatile-shell 内）：
/// ```ignore
/// let opts = floatile_platform::WindowOptions::default();
/// slint::BackendSelector::new()
///     .with_winit_window_attributes_hook(move |attrs| apply_window_options(&opts, attrs))
///     .select()?;
/// ```
pub fn apply_window_options(opts: &WindowOptions, attrs: WindowAttributes) -> WindowAttributes {
    attrs
        .with_transparent(opts.transparent)
        .with_decorations(opts.decorations)
        .with_window_level(if opts.always_on_top {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        })
        .with_inner_size(winit::dpi::LogicalSize::new(opts.size.0, opts.size.1))
}
/// 在窗口创建后应用置顶级别。
///
/// Slint 会在组件属性同步时重写创建前的 winit `WindowAttributes`，因此宿主在事件循环
/// 启动、原生窗口可用后再次调用本函数。禁用时恢复普通窗口级别。
pub fn set_always_on_top(
    window: &winit::window::Window,
    enabled: bool,
) -> Result<(), PlatformError> {
    window.set_window_level(if enabled {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    });
    Ok(())
}

/// 启动交互式窗口拖拽（WM 拖动，适用于无边框窗口）。
///
/// `window` 为 winit 窗口（通过 Slint `WinitWindowAccessor` 取得）。
/// 需在鼠标按下时调用。
pub fn start_window_drag(window: &winit::window::Window) -> Result<(), PlatformError> {
    window
        .drag_window()
        .map_err(|e| PlatformError::Platform(format!("drag_window: {e}")))
}

/// 调整窗口内容区尺寸（逻辑像素）。
///
/// 尺寸上限以 winit 的 max inner size 为准；调用方应先按 `SizeConstraints` 钳制，
/// 再传入期望尺寸。winit 0.30 使用 `request_inner_size` 请求调整。
pub fn resize_window(
    window: &winit::window::Window,
    size: floatile_core::LogicalSize,
) -> Result<(), PlatformError> {
    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(size.width, size.height));
    Ok(())
}

/// 设置窗口外框位置（虚拟桌面逻辑像素）。
///
/// 调用方传入逻辑像素坐标；本函数按 winit 当前 scale factor 换算为物理像素后
/// 调用 `set_outer_position`，与 `resize_window` 保持同一坐标约定。
///
/// 原生 Wayland 下窗口位置由合成器决定，winit 无法设置；返回显式
/// `Unsupported` 以便调用方记录降级，而不是静默忽略。
pub fn set_window_position(
    window: &winit::window::Window,
    position: floatile_core::LogicalPosition,
) -> Result<(), PlatformError> {
    if crate::capability::probe().kind == crate::capability::PlatformKind::Wayland {
        return Err(PlatformError::Unsupported(
            "window position is controlled by the compositor on Wayland",
        ));
    }
    let scale = window.scale_factor();
    let physical = winit::dpi::PhysicalPosition::new(
        (f64::from(position.x) * scale).round(),
        (f64::from(position.y) * scale).round(),
    );
    window.set_outer_position(physical);
    Ok(())
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Windows 点击穿透扩展样式位（不参与 hit-test）。
const WS_EX_TRANSPARENT: isize = 0x0000_0020;
#[cfg_attr(not(windows), allow(dead_code))]
/// Windows 分层窗口扩展样式位（每像素 Alpha，配合透明背景）。
const WS_EX_LAYERED: isize = 0x0008_0000;

#[cfg_attr(not(windows), allow(dead_code))]
/// 计算启用/禁用点击穿透后的 Windows 扩展窗口样式。
///
/// 启用时叠加 `WS_EX_TRANSPARENT`（穿透）与 `WS_EX_LAYERED`（分层，保证 Alpha 生效）；
/// 禁用时仅清除 `WS_EX_TRANSPARENT`，保留既有样式。
fn ex_style_with_click_through(ex_style: isize, enabled: bool) -> isize {
    if enabled {
        ex_style | WS_EX_TRANSPARENT | WS_EX_LAYERED
    } else {
        ex_style & !WS_EX_TRANSPARENT
    }
}

/// 启用或禁用点击穿透。
///
/// - Windows：通过 `WS_EX_TRANSPARENT` + `WS_EX_LAYERED` 实现，鼠标事件穿透到底层应用。
/// - Linux X11：通过 SHAPE 扩展把 Input region 设为空；禁用时以 `None` 恢复默认区域。
/// - 原生 Wayland/其他平台：返回 `PlatformError::Unsupported`。
///
/// 编辑模式等需要交互时调用方必须传入 `false` 恢复；不得静默跳过失败。
pub fn set_click_through(
    window: &winit::window::Window,
    enabled: bool,
) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::set_click_through(window, enabled)
    }

    #[cfg(target_os = "linux")]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let handle = window.window_handle().map_err(|error| {
            PlatformError::Platform(format!("window handle unavailable: {error}"))
        })?;
        let window_id = match handle.as_raw() {
            RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).map_err(|_| {
                PlatformError::Platform("Xlib window ID exceeds X11 protocol width".into())
            })?,
            RawWindowHandle::Xcb(handle) => handle.window.get(),
            _ => {
                return Err(PlatformError::Unsupported(
                    "click-through requires an X11 window handle on Linux",
                ));
            }
        };
        crate::x11::set_click_through(window_id, enabled)
    }

    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        let _ = (window, enabled);
        Err(PlatformError::Unsupported(
            "click-through is not implemented on this platform",
        ))
    }
}

/// 强制移除窗口标题栏与边框。
///
/// winit 0.30 对顶层窗口的 `with_decorations(false)` 不生效（已实测仍保留
/// `WS_CAPTION`），因此这里在窗口创建后直接修改 Win32 样式：清除标题栏/边框/
/// 系统菜单/尺寸边框并切换为 `WS_POPUP`。仅 Windows 需要；其他平台返回
/// `PlatformError::Unsupported`。
pub fn remove_window_decorations(window: &winit::window::Window) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::remove_window_decorations(window)
    }

    #[cfg(not(windows))]
    {
        let _ = window;
        Err(PlatformError::Unsupported(
            "window decorations removal is only needed on Windows",
        ))
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use super::*;
    use windows_sys::Win32::Foundation::{GetLastError, HWND, SetLastError};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub(super) fn set_click_through(
        window: &winit::window::Window,
        enabled: bool,
    ) -> Result<(), PlatformError> {
        let handle = window
            .window_handle()
            .map_err(|e| PlatformError::Platform(format!("window handle unavailable: {e}")))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err(PlatformError::Unsupported(
                "window handle is not a Win32 HWND on Windows",
            ));
        };
        let hwnd: HWND = handle.hwnd.get();

        // SAFETY: hwnd 是当前 winit 窗口的有效原始句柄，GWL_EXSTYLE 是合法索引。
        let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let new_style = ex_style_with_click_through(ex_style, enabled);

        // SAFETY: 同一有效 hwnd；仅修改扩展样式位，不改变窗口所有权或类。
        // 先清零上次错误码，避免把陈旧的错误当作本调用失败。
        unsafe { SetLastError(0) };
        let _prev = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style) };
        // SAFETY: 读取本调用产生的错误码。
        let err = unsafe { GetLastError() };
        if err != 0 {
            return Err(PlatformError::Platform(format!(
                "SetWindowLongPtrW(GWL_EXSTYLE) failed: {err}"
            )));
        }
        Ok(())
    }

    pub(super) fn remove_window_decorations(
        window: &winit::window::Window,
    ) -> Result<(), PlatformError> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_STYLE, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SetWindowPos, ShowWindow, WS_BORDER, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
            WS_POPUP, WS_SIZEBOX, WS_SYSMENU,
        };

        let handle = window
            .window_handle()
            .map_err(|e| PlatformError::Platform(format!("window handle unavailable: {e}")))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err(PlatformError::Unsupported(
                "window handle is not a Win32 HWND on Windows",
            ));
        };
        let hwnd: HWND = handle.hwnd.get();

        // SAFETY: hwnd 是当前 winit 窗口的有效原始句柄，GWL_STYLE 是合法索引；
        // 这里只清除标题栏/边框相关样式位并切换 WS_POPUP，不改窗口所有权或类。
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
        let borderless = (style as u32 | WS_POPUP)
            & !(WS_CAPTION | WS_BORDER | WS_SYSMENU | WS_SIZEBOX | WS_MAXIMIZEBOX | WS_MINIMIZEBOX);

        // SAFETY: 同一有效 hwnd；仅修改样式位。先清零上次错误码。
        unsafe { SetLastError(0) };
        let _prev = unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, borderless as isize) };
        // SAFETY: 读取本调用产生的错误码。
        let err = unsafe { GetLastError() };
        if err != 0 {
            return Err(PlatformError::Platform(format!(
                "SetWindowLongPtrW(GWL_STYLE) failed: {err}"
            )));
        }

        // SAFETY: 通知窗口管理器样式已变化并强制重绘；保留位置/尺寸/z-order。
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                0,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            )
        };
        // SAFETY: 重显窗口应用新样式。
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_floating() {
        let o = WindowOptions::default();
        assert!(o.transparent);
        assert!(!o.decorations);
        assert!(o.always_on_top);
    }

    #[test]
    fn apply_sets_transparent_and_no_decorations() {
        let o = WindowOptions::default();
        let attrs = apply_window_options(&o, WindowAttributes::default());
        assert!(attrs.transparent());
        assert!(!attrs.decorations);
        assert_eq!(attrs.window_level, WindowLevel::AlwaysOnTop);
    }

    #[test]
    fn click_through_enable_adds_transparent_and_layered() {
        let base = 0x0008_0000 | 0x0001_0000;
        let with = ex_style_with_click_through(base, true);
        assert_eq!(with & WS_EX_TRANSPARENT, WS_EX_TRANSPARENT);
        assert_eq!(with & WS_EX_LAYERED, WS_EX_LAYERED);
        assert_eq!(with & 0x0001_0000, 0x0001_0000);
    }

    #[test]
    fn click_through_disable_keeps_other_styles() {
        let base = WS_EX_TRANSPARENT | WS_EX_LAYERED | 0x0001_0000;
        let without = ex_style_with_click_through(base, false);
        assert_eq!(without & WS_EX_TRANSPARENT, 0);
        assert_eq!(without & WS_EX_LAYERED, WS_EX_LAYERED);
        assert_eq!(without & 0x0001_0000, 0x0001_0000);
    }

    #[test]
    fn click_through_disable_is_idempotent() {
        let base = WS_EX_LAYERED;
        assert_eq!(ex_style_with_click_through(base, false), base);
    }
}
