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

/// 探测得到的能力集合。
#[derive(Debug, Clone)]
pub struct PlatformCapabilities {
    pub kind: PlatformKind,
    /// 是否检测到合成器（X11 透明依赖合成器；Wayland 恒为 true）。
    pub compositing: bool,
    /// 原生 Wayland 下点击穿透不可用。
    pub click_through: bool,
    /// 置顶是否可用（Wayland 非 layer-shell 不可用）。
    pub always_on_top: bool,
}
#[cfg(target_os = "linux")]
fn x11_compositor_available() -> bool {
    use x11rb::protocol::xproto::ConnectionExt as _;

    let Ok((connection, screen_number)) = x11rb::connect(None) else {
        return false;
    };
    let selection_name = format!("_NET_WM_CM_S{screen_number}");
    let Ok(atom_cookie) = connection.intern_atom(false, selection_name.as_bytes()) else {
        return false;
    };
    let Ok(atom_reply) = atom_cookie.reply() else {
        return false;
    };
    let Ok(owner_cookie) = connection.get_selection_owner(atom_reply.atom) else {
        return false;
    };
    let Ok(owner_reply) = owner_cookie.reply() else {
        return false;
    };

    owner_reply.owner != x11rb::NONE
}

#[cfg(not(target_os = "linux"))]
fn x11_compositor_available() -> bool {
    false
}

/// 探测当前平台能力。
///
/// Windows 走 DWM 合成桌面路径：Windows 8.1+ 的 DWM 始终合成，透明、点击穿透
/// （`WS_EX_TRANSPARENT`）与置顶（`HWND_TOPMOST`）由 `floatile-platform::window` 落地。
/// Linux X11 查询 `_NET_WM_CM_Sn` selection owner；连接或查询失败时保守判定为无合成器。
/// 其余平台使用显示环境识别会话类型，不把 OS 名称当作能力证明。
pub fn probe() -> PlatformCapabilities {
    #[cfg(target_os = "windows")]
    {
        PlatformCapabilities {
            kind: PlatformKind::Windows,
            compositing: true,
            click_through: true,
            always_on_top: true,
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
                compositing: true,
                click_through: false,
                always_on_top: false,
            },
            PlatformKind::X11 => PlatformCapabilities {
                kind,
                compositing: x11_compositor_available(),
                // S2 的 XShape 实现落地前不得把设计预期报告为可用能力。
                click_through: false,
                always_on_top: true,
            },
            PlatformKind::Unknown => PlatformCapabilities {
                kind,
                compositing: false,
                click_through: false,
                always_on_top: false,
            },
            // 非 Windows 分支永远构造不出 Windows；此分支只为穷尽匹配。
            PlatformKind::Windows => PlatformCapabilities {
                kind,
                compositing: false,
                click_through: false,
                always_on_top: false,
            },
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
        assert!(caps.compositing);
        assert!(caps.click_through);
        assert!(caps.always_on_top);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn wayland_env_detected() {
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
        assert!(!caps.click_through);
        assert!(!caps.always_on_top);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn x11_without_reachable_compositor_degrades() {
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
        assert!(!caps.compositing);
        assert!(!caps.click_through);
        assert!(caps.always_on_top);
    }
}
