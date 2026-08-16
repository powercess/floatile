//! 显示器枚举契约。
//!
//! 平台层返回原生物理像素；逻辑像素与 DPI 换算由后续布局恢复流程统一处理。

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

/// 虚拟桌面中的物理像素位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalPosition {
    pub x: i32,
    pub y: i32,
}

/// 物理像素或毫米尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

/// 平台显示器描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// 布局持久化使用的稳定键。
    pub key: String,
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
