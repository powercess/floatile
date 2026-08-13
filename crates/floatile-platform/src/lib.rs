pub mod capability;
pub mod hotkey;
pub mod window;

pub use capability::{PlatformCapabilities, PlatformKind};
pub use hotkey::{Hotkey, HotkeyModifiers, extract_hotkey_id, register_hotkey, unregister_hotkey};
pub use window::{
    PlatformError, WindowOptions, apply_window_options, remove_window_decorations, resize_window,
    set_click_through, start_window_drag,
};
