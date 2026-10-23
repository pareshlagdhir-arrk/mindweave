//! Templates: multi-part library items (library format 2). A template
//! is placed as several ordinary shapes and lines, scaled from its
//! default size to the placed size and grouped together.
//!
//! Each shape part gets a hidden [`ShapeDef`] in its library, with the id
//! `<template id>.<part number>` (counting every `[[template.part]]`
//! from 1, so ids stay put when an earlier part is skipped). Documents
//! store that id, so the part is drawn while the library is installed
//! and as a placeholder otherwise, like any library shape. Line parts are
//! ordinary Basic Shapes lines.

use std::collections::HashSet;

use toml::{Table, Value};
use uuid::Uuid;

use crate::geometry::Primitive;
use crate::geometry::{Frame, Pt};
use crate::library::{
    load_default_size, load_icon, load_style, optional_number, optional_str, quote, required_str,
    valid_id, valid_name, warn_unknown, Icon, Library, LoadReport, Reader, ShapeDef, ShapeSource,
    StyleDefaults, DEFAULT_ICON, DEFAULT_SIZE, FORMAT_2, MAX_DEFAULT_SIZE,
};
use crate::line::{line_frame, ArrowHead, LineProps, LINE_LIBRARY, LINE_SHAPE};
use crate::shape::{Label, Shape, VAlign};
use crate::text::{HAlign, RichText, RunStyle, LABEL_FONT_SIZE, MAX_FONT_SIZE, MIN_FONT_SIZE};

/// Most parts one template may have.
pub const MAX_TEMPLATE_PARTS: usize = 20;

/// Longest part label, in characters.
const MAX_LABEL_CHARS: usize = 1000;

/// Stroke width of a line part without one.
const LINE_STROKE_WIDTH: f32 = 2.0;

const TEMPLATE_FIELDS: &[&str] = &["id", "name", "icon", "default_size", "part"];
const PART_FIELDS: &[&str] = &[
    "frame",
    "primitive",
    "svg",
    "params",
    "line",
    "style",
    "label",
];
const LINE_FIELDS: &[&str] = &["from", "to", "start_head", "end_head"];
const LABEL_FIELDS: &[&str] = &[
    "text",
    "align",
    "valign",
    "bold",
    "italic",
    "underline",
    "size",
];

/// A multi-part library item.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateDef {
    /// Unique among the library's shapes, templates and part shapes.
    pub id: String,
    pub name: String,
    pub icon: Icon,
    /// The size the parts are drawn at, and of a template placed with a
    /// click.
    pub default_size: (f32, f32),
    /// Back to front; at least one, at most [`MAX_TEMPLATE_PARTS`].
    pub parts: Vec<TemplatePart>,
}

/// What a template part is.
#[derive(Clone, Debug, PartialEq)]
pub enum PartKind {
    /// A shape drawn from the library's hidden part shape `id`.
    Shape { id: String, source: ShapeSource },
    /// A line from `from` to `to`, in template units.
    Line {
        from: Pt,
        to: Pt,
        start_head: ArrowHead,
        end_head: ArrowHead,
    },
}

/// One shape or line of a template.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplatePart {
    /// `[x, y, w, h]` in template units; for a line, the box between its
    /// ends.
    pub frame: [f32; 4],
    pub kind: PartKind,
    /// A line's fill is always `None`.
    pub style: StyleDefaults,
    /// Primitive parameter values, within the primitive's own limits.
    pub params: Vec<(String, f32)>,
    pub label: Option<PartLabel>,
}

/// The text a template part is placed with.
#[derive(Clone, Debug, PartialEq)]
pub struct PartLabel {
    /// `\n` starts a new paragraph.
    pub text: String,
    pub align: HAlign,
    pub valign: VAlign,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Font size, from [`MIN_FONT_SIZE`] to [`MAX_FONT_SIZE`].
    pub size: f32,
}

impl Default for PartLabel {
    fn default() -> Self {
        PartLabel {
            text: String::new(),
            align: HAlign::Center,
            valign: VAlign::Middle,
            bold: false,
            italic: false,
            underline: false,
            size: LABEL_FONT_SIZE,
        }
    }
}

impl PartLabel {
    /// The editable label a placed part gets.
    pub fn to_label(&self) -> Label {
        let style = RunStyle {
            size: self.size,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            ..RunStyle::default()
        };
        let mut text = RichText::new(style.clone(), self.align);
        text.insert(0, &self.text, &style);
        Label {
            text,
            valign: self.valign,
        }
    }
}

/// The shapes and lines of `template` (from `lib`) placed in `frame`
/// (its position and size; rotation and flips are ignored): every part
/// scaled from the template's default size, with its style, parameters
/// and label, back to front. Two or more parts share one new group.
/// Parts whose shape `lib` lacks are left out.
pub fn instantiate_template(lib: &Library, template: &TemplateDef, frame: Frame) -> Vec<Shape> {
    let (w, h) = template.default_size;
    let (sx, sy) = (frame.w / w.max(1.0), frame.h / h.max(1.0));
    let at = |(x, y): Pt| (frame.x + x * sx, frame.y + y * sy);
    let group = (template.parts.len() > 1).then(Uuid::new_v4);
    let mut shapes = Vec::with_capacity(template.parts.len());
    for part in &template.parts {
        let mut shape = match &part.kind {
            PartKind::Shape { id, .. } => {
                let Some(def) = lib.shape(id) else {
                    continue;
                };
                let [x, y, pw, ph] = part.frame;
                let (x, y) = at((x, y));
                let placed = Frame {
                    x,
                    y,
                    w: pw * sx,
                    h: ph * sy,
                    ..Frame::default()
                };
                let mut shape = Shape::new(def, &lib.id, placed);
                for (key, value) in &part.params {
                    shape.set_param(key, *value);
                }
                shape
            }
            PartKind::Line {
                from,
                to,
                start_head,
                end_head,
            } => {
                let def = line_def(&part.style);
                let mut shape = Shape::new(&def, LINE_LIBRARY, line_frame(at(*from), at(*to)));
                shape.line = Some(LineProps {
                    start_head: *start_head,
                    end_head: *end_head,
                    ..LineProps::default()
                });
                shape
            }
        };
        shape.label = part.label.as_ref().map(PartLabel::to_label);
        if let Some(group) = group {
            shape.groups = vec![group];
        }
        shapes.push(shape);
    }
    shapes
}

/// The Line tool's shape in `style`, for placing a line part.
fn line_def(style: &StyleDefaults) -> ShapeDef {
    ShapeDef {
        id: LINE_SHAPE.into(),
        name: "Line".into(),
        icon: Icon::Lucide(DEFAULT_ICON.into()),
        default_size: DEFAULT_SIZE,
        style: style.clone(),
        source: ShapeSource::Primitive(Primitive::Line),
        params: Vec::new(),
    }
}

/// Loads `[[template]]` number `number` (1-based) of a format 2 library,
/// with the hidden shapes of its shape parts. `Err` is the warning for a
/// skipped template; a bad part is skipped with a warning of its own.
pub(crate) fn load_template(
    files: &mut Reader,
    entry: &Value,
    number: usize,
    taken_ids: &HashSet<String>,
    report: &mut LoadReport,
) -> Result<(TemplateDef, Vec<ShapeDef>), String> {
    let Value::Table(table) = entry else {
        return Err(format!(
            "shapes.toml: template #{number} is not a table; skipped"
        ));
    };
    let context = match table.get("id") {
        Some(Value::String(id)) => format!("template `{}`", quote(id)),
        _ => format!("template #{number}"),
    };
    let skip = |e: String| format!("{context}: {e}; skipped");

    let id = valid_id(required_str(table, "id").map_err(skip)?).map_err(skip)?;
    if taken_ids.contains(id) {
        return Err(skip("duplicate id".into()));
    }
    let name = valid_name(required_str(table, "name").map_err(skip)?).map_err(skip)?;
    let icon = required_str(table, "icon").map_err(skip)?;
    let entries = match table.get("part") {
        None => return Err(skip("has no parts".into())),
        Some(Value::Array(entries)) if entries.is_empty() => {
            return Err(skip("has no parts".into()))
        }
        Some(Value::Array(entries)) => entries,
        Some(_) => {
            return Err(skip(
                "parts must be written as [[template.part]] tables".into(),
            ))
        }
    };
    if entries.len() > MAX_TEMPLATE_PARTS {
        return Err(skip(format!(
            "has {} parts; a template may have at most {MAX_TEMPLATE_PARTS}",
            entries.len()
        )));
    }
    warn_unknown(table, TEMPLATE_FIELDS, &context, &mut report.log);
    let icon = load_icon(
        files,
        Some(icon),
        Icon::Lucide(DEFAULT_ICON.into()),
        &context,
        &mut report.notices,
    );
    let default_size = load_default_size(table.get("default_size")).unwrap_or_else(|e| {
        report
            .notices
            .push(format!("{context}: {e}; using 120 × 80"));
        DEFAULT_SIZE
    });

    let mut parts = Vec::with_capacity(entries.len());
    let mut part_shapes = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let part_number = index + 1;
        let part_context = format!("{context} part #{part_number}");
        let part_id = format!("{id}.{part_number}");
        if taken_ids.contains(&part_id) {
            report.notices.push(format!(
                "{part_context}: its shape id `{}` is already in use; skipped",
                quote(&part_id)
            ));
            continue;
        }
        let Value::Table(entry) = entry else {
            report
                .notices
                .push(format!("{part_context}: not a table; skipped"));
            continue;
        };
        match load_part(files, entry, &part_context, &part_id, report) {
            Ok(part) => {
                if let PartKind::Shape { id, source } = &part.kind {
                    part_shapes.push(ShapeDef {
                        id: id.clone(),
                        name: format!("{name} part {part_number}"),
                        icon: match source {
                            ShapeSource::Outline(g) => Icon::Outline(g.clone()),
                            ShapeSource::Primitive(_) => Icon::Lucide(DEFAULT_ICON.into()),
                        },
                        default_size: (part.frame[2].max(1.0), part.frame[3].max(1.0)),
                        style: part.style.clone(),
                        source: source.clone(),
                        params: Vec::new(),
                    });
                }
                parts.push(part);
            }
            Err(e) => report.notices.push(format!("{part_context}: {e}; skipped")),
        }
    }
    if parts.is_empty() {
        return Err(skip("none of its parts could be used".into()));
    }
    let template = TemplateDef {
        id: id.to_owned(),
        name: name.to_owned(),
        icon,
        default_size,
        parts,
    };
    Ok((template, part_shapes))
}

/// Loads one `[[template.part]]`; `Err` is why it is skipped. A shape
/// part's hidden shape gets the id `part_id`.
fn load_part(
    files: &mut Reader,
    table: &Table,
    context: &str,
    part_id: &str,
    report: &mut LoadReport,
) -> Result<TemplatePart, String> {
    let primitive = optional_str(table, "primitive")?;
    let svg = optional_str(table, "svg")?;
    let line = table.get("line");
    let kind = match (primitive, svg, line) {
        (Some(_), Some(_), _) => return Err("has both `primitive` and `svg`".into()),
        (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
            return Err("has both a shape and a `line`".into())
        }
        (None, None, None) => return Err("has none of `primitive`, `svg` and `line`".into()),
        (Some(p), None, None) => PartKind::Shape {
            id: part_id.to_owned(),
            source: ShapeSource::Primitive(
                Primitive::from_name(p)
                    .ok_or_else(|| format!("unknown primitive `{}`", quote(p)))?,
            ),
        },
        (None, Some(rel), None) => PartKind::Shape {
            id: part_id.to_owned(),
            source: ShapeSource::Outline(files.outline(rel)?),
        },
        (None, None, Some(Value::Table(line))) => load_line(line, context, report)?,
        (None, None, Some(_)) => {
            return Err("`line` must be a table such as { from = [0, 0], to = [100, 0] }".into())
        }
    };
    let frame = match &kind {
        PartKind::Line { from, to, .. } => [
            from.0.min(to.0),
            from.1.min(to.1),
            (to.0 - from.0).abs(),
            (to.1 - from.1).abs(),
        ],
        PartKind::Shape { .. } => load_frame(table.get("frame"))?,
    };
    // Only reached for parts that load, as for shapes.
    warn_unknown(table, PART_FIELDS, context, &mut report.log);

    let mut style = match table.get("style") {
        Some(Value::Table(style)) => load_style(style, context, FORMAT_2, report),
        Some(_) => {
            report
                .notices
                .push(format!("{context}: `style` must be a table; defaults used"));
            StyleDefaults::default()
        }
        None if matches!(kind, PartKind::Line { .. }) => StyleDefaults {
            stroke_width: LINE_STROKE_WIDTH,
            ..StyleDefaults::default()
        },
        None => StyleDefaults::default(),
    };
    if matches!(kind, PartKind::Line { .. }) {
        // Lines are strokes only, as the Line tool draws them.
        style.fill = None;
    }
    let params = match (&kind, table.get("params")) {
        (_, None) => Vec::new(),
        (
            PartKind::Shape {
                source: ShapeSource::Primitive(p),
                ..
            },
            Some(Value::Table(values)),
        ) => load_param_values(*p, values, context, report),
        (
            PartKind::Shape {
                source: ShapeSource::Primitive(_),
                ..
            },
            Some(_),
        ) => {
            report.notices.push(format!(
                "{context}: `params` must be a table such as {{ corner_radius = 4 }}; ignored"
            ));
            Vec::new()
        }
        (_, Some(_)) => {
            report.notices.push(format!(
                "{context}: only primitive parts have parameters; `params` ignored"
            ));
            Vec::new()
        }
    };
    let label = match table.get("label") {
        None => None,
        Some(Value::Table(label)) => match load_label(label, context, report) {
            Ok(label) => Some(label),
            Err(e) => {
                report
                    .notices
                    .push(format!("{context}: {e}; the label is left out"));
                None
            }
        },
        Some(_) => {
            report.notices.push(format!(
                "{context}: `label` must be a table such as {{ text = \"Name\" }}; \
                 the label is left out"
            ));
            None
        }
    };
    Ok(TemplatePart {
        frame,
        kind,
        style,
        params,
        label,
    })
}

/// A shape part's `frame = [x, y, w, h]`.
fn load_frame(value: Option<&Value>) -> Result<[f32; 4], String> {
    let message = || {
        format!(
            "`frame` must be [x, y, width, height] with x and y from -{MAX_DEFAULT_SIZE} \
             to {MAX_DEFAULT_SIZE} and width and height from 0 to {MAX_DEFAULT_SIZE}"
        )
    };
    let Some(Value::Array(values)) = value else {
        return Err(message());
    };
    let numbers: Vec<f32> = values.iter().filter_map(number).collect();
    match numbers[..] {
        [x, y, w, h]
            if values.len() == 4
                && [x, y].iter().all(|v| v.abs() <= MAX_DEFAULT_SIZE)
                && [w, h].iter().all(|v| (0.0..=MAX_DEFAULT_SIZE).contains(v)) =>
        {
            Ok([x, y, w, h])
        }
        _ => Err(message()),
    }
}

/// A finite TOML number.
fn number(value: &Value) -> Option<f32> {
    let n = match value {
        Value::Integer(i) => *i as f32,
        Value::Float(f) => *f as f32,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// The point `[x, y]` at `key` of `table`, in template units.
fn load_point(table: &Table, key: &str) -> Result<Pt, String> {
    let message = || {
        format!("`{key}` must be [x, y] with values from -{MAX_DEFAULT_SIZE} to {MAX_DEFAULT_SIZE}")
    };
    let Some(Value::Array(values)) = table.get(key) else {
        return Err(message());
    };
    match values[..] {
        [ref x, ref y] => match (number(x), number(y)) {
            (Some(x), Some(y)) if x.abs() <= MAX_DEFAULT_SIZE && y.abs() <= MAX_DEFAULT_SIZE => {
                Ok((x, y))
            }
            _ => Err(message()),
        },
        _ => Err(message()),
    }
}

/// A line part's `line = { from, to, start_head, end_head }`.
fn load_line(table: &Table, context: &str, report: &mut LoadReport) -> Result<PartKind, String> {
    let from = load_point(table, "from")?;
    let to = load_point(table, "to")?;
    warn_unknown(
        table,
        LINE_FIELDS,
        &format!("{context} line"),
        &mut report.log,
    );
    let mut head = |key: &str| match table.get(key) {
        None => ArrowHead::None,
        Some(value) => value.as_str().and_then(head_from_name).unwrap_or_else(|| {
            report.notices.push(format!(
                "{context}: `{key}` must be one of {}; using \"none\"",
                head_names()
            ));
            ArrowHead::None
        }),
    };
    let start_head = head("start_head");
    let end_head = head("end_head");
    Ok(PartKind::Line {
        from,
        to,
        start_head,
        end_head,
    })
}

/// The file name of `head`, as documents store it (`"hollow-triangle"`).
fn head_name(head: ArrowHead) -> String {
    serde_json::to_value(head)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The head with file name `name`.
fn head_from_name(name: &str) -> Option<ArrowHead> {
    ArrowHead::ALL
        .into_iter()
        .find(|h| head_name(*h) == name.trim())
}

/// Every head's file name, quoted, for messages.
fn head_names() -> String {
    ArrowHead::ALL
        .iter()
        .map(|h| format!("\"{}\"", head_name(*h)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A part's `params = { key = value }` for primitive `kind`: unknown
/// keys and non-numbers are ignored and values outside the primitive's
/// limits clamped, each with a warning.
fn load_param_values(
    kind: Primitive,
    values: &Table,
    context: &str,
    report: &mut LoadReport,
) -> Vec<(String, f32)> {
    let mut params = Vec::new();
    for (key, value) in values {
        let Some((lowest, highest)) = kind.param_range(key) else {
            report.notices.push(format!(
                "{context}: the primitive has no parameter `{}`; ignored",
                quote(key)
            ));
            continue;
        };
        let Some(v) = number(value) else {
            report.notices.push(format!(
                "{context}: `{}` must be a number; ignored",
                quote(key)
            ));
            continue;
        };
        let clamped = v.clamp(lowest, highest);
        if clamped != v {
            report.notices.push(format!(
                "{context}: `{key}` must be from {lowest} to {highest}; using {clamped}"
            ));
        }
        params.push((key.clone(), clamped));
    }
    params
}

/// A part's `label = { text, align, valign, bold, italic, underline,
/// size }`. Bad optional values fall back with a warning; `Err` when the
/// text is missing or too long.
fn load_label(table: &Table, context: &str, report: &mut LoadReport) -> Result<PartLabel, String> {
    let text = required_str(table, "text").map_err(|e| format!("label {e}"))?;
    if text.chars().count() > MAX_LABEL_CHARS {
        return Err(format!(
            "the label text is longer than {MAX_LABEL_CHARS} characters"
        ));
    }
    let context = format!("{context} label");
    warn_unknown(table, LABEL_FIELDS, &context, &mut report.log);
    let defaults = PartLabel::default();
    let mut warn = |e: String| report.notices.push(format!("{context}: {e}; default used"));
    let mut choice = |key: &str, names: &[&str]| match optional_str(table, key) {
        Ok(None) => None,
        Ok(Some(value)) => {
            let found = names.iter().position(|n| *n == value.trim());
            if found.is_none() {
                warn(format!(
                    "`{key}` must be {}",
                    names
                        .iter()
                        .map(|n| format!("\"{n}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            found
        }
        Err(e) => {
            warn(e);
            None
        }
    };
    let align = choice("align", &["left", "center", "right"]).map_or(defaults.align, |i| {
        [HAlign::Left, HAlign::Center, HAlign::Right][i]
    });
    let valign = choice("valign", &["top", "middle", "bottom"]).map_or(defaults.valign, |i| {
        [VAlign::Top, VAlign::Middle, VAlign::Bottom][i]
    });
    let mut flag = |key: &str| match table.get(key) {
        None => false,
        Some(Value::Boolean(b)) => *b,
        Some(_) => {
            report.notices.push(format!(
                "{context}: `{key}` must be true or false; using false"
            ));
            false
        }
    };
    let bold = flag("bold");
    let italic = flag("italic");
    let underline = flag("underline");
    let size = match optional_number(table, "size") {
        Ok(None) => defaults.size,
        Ok(Some(n)) if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&n) => n,
        Ok(Some(_)) => {
            report.notices.push(format!(
                "{context}: `size` must be from {MIN_FONT_SIZE} to {MAX_FONT_SIZE}; \
                 using {LABEL_FONT_SIZE}"
            ));
            defaults.size
        }
        Err(e) => {
            report
                .notices
                .push(format!("{context}: {e}; using {LABEL_FONT_SIZE}"));
            defaults.size
        }
    };
    Ok(PartLabel {
        text: text.to_owned(),
        align,
        valign,
        bold,
        italic,
        underline,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::format::{load, save};
    use crate::geometry::Frame;
    use crate::library::{
        load_library, Charge, Library, LibraryError, LibraryFiles, LibraryItem, LibraryRegistry,
        MAX_SHAPES,
    };
    use crate::line::{ArrowHead, LineDash, LINE_LIBRARY, LINE_SHAPE};
    use crate::shape::VAlign;
    use crate::text::HAlign;

    /// Library files held in memory.
    struct MemFiles(Vec<(String, String)>);

    impl LibraryFiles for MemFiles {
        fn read(&self, rel: &str, charge: Charge) -> Result<String, LibraryError> {
            let (_, text) = self
                .0
                .iter()
                .find(|(path, _)| path == rel)
                .ok_or_else(|| LibraryError::Missing(rel.into()))?;
            charge(text.len())?;
            Ok(text.clone())
        }
    }

    fn lib_toml(format: u32) -> String {
        format!("format = {format}\nid = \"uml-test\"\nname = \"UML Test\"\nversion = \"1\"\n")
    }

    /// Loads a library of format `format` from `shapes` (and `extra`
    /// files), with its notices and log lines as one list.
    fn load_lib(format: u32, shapes: &str, extra: &[(&str, &str)]) -> (Library, Vec<String>) {
        let mut files = vec![
            ("library.toml".to_owned(), lib_toml(format)),
            ("shapes.toml".to_owned(), shapes.to_owned()),
        ];
        files.extend(extra.iter().map(|(p, t)| (p.to_string(), t.to_string())));
        let (lib, report) = load_library(&MemFiles(files), false).unwrap();
        (lib, [report.notices, report.log].concat())
    }

    const CLASS: &str = r##"
[[shape]]
id = "note"
name = "Note"
primitive = "rect"

[[template]]
id = "class"
name = "Class"
icon = "box"
default_size = [100, 50]

[[template.part]]
frame = [0, 0, 100, 20]
primitive = "rect"
params = { corner_radius = 4 }
style = { fill = "ink", stroke = "#112233", stroke_width = 2, dash = "dashed" }
label = { text = "Name", bold = true, italic = true, underline = true, size = 18 }

[[template.part]]
frame = [0, 20, 100, 30]
svg = "box.svg"
label = { text = "+ attr: Type", align = "left", valign = "top" }

[[template.part]]
line = { from = [50, 20], to = [0, 50], start_head = "none", end_head = "hollow-triangle" }
style = { dash = "dotted" }

[[shape]]
id = "after"
name = "After"
primitive = "ellipse"
"##;

    const BOX_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="1" height="1"/></svg>"#;

    fn class_library() -> Library {
        let (lib, warnings) = load_lib(2, CLASS, &[("box.svg", BOX_SVG)]);
        assert!(warnings.is_empty(), "{warnings:?}");
        lib
    }

    #[test]
    fn template_parses_with_shape_and_line_parts() {
        let lib = class_library();
        // The ribbon keeps file order; part shapes are not in it.
        let ids: Vec<&str> = lib.shapes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["note", "after"]);
        assert!(matches!(
            &lib.items[..],
            [
                LibraryItem::Shape(0),
                LibraryItem::Template(t),
                LibraryItem::Shape(1)
            ] if t.id == "class"
        ));
        let t = lib.template("class").unwrap();
        assert_eq!(t.name, "Class");
        assert_eq!(t.icon, Icon::Lucide("box".into()));
        assert_eq!(t.default_size, (100.0, 50.0));
        assert_eq!(t.parts.len(), 3);

        let name = &t.parts[0];
        assert_eq!(name.frame, [0.0, 0.0, 100.0, 20.0]);
        assert!(matches!(
            &name.kind,
            PartKind::Shape { id, source: ShapeSource::Primitive(Primitive::Rect) }
                if id == "class.1"
        ));
        assert_eq!(name.params, [("corner_radius".to_owned(), 4.0)]);
        assert_eq!(name.style.fill.as_deref(), Some("ink"));
        assert_eq!(name.style.stroke.as_deref(), Some("#112233"));
        assert_eq!(name.style.stroke_width, 2.0);
        assert_eq!(name.style.dash, LineDash::Dashed);
        assert_eq!(
            name.label,
            Some(PartLabel {
                text: "Name".into(),
                align: HAlign::Center,
                valign: VAlign::Middle,
                bold: true,
                italic: true,
                underline: true,
                size: 18.0,
            })
        );

        let attrs = &t.parts[1];
        assert!(matches!(
            &attrs.kind,
            PartKind::Shape { id, source: ShapeSource::Outline(_) } if id == "class.2"
        ));
        let label = attrs.label.as_ref().unwrap();
        assert_eq!((label.align, label.valign), (HAlign::Left, VAlign::Top));
        assert_eq!(label.size, LABEL_FONT_SIZE);

        let line = &t.parts[2];
        assert_eq!(
            line.kind,
            PartKind::Line {
                from: (50.0, 20.0),
                to: (0.0, 50.0),
                start_head: ArrowHead::None,
                end_head: ArrowHead::HollowTriangle,
            }
        );
        assert_eq!(line.frame, [0.0, 20.0, 50.0, 30.0]);
        assert_eq!(line.style.fill, None);
        assert_eq!(line.style.dash, LineDash::Dotted);

        // Part shapes are found for drawing, though hidden from the ribbon.
        assert_eq!(
            lib.shape("class.1").unwrap().style.fill.as_deref(),
            Some("ink")
        );
        assert!(lib.shape("class.2").is_some());
        assert!(lib.shape("class.3").is_none());
        assert!(lib.shape("class").is_none());
    }

    #[test]
    fn template_part_limits_warn() {
        let part = "[[template.part]]\nframe = [0, 0, 10, 10]\nprimitive = \"rect\"\n";
        let template = |id: &str, parts: usize| {
            format!(
                "[[template]]\nid = \"{id}\"\nname = \"T\"\nicon = \"box\"\n{}",
                part.repeat(parts)
            )
        };
        let shapes = format!(
            "{}{}{}",
            template("most", MAX_TEMPLATE_PARTS),
            template("too-many", MAX_TEMPLATE_PARTS + 1),
            template("none", 0)
        );
        let (lib, warnings) = load_lib(2, &shapes, &[]);
        assert!(lib.template("most").is_some());
        assert_eq!(
            lib.template("most").unwrap().parts.len(),
            MAX_TEMPLATE_PARTS
        );
        assert!(lib.template("too-many").is_none());
        assert!(lib.template("none").is_none());
        assert!(lib.shape("too-many.1").is_none());
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[0].contains("too-many") && warnings[0].contains("20"),
            "{warnings:?}"
        );
        assert!(warnings[1].contains("none"), "{warnings:?}");

        // Bad parts are skipped one by one; part ids keep their position.
        let shapes = "[[template]]\nid = \"t\"\nname = \"T\"\nicon = \"box\"\n\
                      [[template.part]]\nprimitive = \"rect\"\n\
                      [[template.part]]\nframe = [0, 0, 1, 1]\nprimitive = \"rect\"\nline = { from = [0, 0], to = [1, 1] }\n\
                      [[template.part]]\nframe = [0, 0, 5, 5]\nprimitive = \"ellipse\"\n";
        let (lib, warnings) = load_lib(2, shapes, &[]);
        let t = lib.template("t").unwrap();
        assert_eq!(t.parts.len(), 1);
        assert!(matches!(&t.parts[0].kind, PartKind::Shape { id, .. } if id == "t.3"));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings.iter().all(|w| w.contains("part #")),
            "{warnings:?}"
        );

        // Templates count towards the item limit.
        let mut shapes: String = (0..MAX_SHAPES - 1)
            .map(|i| format!("[[shape]]\nid = \"s{i}\"\nname = \"S\"\nprimitive = \"rect\"\n"))
            .collect();
        shapes.push_str(&template("a", 1));
        shapes.push_str(&template("b", 1));
        let files = MemFiles(vec![
            ("library.toml".into(), lib_toml(2)),
            ("shapes.toml".into(), shapes),
        ]);
        assert_eq!(
            load_library(&files, false).unwrap_err(),
            LibraryError::TooManyShapes(MAX_SHAPES + 1)
        );
    }

    #[test]
    fn format_2_features_in_format_1_warn_and_skip() {
        let shapes = r##"
[[shape]]
id = "dot"
name = "Dot"
primitive = "ellipse"
style = { fill = "ink", stroke = "ink", dash = "dashed" }

[[template]]
id = "class"
name = "Class"
icon = "box"
[[template.part]]
frame = [0, 0, 10, 10]
primitive = "rect"

[[shape]]
id = "plain"
name = "Plain"
primitive = "rect"
style = { fill = "#ABCDEF" }
"##;
        let (lib, warnings) = load_lib(1, shapes, &[]);
        let dot = lib.shape("dot").unwrap();
        assert_eq!(
            (dot.style.fill.as_deref(), dot.style.stroke.as_deref()),
            (None, None)
        );
        assert_eq!(dot.style.dash, LineDash::Solid);
        assert_eq!(
            lib.shape("plain").unwrap().style.fill.as_deref(),
            Some("#ABCDEF")
        );
        assert!(lib.template("class").is_none());
        assert!(lib.shape("class.1").is_none());
        assert_eq!(lib.items.len(), 2);
        // fill, stroke, dash, template.
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(
            warnings.iter().all(|w| w.contains("format 2")),
            "{warnings:?}"
        );
        assert!(warnings.iter().any(|w| w.contains("class")), "{warnings:?}");
    }

    #[test]
    fn instantiate_scales_frames_and_line_ends() {
        let lib = class_library();
        let t = lib.template("class").unwrap();
        // Placed at twice the default size.
        let frame = Frame {
            x: 10.0,
            y: 20.0,
            w: 200.0,
            h: 100.0,
            ..Frame::default()
        };
        let shapes = instantiate_template(&lib, t, frame);
        assert_eq!(shapes.len(), 3);

        let name = &shapes[0];
        assert_eq!(
            (name.library.as_str(), name.shape.as_str()),
            ("uml-test", "class.1")
        );
        assert_eq!(
            (name.frame.x, name.frame.y, name.frame.w, name.frame.h),
            (10.0, 20.0, 200.0, 40.0)
        );
        assert_eq!(name.param("corner_radius"), Some(4.0));
        assert_eq!(name.style.fill.as_deref(), Some("ink"));
        assert_eq!(name.style.dash, LineDash::Dashed);
        let label = name.label.as_ref().unwrap();
        assert_eq!(label.text.plain(), "Name");
        let style = label.text.style_at(0);
        assert!(style.bold && style.italic && style.underline);
        assert_eq!(style.size, 18.0);
        assert_eq!(label.text.paragraphs[0].align, HAlign::Center);

        let attrs = &shapes[1];
        assert_eq!(
            (attrs.frame.x, attrs.frame.y, attrs.frame.w, attrs.frame.h),
            (10.0, 60.0, 200.0, 60.0)
        );
        let label = attrs.label.as_ref().unwrap();
        assert_eq!(label.valign, VAlign::Top);
        assert_eq!(label.text.paragraphs[0].align, HAlign::Left);

        // The line runs from (50, 20) to (0, 50), scaled: (110, 60) to
        // (10, 120), so its frame is flipped horizontally.
        let line = &shapes[2];
        assert!(line.is_line());
        assert_eq!(
            (line.library.as_str(), line.shape.as_str()),
            (LINE_LIBRARY, LINE_SHAPE)
        );
        let f = line.frame;
        assert_eq!((f.x, f.y, f.w, f.h), (10.0, 60.0, 100.0, 60.0));
        assert!(f.flip_x && !f.flip_y);
        let props = line.line.as_ref().unwrap();
        assert_eq!(props.start_head, ArrowHead::None);
        assert_eq!(props.end_head, ArrowHead::HollowTriangle);
        assert_eq!(line.style.dash, LineDash::Dotted);
        assert_eq!(line.style.fill, None);

        // At the default size the parts keep their own frames.
        let at_default = instantiate_template(
            &lib,
            t,
            Frame {
                w: 100.0,
                h: 50.0,
                ..Frame::default()
            },
        );
        let f = at_default[1].frame;
        assert_eq!((f.x, f.y, f.w, f.h), (0.0, 20.0, 100.0, 30.0));
    }

    #[test]
    fn instantiated_parts_share_one_group() {
        let lib = class_library();
        let t = lib.template("class").unwrap();
        let frame = Frame {
            w: 100.0,
            h: 50.0,
            ..Frame::default()
        };
        let first = instantiate_template(&lib, t, frame);
        let group = first[0].groups.clone();
        assert_eq!(group.len(), 1);
        assert!(first.iter().all(|s| s.groups == group));
        let ids: std::collections::HashSet<_> = first.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), first.len());
        // Each placement is a new group of new shapes.
        let second = instantiate_template(&lib, t, frame);
        assert_ne!(second[0].groups, group);
        assert!(second.iter().all(|s| !ids.contains(&s.id)));

        // A one-part template places a single ungrouped shape.
        let shapes = "[[template]]\nid = \"assoc\"\nname = \"Association\"\nicon = \"minus\"\n\
                      [[template.part]]\nline = { from = [0, 0], to = [120, 0] }\n";
        let (lib, warnings) = load_lib(2, shapes, &[]);
        assert!(warnings.is_empty(), "{warnings:?}");
        let placed = instantiate_template(&lib, lib.template("assoc").unwrap(), frame);
        assert_eq!(placed.len(), 1);
        assert!(placed[0].groups.is_empty());
    }

    #[test]
    fn missing_template_library_shows_placeholders() {
        let lib = class_library();
        let t = lib.template("class").unwrap();
        let mut doc = Document::new();
        doc.pages[0].layers[0].objects = instantiate_template(
            &lib,
            t,
            Frame {
                w: 100.0,
                h: 50.0,
                ..Frame::default()
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uml.mindweave");
        save(&doc, &path).unwrap();
        let loaded = load(&path).unwrap();
        let objects = &loaded.pages[0].layers[0].objects;
        assert_eq!(objects, &doc.pages[0].layers[0].objects);

        // With the library installed every part is drawn from it.
        let (with, _) = LibraryRegistry::new(vec![lib.clone()], None);
        let (without, _) = LibraryRegistry::new(Vec::new(), None);
        for s in objects {
            if s.is_line() {
                // Lines draw from their own geometry, library or not.
                continue;
            }
            assert!(with.shape(&s.library, &s.shape).is_some(), "{}", s.shape);
            // Without it the shape is kept, and drawn as a placeholder.
            assert!(without.shape(&s.library, &s.shape).is_none(), "{}", s.shape);
        }
        assert_eq!(objects.iter().filter(|s| !s.is_line()).count(), 2);
    }
}
