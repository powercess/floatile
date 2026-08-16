//! 显示器枚举契约。
//!
//! 平台层返回原生物理像素；逻辑像素与 DPI 换算由后续布局恢复流程统一处理。

use floatile_core::{
    LogicalPosition, LogicalRect, LogicalSize, MonitorKey, MonitorLayout, PhysicalPosition,
    PhysicalSize, ScaleFactor,
};

use crate::capability::{PlatformKind, probe};
use crate::window::PlatformError;

/// 显示器稳定键的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorKeySource {
    /// 对完整 EDID 字节计算稳定指纹。
    Edid,
    /// EDID 不可用时退化为 X11 connector 名称；更换接口后可能变化。
    ConnectorName,
}

/// 平台显示器描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// 布局持久化使用的稳定键。
    pub key: MonitorKey,
    /// 稳定键来源；调用方可据此记录降级。
    pub key_source: MonitorKeySource,
    /// 平台报告的 connector/显示器名称。
    pub name: String,
    /// 虚拟桌面中的物理像素坐标，可为负数。
    pub position: PhysicalPosition,
    /// 当前模式的物理像素尺寸。
    pub size: PhysicalSize,
    /// EDID/RandR 报告的物理毫米尺寸；未知时为 `None`。
    pub physical_size_mm: Option<PhysicalSize>,
    /// 是否为平台主显示器；平台未标记时由枚举器选择首个活动输出。
    pub primary: bool,
}

/// 枚举当前显示协议的活动显示器。
///
/// P0 当前仅实现 Linux X11 RandR。原生 Wayland、Windows、macOS 返回显式不支持，
/// 不回退到 XWayland 或伪造单显示器。
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>, PlatformError> {
    let kind = probe().kind;
    match kind {
        #[cfg(target_os = "linux")]
        PlatformKind::X11 => crate::x11::enumerate_monitors(),
        PlatformKind::Wayland => Err(PlatformError::Unsupported(
            "monitor enumeration is not implemented for native Wayland",
        )),
        PlatformKind::Windows => Err(PlatformError::Unsupported(
            "monitor enumeration is not implemented for Windows",
        )),
        PlatformKind::Unknown => Err(PlatformError::Unsupported(
            "monitor enumeration requires a supported display protocol",
        )),
        #[cfg(not(target_os = "linux"))]
        PlatformKind::X11 => Err(PlatformError::Unsupported(
            "X11 monitor enumeration is only implemented on Linux",
        )),
    }
}

/// 将平台显示器快照归一为布局恢复使用的逻辑布局。
///
/// X11 RandR 报告物理像素；逻辑像素 = 物理像素 / scale factor。scale factor 由
/// 调用方按窗口所在屏提供（winit `Window::scale_factor`）；X11 无统一每屏 DPI，
/// 常态为 1.0，此时逻辑与物理数值一致。`ScaleFactor` 保证有限正数，因此结果
/// 必然为有限值。
pub fn to_monitor_layout(info: &MonitorInfo, scale_factor: ScaleFactor) -> MonitorLayout {
    let sf = scale_factor.get() as f32;
    MonitorLayout {
        key: info.key.clone(),
        bounds: LogicalRect {
            position: LogicalPosition {
                x: info.position.x as f32 / sf,
                y: info.position.y as f32 / sf,
            },
            size: LogicalSize {
                width: info.size.width as f32 / sf,
                height: info.size.height as f32 / sf,
            },
        },
        scale_factor,
        primary: info.primary,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn to_monitor_layout_scales_physical_to_logical() {
        let info = MonitorInfo {
            key: MonitorKey("DP-1".into()),
            key_source: MonitorKeySource::ConnectorName,
            name: "DP-1".into(),
            position: PhysicalPosition { x: 1920, y: 0 },
            size: PhysicalSize {
                width: 2560,
                height: 1440,
            },
            physical_size_mm: None,
            primary: false,
        };
        let layout = to_monitor_layout(&info, ScaleFactor::new(2.0).unwrap());
        assert_eq!(layout.key, MonitorKey("DP-1".into()));
        assert_eq!(layout.bounds.position, LogicalPosition { x: 960.0, y: 0.0 });
        assert_eq!(
            layout.bounds.size,
            LogicalSize {
                width: 1280.0,
                height: 720.0
            }
        );
        assert!(!layout.primary);
    }

    #[test]
    fn to_monitor_layout_unit_scale_preserves_values() {
        let info = MonitorInfo {
            key: MonitorKey("eDP-1".into()),
            key_source: MonitorKeySource::Edid,
            name: "eDP-1".into(),
            position: PhysicalPosition { x: -1600, y: 0 },
            size: PhysicalSize {
                width: 1600,
                height: 900,
            },
            physical_size_mm: Some(PhysicalSize {
                width: 344,
                height: 194,
            }),
            primary: true,
        };
        let layout = to_monitor_layout(&info, ScaleFactor::new(1.0).unwrap());
        assert_eq!(
            layout.bounds.position,
            LogicalPosition { x: -1600.0, y: 0.0 }
        );
        assert_eq!(
            layout.bounds.size,
            LogicalSize {
                width: 1600.0,
                height: 900.0
            }
        );
        assert!(layout.primary);
    }
}
