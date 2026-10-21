//! Mindweave core: pure-Rust document model and file format.

pub mod clipboard;
pub mod color;
pub mod document;
pub mod files;
pub mod format;
pub mod geometry;
pub mod history;
pub mod library;
pub mod session;
pub mod settings;
pub mod shape;
pub mod svg;
pub mod text;
pub mod text_history;
pub mod transform;
pub mod viewport;

pub use clipboard::{
    duplicate, from_clipboard, prepare_paste, to_clipboard, CLIPBOARD_MARKER, CLIPBOARD_VERSION,
    MAX_CLIPBOARD_BYTES, MAX_CLIPBOARD_SHAPES,
};
pub use color::{
    auto_text_color, format_hex_color, normalize_hex_color, parse_hex_color, relative_luminance,
};
pub use document::{BackgroundPattern, CanvasSettings, Document, Layer, Page, Viewport};
pub use files::{rename_document, validate_new_stem, RenameError, MAX_STEM_LEN};
pub use format::{load, save, FormatError, FORMAT_VERSION};
pub use library::{
    builtin_libraries, load_library, user_libraries_dir, Charge, DirFiles, EmbeddedFiles, Icon,
    Library, LibraryError, LibraryFiles, LibraryRegistry, LoadReport, ParamDef, ShapeDef,
    ShapeSource, StyleDefaults, LIBRARY_FORMAT, MAX_FILE_BYTES, MAX_LIBRARY_BYTES, MAX_SHAPES,
    MAX_USER_LIBRARIES,
};
pub use session::DocumentSession;
pub use settings::{
    load_settings, save_settings, settings_path, AppSettings, RecentFile, WindowSize, MAX_RECENT,
};
pub use shape::{Shape, Style};
pub use text::{
    HAlign, Mixed, Paragraph, RichText, Run, RunStyle, StyleChange, StyleSummary,
    DEFAULT_FONT_SIZE, LABEL_FONT_SIZE, LINE_BREAK, MAX_FONT_SIZE, MIN_FONT_SIZE,
};
pub use text_history::{EditKind, EditSnapshot, TextHistory};
pub use transform::{
    drag_create, frame_aabb, handle_positions, marquee_selects, resize, rotate,
    rotate_handle_position, selection_bounds, translate, Handle, CLICK_DRAG_THRESHOLD,
    DUPLICATE_OFFSET, MIN_SIZE,
};
pub use viewport::{
    dot_stride, grid_lines, is_major_line, pinch_zoom_factor, scroll_zoom_factor, stride_offset,
    GridLines, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP,
};
