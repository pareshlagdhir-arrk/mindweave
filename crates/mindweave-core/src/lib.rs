//! Mindweave core: pure-Rust document model and file format.

pub mod document;
pub mod format;
pub mod session;
pub mod viewport;

pub use document::{BackgroundPattern, CanvasSettings, Document, Layer, Page, Viewport};
pub use format::{load, save, FormatError, FORMAT_VERSION};
pub use session::DocumentSession;
pub use viewport::{grid_lines, GridLines, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP};
