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
    /// macOS 对 CGDirectDisplayID 分配的稳定 UUID（跨重启稳定）。
    DisplayUuid,
    /// Windows 显示设备名（例如 `\\.\DISPLAY1`，同一显示拓扑下跨重启稳定）。
    DisplayDeviceName,
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
/// Linux X11 走 RandR；Windows 走 EnumDisplayMonitors；macOS 走 NSScreen +
/// CoreGraphics（须在主线程调用）。原生 Wayland 返回显式不支持。
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>, PlatformError> {
    let kind = probe().kind;
    match kind {
        #[cfg(target_os = "linux")]
        PlatformKind::X11 => crate::x11::enumerate_monitors(),
        #[cfg(target_os = "macos")]
        PlatformKind::MacOS => macos_impl::enumerate_monitors(),
        PlatformKind::Wayland => Err(PlatformError::Unsupported(
            "monitor enumeration is not implemented for native Wayland",
        )),
        #[cfg(windows)]
        PlatformKind::Windows => windows_impl::enumerate_monitors(),
        #[cfg(not(windows))]
        PlatformKind::Windows => Err(PlatformError::Unsupported(
            "Windows monitor enumeration is unavailable on this platform",
        )),
        PlatformKind::Unknown => Err(PlatformError::Unsupported(
            "monitor enumeration requires a supported display protocol",
        )),
        #[cfg(not(target_os = "linux"))]
        PlatformKind::X11 => Err(PlatformError::Unsupported(
            "X11 monitor enumeration is only implemented on Linux",
        )),
        #[cfg(not(target_os = "macos"))]
        PlatformKind::MacOS => Err(PlatformError::Unsupported(
            "macOS monitor enumeration is only implemented on macOS",
        )),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use super::*;
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

    unsafe extern "system" fn collect_monitor(
        monitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is the exclusive Vec pointer passed to EnumDisplayMonitors below and the
        // callback is invoked synchronously before that Vec is used again.
        let monitors = unsafe { &mut *(data as *mut Vec<MonitorInfo>) };
        // SAFETY: zero is a valid initial bit pattern for MONITORINFOEXW; cbSize is set next.
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `monitor` is provided by EnumDisplayMonitors and `info` has the required size.
        if unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast::<MONITORINFO>()) } == 0 {
            return 1;
        }
        let end = info
            .szDevice
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(info.szDevice.len());
        let name = String::from_utf16_lossy(&info.szDevice[..end]);
        let rect = info.monitorInfo.rcMonitor;
        let Ok(width) = u32::try_from(rect.right.saturating_sub(rect.left)) else {
            return 1;
        };
        let Ok(height) = u32::try_from(rect.bottom.saturating_sub(rect.top)) else {
            return 1;
        };
        if width == 0 || height == 0 || name.is_empty() {
            return 1;
        }
        monitors.push(MonitorInfo {
            key: MonitorKey(format!("windows-device-{name}")),
            key_source: MonitorKeySource::DisplayDeviceName,
            name,
            position: PhysicalPosition {
                x: rect.left,
                y: rect.top,
            },
            size: PhysicalSize { width, height },
            physical_size_mm: None,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
        1
    }

    pub(super) fn enumerate_monitors() -> Result<Vec<MonitorInfo>, PlatformError> {
        let mut monitors: Vec<MonitorInfo> = Vec::new();
        // SAFETY: callback is synchronous; LPARAM points to `monitors` for the duration of this call.
        let succeeded = unsafe {
            EnumDisplayMonitors(
                0,
                std::ptr::null(),
                Some(collect_monitor),
                (&raw mut monitors) as LPARAM,
            )
        };
        if succeeded == 0 {
            return Err(PlatformError::Platform(
                "EnumDisplayMonitors failed".to_owned(),
            ));
        }
        if monitors.is_empty() {
            return Err(PlatformError::Platform(
                "Windows reported no active monitors".to_owned(),
            ));
        }
        if !monitors.iter().any(|monitor| monitor.primary)
            && let Some(first) = monitors.first_mut()
        {
            first.primary = true;
        }
        Ok(monitors)
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

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos_impl {
    use super::*;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSNumber, NSString};

    /// CoreFoundation/CoreGraphics 的少量 C 符号，本 crate 内按需声明。
    ///
    /// 这些符号是 macOS 公开 API；只在 `floatile-platform` 内使用，不外泄到业务层。
    mod ffi {
        use std::ffi::{c_char, c_void};

        pub type CFUUIDRef = *const c_void;
        pub type CFStringRef = *const c_void;
        pub type CFAllocatorRef = *const c_void;

        /// UTF-8 编码常量（`kCFStringEncodingUTF8`）。
        pub const UTF8_ENCODING: u32 = 0x0800_0100;

        /// `CGSize` 的 ABI 等价结构体（两个 double）。
        #[repr(C)]
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct CGSize {
            pub width: f64,
            pub height: f64,
        }

        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            pub fn CGMainDisplayID() -> u32;
            pub fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> CFUUIDRef;
            pub fn CGDisplayScreenSize(display: u32) -> CGSize;
        }

        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            pub fn CFUUIDCreateString(alloc: CFAllocatorRef, uuid: CFUUIDRef) -> CFStringRef;
            pub fn CFStringGetLength(string: CFStringRef) -> isize;
            pub fn CFStringGetCString(
                string: CFStringRef,
                buffer: *mut c_char,
                buffer_size: isize,
                encoding: u32,
            ) -> i8;
        }
    }

    /// 从 NSScreen 的 `deviceDescription` 取出 `NSScreenNumber`（CGDirectDisplayID）。
    fn screen_display_id(screen: &NSScreen) -> Result<u32, PlatformError> {
        let description = screen.deviceDescription();
        let key = NSString::from_str("NSScreenNumber");
        let number = description.objectForKey(&key).ok_or_else(|| {
            PlatformError::Platform("NSScreen deviceDescription 缺少 NSScreenNumber".into())
        })?;
        let number = number
            .downcast::<NSNumber>()
            .map_err(|_| PlatformError::Platform("NSScreenNumber 不是 NSNumber".into()))?;
        Ok(number.as_u32())
    }

    /// 取显示器稳定 UUID 字符串；失败时回退为 `macos-display-<id>`。
    fn display_key(display_id: u32) -> (MonitorKey, MonitorKeySource) {
        // SAFETY: display_id 为当前会话的活动 CGDirectDisplayID；符号均为只读查询，
        // 返回的 CF 对象按 Create 规则持有，这里立即转字符串且不释放（进程级缓存）。
        let uuid = unsafe { ffi::CGDisplayCreateUUIDFromDisplayID(display_id) };
        if !uuid.is_null() {
            // SAFETY: uuid 是刚创建的合法 CFUUIDRef。
            let string = unsafe { ffi::CFUUIDCreateString(std::ptr::null(), uuid) };
            if !string.is_null()
                && let Some(text) = cf_string_to_rust(string)
            {
                return (
                    MonitorKey(format!("macos-uuid-{text}")),
                    MonitorKeySource::DisplayUuid,
                );
            }
        }
        (
            MonitorKey(format!("macos-display-{display_id}")),
            MonitorKeySource::ConnectorName,
        )
    }

    /// 将 UTF-8 编码的 CFStringRef 转换为 Rust String（有界读取）。
    fn cf_string_to_rust(string: ffi::CFStringRef) -> Option<String> {
        // SAFETY: string 为合法 CFStringRef。
        let length = unsafe { ffi::CFStringGetLength(string) };
        if length <= 0 {
            return Some(String::new());
        }
        // UTF-8 最多 3 字节/UTF-16 单元；加 1 个 NUL。
        let capacity = length.checked_mul(3)?.checked_add(1)?;
        let mut buffer = vec![0u8; capacity as usize];
        // SAFETY: buffer 容量按最坏 UTF-8 扩张预留；CFStringGetCString 写入 NUL 结尾。
        let ok = unsafe {
            ffi::CFStringGetCString(
                string,
                buffer.as_mut_ptr().cast(),
                capacity,
                ffi::UTF8_ENCODING,
            )
        };
        if ok == 0 {
            return None;
        }
        let end = buffer
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(buffer.len());
        buffer.truncate(end);
        String::from_utf8(buffer).ok()
    }

    pub(super) fn enumerate_monitors() -> Result<Vec<MonitorInfo>, PlatformError> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| PlatformError::Platform("NSScreen 枚举必须在主线程执行".into()))?;
        let screens = NSScreen::screens(mtm);

        // Cocoa 坐标原点在主屏左下角；换算为「虚拟桌面左上角原点」的总高（backing 像素）。
        let total_height = screens
            .iter()
            .map(|screen| {
                let backing = screen.convertRectToBacking(screen.frame());
                backing.origin.y + backing.size.height
            })
            .fold(0.0f64, f64::max);

        let main_id = {
            // SAFETY: 只读查询当前主显示器 ID。
            unsafe { ffi::CGMainDisplayID() }
        };

        let mut monitors = Vec::with_capacity(screens.len());
        for screen in screens.iter() {
            let frame = screen.frame();
            let backing = screen.convertRectToBacking(frame);
            let display_id = screen_display_id(&screen)?;

            // 左上角原点物理像素坐标；负坐标表示位于主屏左侧/上方。
            let x = backing.origin.x;
            let y = total_height - (backing.origin.y + backing.size.height);
            let width = backing.size.width;
            let height = backing.size.height;

            let (key, key_source) = display_key(display_id);
            // SAFETY: display_id 为活动显示器 ID；屏幕尺寸返回毫米。
            let mm = unsafe { ffi::CGDisplayScreenSize(display_id) };
            let physical_size_mm = if mm.width > 0.0 && mm.height > 0.0 {
                Some(PhysicalSize {
                    width: mm.width.round() as u32,
                    height: mm.height.round() as u32,
                })
            } else {
                None
            };

            let localized_name = screen.localizedName().to_string();
            let name = if localized_name.is_empty() {
                format!("macos-display-{display_id}")
            } else {
                localized_name
            };
            monitors.push(MonitorInfo {
                key,
                key_source,
                name,
                position: PhysicalPosition {
                    x: x.round() as i32,
                    y: y.round() as i32,
                },
                size: PhysicalSize {
                    width: width.round() as u32,
                    height: height.round() as u32,
                },
                physical_size_mm,
                primary: display_id == main_id,
            });
        }
        Ok(monitors)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_enumerates_an_active_primary_monitor() {
        let monitors = enumerate_monitors().unwrap();
        assert!(!monitors.is_empty());
        assert!(monitors.iter().any(|monitor| monitor.primary));
        assert!(monitors.iter().all(|monitor| {
            monitor.size.width > 0
                && monitor.size.height > 0
                && !monitor.name.is_empty()
                && monitor.key_source == MonitorKeySource::DisplayDeviceName
        }));
    }

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
