//! Mindweave core: pure-Rust document model and file format.

pub mod color;
pub mod document;
pub mod files;
pub mod format;
pub mod geometry;
pub mod session;
pub mod settings;
pub mod svg;
pub mod viewport;

pub use color::{format_hex_color, normalize_hex_color, parse_hex_color};
pub use document::{BackgroundPattern, CanvasSettings, Document, Layer, Page, Viewport};
pub use files::{rename_document, validate_new_stem, RenameError, MAX_STEM_LEN};
pub use format::{load, save, FormatError, FORMAT_VERSION};
pub use session::DocumentSession;
pub use settings::{
    load_settings, save_settings, settings_path, AppSettings, RecentFile, WindowSize, MAX_RECENT,
};
pub use viewport::{
    dot_stride, grid_lines, is_major_line, pinch_zoom_factor, scroll_zoom_factor, stride_offset,
    GridLines, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP,
};
