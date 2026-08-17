pub mod capability;
pub mod hotkey;
pub mod metrics;
pub mod monitor;
pub mod window;

#[cfg(target_os = "linux")]
mod x11;

use std::path::PathBuf;

/// 宿主数据目录（布局数据库、未来插件数据等）。
///
/// 平台约定：
/// - Linux：`$XDG_DATA_HOME/floatile`，未设置时为 `~/.local/share/floatile`
/// - Windows：`%APPDATA%\floatile`
/// - macOS：`~/Library/Application Support/floatile`
///
/// 返回的目录可能不存在；调用方负责 `create_dir_all`。不引入新依赖，
/// 避免为路径解析增加第三方 crate。
pub fn data_dir() -> Result<PathBuf, PlatformError> {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA")
            .ok_or_else(|| PlatformError::Platform("APPDATA 未设置".to_owned()))?;
        Ok(PathBuf::from(base).join("floatile"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| PlatformError::Platform("HOME 未设置".to_owned()))?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("floatile"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(xdg).join("floatile"));
        }
        let home = std::env::var_os("HOME")
            .ok_or_else(|| PlatformError::Platform("HOME 未设置".to_owned()))?;
        Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("floatile"))
    }
}

pub use capability::{
    CapabilityState, CapabilityUnavailableReason, PlatformCapabilities, PlatformKind,
};
#[cfg(windows)]
pub use hotkey::install_hotkey_message_hook;
pub use hotkey::{
    Hotkey, HotkeyListener, HotkeyModifiers, listen_hotkey, register_hotkey, unregister_hotkey,
};
pub use metrics::{MetricsError, ProcessMetrics, process_metrics};
pub use monitor::{MonitorInfo, MonitorKeySource, enumerate_monitors, to_monitor_layout};
pub use window::{
    PlatformError, WindowOptions, apply_window_options, remove_window_decorations, resize_window,
    set_always_on_top, set_click_through, set_window_position, start_window_drag,
};
