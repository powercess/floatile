pub mod capability;
pub mod window;

pub use capability::{PlatformCapabilities, PlatformKind};
pub use window::{PlatformError, WindowOptions, apply_window_options, start_window_drag};
