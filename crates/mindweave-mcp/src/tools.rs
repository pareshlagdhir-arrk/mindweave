//! The tools: each opens the document at `path`, runs one command of the
//! core command layer, saves when it changed something, and answers with
//! compact JSON (created ids and a short summary).

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use mindweave_core::export::{export_svg, ExportOptions, ExportRegion, Theme};
use mindweave_core::ops::{
    artboard_presets, ArrangeHow, ArtboardSize, ConnectOptions, ObjectFilter, ObjectKind,
    StyleInput, TextInput, TopicEdit, TopicNode, TopicPosition, Workspace,
};
use mindweave_core::{
    builtin_libraries, search_icons, user_icons_dir, user_libraries_dir, LibraryRegistry,
    TOPIC_ICON_SIZE, TOPIC_IMAGE_SIZE,
};
use serde_json::{json, Map, Value};

use crate::args::{check_folder, expand, write_atomic, Args};
use crate::pictures;
use crate::render;

/// Why a tool call failed.
#[derive(Debug, PartialEq)]
pub enum ToolError {
    /// No tool has this name (a protocol error).
    UnknownTool,
    /// The tool ran and failed; the message says why.
    Failed(String),
}

/// What a tool answers: MCP content items.
type Content = Vec<Value>;
/// A tool's result.
type ToolResult = Result<Content, String>;

/// What the tools share: the libraries shapes come from.
pub struct Context {
    libs: Arc<LibraryRegistry>,
    /// Where the icon sets imported in the app are.
    icons_dir: std::path::PathBuf,
}

/// One tool.
struct Tool {
    name: &'static str,
    description: &'static str,
    /// Its arguments other than `path`: (name, schema).
    props: fn() -> Vec<(&'static str, Value)>,
    /// Required arguments other than `path`.
    required: &'static [&'static str],
    /// Whether `path` is required.
    needs_path: bool,
    run: fn(&Context, &Args) -> ToolResult,
}

// Schema helpers.

fn s(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn num(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}

fn int(desc: &str) -> Value {
    json!({"type": "integer", "minimum": 0, "description": desc})
}

fn boolean(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}

fn one_of(desc: &str, values: &[&str]) -> Value {
    json!({"type": "string", "enum": values, "description": desc})
}

fn id(desc: &str) -> Value {
    json!({"type": "string", "description": format!("{desc} (an object id)")})
}

fn ids(desc: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": crate::args::MAX_IDS, "description": desc})
}

const COLOR: &str = "#RRGGBB, `auto` (follows the theme: canvas colour for fills, text colour for strokes), `ink` (the theme's text colour) or `none`";
const HEADS: [&str; 7] = [
    "none",
    "arrow",
    "triangle",
    "diamond",
    "circle",
    "hollow-triangle",
    "hollow-diamond",
];

fn page() -> (&'static str, Value) {
    (
        "page",
        int("Page index from 0; the active page when left out"),
    )
}

fn style_props() -> Vec<(&'static str, Value)> {
    vec![
        ("fill", s(&format!("Fill colour: {COLOR}"))),
        ("stroke", s(&format!("Outline colour: {COLOR}"))),
        ("stroke_width", num("Outline width, 0 to 100")),
        (
            "dash",
            one_of("Outline pattern", &["solid", "dashed", "dotted"]),
        ),
        ("opacity", num("0 (invisible) to 1 (opaque)")),
        ("corner_radius", num("Corner radius for rectangles")),
    ]
}

fn text_look_props() -> Vec<(&'static str, Value)> {
    vec![
        (
            "font",
            s("Font family name; empty for the app's default font"),
        ),
        (
            "size",
            num("Font size, 1 to 1000 (default 16 for text boxes, 14 for labels)"),
        ),
        ("bold", boolean("Bold")),
        ("italic", boolean("Italic")),
        (
            "color",
            s("Text colour #RRGGBB, or `auto` to contrast with what is behind"),
        ),
        (
            "align",
            one_of("Horizontal alignment", &["left", "center", "right"]),
        ),
    ]
}

fn line_props() -> Vec<(&'static str, Value)> {
    vec![
        (
            "start_head",
            one_of("Mark at the start (default none)", &HEADS),
        ),
        ("end_head", one_of("Mark at the end", &HEADS)),
        (
            "dash",
            one_of("Line pattern", &["solid", "dashed", "dotted"]),
        ),
        (
            "route",
            one_of(
                "Path between the ends (default straight)",
                &["straight", "angular", "rounded", "curved"],
            ),
        ),
        ("text", s("A label drawn on the middle of the line")),
        ("stroke", s(&format!("Line colour: {COLOR}"))),
        ("stroke_width", num("Line width, 0 to 100 (default 2)")),
    ]
}

fn region_props() -> Vec<(&'static str, Value)> {
    vec![
        page(),
        (
            "artboard_id",
            id("Export only this artboard, cut to its frame"),
        ),
        (
            "ids",
            ids("Export only these objects (a central topic brings its whole map)"),
        ),
        (
            "theme",
            one_of(
                "Colours for `auto` and `ink` (default light)",
                &["light", "dark"],
            ),
        ),
        (
            "padding",
            num("Space round the picture (default 24; not for an artboard)"),
        ),
    ]
}

fn task_schema() -> Value {
    json!({
        "type": "object",
        "description": "Makes the topic a task (topics added under a task are tasks too). A task with subtasks rolls up their progress (the average), status and dates; its own progress and status are ignored.",
        "properties": {
            "description": {"type": "string", "description": "What needs doing, in simple Markdown like a note (empty clears it)"},
            "start": {"type": "string", "description": "Start date, YYYY-MM-DD (empty clears it)"},
            "end": {"type": "string", "description": "End date, YYYY-MM-DD (empty clears it)"},
            "progress": {"type": "integer", "minimum": 0, "maximum": 100, "description": "Percent done; 100 is done"},
            "status": {"type": "string", "enum": ["not-started", "in-progress", "blocked", "done"], "description": "done sets progress to 100, not-started to 0"},
            "remove": {"type": "boolean", "description": "true: make it a plain topic again"}
        }
    })
}

fn topic_node_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "text": {"type": "string"},
            "note": {"type": "string", "description": "Note in simple Markdown"},
            "equation": {"type": "string", "description": "LaTeX"},
            "link": {"type": "string"},
            "task": task_schema(),
            "children": {"type": "array", "items": {"type": "object"}, "description": "Child topics, same shape"}
        },
        "required": ["text"]
    })
}

/// Every tool, in the order listed.
const TOOLS: &[Tool] = &[
    Tool {
        name: "create_document",
        description: "Create a new, empty .mindweave document (one page, one layer) at `path` and save it. Refuses to replace an existing file.",
        props: || vec![("page_name", s("Name of the first page (default \"Page 1\")"))],
        required: &[],
        needs_path: true,
        run: create_document,
    },
    Tool {
        name: "document_summary",
        description: "Describe a document: its pages (index, name, layers, object counts by kind, mind map and artboard ids) and the active page.",
        props: Vec::new,
        required: &[],
        needs_path: true,
        run: document_summary,
    },
    Tool {
        name: "add_page",
        description: "Add a page at the end of the document and make it the active page. Returns its index.",
        props: || vec![("name", s("Page name (default \"Page N\")"))],
        required: &[],
        needs_path: true,
        run: add_page,
    },
    Tool {
        name: "add_layer",
        description: "Add a layer on top of a page and make it that page's active layer (new objects go there). Returns its index (0 is the top layer).",
        props: || vec![page(), ("name", s("Layer name (default \"Layer N\")"))],
        required: &[],
        needs_path: true,
        run: add_layer,
    },
    Tool {
        name: "set_active",
        description: "Make a page (and optionally one of its layers) active. Tools that take an optional `page` use the active page, and new objects go on its active layer.",
        props: || vec![("page", int("Page index from 0")), ("layer", int("Layer index from 0 (the top layer)"))],
        required: &["page"],
        needs_path: true,
        run: set_active,
    },
    Tool {
        name: "list_objects",
        description: "List the objects of a page, bottom layer first and back to front, with id, kind, frame (x, y, w, h), text, style, groups, artboard, topic tree links and line ends. Filters combine.",
        props: || vec![
            page(),
            ("kind", one_of("Only this kind", &["shape", "text", "line", "relationship", "topic", "summary", "boundary", "artboard", "image", "path"])),
            ("text_contains", s("Only objects whose text or name contains this (any case)")),
            ("artboard", id("Only members of this artboard")),
            ("mind_map", id("Only objects of this mind map (its central topic id)")),
            ("full", boolean("Show texts and notes in full and add text style and parameters (default false)")),
        ],
        required: &[],
        needs_path: true,
        run: list_objects,
    },
    Tool {
        name: "get_object",
        description: "Everything about one object: kind, frame, full text and text style, style, parameters, topic parent/children/note/equation/link/task (rolled up for parent tasks), line ends, boundary or artboard members.",
        props: || vec![("id", id("The object"))],
        required: &["id"],
        needs_path: true,
        run: get_object,
    },
    Tool {
        name: "validate",
        description: "Check the document for broken references (lines attached to missing objects, topics without their map, boundaries or artboard members that are gone, duplicate ids) and shapes from libraries that are not installed.",
        props: Vec::new,
        required: &[],
        needs_path: true,
        run: validate,
    },
    Tool {
        name: "list_libraries",
        description: "The installed shape libraries (built-in and user) with their shapes and templates: ids for add_shape, names and default sizes. `path` is optional.",
        props: Vec::new,
        required: &[],
        needs_path: false,
        run: list_libraries,
    },
    Tool {
        name: "list_artboard_presets",
        description: "The ready-made artboard sizes (phones, tablets, desktop, paper, slides, social) for add_artboard. `path` is optional.",
        props: Vec::new,
        required: &[],
        needs_path: false,
        run: list_artboard_presets,
    },
    Tool {
        name: "list_icons",
        description: "Find icons in the icon sets imported in the Mindweave app, for add_image and set_topic_picture. Returns the sets and up to `limit` icons whose names hold every word of `query`, each as \"Set/name\". `path` is optional.",
        props: || vec![
            ("query", s("Words the icon name must hold, such as \"arrow left\" (default: all icons)")),
            ("set", s("Only this icon set")),
            ("limit", json!({"type": "integer", "minimum": 1, "maximum": 500, "description": "Most icons returned (default 50)"})),
        ],
        required: &[],
        needs_path: false,
        run: list_icons,
    },
    Tool {
        name: "add_mind_map",
        description: "Add a mind map: a central topic, optionally with a whole tree of topics under it in one call. Returns the map id (the central topic's id) and the ids of the topics added, depth first. Topics are laid out automatically.",
        props: || vec![
            page(),
            ("central_text", s("Text of the central topic")),
            ("x", num("Centre of the central topic (default: right of what is on the page)")),
            ("y", num("Centre of the central topic")),
            ("structure", one_of("Layout: map (branches both sides, default), logic (all to the right), org (downwards)", &["map", "logic", "org"])),
            ("topics", json!({"type": "array", "items": topic_node_schema(), "description": "Main topics with their subtrees: {text, note?, equation?, link?, task?, children?}"})),
        ],
        required: &["central_text"],
        needs_path: true,
        run: add_mind_map,
    },
    Tool {
        name: "add_topics",
        description: "Add topics to a mind map relative to a topic: as its children (default), or as siblings after or before it, in the order given. Pass `texts` for one level or `tree` for nested topics with notes. Returns the new ids.",
        props: || vec![
            ("topic_id", id("The topic to add to or beside")),
            ("texts", json!({"type": "array", "items": {"type": "string"}, "description": "Texts of the new topics"})),
            ("tree", json!({"type": "array", "items": topic_node_schema(), "description": "Nested topics (children of topic_id): {text, note?, equation?, link?, task?, children?}"})),
            ("position", one_of("Where the new topics go (default child)", &["child", "after", "before"])),
        ],
        required: &["topic_id"],
        needs_path: true,
        run: add_topics,
    },
    Tool {
        name: "set_topic",
        description: "Change a topic's text, note, equation, link or task. An empty note, equation or link removes it.",
        props: || vec![
            ("id", id("The topic")),
            ("text", s("New text")),
            ("note", s("Note in simple Markdown: one paragraph per line, `- ` bullets, `1. ` numbered items, two spaces per nesting level, **bold**, *italic*")),
            ("equation", s("LaTeX shown below the text, such as E = mc^2")),
            ("link", s("Web address or file link opened from the topic")),
            ("task", task_schema()),
        ],
        required: &["id"],
        needs_path: true,
        run: set_topic,
    },
    Tool {
        name: "add_summary",
        description: "Add a summary topic with a bracket beside sibling topics (same parent). Returns its id.",
        props: || vec![("topic_ids", ids("Sibling topics to sum up")), ("text", s("Summary text (default \"Summary\")"))],
        required: &["topic_ids"],
        needs_path: true,
        run: add_summary,
    },
    Tool {
        name: "add_boundary",
        description: "Draw a boundary round topics and their subtrees. Returns its id (an existing boundary round exactly these topics is returned).",
        props: || vec![("topic_ids", ids("Topics to enclose"))],
        required: &["topic_ids"],
        needs_path: true,
        run: add_boundary,
    },
    Tool {
        name: "add_relationship",
        description: "Draw a relationship (a curved, dashed arrow) between two topics, optionally labelled. Returns its id.",
        props: || vec![("from_id", id("Topic it starts at")), ("to_id", id("Topic it points to")), ("text", s("Label"))],
        required: &["from_id", "to_id"],
        needs_path: true,
        run: add_relationship,
    },
    Tool {
        name: "set_structure",
        description: "Change a mind map's layout and branch line style.",
        props: || vec![
            ("map_id", id("The map's central topic")),
            ("structure", one_of("Layout", &["map", "logic", "org"])),
            ("branch_style", one_of("Branch lines", &["tapered", "curve", "elbow", "straight"])),
        ],
        required: &["map_id"],
        needs_path: true,
        run: set_structure,
    },
    Tool {
        name: "add_shape",
        description: "Place a library shape or template (see list_libraries; Basic Shapes: rectangle, ellipse, line, triangle, diamond, star) with its top-left corner at (x, y). A template comes as several grouped shapes. Placed inside an artboard, it joins it. Returns the new ids.",
        props: || {
            let mut p = vec![
                page(),
                ("library", s("Library id (default basic-shapes)")),
                ("shape", s("Shape or template id, such as rectangle")),
                ("x", num("Left edge")),
                ("y", num("Top edge")),
                ("w", num("Width (default: the shape's default size)")),
                ("h", num("Height")),
                ("text", s("Label inside the shape; \\n starts a new line")),
            ];
            p.extend(style_props());
            p
        },
        required: &["shape", "x", "y"],
        needs_path: true,
        run: add_shape,
    },
    Tool {
        name: "add_image",
        description: "Place a picture on the page: an image file (PNG, JPEG, GIF, WebP, BMP, TIFF or SVG, at most 20 MB) or an icon from list_icons. `x`, `y` is the top-left corner. Give `w` or `h` (the other keeps the proportions), both, or `longest`; by default it keeps its own size up to 600 units (icons: 64). Inside an artboard it joins it. Returns its id.",
        props: || vec![
            page(),
            ("file", s("Absolute path of the image file")),
            ("icon", s("An icon, as list_icons names it: \"Set/name\"")),
            ("x", num("Left edge")),
            ("y", num("Top edge")),
            ("w", num("Width")),
            ("h", num("Height")),
            ("longest", num("Size of the longest side, proportions kept")),
        ],
        required: &["x", "y"],
        needs_path: true,
        run: add_image,
    },
    Tool {
        name: "set_topic_picture",
        description: "Show a picture above a topic's text, as the app's topic picture: an icon from list_icons or an image file. `size` is its longest side (default 32 for icons, 160 for files). `remove: true` takes the picture away. The topic grows to fit.",
        props: || vec![
            ("id", id("The topic")),
            ("file", s("Absolute path of the image file")),
            ("icon", s("An icon, as list_icons names it: \"Set/name\"")),
            ("size", num("Longest side of the picture (12 to 800)")),
            ("remove", boolean("true: remove the topic's picture")),
        ],
        required: &["id"],
        needs_path: true,
        run: set_topic_picture,
    },
    Tool {
        name: "add_text",
        description: "Add a free text box with its top-left corner at (x, y). Without `width` it grows with its text; with it, it wraps. Returns its id.",
        props: || {
            let mut p = vec![
                page(),
                ("x", num("Left edge")),
                ("y", num("Top edge")),
                ("text", s("The text; \\n starts a new paragraph")),
                ("width", num("Wrap width")),
            ];
            p.extend(text_look_props());
            p
        },
        required: &["x", "y", "text"],
        needs_path: true,
        run: add_text,
    },
    Tool {
        name: "connect",
        description: "Draw a connector between two objects (shapes, text, topics), attached at both ends so it follows them when they move. Ends meet the outlines. Default: straight, with a triangle at the end. Returns its id.",
        props: || {
            let mut p = vec![("from_id", id("Object it starts at")), ("to_id", id("Object it ends at"))];
            p.extend(line_props());
            p
        },
        required: &["from_id", "to_id"],
        needs_path: true,
        run: connect,
    },
    Tool {
        name: "add_line",
        description: "Draw a free line from (x1, y1) to (x2, y2), not attached to anything (default: no heads). Returns its id.",
        props: || {
            let mut p = vec![page(), ("x1", num("Start x")), ("y1", num("Start y")), ("x2", num("End x")), ("y2", num("End y"))];
            p.extend(line_props());
            p
        },
        required: &["x1", "y1", "x2", "y2"],
        needs_path: true,
        run: add_line,
    },
    Tool {
        name: "group",
        description: "Group objects (at least two, on one layer) so they move and are selected together. Returns the group id.",
        props: || vec![("ids", ids("Objects to group"))],
        required: &["ids"],
        needs_path: true,
        run: group,
    },
    Tool {
        name: "ungroup",
        description: "Take objects out of their outermost group.",
        props: || vec![("ids", ids("Grouped objects"))],
        required: &["ids"],
        needs_path: true,
        run: ungroup,
    },
    Tool {
        name: "set_style",
        description: "Change fill, outline, dash, opacity or corner radius of objects. Leave out what should stay.",
        props: || {
            let mut p = vec![("ids", ids("Objects to change"))];
            p.extend(style_props());
            p
        },
        required: &["ids"],
        needs_path: true,
        run: set_style,
    },
    Tool {
        name: "set_text",
        description: "Change the text of an object (a shape's label, a text box, a topic, a line's label) and how all of it looks. Leave out what should stay.",
        props: || {
            let mut p = vec![("id", id("The object")), ("text", s("New text; \\n starts a new paragraph"))];
            p.extend(text_look_props());
            p.push(("underline", boolean("Underline")));
            p.push(("valign", one_of("Vertical placement in a shape", &["top", "middle", "bottom"])));
            p
        },
        required: &["id"],
        needs_path: true,
        run: set_text,
    },
    Tool {
        name: "set_name",
        description: "Rename an artboard (its title above it), or name another object.",
        props: || vec![("id", id("The object")), ("name", s("New name"))],
        required: &["id", "name"],
        needs_path: true,
        run: set_name,
    },
    Tool {
        name: "move_objects",
        description: "Move objects by (dx, dy). Artboards bring their members; a central topic moves its whole map; another topic is offset with its subtree. Moving into or out of an artboard changes membership; attached lines follow.",
        props: || vec![("ids", ids("Objects to move")), ("dx", num("Rightwards")), ("dy", num("Downwards"))],
        required: &["ids", "dx", "dy"],
        needs_path: true,
        run: move_objects,
    },
    Tool {
        name: "set_frame",
        description: "Put an object's top-left corner at (x, y) and optionally resize it to w × h (a text box then wraps at w). Not for lines, boundaries or non-central topics.",
        props: || vec![("id", id("The object")), ("x", num("Left edge")), ("y", num("Top edge")), ("w", num("Width")), ("h", num("Height"))],
        required: &["id", "x", "y"],
        needs_path: true,
        run: set_frame,
    },
    Tool {
        name: "delete_objects",
        description: "Delete objects: topics with their subtrees (and their summaries and relationships), artboards with their members. Lines attached to deleted objects stay, let go. Returns every removed id.",
        props: || vec![("ids", ids("Objects to delete"))],
        required: &["ids"],
        needs_path: true,
        run: delete_objects,
    },
    Tool {
        name: "arrange",
        description: "Change the stacking order of objects on their layer.",
        props: || vec![("ids", ids("Objects to move")), ("order", one_of("Where to", &["front", "back", "forward", "backward"]))],
        required: &["ids", "order"],
        needs_path: true,
        run: arrange,
    },
    Tool {
        name: "add_artboard",
        description: "Add an artboard (a named frame, like a phone screen, slide or page) from a preset or a custom width and height. Placed right of the last artboard unless x/y are given. Objects added later with their centre inside it become its members and are clipped to it. Returns its id.",
        props: || vec![
            page(),
            ("preset", s("Preset name from list_artboard_presets, such as \"iPhone 16\", \"A4\" or \"Slide 16:9\"")),
            ("width", num("Custom width")),
            ("height", num("Custom height")),
            ("name", s("Title shown above it (default: the preset's name or \"Artboard\")")),
            ("x", num("Left edge")),
            ("y", num("Top edge")),
        ],
        required: &[],
        needs_path: true,
        run: add_artboard,
    },
    Tool {
        name: "export_svg",
        description: "Export a page, an artboard or some objects as SVG text. Writes it to `out_path` when given (a .svg file), else returns it.",
        props: || {
            let mut p = region_props();
            p.push(("out_path", s("Absolute path of a .svg file to write; an existing file there is replaced")));
            p
        },
        required: &[],
        needs_path: true,
        run: export_svg_tool,
    },
    Tool {
        name: "export_png",
        description: "Render a page, an artboard or some objects to a PNG image and return it (at most 2048 px a side), so you can look at your work. Optionally also writes it to `out_path`. Text is approximate until the app opens the file.",
        props: || {
            let mut p = region_props();
            p.push(("scale", num("Pixels per canvas unit (default 1; at most 8192 px a side)")));
            p.push(("out_path", s("Absolute path of a .png file to write at full size; an existing file there is replaced")));
            p
        },
        required: &[],
        needs_path: true,
        run: export_png_tool,
    },
];

impl Context {
    /// A context with the built-in libraries and those in `libraries_dir`
    /// (the app's user libraries folder when `None`); also returns the
    /// libraries' load notices.
    pub fn new(libraries_dir: Option<&Path>) -> (Context, Vec<String>) {
        Self::with_icons(libraries_dir, None)
    }

    /// [`Self::new`], with the icon sets in `icons_dir` (the app's icons
    /// folder when `None`).
    pub fn with_icons(
        libraries_dir: Option<&Path>,
        icons_dir: Option<&Path>,
    ) -> (Context, Vec<String>) {
        let default_dir = user_libraries_dir();
        let dir = libraries_dir.unwrap_or(&default_dir);
        let (libs, report) = LibraryRegistry::new(builtin_libraries(), Some(dir));
        (
            Context {
                libs: Arc::new(libs),
                icons_dir: icons_dir.map_or_else(user_icons_dir, Path::to_path_buf),
            },
            report.notices,
        )
    }

    /// The `tools/list` entries.
    pub fn tool_list(&self) -> Vec<Value> {
        TOOLS.iter().map(tool_json).collect()
    }

    /// Runs tool `name` with `args`.
    pub fn call(&self, name: &str, args: &Value) -> Result<Content, ToolError> {
        let tool = TOOLS
            .iter()
            .find(|t| t.name == name)
            .ok_or(ToolError::UnknownTool)?;
        let empty = Map::new();
        let map = match args {
            Value::Object(m) => m,
            Value::Null => &empty,
            _ => return Err(ToolError::Failed("The arguments must be an object.".into())),
        };
        let known: Vec<&str> = (tool.props)().iter().map(|(k, _)| *k).collect();
        if let Some(bad) = map
            .keys()
            .find(|k| *k != "path" && !known.contains(&k.as_str()))
        {
            return Err(ToolError::Failed(format!(
                "{name} has no argument `{bad}`; it takes: path, {}.",
                known.join(", ")
            )));
        }
        (tool.run)(self, &Args(map)).map_err(ToolError::Failed)
    }

    /// Opens the document named by `args`.
    fn open(&self, args: &Args) -> Result<Workspace, String> {
        let path = args.doc_path()?;
        if !path.exists() {
            return Err(format!(
                "{} does not exist; call create_document first.",
                path.display()
            ));
        }
        let mut ws = Workspace::open(&path, self.libs.clone()).map_err(|e| e.to_string())?;
        ws.set_equation_measure(Box::new(render::equation_size));
        Ok(ws)
    }

    /// Opens the document, runs `f` and saves.
    fn edit<T>(
        &self,
        args: &Args,
        f: impl FnOnce(&mut Workspace) -> Result<T, mindweave_core::ops::OpError>,
    ) -> Result<T, String> {
        let mut ws = self.open(args)?;
        let out = f(&mut ws).map_err(|e| e.to_string())?;
        ws.save().map_err(|e| e.to_string())?;
        Ok(out)
    }
}

/// The `tools/list` entry of `tool`.
fn tool_json(tool: &Tool) -> Value {
    let mut props = Map::new();
    props.insert(
        "path".into(),
        s("Absolute path of the .mindweave file (a leading ~/ is the home folder)"),
    );
    for (k, v) in (tool.props)() {
        props.insert(k.into(), v);
    }
    let mut required: Vec<&str> = Vec::new();
    if tool.needs_path {
        required.push("path");
    }
    required.extend(tool.required);
    json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        },
    })
}

/// A result: `summary` for people and `data` (an object) for agents, as
/// one compact JSON text.
fn reply(summary: impl Into<String>, data: Value) -> ToolResult {
    let mut obj = match data {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => {
            let mut m = Map::new();
            m.insert("result".into(), other);
            m
        }
    };
    obj.insert("summary".into(), Value::String(summary.into()));
    Ok(vec![
        json!({"type": "text", "text": Value::Object(obj).to_string()}),
    ])
}

/// `v` as JSON.
fn to_json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

// Tools.

fn create_document(ctx: &Context, a: &Args) -> ToolResult {
    let path = a.doc_path()?;
    check_folder(&path)?;
    let ws = Workspace::create(&path, a.str_opt("page_name")?, ctx.libs.clone())
        .map_err(|e| e.to_string())?;
    reply(
        format!("Created {}.", path.display()),
        json!({"document": to_json(&ws.summary())}),
    )
}

fn document_summary(ctx: &Context, a: &Args) -> ToolResult {
    let ws = ctx.open(a)?;
    let sum = ws.summary();
    let objects: usize = sum
        .pages
        .iter()
        .flat_map(|p| &p.layers)
        .map(|l| l.objects)
        .sum();
    reply(
        format!("{} page(s), {objects} object(s).", sum.pages.len()),
        json!({"document": to_json(&sum)}),
    )
}

fn add_page(ctx: &Context, a: &Args) -> ToolResult {
    let name = a.str_opt("name")?;
    let idx = ctx.edit(a, |ws| ws.add_page(name))?;
    reply(
        format!("Added page {idx}; it is now active."),
        json!({"page": idx}),
    )
}

fn add_layer(ctx: &Context, a: &Args) -> ToolResult {
    let (page, name) = (a.usize_opt("page")?, a.str_opt("name")?);
    let idx = ctx.edit(a, |ws| ws.add_layer(page, name))?;
    reply(
        format!("Added layer {idx} (the top layer)."),
        json!({"layer": idx}),
    )
}

fn set_active(ctx: &Context, a: &Args) -> ToolResult {
    let page = a.usize_opt("page")?.ok_or("`page` is required.")?;
    let layer = a.usize_opt("layer")?;
    ctx.edit(a, |ws| ws.set_active(page, layer))?;
    reply(format!("Page {page} is active."), Value::Null)
}

fn list_objects(ctx: &Context, a: &Args) -> ToolResult {
    let ws = ctx.open(a)?;
    let filter = ObjectFilter {
        kind: a.parse_opt::<ObjectKind>("kind", "an object kind")?,
        text_contains: a.str_opt("text_contains")?.map(str::to_string),
        artboard: a.id_opt("artboard")?,
        mind_map: a.id_opt("mind_map")?,
    };
    let full = a.bool_opt("full")?.unwrap_or(false);
    let objects = ws
        .list_objects(a.usize_opt("page")?, &filter, full)
        .map_err(|e| e.to_string())?;
    reply(
        format!("{} object(s).", objects.len()),
        json!({"objects": to_json(&objects)}),
    )
}

fn get_object(ctx: &Context, a: &Args) -> ToolResult {
    let ws = ctx.open(a)?;
    let info = ws.get_object(a.id_req("id")?).map_err(|e| e.to_string())?;
    reply(
        format!("A {}.", info.kind.name()),
        json!({"object": to_json(&info)}),
    )
}

fn validate(ctx: &Context, a: &Args) -> ToolResult {
    let ws = ctx.open(a)?;
    let problems = ws.validate();
    let summary = if problems.is_empty() {
        "No problems found.".to_string()
    } else {
        format!("{} problem(s).", problems.len())
    };
    reply(
        summary,
        json!({"valid": problems.iter().all(|p| p.severity != mindweave_core::ops::Severity::Error), "problems": to_json(&problems)}),
    )
}

fn list_libraries(ctx: &Context, a: &Args) -> ToolResult {
    if a.get("path").is_some() {
        a.doc_path()?;
    }
    let libs = mindweave_core::ops::list_libraries(&ctx.libs);
    reply(
        format!("{} libraries.", libs.len()),
        json!({"libraries": to_json(&libs)}),
    )
}

fn list_artboard_presets(_: &Context, a: &Args) -> ToolResult {
    if a.get("path").is_some() {
        a.doc_path()?;
    }
    let presets = artboard_presets();
    reply(
        format!("{} presets.", presets.len()),
        json!({"presets": to_json(&presets)}),
    )
}

/// The optional point `(x, y)`: both or neither.
fn point(a: &Args, x: &str, y: &str) -> Result<Option<(f32, f32)>, String> {
    match (a.f32_opt(x)?, a.f32_opt(y)?) {
        (Some(x), Some(y)) => Ok(Some((x, y))),
        (None, None) => Ok(None),
        _ => Err(format!("Give both `{x}` and `{y}`, or neither.")),
    }
}

fn add_mind_map(ctx: &Context, a: &Args) -> ToolResult {
    let text = a.str_req("central_text")?;
    let at = point(a, "x", "y")?;
    let structure = a.str_opt("structure")?;
    let tree: Option<Vec<TopicNode>> = a.parse_opt(
        "topics",
        "a list of {text, note?, equation?, link?, task?, children?}",
    )?;
    let page = a.usize_opt("page")?;
    let (map, topics) = ctx.edit(a, |ws| {
        let map = ws.add_mind_map(page, text, at, structure)?;
        let topics = match &tree {
            Some(t) if !t.is_empty() => ws.add_tree(map, t)?,
            _ => Vec::new(),
        };
        Ok((map, topics))
    })?;
    reply(
        format!(
            "Added a mind map with {} topic(s) besides the central topic.",
            topics.len()
        ),
        json!({"map_id": map, "topic_ids": topics}),
    )
}

fn add_topics(ctx: &Context, a: &Args) -> ToolResult {
    let topic = a.id_req("topic_id")?;
    let position: TopicPosition = a
        .parse_opt("position", "child, after or before")?
        .unwrap_or_default();
    let texts: Option<Vec<String>> = a.get("texts").map(|_| a.strings_req("texts")).transpose()?;
    let tree: Option<Vec<TopicNode>> = a.parse_opt(
        "tree",
        "a list of {text, note?, equation?, link?, task?, children?}",
    )?;
    let ids = match (texts, tree) {
        (Some(texts), None) => ctx.edit(a, |ws| ws.add_topics(topic, &texts, position))?,
        (None, Some(tree)) => {
            if position != TopicPosition::Child {
                return Err("`tree` adds children; leave out `position`.".into());
            }
            ctx.edit(a, |ws| ws.add_tree(topic, &tree))?
        }
        _ => return Err("Pass either `texts` or `tree`.".into()),
    };
    reply(
        format!("Added {} topic(s).", ids.len()),
        json!({"ids": ids}),
    )
}

fn set_topic(ctx: &Context, a: &Args) -> ToolResult {
    let id = a.id_req("id")?;
    let edit: TopicEdit =
        serde_json::from_value(a.subset(&["text", "note", "equation", "link", "task"]))
            .map_err(|e| e.to_string())?;
    ctx.edit(a, |ws| ws.set_topic(id, &edit))?;
    reply("Topic changed.", json!({"id": id}))
}

fn add_summary(ctx: &Context, a: &Args) -> ToolResult {
    let topics = a.ids_req("topic_ids")?;
    let text = a.str_opt("text")?;
    let id = ctx.edit(a, |ws| ws.add_summary(&topics, text))?;
    reply("Added a summary.", json!({"id": id}))
}

fn add_boundary(ctx: &Context, a: &Args) -> ToolResult {
    let topics = a.ids_req("topic_ids")?;
    let id = ctx.edit(a, |ws| ws.add_boundary(&topics))?;
    reply("Added a boundary.", json!({"id": id}))
}

fn add_relationship(ctx: &Context, a: &Args) -> ToolResult {
    let (from, to) = (a.id_req("from_id")?, a.id_req("to_id")?);
    let text = a.str_opt("text")?;
    let id = ctx.edit(a, |ws| ws.add_relationship(from, to, text))?;
    reply("Added a relationship.", json!({"id": id}))
}

fn set_structure(ctx: &Context, a: &Args) -> ToolResult {
    let map = a.id_req("map_id")?;
    let (structure, branch) = (a.str_opt("structure")?, a.str_opt("branch_style")?);
    ctx.edit(a, |ws| ws.set_structure(map, structure, branch))?;
    reply("Map changed.", json!({"id": map}))
}

/// The style arguments, if any are given.
fn style_input(a: &Args) -> Result<Option<StyleInput>, String> {
    let v = a.subset(&[
        "fill",
        "stroke",
        "stroke_width",
        "dash",
        "opacity",
        "corner_radius",
    ]);
    if v.as_object().is_some_and(Map::is_empty) {
        return Ok(None);
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// The text look arguments.
fn text_input(a: &Args) -> Result<TextInput, String> {
    serde_json::from_value(a.subset(&[
        "text",
        "font",
        "size",
        "bold",
        "italic",
        "underline",
        "color",
        "align",
        "valign",
    ]))
    .map_err(|e| e.to_string())
}

/// The line arguments.
fn connect_options(a: &Args) -> Result<ConnectOptions, String> {
    serde_json::from_value(a.subset(&[
        "start_head",
        "end_head",
        "dash",
        "route",
        "text",
        "stroke",
        "stroke_width",
    ]))
    .map_err(|e| e.to_string())
}

fn add_shape(ctx: &Context, a: &Args) -> ToolResult {
    let library = a.str_opt("library")?.unwrap_or("basic-shapes");
    let shape = a.str_req("shape")?;
    let at = (a.f32_req("x")?, a.f32_req("y")?);
    let size = point(a, "w", "h")?;
    let text = a.str_opt("text")?;
    let style = style_input(a)?;
    let page = a.usize_opt("page")?;
    let ids = ctx.edit(a, |ws| {
        ws.add_shape(page, library, shape, at, size, text, style.as_ref())
    })?;
    reply(
        format!("Added {} shape(s).", ids.len()),
        json!({"ids": ids}),
    )
}

fn list_icons(ctx: &Context, a: &Args) -> ToolResult {
    if a.get("path").is_some() {
        a.doc_path()?;
    }
    let query = a.str_opt("query")?.unwrap_or("");
    let set = a.str_opt("set")?;
    let limit = a.usize_opt("limit")?.unwrap_or(50).clamp(1, 500);
    let sets = pictures::icon_sets(&ctx.icons_dir);
    if sets.is_empty() {
        return reply(
            format!(
                "No icon sets are installed (in {}). Import one in the Mindweave app (Icons pane, +), or use pictures from files.",
                ctx.icons_dir.display()
            ),
            json!({"sets": [], "icons": []}),
        );
    }
    let found = search_icons(&sets, set, query);
    let icons: Vec<Value> = found
        .iter()
        .take(limit)
        .map(|(s, i)| json!({"icon": format!("{}/{}", s.name, i.name), "group": i.group}))
        .collect();
    let set_list: Vec<Value> = sets
        .iter()
        .map(|s| json!({"name": s.name, "icons": s.icons.len()}))
        .collect();
    reply(
        format!("{} matching icon(s); showing {}.", found.len(), icons.len()),
        json!({"sets": set_list, "icons": icons}),
    )
}

/// The picture `a` names with `file` or `icon` (exactly one), and whether
/// it is an icon.
fn picture_arg(ctx: &Context, a: &Args) -> Result<(mindweave_core::ops::Picture, bool), String> {
    match (a.str_opt("file")?, a.str_opt("icon")?) {
        (Some(file), None) => Ok((pictures::read_file(file)?, false)),
        (None, Some(icon)) => Ok((pictures::read_icon(&ctx.icons_dir, icon)?, true)),
        (Some(_), Some(_)) => Err("Pass either `file` or `icon`, not both.".into()),
        (None, None) => Err("Pass `file` (an image file) or `icon` (see list_icons).".into()),
    }
}

fn add_image(ctx: &Context, a: &Args) -> ToolResult {
    let at = (a.f32_req("x")?, a.f32_req("y")?);
    let size = (a.f32_opt("w")?, a.f32_opt("h")?);
    let (picture, icon) = picture_arg(ctx, a)?;
    // Icons are placed as the Icons pane places them.
    let longest = a.f32_opt("longest")?.or(icon.then_some(64.0));
    let page = a.usize_opt("page")?;
    let id = ctx.edit(a, |ws| ws.add_image(page, picture, at, size, longest))?;
    reply("Added a picture.", json!({"id": id}))
}

fn set_topic_picture(ctx: &Context, a: &Args) -> ToolResult {
    let id = a.id_req("id")?;
    if a.bool_opt("remove")? == Some(true) {
        if a.get("file").is_some() || a.get("icon").is_some() {
            return Err("`remove` takes no `file` or `icon`.".into());
        }
        ctx.edit(a, |ws| ws.set_topic_picture(id, None, TOPIC_ICON_SIZE))?;
        return reply("Removed the topic's picture.", json!({"id": id}));
    }
    let (picture, icon) = picture_arg(ctx, a)?;
    let size = a.f32_opt("size")?.unwrap_or(if icon {
        TOPIC_ICON_SIZE
    } else {
        TOPIC_IMAGE_SIZE
    });
    ctx.edit(a, |ws| ws.set_topic_picture(id, Some(picture), size))?;
    reply("Set the topic's picture.", json!({"id": id}))
}

fn add_text(ctx: &Context, a: &Args) -> ToolResult {
    let at = (a.f32_req("x")?, a.f32_req("y")?);
    let text = a.str_req("text")?;
    let width = a.f32_opt("width")?;
    let mut look = text_input(a)?;
    look.text = None;
    let page = a.usize_opt("page")?;
    let id = ctx.edit(a, |ws| ws.add_text(page, at, text, width, Some(&look)))?;
    reply("Added a text box.", json!({"id": id}))
}

fn connect(ctx: &Context, a: &Args) -> ToolResult {
    let (from, to) = (a.id_req("from_id")?, a.id_req("to_id")?);
    let opts = connect_options(a)?;
    let id = ctx.edit(a, |ws| ws.connect(from, to, &opts))?;
    reply("Connected.", json!({"id": id}))
}

fn add_line(ctx: &Context, a: &Args) -> ToolResult {
    let p1 = (a.f32_req("x1")?, a.f32_req("y1")?);
    let p2 = (a.f32_req("x2")?, a.f32_req("y2")?);
    let opts = connect_options(a)?;
    let page = a.usize_opt("page")?;
    let id = ctx.edit(a, |ws| ws.add_line(page, p1, p2, &opts))?;
    reply("Added a line.", json!({"id": id}))
}

fn group(ctx: &Context, a: &Args) -> ToolResult {
    let ids = a.ids_req("ids")?;
    let id = ctx.edit(a, |ws| ws.group(&ids))?;
    reply("Grouped.", json!({"group_id": id}))
}

fn ungroup(ctx: &Context, a: &Args) -> ToolResult {
    let ids = a.ids_req("ids")?;
    ctx.edit(a, |ws| ws.ungroup(&ids))?;
    reply("Ungrouped.", Value::Null)
}

fn set_style(ctx: &Context, a: &Args) -> ToolResult {
    let ids = a.ids_req("ids")?;
    let style = style_input(a)?.ok_or("Give at least one style property to change.")?;
    ctx.edit(a, |ws| ws.set_style(&ids, &style))?;
    reply(format!("Restyled {} object(s).", ids.len()), Value::Null)
}

fn set_text(ctx: &Context, a: &Args) -> ToolResult {
    let id = a.id_req("id")?;
    let input = text_input(a)?;
    ctx.edit(a, |ws| ws.set_text(id, &input))?;
    reply("Text changed.", json!({"id": id}))
}

fn set_name(ctx: &Context, a: &Args) -> ToolResult {
    let id = a.id_req("id")?;
    let name = a.str_req("name")?;
    ctx.edit(a, |ws| ws.set_name(id, name))?;
    reply("Renamed.", json!({"id": id}))
}

fn move_objects(ctx: &Context, a: &Args) -> ToolResult {
    let ids = a.ids_req("ids")?;
    let (dx, dy) = (a.f32_req("dx")?, a.f32_req("dy")?);
    ctx.edit(a, |ws| ws.move_objects(&ids, dx, dy))?;
    reply(format!("Moved {} object(s).", ids.len()), Value::Null)
}

fn set_frame(ctx: &Context, a: &Args) -> ToolResult {
    let id = a.id_req("id")?;
    let at = (a.f32_req("x")?, a.f32_req("y")?);
    let size = point(a, "w", "h")?;
    ctx.edit(a, |ws| ws.set_frame(id, at, size))?;
    reply("Placed.", json!({"id": id}))
}

fn delete_objects(ctx: &Context, a: &Args) -> ToolResult {
    let ids = a.ids_req("ids")?;
    let removed = ctx.edit(a, |ws| ws.delete(&ids))?;
    reply(
        format!("Deleted {} object(s).", removed.len()),
        json!({"removed": removed}),
    )
}

fn arrange(ctx: &Context, a: &Args) -> ToolResult {
    let ids = a.ids_req("ids")?;
    let how: ArrangeHow = a.parse_req("order", "front, back, forward or backward")?;
    ctx.edit(a, |ws| ws.arrange(&ids, how))?;
    reply("Rearranged.", Value::Null)
}

fn add_artboard(ctx: &Context, a: &Args) -> ToolResult {
    let size = match (
        a.str_opt("preset")?,
        a.f32_opt("width")?,
        a.f32_opt("height")?,
    ) {
        (Some(p), None, None) => ArtboardSize::Preset(p.to_string()),
        (None, Some(width), Some(height)) => ArtboardSize::Size { width, height },
        _ => return Err("Give either `preset`, or both `width` and `height`.".into()),
    };
    let name = a.str_opt("name")?;
    let at = point(a, "x", "y")?;
    let page = a.usize_opt("page")?;
    let (id, frame) = ctx.edit(a, |ws| {
        let id = ws.add_artboard(page, &size, name, at)?;
        Ok((id, ws.get_object(id)?.frame))
    })?;
    reply(
        "Added an artboard.",
        json!({"id": id, "frame": to_json(&frame)}),
    )
}

/// The SVG export the arguments ask for, with its page.
fn export(ctx: &Context, a: &Args) -> Result<mindweave_core::export::SvgExport, String> {
    let ws = ctx.open(a)?;
    let region = match (a.id_opt("artboard_id")?, a.get("ids")) {
        (Some(_), Some(_)) => return Err("Give `artboard_id` or `ids`, not both.".into()),
        (Some(board), None) => ExportRegion::Artboard(board),
        (None, Some(_)) => ExportRegion::Objects(a.ids_req("ids")?),
        (None, None) => ExportRegion::Page,
    };
    let page = match a.usize_opt("page")? {
        Some(p) => p,
        None => match &region {
            // An artboard's or objects' own page.
            ExportRegion::Artboard(id) => ws.get_object(*id).map_err(|e| e.to_string())?.page,
            ExportRegion::Objects(ids) => ws.get_object(ids[0]).map_err(|e| e.to_string())?.page,
            ExportRegion::Page => ws.doc().active_page,
        },
    };
    let theme = match a.str_opt("theme")? {
        None | Some("light") => Theme::Light,
        Some("dark") => Theme::Dark,
        Some(t) => return Err(format!("Unknown theme `{t}`; use light or dark.")),
    };
    let padding = a.f32_opt("padding")?.unwrap_or(24.0).clamp(0.0, 1000.0);
    let renderer = render::equation_svg;
    export_svg(
        ws.doc(),
        page,
        &region,
        &ExportOptions {
            theme,
            padding,
            libs: Some(&ctx.libs),
            equation: Some(&renderer),
        },
    )
    .map_err(|e| e.to_string())
}

/// The output path argument, which must end in `.ext`.
fn out_path(a: &Args, ext: &str) -> Result<Option<std::path::PathBuf>, String> {
    let Some(p) = a.str_opt("out_path")? else {
        return Ok(None);
    };
    let path = expand(p)?;
    if path.extension().and_then(|e| e.to_str()) != Some(ext) {
        return Err(format!("`out_path` must end in .{ext}."));
    }
    check_folder(&path)?;
    Ok(Some(path))
}

fn export_svg_tool(ctx: &Context, a: &Args) -> ToolResult {
    let out = out_path(a, "svg")?;
    let svg = export(ctx, a)?;
    let size = json!({"width": svg.width, "height": svg.height});
    match out {
        Some(path) => {
            write_atomic(&path, svg.svg.as_bytes())?;
            reply(
                format!("Wrote {}.", path.display()),
                json!({"out_path": path, "size": size}),
            )
        }
        None => Ok(vec![
            json!({"type": "text", "text": json!({"summary": "SVG exported.", "size": size}).to_string()}),
            json!({"type": "text", "text": svg.svg}),
        ]),
    }
}

fn export_png_tool(ctx: &Context, a: &Args) -> ToolResult {
    let out = out_path(a, "png")?;
    let scale = a.f32_opt("scale")?.unwrap_or(1.0);
    if !(scale > 0.0 && scale <= 16.0) {
        return Err("`scale` must be above 0 and at most 16.".into());
    }
    let svg = export(ctx, a)?;
    // The returned image stays small enough for a model to look at; the
    // file gets the full size.
    let png = render::rasterise(&svg.svg, scale, render::MAX_INLINE_SIDE)?;
    let mut info = json!({"width": png.width, "height": png.height, "canvas_size": {"width": svg.width, "height": svg.height}});
    if let Some(path) = &out {
        let full = render::rasterise(&svg.svg, scale, render::MAX_PNG_SIDE)?;
        write_atomic(path, &full.bytes)?;
        info["out_path"] = json!(path);
        info["file_size"] = json!({"width": full.width, "height": full.height});
    }
    info["summary"] = json!(format!("PNG {}×{} px.", png.width, png.height));
    Ok(vec![
        json!({"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png.bytes), "mimeType": "image/png"}),
        json!({"type": "text", "text": info.to_string()}),
    ])
}
