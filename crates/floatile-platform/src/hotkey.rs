//! 全局热键注册（S2：展示模式点击穿透后仍可切回编辑模式）。
//!
//! 设计约束：热键依赖原生窗口句柄与消息钩子（`msg_hook`），全部收敛在本 crate；
//! `floatile-shell` 只负责在 winit `EventLoopBuilder` 上安装 `msg_hook` 并调用
//! [`extract_hotkey_id`] 解析命中，业务逻辑不接触平台细节。

use crate::PlatformError;

/// 热键修饰键集合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HotkeyModifiers {
    /// Alt。
    pub alt: bool,
    /// Ctrl。
    pub control: bool,
    /// Shift。
    pub shift: bool,
    /// Windows 键。
    pub win: bool,
}

impl HotkeyModifiers {
    /// 无修饰键。
    pub const fn none() -> Self {
        Self {
            alt: false,
            control: false,
            shift: false,
            win: false,
        }
    }
}

/// 全局热键定义。
///
/// `id` 由调用方分配并保持唯一（Windows 上需落在 0x0000–0xBFFF 应用定义区间）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    /// 应用分配的热键 ID（用于标识命中）。
    pub id: u32,
    /// 修饰键。
    pub modifiers: HotkeyModifiers,
    /// 虚拟键码（如 VK_F8 = 0x77）。
    pub virtual_key: u32,
}

/// 注册全局热键。
///
/// - Windows：`RegisterHotKey` 注册到当前线程的消息队列，事件循环收到 `WM_HOTKEY`。
/// - 其他平台：返回 `PlatformError::Unsupported`，调用方必须显式降级
///   （如仅允许编辑模式内触发），不得静默跳过。
pub fn register_hotkey(
    window: &winit::window::Window,
    hotkey: Hotkey,
) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::register_hotkey(window, hotkey)
    }

    #[cfg(not(windows))]
    {
        let _ = (window, hotkey);
        Err(PlatformError::Unsupported(
            "global hotkeys are only implemented on Windows",
        ))
    }
}

/// 注销全局热键。
pub fn unregister_hotkey(
    window: &winit::window::Window,
    hotkey_id: u32,
) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::unregister_hotkey(window, hotkey_id)
    }

    #[cfg(not(windows))]
    {
        let _ = (window, hotkey_id);
        Err(PlatformError::Unsupported(
            "global hotkeys are only implemented on Windows",
        ))
    }
}

/// 从 Windows `MSG` 指针提取热键 ID。
///
/// 供 winit `EventLoopBuilderExtWindows::with_msg_hook` 使用：命中 `WM_HOTKEY`
/// 时返回 `Some(id)`，其他消息返回 `None`。非 Windows 平台恒为 `None`。
///
/// # Safety
///
/// `msg` 必须指向 winit 消息循环派发中的有效 `windows_sys::MSG`。
#[allow(unsafe_code)]
pub unsafe fn extract_hotkey_id(msg: *const std::ffi::c_void) -> Option<u32> {
    #[cfg(windows)]
    {
        // SAFETY: 调用方承诺 `msg` 是有效的 MSG 指针；只读 message/wParam 字段。
        let msg = unsafe { &*(msg.cast::<windows_sys::Win32::UI::WindowsAndMessaging::MSG>()) };
        if msg.message == windows_sys::Win32::UI::WindowsAndMessaging::WM_HOTKEY {
            Some(msg.wParam as u32)
        } else {
            None
        }
    }

    #[cfg(not(windows))]
    {
        let _ = msg;
        None
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use super::*;
    use windows_sys::Win32::Foundation::{GetLastError, HWND, SetLastError};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub(super) fn modifiers_bits(m: HotkeyModifiers) -> u32 {
        let mut bits = MOD_NOREPEAT;
        if m.alt {
            bits |= MOD_ALT;
        }
        if m.control {
            bits |= MOD_CONTROL;
        }
        if m.shift {
            bits |= MOD_SHIFT;
        }
        if m.win {
            bits |= MOD_WIN;
        }
        bits
    }

    fn hwnd_of(window: &winit::window::Window) -> Result<HWND, PlatformError> {
        let handle = window
            .window_handle()
            .map_err(|e| PlatformError::Platform(format!("window handle unavailable: {e}")))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err(PlatformError::Unsupported(
                "window handle is not a Win32 HWND on Windows",
            ));
        };
        Ok(handle.hwnd.get())
    }

    pub(super) fn register_hotkey(
        window: &winit::window::Window,
        hotkey: Hotkey,
    ) -> Result<(), PlatformError> {
        let hwnd = hwnd_of(window)?;
        // SAFETY: hwnd 是当前 winit 窗口的有效句柄；id/修饰键/虚拟键码为合法入参。
        // 先清零上次错误码，避免把陈旧错误当作本调用失败。
        unsafe { SetLastError(0) };
        let ok = unsafe {
            RegisterHotKey(
                hwnd,
                hotkey.id as i32,
                modifiers_bits(hotkey.modifiers),
                hotkey.virtual_key,
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(PlatformError::Platform(format!(
                "RegisterHotKey failed: {err}"
            )));
        }
        Ok(())
    }

    pub(super) fn unregister_hotkey(
        window: &winit::window::Window,
        hotkey_id: u32,
    ) -> Result<(), PlatformError> {
        let hwnd = hwnd_of(window)?;
        // SAFETY: 同一有效 hwnd；注销此前注册的热键。
        unsafe { SetLastError(0) };
        let ok = unsafe { UnregisterHotKey(hwnd, hotkey_id as i32) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(PlatformError::Platform(format!(
                "UnregisterHotKey failed: {err}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    };

    #[cfg(windows)]
    #[test]
    fn modifier_bits_combine_flags() {
        let m = HotkeyModifiers {
            control: true,
            shift: true,
            ..HotkeyModifiers::none()
        };
        let bits = windows_impl::modifiers_bits(m);
        assert_eq!(bits & MOD_CONTROL, MOD_CONTROL);
        assert_eq!(bits & MOD_SHIFT, MOD_SHIFT);
        assert_eq!(bits & MOD_NOREPEAT, MOD_NOREPEAT);
        assert_eq!(bits & MOD_ALT, 0);
        assert_eq!(bits & MOD_WIN, 0);
    }

    #[cfg(windows)]
    #[test]
    fn modifier_bits_none_only_norepeat() {
        let bits = windows_impl::modifiers_bits(HotkeyModifiers::none());
        assert_eq!(bits, MOD_NOREPEAT);
    }
}
