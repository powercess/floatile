//! Floatile shell 的可测试编排逻辑。
//!
//! 状态机与降级决策与 UI 无关，可在无 Slint 环境单测；`main.rs` 只负责把事件
//! 接线到这些纯逻辑并驱动 Slint 窗口。

use floatile_core::{LogicalPosition, LogicalSize, SizeConstraints, WidgetMode};

/// 模式切换后的降级结果，供 UI 层决定是否开启点击穿透。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeEffect {
    /// 进入编辑模式：关闭点击穿透，显示宿主控件。
    Edit,
    /// 进入展示模式：按能力开启点击穿透，隐藏宿主控件。
    Show { click_through: bool },
}

/// Shell 模式控制器：维护当前模式并把模式切换映射到宿主行为。
///
/// `click_through_supported` 来自 `floatile-platform` 的能力探测；不支持穿透时
/// 展示模式降级为普通可交互窗口（平台矩阵中 Wayland 的既定降级）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellController {
    pub mode: WidgetMode,
    pub click_through_supported: bool,
}

impl ShellController {
    /// 初始化为编辑模式，便于用户第一时间看到编辑控件。
    pub fn new(click_through_supported: bool) -> Self {
        Self {
            mode: WidgetMode::Edit,
            click_through_supported,
        }
    }

    /// 切换模式并返回切换后应执行的宿主动作。
    pub fn toggle_mode(&mut self) -> ModeEffect {
        self.mode = self.mode.toggle();
        self.current_effect()
    }

    /// 强制恢复编辑模式。恢复热键必须幂等，不能在已处于编辑模式时切到展示模式。
    pub fn restore_edit_mode(&mut self) -> ModeEffect {
        self.mode = WidgetMode::Edit;
        self.current_effect()
    }

    /// 返回当前期望模式对应的宿主动作，用于窗口重映射后重新同步平台状态。
    pub fn current_effect(&self) -> ModeEffect {
        match self.mode {
            WidgetMode::Edit => ModeEffect::Edit,
            WidgetMode::Show => ModeEffect::Show {
                click_through: self.click_through_supported,
            },
        }
    }

    /// 展示模式下当前是否应开启点击穿透。
    pub fn click_through_enabled(&self) -> bool {
        self.mode == WidgetMode::Show && self.click_through_supported
    }
}

/// 缩放手柄拖动产生的新尺寸（已按约束钳制）。
///
/// 由 UI 层计算期望尺寸后调用 `SizeConstraints::clamp` 得出。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResizeRequest {
    pub size: LogicalSize,
}

/// 将期望尺寸按约束钳制为可用的窗口尺寸。
pub fn clamp_size(size: LogicalSize, constraints: &SizeConstraints) -> LogicalSize {
    constraints.clamp(size)
}
/// 判断逻辑像素坐标是否属于宿主窗口拖动区域。
///
/// 原生拖动必须在 Slint 处理按下事件前启动；顶部控件和右下缩放手柄仍交给 Slint。
pub fn is_window_drag_region(
    position: LogicalPosition,
    size: LogicalSize,
    mode: WidgetMode,
) -> bool {
    if mode != WidgetMode::Edit
        || position.x < 0.0
        || position.y < 0.0
        || position.x >= size.width
        || position.y >= size.height
    {
        return false;
    }

    const CONTROL_LEFT: f32 = 8.0;
    const CONTROL_RIGHT: f32 = 176.0;
    const CONTROL_TOP: f32 = 8.0;
    const CONTROL_BOTTOM: f32 = 32.0;
    const RESIZE_HANDLE_SIZE: f32 = 24.0;

    let in_control_strip = position.x >= CONTROL_LEFT
        && position.x < CONTROL_RIGHT
        && position.y >= CONTROL_TOP
        && position.y < CONTROL_BOTTOM;
    let in_resize_handle = position.x >= size.width - RESIZE_HANDLE_SIZE
        && position.y >= size.height - RESIZE_HANDLE_SIZE;

    !in_control_strip && !in_resize_handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_edit_mode() {
        let c = ShellController::new(true);
        assert_eq!(c.mode, WidgetMode::Edit);
        assert!(!c.click_through_enabled());
    }

    #[test]
    fn toggle_to_show_enables_click_through_when_supported() {
        let mut c = ShellController::new(true);
        assert_eq!(
            c.toggle_mode(),
            ModeEffect::Show {
                click_through: true
            }
        );
        assert!(c.click_through_enabled());
    }

    #[test]
    fn toggle_to_show_degrades_without_click_through() {
        let mut c = ShellController::new(false);
        assert_eq!(
            c.toggle_mode(),
            ModeEffect::Show {
                click_through: false
            }
        );
        assert!(!c.click_through_enabled());
    }

    #[test]
    fn toggle_round_trip_returns_to_edit() {
        let mut c = ShellController::new(true);
        c.toggle_mode();
        assert_eq!(c.toggle_mode(), ModeEffect::Edit);
        assert!(!c.click_through_enabled());
    }

    #[test]
    fn edit_recovery_is_idempotent_and_disables_click_through() {
        let mut c = ShellController::new(true);
        assert_eq!(c.restore_edit_mode(), ModeEffect::Edit);
        assert_eq!(c.mode, WidgetMode::Edit);
        c.toggle_mode();
        assert!(c.click_through_enabled());
        assert_eq!(c.restore_edit_mode(), ModeEffect::Edit);
        assert_eq!(c.mode, WidgetMode::Edit);
        assert!(!c.click_through_enabled());
    }

    #[test]
    fn clamp_uses_size_constraints() {
        let constraints = SizeConstraints::default();
        let tiny = LogicalSize {
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(clamp_size(tiny, &constraints), constraints.min);
        let huge = LogicalSize {
            width: 9999.0,
            height: 9999.0,
        };
        assert_eq!(clamp_size(huge, &constraints), constraints.max);
    }

    #[test]
    fn drag_region_excludes_controls_resize_handle_and_show_mode() {
        let size = LogicalSize {
            width: 260.0,
            height: 120.0,
        };

        assert!(is_window_drag_region(
            LogicalPosition { x: 200.0, y: 60.0 },
            size,
            WidgetMode::Edit
        ));
        assert!(!is_window_drag_region(
            LogicalPosition { x: 92.0, y: 20.0 },
            size,
            WidgetMode::Edit
        ));
        assert!(!is_window_drag_region(
            LogicalPosition { x: 248.0, y: 108.0 },
            size,
            WidgetMode::Edit
        ));
        assert!(!is_window_drag_region(
            LogicalPosition { x: 200.0, y: 60.0 },
            size,
            WidgetMode::Show
        ));
    }
}
