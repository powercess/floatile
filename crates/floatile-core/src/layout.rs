//! 布局持久化模型：widget 实例的几何、层级与模式。
//!
//! 纯领域类型，无 I/O；校验规则可单测，供 `floatile-store` 持久化与
//! `floatile-shell` 恢复时共用。

use serde::{Deserialize, Serialize};

use crate::{
    InstanceId, LogicalPosition, LogicalRect, LogicalSize, MonitorKey, PhysicalSize, PluginId,
    ScaleFactor, WidgetMode,
};

/// 布局记录：一个 widget 实例在宿主上的几何与展示状态。
///
/// 坐标与尺寸均为逻辑像素；`monitor_key` 是显示器指纹（EDID/product 标识），
/// 原屏缺失时由恢复逻辑降级到主屏并标记 `lost_monitor`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetLayout {
    /// 运行时组件实例 ID。
    pub instance_id: InstanceId,
    /// 来源插件 ID（内建参考时钟等使用保留命名空间）。
    pub plugin_id: PluginId,
    /// 期望显示器的稳定标识；空表示跟随主屏。
    ///
    /// 原屏临时缺失时不得覆盖此字段，否则无法在显示器重新接入后回到原屏。
    pub monitor_key: Option<MonitorKey>,
    /// 相对于期望显示器工作区原点的逻辑像素矩形。
    pub rect: LogicalRect,
    /// 保存时窗口内容区的物理像素尺寸，用于 DPI 诊断和迁移校验。
    pub physical_size: PhysicalSize,
    /// 保存时窗口所在显示器的 scale factor。
    pub scale_factor: ScaleFactor,
    /// 上次恢复时是否因原屏缺失而降级到主屏。
    pub lost_monitor: bool,
    /// 层级（数值越大越靠上）。
    pub z: u32,
    /// 展示模式。
    pub mode: WidgetMode,
    /// 记录格式版本号；恢复逻辑拒绝无法解释的旧版或新版记录。
    pub version: u32,
    /// 更新时间（Unix 秒）。
    pub updated_at: u64,
}

/// 布局记录的序列化版本。
pub const LAYOUT_RECORD_VERSION: u32 = 1;

impl WidgetLayout {
    /// 校验布局记录；返回可读的错误原因。
    pub fn validate(&self) -> Result<(), LayoutValidationError> {
        if self.version != LAYOUT_RECORD_VERSION {
            return Err(LayoutValidationError::InvalidVersion);
        }
        if self
            .monitor_key
            .as_ref()
            .is_some_and(|key| key.as_str().is_empty())
        {
            return Err(LayoutValidationError::EmptyMonitorKey);
        }
        let rect = self.rect;
        if ![
            rect.position.x,
            rect.position.y,
            rect.size.width,
            rect.size.height,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err(LayoutValidationError::NonFiniteGeometry);
        }
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return Err(LayoutValidationError::NonPositiveSize);
        }
        if self.physical_size.width == 0 || self.physical_size.height == 0 {
            return Err(LayoutValidationError::NonPositivePhysicalSize);
        }
        let expected_physical_width = f64::from(rect.size.width) * self.scale_factor.get();
        let expected_physical_height = f64::from(rect.size.height) * self.scale_factor.get();
        if !expected_physical_width.is_finite()
            || !expected_physical_height.is_finite()
            || (expected_physical_width - f64::from(self.physical_size.width)).abs() > 1.0
            || (expected_physical_height - f64::from(self.physical_size.height)).abs() > 1.0
        {
            return Err(LayoutValidationError::InconsistentPhysicalSize);
        }
        if self.lost_monitor && self.monitor_key.is_none() {
            return Err(LayoutValidationError::LostMonitorWithoutKey);
        }
        if self.z == 0 {
            return Err(LayoutValidationError::ZeroZ);
        }
        Ok(())
    }
}

/// 布局校验错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutValidationError {
    #[error("布局记录版本无效（应为 {LAYOUT_RECORD_VERSION}）")]
    InvalidVersion,
    #[error("显示器稳定标识不得为空")]
    EmptyMonitorKey,
    #[error("布局几何必须全部为有限数")]
    NonFiniteGeometry,
    #[error("逻辑尺寸必须为正")]
    NonPositiveSize,
    #[error("物理尺寸必须为正")]
    NonPositivePhysicalSize,
    #[error("物理尺寸与逻辑尺寸及 scale factor 不一致")]
    InconsistentPhysicalSize,
    #[error("lost_monitor 需要保留原显示器标识")]
    LostMonitorWithoutKey,
    #[error("层级 z 必须大于 0")]
    ZeroZ,
}

/// 恢复计算使用的活动显示器快照。
///
/// `bounds` 是平台归一后的虚拟桌面逻辑像素矩形；恢复逻辑不自行猜测物理坐标空间。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorLayout {
    pub key: MonitorKey,
    pub bounds: LogicalRect,
    pub scale_factor: ScaleFactor,
    pub primary: bool,
}

/// 一条持久化布局在当前显示器拓扑中的运行时落点。
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveredLayout<'a> {
    /// 实际承载窗口的活动显示器；原屏缺失时为主屏。
    pub monitor: &'a MonitorLayout,
    /// 虚拟桌面逻辑像素坐标，已钳制到目标显示器内。
    pub rect: LogicalRect,
    /// 是否因期望显示器缺失而使用了主屏。
    pub lost_monitor: bool,
}

/// 布局恢复失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutRecoveryError {
    #[error("持久化布局无效: {0}")]
    InvalidLayout(#[from] LayoutValidationError),
    #[error("活动显示器列表为空")]
    NoMonitors,
    #[error("显示器 `{0}` 的稳定标识为空")]
    EmptyMonitorKey(String),
    #[error("显示器 `{0}` 的逻辑边界必须全部为有限数")]
    NonFiniteMonitorBounds(String),
    #[error("显示器 `{0}` 的逻辑尺寸必须为正")]
    NonPositiveMonitorSize(String),
    #[error("活动显示器列表没有主屏")]
    MissingPrimary,
    #[error("活动显示器列表包含多个主屏")]
    MultiplePrimary,
    #[error("活动显示器列表包含重复标识 `{0}`")]
    DuplicateMonitorKey(String),
    #[error("恢复后的窗口几何超出有限数范围")]
    NonFiniteRecoveredGeometry,
}

/// 将持久化的 monitor-local 逻辑布局映射到当前活动显示器拓扑。
///
/// 原屏缺失时仅返回主屏落点和 `lost_monitor = true`，不会覆盖调用方保存的原显示器标识或矩形。
/// 因此同一记录在原屏重新接入后可恢复到原来的 monitor-local 位置。
pub fn recover_layout<'a>(
    layout: &WidgetLayout,
    monitors: &'a [MonitorLayout],
) -> Result<RecoveredLayout<'a>, LayoutRecoveryError> {
    layout.validate()?;
    let primary = validate_monitor_topology(monitors)?;
    let requested = layout
        .monitor_key
        .as_ref()
        .and_then(|key| monitors.iter().find(|monitor| monitor.key == *key));
    let lost_monitor = layout.monitor_key.is_some() && requested.is_none();
    let monitor = requested.unwrap_or(primary);
    let rect = place_on_monitor(layout.rect, monitor.bounds)?;

    Ok(RecoveredLayout {
        monitor,
        rect,
        lost_monitor,
    })
}

fn validate_monitor_topology(
    monitors: &[MonitorLayout],
) -> Result<&MonitorLayout, LayoutRecoveryError> {
    if monitors.is_empty() {
        return Err(LayoutRecoveryError::NoMonitors);
    }

    let mut primary = None;
    for (index, monitor) in monitors.iter().enumerate() {
        let key = monitor.key.as_str();
        if key.is_empty() {
            return Err(LayoutRecoveryError::EmptyMonitorKey(key.into()));
        }
        let bounds = monitor.bounds;
        if ![
            bounds.position.x,
            bounds.position.y,
            bounds.size.width,
            bounds.size.height,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err(LayoutRecoveryError::NonFiniteMonitorBounds(key.into()));
        }
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return Err(LayoutRecoveryError::NonPositiveMonitorSize(key.into()));
        }
        if monitors[..index]
            .iter()
            .any(|existing| existing.key == monitor.key)
        {
            return Err(LayoutRecoveryError::DuplicateMonitorKey(key.into()));
        }
        if monitor.primary && primary.replace(monitor).is_some() {
            return Err(LayoutRecoveryError::MultiplePrimary);
        }
    }
    primary.ok_or(LayoutRecoveryError::MissingPrimary)
}

fn place_on_monitor(
    saved: LogicalRect,
    monitor_bounds: LogicalRect,
) -> Result<LogicalRect, LayoutRecoveryError> {
    let size = LogicalSize {
        width: saved.size.width.min(monitor_bounds.size.width),
        height: saved.size.height.min(monitor_bounds.size.height),
    };
    let desired = LogicalPosition {
        x: monitor_bounds.position.x + saved.position.x,
        y: monitor_bounds.position.y + saved.position.y,
    };
    let max = LogicalPosition {
        x: monitor_bounds.position.x + monitor_bounds.size.width - size.width,
        y: monitor_bounds.position.y + monitor_bounds.size.height - size.height,
    };
    if ![desired.x, desired.y, max.x, max.y]
        .into_iter()
        .all(f32::is_finite)
    {
        return Err(LayoutRecoveryError::NonFiniteRecoveredGeometry);
    }

    Ok(LogicalRect {
        position: LogicalPosition {
            x: desired.x.clamp(monitor_bounds.position.x, max.x),
            y: desired.y.clamp(monitor_bounds.position.y, max.y),
        },
        size,
    })
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sample() -> WidgetLayout {
        WidgetLayout {
            instance_id: InstanceId(1),
            plugin_id: PluginId("dev.floatile.clock".into()),
            monitor_key: Some(MonitorKey("edid-abc123".into())),
            rect: LogicalRect {
                position: LogicalPosition { x: 120.0, y: 80.0 },
                size: LogicalSize {
                    width: 260.0,
                    height: 120.0,
                },
            },
            physical_size: PhysicalSize {
                width: 260,
                height: 120,
            },
            scale_factor: ScaleFactor::new(1.0).unwrap(),
            lost_monitor: false,
            z: 10,
            mode: WidgetMode::Edit,
            version: LAYOUT_RECORD_VERSION,
            updated_at: 1_700_000_000,
        }
    }

    fn monitor(
        key: &str,
        position: LogicalPosition,
        size: LogicalSize,
        scale_factor: f64,
        primary: bool,
    ) -> MonitorLayout {
        MonitorLayout {
            key: MonitorKey(key.into()),
            bounds: LogicalRect { position, size },
            scale_factor: ScaleFactor::new(scale_factor).unwrap(),
            primary,
        }
    }

    fn topology() -> Vec<MonitorLayout> {
        vec![
            monitor(
                "primary",
                LogicalPosition { x: 0.0, y: 0.0 },
                LogicalSize {
                    width: 1920.0,
                    height: 1080.0,
                },
                1.0,
                true,
            ),
            monitor(
                "edid-abc123",
                LogicalPosition { x: -1536.0, y: 0.0 },
                LogicalSize {
                    width: 1536.0,
                    height: 864.0,
                },
                1.25,
                false,
            ),
        ]
    }

    #[test]
    fn valid_layout_passes() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut layout = sample();
        layout.version = 0;
        assert_eq!(
            layout.validate(),
            Err(LayoutValidationError::InvalidVersion)
        );
        layout.version = LAYOUT_RECORD_VERSION + 1;
        assert_eq!(
            layout.validate(),
            Err(LayoutValidationError::InvalidVersion)
        );
    }

    #[test]
    fn non_positive_size_rejected() {
        let mut l = sample();
        l.rect.size.width = 0.0;
        assert_eq!(l.validate(), Err(LayoutValidationError::NonPositiveSize));
        l = sample();
        l.rect.size.height = -1.0;
        assert_eq!(l.validate(), Err(LayoutValidationError::NonPositiveSize));
    }

    #[test]
    fn non_finite_geometry_rejected() {
        let mut layout = sample();
        layout.rect.position.x = f32::NAN;
        assert_eq!(
            layout.validate(),
            Err(LayoutValidationError::NonFiniteGeometry)
        );
    }

    #[test]
    fn invalid_physical_size_and_lost_monitor_state_rejected() {
        let mut layout = sample();
        layout.physical_size.width = 0;
        assert_eq!(
            layout.validate(),
            Err(LayoutValidationError::NonPositivePhysicalSize)
        );

        layout = sample();
        layout.physical_size.width += 2;
        assert_eq!(
            layout.validate(),
            Err(LayoutValidationError::InconsistentPhysicalSize)
        );

        layout = sample();
        layout.monitor_key = None;
        layout.lost_monitor = true;
        assert_eq!(
            layout.validate(),
            Err(LayoutValidationError::LostMonitorWithoutKey)
        );
    }

    #[test]
    fn recovery_maps_monitor_local_position_to_selected_monitor() {
        let monitors = topology();
        let recovered = recover_layout(&sample(), &monitors).unwrap();
        assert_eq!(recovered.monitor.key, MonitorKey("edid-abc123".into()));
        assert_eq!(
            recovered.monitor.scale_factor,
            ScaleFactor::new(1.25).unwrap()
        );
        assert_eq!(
            recovered.rect.position,
            LogicalPosition {
                x: -1416.0,
                y: 80.0
            }
        );
        assert_eq!(recovered.rect.size, sample().rect.size);
        assert!(!recovered.lost_monitor);
    }

    #[test]
    fn missing_monitor_falls_back_without_overwriting_saved_target() {
        let mut layout = sample();
        layout.lost_monitor = true;
        let monitors = vec![topology().remove(0)];
        let saved_key = layout.monitor_key.clone();
        let saved_rect = layout.rect;

        let recovered = recover_layout(&layout, &monitors).unwrap();

        assert_eq!(recovered.monitor.key, MonitorKey("primary".into()));
        assert_eq!(recovered.rect.position, saved_rect.position);
        assert!(recovered.lost_monitor);
        assert_eq!(layout.monitor_key, saved_key);
        assert_eq!(layout.rect, saved_rect);
    }

    #[test]
    fn reconnected_monitor_clears_recovery_degradation() {
        let mut layout = sample();
        layout.lost_monitor = true;
        let monitors = topology();
        let recovered = recover_layout(&layout, &monitors).unwrap();
        assert_eq!(recovered.monitor.key, MonitorKey("edid-abc123".into()));
        assert!(!recovered.lost_monitor);
    }

    #[test]
    fn recovery_keeps_oversized_or_offscreen_layout_visible() {
        let mut layout = sample();
        layout.monitor_key = None;
        layout.rect = LogicalRect {
            position: LogicalPosition {
                x: 5000.0,
                y: -200.0,
            },
            size: LogicalSize {
                width: 3000.0,
                height: 1200.0,
            },
        };
        layout.physical_size = PhysicalSize {
            width: 3000,
            height: 1200,
        };
        let monitors = topology();
        let recovered = recover_layout(&layout, &monitors).unwrap();
        assert_eq!(recovered.rect, monitors[0].bounds);
    }

    #[test]
    fn invalid_monitor_topology_is_rejected() {
        let mut monitors = topology();
        monitors[1].key = monitors[0].key.clone();
        assert!(matches!(
            recover_layout(&sample(), &monitors),
            Err(LayoutRecoveryError::DuplicateMonitorKey(_))
        ));

        monitors = topology();
        monitors[0].primary = false;
        assert_eq!(
            recover_layout(&sample(), &monitors),
            Err(LayoutRecoveryError::MissingPrimary)
        );
    }

    #[test]
    fn zero_z_rejected() {
        let mut l = sample();
        l.z = 0;
        assert_eq!(l.validate(), Err(LayoutValidationError::ZeroZ));
    }

    #[test]
    fn layout_serde_roundtrip() {
        let l = sample();
        let json = serde_json::to_string(&l).unwrap();
        let back: WidgetLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(l, back);
    }
}
