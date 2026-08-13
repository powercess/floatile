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

/// 启动交互式窗口拖拽（WM 拖动，适用于无边框窗口）。
///
/// `window` 为 winit 窗口（通过 Slint `WinitWindowAccessor` 取得）。
/// 需在鼠标按下时调用。
pub fn start_window_drag(window: &winit::window::Window) -> Result<(), PlatformError> {
    window
        .drag_window()
        .map_err(|e| PlatformError::Platform(format!("drag_window: {e}")))
}

/// Windows 点击穿透扩展样式位（不参与 hit-test）。
const WS_EX_TRANSPARENT: isize = 0x0000_0020;
/// Windows 分层窗口扩展样式位（每像素 Alpha，配合透明背景）。
const WS_EX_LAYERED: isize = 0x0008_0000;

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
/// - Windows：通过 `WS_EX_TRANSPARENT` + `WS_EX_LAYERED` 实现，鼠标事件穿透到底层应用；
///   编辑模式等需要交互时调用方必须传入 `false` 恢复。
/// - 其他平台：返回 `PlatformError::Unsupported`，调用方必须按能力矩阵降级，
///   不得静默跳过。
pub fn set_click_through(
    window: &winit::window::Window,
    enabled: bool,
) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::set_click_through(window, enabled)
    }

    #[cfg(not(windows))]
    {
        let _ = (window, enabled);
        Err(PlatformError::Unsupported(
            "click-through is only implemented on Windows",
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
