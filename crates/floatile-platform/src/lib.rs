pub mod capability;
pub mod hotkey;
pub mod metrics;
pub mod monitor;
pub mod window;

#[cfg(target_os = "linux")]
mod x11;

pub use capability::{
    CapabilityState, CapabilityUnavailableReason, PlatformCapabilities, PlatformKind,
};
#[cfg(windows)]
pub use hotkey::install_hotkey_message_hook;
pub use hotkey::{
    Hotkey, HotkeyListener, HotkeyModifiers, listen_hotkey, register_hotkey, unregister_hotkey,
};
pub use metrics::{MetricsError, ProcessMetrics, process_metrics};
pub use monitor::{
    MonitorInfo, MonitorKeySource, PhysicalPosition, PhysicalSize, enumerate_monitors,
};
pub use window::{
    PlatformError, WindowOptions, apply_window_options, remove_window_decorations, resize_window,
    set_always_on_top, set_click_through, start_window_drag,
};
