//! Floatile shell 的可测试编排逻辑。
//!
//! 状态机与降级决策与 UI 无关，可在无 Slint 环境单测；`main.rs` 只负责把事件
//! 接线到这些纯逻辑并驱动 Slint 窗口。

use floatile_core::layout::LAYOUT_RECORD_VERSION;
use floatile_core::{
    InstanceId, LayoutValidationError, LogicalPosition, LogicalRect, LogicalSize, MonitorLayout,
    PhysicalSize, PluginId, ScaleFactor, SizeConstraints, WidgetLayout, WidgetMode,
};
use floatile_ui_schema::path::PathSegments;
use serde_json::Value;

pub mod plugin_manager;
pub mod runtime_ui;

/// 单窗口宿主内建参考时钟的实例 ID。
pub const CLOCK_INSTANCE_ID: InstanceId = InstanceId(1);
/// 内建参考时钟的插件命名空间（保留前缀，不面向第三方插件）。
pub const BUILTIN_CLOCK_PLUGIN: &str = "builtin.clock";

/// 单实例宿主的固定层级。
const SINGLE_WINDOW_Z: u32 = 1;

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

/// Shell 当前支持的最小插件 UI 投影。
///
/// 这是把已验证 `widget.ftui` 映射到现有 Slint shell 的过渡层：当前仅消费
/// 单文本时钟插件所需的 `Column -> Text($.time)` 形状，不伪装成完整 renderer。
/// renderer 构建期输出的 binding 槽位的宿主消费模型（单一事实源）。
///
/// 宿主的 `slint!` 静态地把生成的 `ClockPluginUI.<prop>` 绑定到宿主属性
/// （参考时钟为 `prop_time: root.time-text`）；runtime 线程沿 `path` 从权威
/// State Patch 提取展示标量并写入宿主属性。`prop` 保留 renderer 的生成属性名，
/// 与 `floatile_renderer::BindingSlot{path,prop}` 逐字段对应，不手写第二份提取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBinding {
    /// State JSONPath（如 `$.time`），由 renderer 从已验证 IR 生成。
    pub path: String,
    /// 生成的宿主属性名（如 `prop_time`），静态映射到 ClockPluginUI 绑定槽位。
    pub prop: String,
}

/// 投影权威 State 到展示标量时的失败。
///
/// 投影是受信任 host 代码：路径由 renderer（已验证 IR）生成，值来自 runtime 的
/// State Patch（已 schema 校验）。缺失/非法字段一律返回错误并交由调用方记录，
/// 绝不 panic，保证任意恶意/超限 patch 都不能拖垮投影路径。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    #[error("State 路径无效: {0}")]
    InvalidPath(String),
    #[error("State 字段 `{0}` 缺失或不可遍历")]
    MissingField(String),
    #[error("binding `{path}` 的值不是字符串")]
    NonString { path: String },
}

/// 把 runtime 的权威 State 沿 renderer binding 槽位路径解析为展示标量字符串。
///
/// 只接受标量 `string` 作为窗口可显示文本；缺字段/非字符串返回错误（失败关闭），
/// 不 panic、不部分投影。
pub fn resolve_binding_string(
    binding: &PluginBinding,
    state: &Value,
) -> Result<String, ProjectionError> {
    let segments = PathSegments::parse(&binding.path)
        .map_err(|e| ProjectionError::InvalidPath(e.to_string()))?;
    let mut current = state;
    for segment in segments.segments() {
        current = current
            .get(segment)
            .ok_or_else(|| ProjectionError::MissingField(segment.clone()))?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ProjectionError::NonString {
            path: binding.path.clone(),
        })
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

/// 窗口当前几何快照（虚拟桌面逻辑矩形 + 物理尺寸 + DPI），供持久化构造使用。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSnapshot {
    /// 虚拟桌面逻辑像素矩形（恢复应用与持久化换算的输入）。
    pub rect: LogicalRect,
    /// 窗口内容区物理像素尺寸（保存时由 winit 报告）。
    pub physical_size: PhysicalSize,
    /// 窗口所在显示器的 scale factor。
    pub scale_factor: ScaleFactor,
    /// 保存时的展示模式。
    pub mode: WidgetMode,
}

/// 从窗口当前虚拟桌面几何与显示器拓扑构造持久化布局记录。
///
/// 保存语义（`WidgetLayout` 契约）：`rect` 必须是 monitor-local——相对期望显示器
/// 工作区原点的逻辑像素；`monitor_key` 取窗口中心所在的活动显示器，找不到时
/// 回退主屏。显示器列表为空时返回 `Ok(None)`，由调用方记录并跳过保存。
pub fn layout_from_window(
    instance_id: InstanceId,
    plugin_id: PluginId,
    snapshot: WindowSnapshot,
    monitors: &[MonitorLayout],
    updated_at: u64,
) -> Result<Option<WidgetLayout>, LayoutValidationError> {
    let WindowSnapshot {
        rect: window_rect,
        physical_size,
        scale_factor,
        mode,
    } = snapshot;
    let center = LogicalPosition {
        x: window_rect.position.x + window_rect.size.width / 2.0,
        y: window_rect.position.y + window_rect.size.height / 2.0,
    };
    let monitor = monitors
        .iter()
        .find(|monitor| contains(monitor.bounds, center))
        .or_else(|| monitors.iter().find(|monitor| monitor.primary));
    let Some(monitor) = monitor else {
        return Ok(None);
    };
    let rect = LogicalRect {
        position: LogicalPosition {
            x: window_rect.position.x - monitor.bounds.position.x,
            y: window_rect.position.y - monitor.bounds.position.y,
        },
        size: window_rect.size,
    };
    let layout = WidgetLayout {
        instance_id,
        plugin_id,
        monitor_key: Some(monitor.key.clone()),
        rect,
        physical_size,
        scale_factor,
        lost_monitor: false,
        z: SINGLE_WINDOW_Z,
        mode,
        version: LAYOUT_RECORD_VERSION,
        updated_at,
    };
    layout.validate()?;
    Ok(Some(layout))
}

fn contains(bounds: LogicalRect, point: LogicalPosition) -> bool {
    point.x >= bounds.position.x
        && point.x < bounds.position.x + bounds.size.width
        && point.y >= bounds.position.y
        && point.y < bounds.position.y + bounds.size.height
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use floatile_core::MonitorKey;
    use serde_json::json;

    fn monitor(key: &str, x: f32, y: f32, w: f32, h: f32, primary: bool) -> MonitorLayout {
        MonitorLayout {
            key: MonitorKey(key.into()),
            bounds: LogicalRect {
                position: LogicalPosition { x, y },
                size: LogicalSize {
                    width: w,
                    height: h,
                },
            },
            scale_factor: ScaleFactor::new(1.0).unwrap(),
            primary,
        }
    }

    fn snapshot(rect: LogicalRect, mode: WidgetMode) -> WindowSnapshot {
        WindowSnapshot {
            rect,
            physical_size: PhysicalSize {
                width: rect.size.width as u32,
                height: rect.size.height as u32,
            },
            scale_factor: ScaleFactor::new(1.0).unwrap(),
            mode,
        }
    }

    #[test]
    fn layout_is_monitor_local_to_containing_screen() {
        let monitors = [
            monitor("eDP-1", 0.0, 0.0, 1920.0, 1080.0, true),
            monitor("DP-1", 1920.0, 0.0, 2560.0, 1440.0, false),
        ];
        let layout = layout_from_window(
            CLOCK_INSTANCE_ID,
            PluginId(BUILTIN_CLOCK_PLUGIN.into()),
            snapshot(
                LogicalRect {
                    position: LogicalPosition {
                        x: 2100.0,
                        y: 300.0,
                    },
                    size: LogicalSize {
                        width: 260.0,
                        height: 120.0,
                    },
                },
                WidgetMode::Edit,
            ),
            &monitors,
            1_700_000_000,
        )
        .unwrap()
        .unwrap();
        assert_eq!(layout.monitor_key, Some(MonitorKey("DP-1".into())));
        // monitor-local：相对 DP-1 原点（1920,0）
        assert_eq!(layout.rect.position, LogicalPosition { x: 180.0, y: 300.0 });
        assert!(!layout.lost_monitor);
        layout.validate().unwrap();
    }

    #[test]
    fn layout_falls_back_to_primary_when_center_outside_all() {
        let monitors = [monitor("eDP-1", 0.0, 0.0, 1920.0, 1080.0, true)];
        let layout = layout_from_window(
            CLOCK_INSTANCE_ID,
            PluginId(BUILTIN_CLOCK_PLUGIN.into()),
            snapshot(
                LogicalRect {
                    position: LogicalPosition {
                        x: 5000.0,
                        y: 5000.0,
                    },
                    size: LogicalSize {
                        width: 260.0,
                        height: 120.0,
                    },
                },
                WidgetMode::Show,
            ),
            &monitors,
            1_700_000_001,
        )
        .unwrap()
        .unwrap();
        assert_eq!(layout.monitor_key, Some(MonitorKey("eDP-1".into())));
        // 相对主屏原点的 monitor-local 矩形
        assert_eq!(
            layout.rect.position,
            LogicalPosition {
                x: 5000.0,
                y: 5000.0
            }
        );
    }

    #[test]
    fn layout_returns_none_without_monitors() {
        let result = layout_from_window(
            CLOCK_INSTANCE_ID,
            PluginId(BUILTIN_CLOCK_PLUGIN.into()),
            snapshot(
                LogicalRect {
                    position: LogicalPosition { x: 100.0, y: 100.0 },
                    size: LogicalSize {
                        width: 260.0,
                        height: 120.0,
                    },
                },
                WidgetMode::Edit,
            ),
            &[],
            1_700_000_002,
        )
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn layout_negative_screen_offset_is_preserved() {
        let monitors = [monitor("DP-1", -1600.0, 0.0, 1600.0, 900.0, false)];
        let layout = layout_from_window(
            CLOCK_INSTANCE_ID,
            PluginId(BUILTIN_CLOCK_PLUGIN.into()),
            snapshot(
                LogicalRect {
                    position: LogicalPosition {
                        x: -1500.0,
                        y: 200.0,
                    },
                    size: LogicalSize {
                        width: 260.0,
                        height: 120.0,
                    },
                },
                WidgetMode::Edit,
            ),
            &monitors,
            1_700_000_003,
        )
        .unwrap()
        .unwrap();
        assert_eq!(layout.monitor_key, Some(MonitorKey("DP-1".into())));
        assert_eq!(layout.rect.position, LogicalPosition { x: 100.0, y: 200.0 });
    }

    #[test]
    fn layout_validates_dpi_consistency() {
        let monitors = [monitor("eDP-1", 0.0, 0.0, 1920.0, 1080.0, true)];
        let layout = layout_from_window(
            CLOCK_INSTANCE_ID,
            PluginId(BUILTIN_CLOCK_PLUGIN.into()),
            WindowSnapshot {
                rect: LogicalRect {
                    position: LogicalPosition { x: 10.0, y: 10.0 },
                    size: LogicalSize {
                        width: 260.0,
                        height: 120.0,
                    },
                },
                physical_size: PhysicalSize {
                    width: 520,
                    height: 240,
                },
                scale_factor: ScaleFactor::new(2.0).unwrap(),
                mode: WidgetMode::Edit,
            },
            &monitors,
            1_700_000_004,
        )
        .unwrap()
        .unwrap();
        assert_eq!(layout.scale_factor, ScaleFactor::new(2.0).unwrap());
        layout.validate().unwrap();
    }

    #[test]
    fn resolves_clock_binding_to_text() {
        let binding = PluginBinding {
            path: "$.time".into(),
            prop: "prop_time".into(),
        };
        let state = json!({"time": "12:34:56", "running": true});
        assert_eq!(
            resolve_binding_string(&binding, &state).unwrap(),
            "12:34:56"
        );
    }

    #[test]
    fn projection_rejects_missing_and_non_string_fields() {
        let binding = PluginBinding {
            path: "$.time".into(),
            prop: "prop_time".into(),
        };
        // 缺失字段 → 失败关闭，不 panic。
        assert_eq!(
            resolve_binding_string(&binding, &json!({"running": true})),
            Err(ProjectionError::MissingField("time".into()))
        );
        // 非字符串值 → 拒绝。
        assert!(matches!(
            resolve_binding_string(&binding, &json!({"time": 42})),
            Err(ProjectionError::NonString { .. })
        ));
        // 非法路径 → 拒绝。
        assert!(matches!(
            resolve_binding_string(
                &PluginBinding {
                    path: "not-a-path".into(),
                    prop: "p".into()
                },
                &json!({})
            ),
            Err(ProjectionError::InvalidPath(_))
        ));
    }

    /// 恶意/超限 State Patch 不能拖垮投影：对象/数组/null/缺失一律返回错误而非 panic。
    #[test]
    fn projection_never_panics_on_hostile_state() {
        let binding = PluginBinding {
            path: "$.time".into(),
            prop: "prop_time".into(),
        };
        for state in [
            json!({"time": {}}),
            json!({"time": []}),
            json!({"time": null}),
            json!({}),
            json!(null),
        ] {
            let _ = resolve_binding_string(&binding, &state);
        }
    }

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
