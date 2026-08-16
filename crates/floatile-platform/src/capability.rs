//! 平台能力探测：运行环境（Windows / X11 / Wayland / 未知）与降级所需的可用性信息。
//!
//! 探测结果驱动 `docs/platform-matrix/platform-matrix.md` 中的降级策略。

/// 当前运行环境。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    /// Windows（DWM 合成桌面）。
    Windows,
    /// X11（含 XWayland）。
    X11,
    /// 原生 Wayland。
    Wayland,
    /// 未知/无显示环境。
    Unknown,
}

/// 能力不可用的可观测原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityUnavailableReason {
    /// 没有可连接的显示服务器。
    DisplayUnavailable,
    /// X11 未检测到合成器 selection owner。
    CompositorNotDetected,
    /// 显示服务器没有所需扩展。
    ExtensionUnavailable,
    /// 窗口管理器未声明所需 EWMH 能力。
    WindowManagerUnsupported,
    /// 当前显示协议不提供该能力。
    ProtocolUnsupported,
    /// 当前平台实现尚未提供该能力。
    NotImplemented,
    /// 显示服务器可连接，但能力查询失败。
    ProbeFailed,
}

/// 单项平台能力的探测结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityState {
    Available,
    Unavailable(CapabilityUnavailableReason),
}

impl CapabilityState {
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }

    pub const fn unavailable(reason: CapabilityUnavailableReason) -> Self {
        Self::Unavailable(reason)
    }

    pub const fn unavailable_reason(self) -> Option<CapabilityUnavailableReason> {
        match self {
            Self::Available => None,
            Self::Unavailable(reason) => Some(reason),
        }
    }
}

/// 探测得到的能力集合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub kind: PlatformKind,
    /// X11 透明依赖合成器；Wayland 协议本身由合成器提供。
    pub compositing: CapabilityState,
    /// 展示模式能否安全启用点击穿透。
    pub click_through: CapabilityState,
    /// 窗口能否保持在普通窗口之上。
    pub always_on_top: CapabilityState,
}

#[cfg(not(target_os = "windows"))]
fn unavailable_capabilities(
    kind: PlatformKind,
    reason: CapabilityUnavailableReason,
) -> PlatformCapabilities {
    PlatformCapabilities {
        kind,
        compositing: CapabilityState::unavailable(reason),
        click_through: CapabilityState::unavailable(reason),
        always_on_top: CapabilityState::unavailable(reason),
    }
}

/// 探测当前平台能力。
///
/// Windows 走 DWM 合成桌面路径：Windows 8.1+ 的 DWM 始终合成，透明、点击穿透
/// （`WS_EX_TRANSPARENT`）与置顶（`HWND_TOPMOST`）由 `floatile-platform::window` 落地。
/// Linux X11 查询 compositor selection、SHAPE 扩展与 EWMH `_NET_WM_STATE_ABOVE`；
/// 连接或查询失败时返回明确降级原因。其余平台使用显示环境识别会话类型，不把 OS
/// 名称当作能力证明。
pub fn probe() -> PlatformCapabilities {
    #[cfg(target_os = "windows")]
    {
        PlatformCapabilities {
            kind: PlatformKind::Windows,
            compositing: CapabilityState::Available,
            click_through: CapabilityState::Available,
            always_on_top: CapabilityState::Available,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::env;

        let session = env::var("XDG_SESSION_TYPE").unwrap_or_default();
        let has_wayland_display = env::var("WAYLAND_DISPLAY").is_ok();
        let has_x11_display = env::var("DISPLAY").is_ok();

        let kind = if has_wayland_display
            || (session.eq_ignore_ascii_case("wayland") && !has_x11_display)
        {
            PlatformKind::Wayland
        } else if has_x11_display || session.eq_ignore_ascii_case("x11") {
            PlatformKind::X11
        } else {
            PlatformKind::Unknown
        };

        match kind {
            PlatformKind::Wayland => PlatformCapabilities {
                kind,
                compositing: CapabilityState::Available,
                click_through: CapabilityState::unavailable(
                    CapabilityUnavailableReason::ProtocolUnsupported,
                ),
                always_on_top: CapabilityState::unavailable(
                    CapabilityUnavailableReason::ProtocolUnsupported,
                ),
            },
            PlatformKind::X11 => {
                #[cfg(target_os = "linux")]
                {
                    crate::x11::probe_capabilities()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    unavailable_capabilities(kind, CapabilityUnavailableReason::NotImplemented)
                }
            }
            PlatformKind::Unknown => {
                unavailable_capabilities(kind, CapabilityUnavailableReason::DisplayUnavailable)
            }
            // 非 Windows 分支永远构造不出 Windows；此分支只为穷尽匹配。
            PlatformKind::Windows => {
                unavailable_capabilities(kind, CapabilityUnavailableReason::NotImplemented)
            }
        }
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "windows"))]
    use std::ffi::OsString;
    #[cfg(not(target_os = "windows"))]
    use std::sync::Mutex;

    #[cfg(not(target_os = "windows"))]
    static DISPLAY_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(not(target_os = "windows"))]
    fn restore_env(key: &str, value: Option<OsString>) {
        use std::env;
        // SAFETY: Callers hold DISPLAY_ENV_LOCK for the complete mutation/probe/restore sequence.
        unsafe {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_dwm_capabilities_reported() {
        let caps = probe();
        assert_eq!(caps.kind, PlatformKind::Windows);
        assert!(caps.compositing.is_available());
        assert!(caps.click_through.is_available());
        assert!(caps.always_on_top.is_available());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn wayland_env_reports_protocol_degradation() {
        use std::env;
        let _guard = DISPLAY_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old_wayland = env::var_os("WAYLAND_DISPLAY");
        let old_x11 = env::var_os("DISPLAY");

        // SAFETY: Display-variable tests serialize mutation through DISPLAY_ENV_LOCK.
        unsafe {
            env::set_var("WAYLAND_DISPLAY", "wayland-0");
            env::remove_var("DISPLAY");
        }
        let caps = probe();
        restore_env("WAYLAND_DISPLAY", old_wayland);
        restore_env("DISPLAY", old_x11);

        assert_eq!(caps.kind, PlatformKind::Wayland);
        assert!(caps.compositing.is_available());
        assert_eq!(
            caps.click_through.unavailable_reason(),
            Some(CapabilityUnavailableReason::ProtocolUnsupported)
        );
        assert_eq!(
            caps.always_on_top.unavailable_reason(),
            Some(CapabilityUnavailableReason::ProtocolUnsupported)
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unreachable_x11_display_reports_connection_degradation() {
        use std::env;
        let _guard = DISPLAY_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old_wayland = env::var_os("WAYLAND_DISPLAY");
        let old_x11 = env::var_os("DISPLAY");

        // SAFETY: Display-variable tests serialize mutation through DISPLAY_ENV_LOCK.
        unsafe {
            env::remove_var("WAYLAND_DISPLAY");
            env::set_var("DISPLAY", "floatile-invalid-display");
        }
        let caps = probe();
        restore_env("WAYLAND_DISPLAY", old_wayland);
        restore_env("DISPLAY", old_x11);

        assert_eq!(caps.kind, PlatformKind::X11);
        assert_eq!(
            caps.compositing.unavailable_reason(),
            Some(CapabilityUnavailableReason::DisplayUnavailable)
        );
        assert_eq!(caps.click_through, caps.compositing);
        assert_eq!(caps.always_on_top, caps.compositing);
    }
}
