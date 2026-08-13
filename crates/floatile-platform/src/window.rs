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
}
