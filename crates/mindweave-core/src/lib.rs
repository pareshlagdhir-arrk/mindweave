//! Mindweave core: pure-Rust document model and file format.

pub mod document;
pub mod format;
pub mod session;
pub mod settings;
pub mod viewport;

pub use document::{BackgroundPattern, CanvasSettings, Document, Layer, Page, Viewport};
pub use format::{load, save, FormatError, FORMAT_VERSION};
pub use session::DocumentSession;
pub use settings::{
    load_settings, save_settings, settings_path, AppSettings, RecentFile, WindowSize, MAX_RECENT,
};
pub use viewport::{
    dot_stride, grid_lines, pinch_zoom_factor, scroll_zoom_factor, GridLines, ZOOM_MAX, ZOOM_MIN,
    ZOOM_STEP,
};
