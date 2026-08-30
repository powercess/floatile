//! 全局热键注册（S2：展示模式点击穿透后仍可切回编辑模式）。
//!
//! 设计约束：热键依赖原生窗口句柄与消息循环，全部收敛在本 crate；
//! `floatile-shell` 只安装安全的平台回调并处理命中的热键 ID。

#[cfg(target_os = "linux")]
use std::sync::mpsc::SyncSender;
#[cfg(target_os = "linux")]
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
/// Linux X11 持有注册 passive grab 的连接和事件线程；macOS 持有 Carbon 事件处理器与
/// 热键引用。销毁或调用 [`Self::stop`] 时释放资源。Windows 继续由 winit `msg_hook`
/// 派发 `WM_HOTKEY`。
pub struct HotkeyListener {
    #[cfg(target_os = "linux")]
    stop: Option<SyncSender<()>>,
    #[cfg(target_os = "linux")]
    worker: Option<JoinHandle<()>>,
    #[cfg(target_os = "macos")]
    macos: Option<macos_impl::RegisteredHotkey>,
}

impl HotkeyListener {
    fn shutdown(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if let Some(stop) = self.stop.take() {
                let _ = stop.try_send(());
            }
            if let Some(worker) = self.worker.take()
                && worker.join().is_err()
            {
                tracing::warn!("global hotkey listener panicked");
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(mut registered) = self.macos.take() {
            registered.unregister();
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
/// 更新投递回 UI 事件循环。macOS 使用 Carbon `RegisterEventHotKey`（无需辅助功能授权），
/// 回调在主线程事件循环派发。其他平台返回显式不支持。
pub fn listen_hotkey(
    hotkey: Hotkey,
    on_trigger: impl Fn() + Send + 'static,
) -> Result<HotkeyListener, PlatformError> {
    #[cfg(target_os = "linux")]
    {
        linux_impl::listen_hotkey(hotkey, on_trigger)
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::listen_hotkey(hotkey, on_trigger)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (hotkey, on_trigger);
        Err(PlatformError::Unsupported(
            "background global hotkey listening is not implemented on this platform",
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

/// 把全局热键注册到当前线程消息队列，而不是某个可视 HWND。
///
/// Windows 的 `RegisterHotKey(NULL, ...)` 会把 `WM_HOTKEY` 投递到调用线程；这让托盘型
/// 宿主无需创建伪装的隐藏窗口。其他平台显式返回不支持。
pub fn register_thread_hotkey(hotkey: Hotkey) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::register_thread_hotkey(hotkey)
    }

    #[cfg(not(windows))]
    {
        let _ = hotkey;
        Err(PlatformError::Unsupported(
            "thread-queue global hotkeys are only implemented on Windows",
        ))
    }
}

/// 注销当前线程消息队列上的全局热键。
pub fn unregister_thread_hotkey(hotkey_id: u32) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows_impl::unregister_thread_hotkey(hotkey_id)
    }

    #[cfg(not(windows))]
    {
        let _ = hotkey_id;
        Err(PlatformError::Unsupported(
            "thread-queue global hotkeys are only implemented on Windows",
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

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos_impl {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::sync::Mutex;

    use super::*;
    const HOTKEY_SIGNATURE: u32 = 0x4650_4C45;

    /// Carbon Event Manager 的少量 C 符号。
    mod ffi {
        use std::ffi::c_void;

        pub type EventTargetRef = *mut c_void;
        pub type EventHandlerRef = *mut c_void;
        pub type EventHandlerCallRef = *mut c_void;
        pub type EventRef = *mut c_void;
        pub type EventHotKeyRef = *mut c_void;
        pub type OSStatus = i32;

        #[repr(C)]
        #[derive(Debug, Clone, Copy)]
        pub struct EventHotKeyID {
            pub signature: u32,
            pub id: u32,
        }

        #[repr(C)]
        #[derive(Debug, Clone, Copy)]
        pub struct EventTypeSpec {
            pub event_class: u32,
            pub event_kind: u32,
        }

        pub type EventHandlerUPP =
            unsafe extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

        /// `'keyb'`
        pub const EVENT_CLASS_KEYBOARD: u32 = 0x6B65_7962;
        pub const EVENT_HOTKEY_PRESSED: u32 = 1;
        /// `'----'`（事件直接对象参数）。
        pub const EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D_2D2D;
        /// `'hkid'`（热键 ID 类型）。
        pub const TYPE_EVENT_HOTKEY_ID: u32 = 0x686B_6964;

        pub const MOD_CMD: u32 = 1 << 8;
        pub const MOD_SHIFT: u32 = 1 << 9;
        pub const MOD_OPTION: u32 = 1 << 11;
        pub const MOD_CONTROL: u32 = 1 << 12;

        pub const NO_ERR: OSStatus = 0;

        #[link(name = "Carbon", kind = "framework")]
        unsafe extern "C" {
            pub fn GetEventDispatcherTarget() -> EventTargetRef;
            pub fn RegisterEventHotKey(
                hot_key_code: u32,
                hot_key_modifiers: u32,
                hot_key_id: EventHotKeyID,
                target: EventTargetRef,
                options: u32,
                out_ref: *mut EventHotKeyRef,
            ) -> OSStatus;
            pub fn InstallEventHandler(
                target: EventTargetRef,
                handler: EventHandlerUPP,
                num_types: usize,
                type_list: *const EventTypeSpec,
                user_data: *mut c_void,
                out_ref: *mut EventHandlerRef,
            ) -> OSStatus;
            pub fn RemoveEventHandler(handler_ref: EventHandlerRef) -> OSStatus;
            pub fn UnregisterEventHotKey(hot_key_ref: EventHotKeyRef) -> OSStatus;
            pub fn GetEventParameter(
                event: EventRef,
                name: u32,
                desired_type: u32,
                actual_type: *mut u32,
                buffer_size: usize,
                actual_size: *mut usize,
                data: *mut c_void,
            ) -> OSStatus;
        }
    }

    type CallbackBox = Mutex<Option<Box<dyn Fn() + Send>>>;

    fn carbon_modifiers(modifiers: HotkeyModifiers) -> u32 {
        let mut bits = 0;
        if modifiers.alt {
            bits |= ffi::MOD_OPTION;
        }
        if modifiers.control {
            bits |= ffi::MOD_CONTROL;
        }
        if modifiers.shift {
            bits |= ffi::MOD_SHIFT;
        }
        if modifiers.win {
            bits |= ffi::MOD_CMD;
        }
        bits
    }

    /// Carbon 事件处理器：读取热键 ID 并调用宿主回调。
    unsafe extern "C" fn hotkey_handler(
        _call: ffi::EventHandlerCallRef,
        event: ffi::EventRef,
        user_data: *mut c_void,
    ) -> ffi::OSStatus {
        let mut hotkey_id = ffi::EventHotKeyID {
            signature: 0,
            id: 0,
        };
        // SAFETY: event 是 Carbon 派发的合法事件引用；数据写入 hotkey_id。
        let status = unsafe {
            ffi::GetEventParameter(
                event,
                ffi::EVENT_PARAM_DIRECT_OBJECT,
                ffi::TYPE_EVENT_HOTKEY_ID,
                0,
                size_of::<ffi::EventHotKeyID>(),
                std::ptr::null_mut(),
                &mut hotkey_id as *mut _ as *mut c_void,
            )
        };
        if status != ffi::NO_ERR || hotkey_id.signature != HOTKEY_SIGNATURE {
            return ffi::NO_ERR;
        }
        // SAFETY: user_data 是 listen_hotkey 注册时泄漏的 CallbackBox 指针；
        // 事件在注册注销之间串行派发，指针有效。
        let callback_box = unsafe { &*(user_data as *const CallbackBox) };
        let callback = callback_box
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(callback) = callback.as_ref() {
            callback();
        }
        ffi::NO_ERR
    }

    pub(super) struct RegisteredHotkey {
        hot_key_ref: ffi::EventHotKeyRef,
        handler_ref: ffi::EventHandlerRef,
        callback: *mut CallbackBox,
    }

    impl RegisteredHotkey {
        pub(super) fn unregister(&mut self) {
            // SAFETY: 引用来自注册成功时的返回值；先注销 handler 阻止后续回调，
            // 再注销热键并释放回调盒。
            unsafe {
                ffi::RemoveEventHandler(self.handler_ref);
                ffi::UnregisterEventHotKey(self.hot_key_ref);
                drop(Box::from_raw(self.callback));
            }
            self.handler_ref = std::ptr::null_mut();
            self.hot_key_ref = std::ptr::null_mut();
            self.callback = std::ptr::null_mut();
        }
    }

    pub(super) fn listen_hotkey(
        hotkey: Hotkey,
        on_trigger: impl Fn() + Send + 'static,
    ) -> Result<HotkeyListener, PlatformError> {
        let callback = Box::new(CallbackBox::new(Some(Box::new(on_trigger))));
        let callback_ptr = Box::into_raw(callback);

        let mut hot_key_ref: ffi::EventHotKeyRef = std::ptr::null_mut();
        let hot_key_id = ffi::EventHotKeyID {
            signature: HOTKEY_SIGNATURE,
            id: hotkey.id,
        };
        // SAFETY: 参数合法；out_ref 指向 hot_key_ref；target 为事件分发目标。
        let status = unsafe {
            ffi::RegisterEventHotKey(
                hotkey.virtual_key,
                carbon_modifiers(hotkey.modifiers),
                hot_key_id,
                ffi::GetEventDispatcherTarget(),
                0,
                &mut hot_key_ref,
            )
        };
        if status != ffi::NO_ERR {
            // SAFETY: 释放尚未被 handler 引用的回调盒。
            unsafe { drop(Box::from_raw(callback_ptr)) };
            return Err(PlatformError::Platform(format!(
                "RegisterEventHotKey failed: {status}"
            )));
        }

        let spec = ffi::EventTypeSpec {
            event_class: ffi::EVENT_CLASS_KEYBOARD,
            event_kind: ffi::EVENT_HOTKEY_PRESSED,
        };
        let mut handler_ref: ffi::EventHandlerRef = std::ptr::null_mut();
        // SAFETY: hotkey_handler 是 'static extern fn；user_data 指向泄漏的回调盒；
        // type_list 指向栈上 spec（InstallEventHandler 同步拷贝）。
        let status = unsafe {
            ffi::InstallEventHandler(
                ffi::GetEventDispatcherTarget(),
                hotkey_handler,
                1,
                &spec,
                callback_ptr.cast(),
                &mut handler_ref,
            )
        };
        if status != ffi::NO_ERR {
            // SAFETY: 回滚已注册热键并释放回调盒。
            unsafe {
                ffi::UnregisterEventHotKey(hot_key_ref);
                drop(Box::from_raw(callback_ptr));
            }
            return Err(PlatformError::Platform(format!(
                "InstallEventHandler failed: {status}"
            )));
        }

        Ok(HotkeyListener {
            macos: Some(RegisteredHotkey {
                hot_key_ref,
                handler_ref,
                callback: callback_ptr,
            }),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn carbon_modifiers_map_host_hotkey() {
            let mask = carbon_modifiers(HotkeyModifiers {
                control: true,
                shift: true,
                ..HotkeyModifiers::none()
            });
            assert_eq!(mask & ffi::MOD_CONTROL, ffi::MOD_CONTROL);
            assert_eq!(mask & ffi::MOD_SHIFT, ffi::MOD_SHIFT);
            assert_eq!(mask & ffi::MOD_CMD, 0);
            assert_eq!(mask & ffi::MOD_OPTION, 0);
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

    pub(super) fn register_thread_hotkey(hotkey: Hotkey) -> Result<(), PlatformError> {
        // SAFETY: NULL HWND requests delivery to the current thread's message queue. The
        // Slint/winit event loop is created and run on this same thread and owns the msg hook.
        unsafe { SetLastError(0) };
        let ok = unsafe {
            RegisterHotKey(
                0,
                hotkey.id as i32,
                modifiers_bits(hotkey.modifiers),
                hotkey.virtual_key,
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(PlatformError::Platform(format!(
                "thread RegisterHotKey failed: {err}"
            )));
        }
        Ok(())
    }

    pub(super) fn unregister_thread_hotkey(hotkey_id: u32) -> Result<(), PlatformError> {
        // SAFETY: NULL HWND and id identify the registration made on this same UI thread.
        unsafe { SetLastError(0) };
        let ok = unsafe { UnregisterHotKey(0, hotkey_id as i32) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(PlatformError::Platform(format!(
                "thread UnregisterHotKey failed: {err}"
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
