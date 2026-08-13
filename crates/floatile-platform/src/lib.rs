pub mod capability;
pub mod window;

pub use capability::{PlatformCapabilities, PlatformKind};
pub use window::{
    PlatformError, WindowOptions, apply_window_options, set_click_through, start_window_drag,
};
