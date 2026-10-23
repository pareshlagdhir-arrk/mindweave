//! Mindweave core: pure-Rust document model and file format.

pub mod arrange;
pub mod artboard;
pub mod clipboard;
pub mod color;
pub mod document;
pub mod files;
pub mod format;
pub mod geometry;
pub mod history;
pub mod icons;
pub mod image;
pub mod library;
pub mod line;
pub mod mindmap;
pub mod path_shape;
pub mod session;
pub mod settings;
pub mod shape;
pub mod svg;
pub mod svg_import;
pub mod template;
pub mod text;
pub mod text_history;
pub mod transform;
pub mod viewport;

pub use arrange::{expand_to_units, unit_members, unit_of, whole_units_only, Arrange};
pub use artboard::{
    artboard_placement, membership, with_members, ArtboardPreset, ArtboardProps, PresetCategory,
    ARTBOARD_GAP, ARTBOARD_PRESETS, ARTBOARD_SHAPE,
};
pub use clipboard::{
    duplicate, from_clipboard, is_mindweave_payload, is_newer_payload, plain_text_box,
    prepare_paste, to_clipboard, CLIPBOARD_MARKER, CLIPBOARD_VERSION, MAX_CLIPBOARD_BYTES,
    MAX_CLIPBOARD_SHAPES, MAX_PASTE_CHARS,
};
pub use color::{
    auto_text_color, format_hex_color, is_auto_color, is_ink_color, normalize_hex_color,
    normalize_style_color, parse_hex_color, relative_luminance, AUTO_COLOR, INK_COLOR,
};
pub use document::{BackgroundPattern, CanvasSettings, Document, Layer, Page, Viewport};
pub use files::{rename_document, validate_new_stem, RenameError, MAX_STEM_LEN};
pub use format::{load, save, FormatError, FORMAT_VERSION};
pub use icons::{
    import_icon_folder, import_icon_zip, load_icon_sets, search_icons, user_icons_dir, IconEntry,
    IconImportError, IconSet,
};
pub use image::{
    collect_images, image_id, placed_size, resolve_images, ImageData, ImageFormat, ImageStore,
    StoredImage, IMAGE_SHAPE, MAX_IMAGE_BYTES,
};
pub use library::{
    builtin_libraries, load_library, user_libraries_dir, Charge, DirFiles, EmbeddedFiles, Icon,
    Library, LibraryError, LibraryFiles, LibraryItem, LibraryRegistry, LoadReport, ParamDef,
    ShapeDef, ShapeSource, StyleDefaults, LIBRARY_FORMAT, MAX_FILE_BYTES, MAX_LIBRARY_BYTES,
    MAX_SHAPES, MAX_USER_LIBRARIES,
};
pub use line::{
    bend_through, double_outline, edge_point, line_frame, line_geometry, line_geometry_min_ring,
    line_jumps, line_midpoint, route_line, ArrowHead, LineDash, LineEnd, LineJump, LineProps,
    LineRoute, Target, JUMP_SIZES, LINE_LIBRARY, LINE_SHAPE,
};
pub use mindmap::{
    branch_style, clamp_topic_picture, fit_topic_picture, map_decor, rounded_polygon,
    BoundaryProps, BranchStyle, Direction, Equation, MapDecor, Structure, TopicParts, TopicProps,
    BOUNDARY_SHAPE, BRANCH_COLORS, TOPIC_ICON_SIZE, TOPIC_IMAGE_SIZE, TOPIC_PADDING, TOPIC_SHAPE,
    TOPIC_TEXT_MAX,
};
pub use path_shape::{PathProps, PATH_SHAPE};
pub use session::DocumentSession;
pub use settings::{
    load_settings, save_settings, settings_path, AppSettings, RecentFile, WindowSize, MAX_RECENT,
    MAX_RECENT_COLORS, MAX_RECENT_FONTS,
};
pub use shape::{Label, Shape, Style, TextBoxProps, VAlign, TEXT_LIBRARY, TEXT_SHAPE};
pub use svg_import::{
    import_svg, place_import, SvgImport, SvgImportError, MAX_IMPORT_BYTES, MAX_IMPORT_OBJECTS,
};
pub use template::{
    instantiate_line_template, instantiate_template, PartKind, PartLabel, TemplateDef,
    TemplatePart, MAX_TEMPLATE_PARTS,
};
pub use text::{
    HAlign, Mixed, Paragraph, RichText, Run, RunStyle, StyleChange, StyleSummary,
    DEFAULT_FONT_SIZE, LABEL_FONT_SIZE, LINE_BREAK, MAX_FONT_SIZE, MIN_FONT_SIZE,
};
pub use text_history::{EditKind, EditSnapshot, TextHistory};
pub use transform::{
    drag_create, frame_aabb, handle_positions, marquee_selects, resize, resize_with_text_boxes,
    rotate, rotate_handle_position, selection_bounds, translate, Handle, CLICK_DRAG_THRESHOLD,
    DUPLICATE_OFFSET, MIN_SIZE,
};
pub use viewport::{
    dot_stride, grid_lines, is_major_line, pinch_zoom_factor, scroll_zoom_factor, stride_offset,
    GridLines, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP,
};
