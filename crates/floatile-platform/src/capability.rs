//! 平台能力探测：运行环境（X11 / Wayland / 未知）与降级所需的可用性信息。
//!
//! 探测结果驱动 `docs/platform-matrix/platform-matrix.md` 中的降级策略。

use std::env;

/// 当前运行环境。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
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

/// 探测当前平台能力（纯 env 探测，无 I/O 副作用失败时归为 Unknown）。
pub fn probe() -> PlatformCapabilities {
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let has_wayland_display = env::var("WAYLAND_DISPLAY").is_ok();
    let has_x11_display = env::var("DISPLAY").is_ok();

    let kind =
        if has_wayland_display || (session.eq_ignore_ascii_case("wayland") && !has_x11_display) {
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
            // X11 下无法仅凭 env 确知合成器；S2 真实探测落地前保守降级为不透明。
            compositing: false,
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
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static DISPLAY_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore_env(key: &str, value: Option<OsString>) {
        // SAFETY: Callers hold DISPLAY_ENV_LOCK for the complete mutation/probe/restore sequence.
        unsafe {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }

    #[test]
    fn wayland_env_detected() {
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

    #[test]
    fn x11_env_detected() {
        let _guard = DISPLAY_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old_wayland = env::var_os("WAYLAND_DISPLAY");
        let old_x11 = env::var_os("DISPLAY");

        // SAFETY: Display-variable tests serialize mutation through DISPLAY_ENV_LOCK.
        unsafe {
            env::remove_var("WAYLAND_DISPLAY");
            env::set_var("DISPLAY", ":0");
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
