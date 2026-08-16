//! 全局热键注册（S2：展示模式点击穿透后仍可切回编辑模式）。
//!
//! 设计约束：热键依赖原生窗口句柄与消息循环，全部收敛在本 crate；
//! `floatile-shell` 只安装安全的平台回调并处理命中的热键 ID。

use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;

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
    /// 平台键符号。字母键使用大写 ASCII/KeySym 值（如 E = `0x45`）；
    /// Windows 将其作为虚拟键码，X11 将其作为 KeySym 查找 keycode。
    pub virtual_key: u32,
}

/// 后台热键监听注册。
///
/// 当前用于 Linux X11：持有注册 passive grab 的连接和事件线程；销毁或调用 [`Self::stop`]
/// 时释放资源。Windows 继续由 winit `msg_hook` 派发 `WM_HOTKEY`。
pub struct HotkeyListener {
    stop: Option<SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl HotkeyListener {
    fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.try_send(());
        }
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::warn!("global hotkey listener panicked");
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 在支持后台派发的显示协议上监听全局热键。
///
/// Linux X11 使用 root-window passive grab；回调在平台工作线程执行，调用方必须把 UI
/// 更新投递回 UI 事件循环。其他平台返回显式不支持。
pub fn listen_hotkey(
    hotkey: Hotkey,
    on_trigger: impl Fn() + Send + 'static,
) -> Result<HotkeyListener, PlatformError> {
    #[cfg(target_os = "linux")]
    {
        linux_impl::listen_hotkey(hotkey, on_trigger)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (hotkey, on_trigger);
        Err(PlatformError::Unsupported(
            "background global hotkey listening is only implemented on Linux X11",
        ))
    }
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

/// 在 Windows winit 事件循环上安装安全的 `WM_HOTKEY` 消息钩子。
///
/// 原始 `MSG` 指针只在平台 crate 内按 winit 的回调合约读取；业务层只接收热键 ID。
#[cfg(windows)]
pub fn install_hotkey_message_hook<T>(
    event_loop_builder: &mut winit::event_loop::EventLoopBuilder<T>,
    mut on_hotkey: impl FnMut(u32) -> bool + 'static,
) {
    use winit::platform::windows::EventLoopBuilderExtWindows;

    event_loop_builder
        .with_msg_hook(move |msg| windows_impl::extract_hotkey_id(msg).is_some_and(&mut on_hotkey));
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask, Window};

    fn modifier_mask(modifiers: HotkeyModifiers) -> ModMask {
        let mut mask = ModMask::default();
        if modifiers.alt {
            mask |= ModMask::M1;
        }
        if modifiers.control {
            mask |= ModMask::CONTROL;
        }
        if modifiers.shift {
            mask |= ModMask::SHIFT;
        }
        if modifiers.win {
            mask |= ModMask::M4;
        }
        mask
    }

    fn keycode_for_keysym<C: Connection>(connection: &C, keysym: u32) -> Result<u8, PlatformError> {
        let setup = connection.setup();
        let count = setup
            .max_keycode
            .checked_sub(setup.min_keycode)
            .and_then(|difference| difference.checked_add(1))
            .ok_or_else(|| PlatformError::Platform("invalid X11 keycode range".into()))?;
        let mapping = connection
            .get_keyboard_mapping(setup.min_keycode, count)
            .map_err(|error| {
                PlatformError::Platform(format!("request X11 keyboard mapping: {error}"))
            })?
            .reply()
            .map_err(|error| {
                PlatformError::Platform(format!("read X11 keyboard mapping: {error}"))
            })?;
        let width = usize::from(mapping.keysyms_per_keycode);
        if width == 0 {
            return Err(PlatformError::Platform(
                "X11 keyboard mapping has zero symbols per keycode".into(),
            ));
        }
        let index = mapping
            .keysyms
            .chunks(width)
            .position(|symbols| symbols.contains(&keysym))
            .ok_or(PlatformError::Unsupported(
                "requested hotkey symbol is absent from the X11 keyboard map",
            ))?;
        let offset = u8::try_from(index)
            .map_err(|_| PlatformError::Platform("X11 keycode index exceeds u8".into()))?;
        setup
            .min_keycode
            .checked_add(offset)
            .ok_or_else(|| PlatformError::Platform("X11 keycode overflow".into()))
    }

    fn grab_variants<C: Connection>(
        connection: &C,
        root: Window,
        keycode: u8,
        modifiers: ModMask,
    ) -> Result<(), PlatformError> {
        // CapsLock 与常见 Mod2 NumLock 不应改变宿主恢复热键。
        let variants = [
            modifiers,
            modifiers | ModMask::LOCK,
            modifiers | ModMask::M2,
            modifiers | ModMask::LOCK | ModMask::M2,
        ];
        for mask in variants {
            connection
                .grab_key(false, root, mask, keycode, GrabMode::ASYNC, GrabMode::ASYNC)
                .map_err(|error| {
                    PlatformError::Platform(format!("register X11 global hotkey: {error}"))
                })?
                .check()
                .map_err(|error| {
                    PlatformError::Platform(format!("activate X11 global hotkey: {error}"))
                })?;
        }
        connection
            .flush()
            .map_err(|error| PlatformError::Platform(format!("flush X11 hotkey grabs: {error}")))
    }

    pub(super) fn listen_hotkey(
        hotkey: Hotkey,
        on_trigger: impl Fn() + Send + 'static,
    ) -> Result<HotkeyListener, PlatformError> {
        let (connection, screen_number) = x11rb::connect(None)
            .map_err(|error| PlatformError::Platform(format!("connect to X11: {error}")))?;
        let root = connection
            .setup()
            .roots
            .get(screen_number)
            .ok_or_else(|| PlatformError::Platform("X11 screen index is invalid".into()))?
            .root;
        let keycode = keycode_for_keysym(&connection, hotkey.virtual_key)?;
        let modifiers = modifier_mask(hotkey.modifiers);
        grab_variants(&connection, root, keycode, modifiers)?;

        let (stop, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("floatile-x11-hotkey".into())
            .spawn(move || {
                let mut last_trigger = None::<Instant>;
                loop {
                    match receiver.recv_timeout(Duration::from_millis(25)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }

                    loop {
                        match connection.poll_for_event() {
                            Ok(Some(Event::KeyPress(event))) if event.detail == keycode => {
                                let now = Instant::now();
                                let repeated = last_trigger.is_some_and(|previous| {
                                    now.duration_since(previous) < Duration::from_millis(200)
                                });
                                if !repeated {
                                    last_trigger = Some(now);
                                    on_trigger();
                                }
                            }
                            Ok(Some(Event::KeyRelease(event))) if event.detail == keycode => {}
                            Ok(Some(_)) => {}
                            Ok(None) => break,
                            Err(error) => {
                                tracing::warn!(%error, "X11 hotkey event polling failed");
                                return;
                            }
                        }
                    }
                }

                if let Ok(cookie) = connection.ungrab_key(keycode, root, ModMask::ANY)
                    && let Err(error) = cookie.check()
                {
                    tracing::warn!(%error, "X11 hotkey release failed");
                }
                if let Err(error) = connection.flush() {
                    tracing::warn!(%error, "X11 hotkey release flush failed");
                }
            })
            .map_err(|error| {
                PlatformError::Platform(format!("spawn X11 hotkey listener: {error}"))
            })?;

        Ok(HotkeyListener {
            stop: Some(stop),
            worker: Some(worker),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn x11_modifier_mask_maps_host_hotkey() {
            let mask = modifier_mask(HotkeyModifiers {
                control: true,
                shift: true,
                ..HotkeyModifiers::none()
            });
            assert_eq!(mask & ModMask::CONTROL, ModMask::CONTROL);
            assert_eq!(mask & ModMask::SHIFT, ModMask::SHIFT);
            assert_eq!(mask & ModMask::M1, ModMask::default());
        }
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

    pub(super) fn extract_hotkey_id(msg: *const std::ffi::c_void) -> Option<u32> {
        // SAFETY: 该函数仅由 `install_hotkey_message_hook` 传给 winit 的回调调用；
        // winit 保证参数指向消息循环当前派发的有效 MSG，只读取 message/wParam 字段。
        let msg = unsafe { &*(msg.cast::<windows_sys::Win32::UI::WindowsAndMessaging::MSG>()) };
        if msg.message == windows_sys::Win32::UI::WindowsAndMessaging::WM_HOTKEY {
            Some(msg.wParam as u32)
        } else {
            None
        }
    }
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
