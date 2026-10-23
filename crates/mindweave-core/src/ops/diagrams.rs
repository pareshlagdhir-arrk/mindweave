//! Diagrams: library shapes and templates, text boxes, lines and
//! connectors, groups, styles, text, moving, stacking and deleting.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::mindmaps::{add_line, is_topic_kind};
use super::read::{kind_of, ObjectKind};
use super::{
    check_batch, check_number, check_text, notes, parse_name, settle_membership, OpError, OpResult,
    Workspace,
};
use crate::arrange::Arrange;
use crate::color::{normalize_hex_color, normalize_style_color};
use crate::geometry::{Frame, Primitive};
use crate::library::{LibraryItem, LibraryRegistry, ShapeSource};
use crate::line::{
    line_frame, ArrowHead, LineDash, LineProps, LineRoute, LINE_LIBRARY, LINE_SHAPE,
};
use crate::shape::{Label, Shape, VAlign};
use crate::template::{instantiate_line_template, instantiate_template};
use crate::text::{HAlign, StyleChange, MAX_FONT_SIZE, MIN_FONT_SIZE};
use crate::transform::frame_aabb;

const COLORS: &str =
    "`#RRGGBB`, `auto` (follows the theme), `ink` (the theme's text colour) or `none`";
const HEADS: &[&str] = &[
    "none",
    "arrow",
    "triangle",
    "diamond",
    "circle",
    "hollow-triangle",
    "hollow-diamond",
];
const DASHES: &[&str] = &["solid", "dashed", "dotted"];
const ROUTES: &[&str] = &["straight", "angular", "rounded", "curved"];

/// Object ids by the page and layer they are on.
type ByLayer = Vec<((usize, usize), Vec<Uuid>)>;

/// A shape or template a library offers.
#[derive(Clone, Debug, Serialize)]
pub struct LibraryItemInfo {
    pub id: String,
    pub name: String,
    /// `shape` or `template` (several parts, placed as a group).
    pub kind: &'static str,
    pub default_size: (f32, f32),
    /// Parameters such as `corner_radius`, with their range.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
}

/// An installed library.
#[derive(Clone, Debug, Serialize)]
pub struct LibraryInfo {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub items: Vec<LibraryItemInfo>,
}

/// Style changes; `None` leaves a property as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StyleInput {
    /// `#RRGGBB`, `auto`, `ink` or `none`.
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f32>,
    /// `solid`, `dashed` or `dotted`.
    pub dash: Option<String>,
    /// From 0 to 1.
    pub opacity: Option<f32>,
    /// For shapes with a `corner_radius` parameter (rectangles).
    pub corner_radius: Option<f32>,
}

/// Text and its look; `None` leaves a property as it is.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextInput {
    /// New text; `\n` starts a paragraph.
    pub text: Option<String>,
    pub font: Option<String>,
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// `#RRGGBB`, or `auto` for a colour contrasting with what is behind.
    pub color: Option<String>,
    /// `left`, `center` or `right`.
    pub align: Option<String>,
    /// For shape labels: `top`, `middle` or `bottom`.
    pub valign: Option<String>,
}

/// How a connector looks.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectOptions {
    /// Head at the start; `none` by default.
    pub start_head: Option<String>,
    /// Head at the end; `triangle` by default for connectors.
    pub end_head: Option<String>,
    pub dash: Option<String>,
    /// `straight` (default), `angular`, `rounded` or `curved`.
    pub route: Option<String>,
    /// A label on the line.
    pub text: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f32>,
}

/// A stacking-order change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArrangeHow {
    Front,
    Back,
    Forward,
    Backward,
}

/// A style colour argument: `None` for no colour.
fn parse_color(what: &str, value: &str) -> OpResult<Option<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return Ok(None);
    }
    normalize_style_color(v)
        .map(Some)
        .ok_or_else(|| OpError::invalid(format!("Bad {what} colour `{value}`; use {COLORS}.")))
}

/// A text colour argument: `None` for auto.
fn parse_text_color(value: &str) -> OpResult<Option<String>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    normalize_hex_color(v).map(Some).ok_or_else(|| {
        OpError::invalid(format!(
            "Bad text colour `{value}`; use `#RRGGBB` or `auto`."
        ))
    })
}

/// A line head argument.
fn parse_head(value: &str) -> OpResult<ArrowHead> {
    parse_name("arrowhead", value, HEADS)
}

/// Checks a font size.
fn check_size(size: f32) -> OpResult<f32> {
    if size.is_finite() && (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size) {
        Ok(size)
    } else {
        Err(OpError::invalid(format!(
            "Font size must be from {MIN_FONT_SIZE} to {MAX_FONT_SIZE}."
        )))
    }
}

/// Whether `shape` (of `ws`'s libraries) has parameter `key`.
fn has_param(ws: &Workspace, shape: &Shape, key: &str) -> bool {
    shape.params.contains_key(key)
        || ws
            .libraries()
            .shape(&shape.library, &shape.shape)
            .is_some_and(|d| match &d.source {
                ShapeSource::Primitive(p) => p.param_keys().contains(&key),
                ShapeSource::Outline(_) => false,
            })
}

/// Applies the checked style changes to `shape`.
#[derive(Clone, Debug, Default)]
struct StyleChanges {
    fill: Option<Option<String>>,
    stroke: Option<Option<String>>,
    stroke_width: Option<f32>,
    dash: Option<LineDash>,
    opacity: Option<f32>,
    corner_radius: Option<f32>,
}

impl StyleChanges {
    /// Checks `input`.
    fn parse(input: &StyleInput) -> OpResult<StyleChanges> {
        let width = |v: f32, what: &str, max: f32| {
            if v.is_finite() && (0.0..=max).contains(&v) {
                Ok(v)
            } else {
                Err(OpError::invalid(format!("{what} must be from 0 to {max}.")))
            }
        };
        Ok(StyleChanges {
            fill: input
                .fill
                .as_deref()
                .map(|c| parse_color("fill", c))
                .transpose()?,
            stroke: input
                .stroke
                .as_deref()
                .map(|c| parse_color("stroke", c))
                .transpose()?,
            stroke_width: input
                .stroke_width
                .map(|v| width(v, "stroke_width", 100.0))
                .transpose()?,
            dash: input
                .dash
                .as_deref()
                .map(|d| parse_name("dash", d, DASHES))
                .transpose()?,
            opacity: input
                .opacity
                .map(|v| width(v, "opacity", 1.0))
                .transpose()?,
            corner_radius: input
                .corner_radius
                .map(|v| width(v, "corner_radius", 1000.0))
                .transpose()?,
        })
    }

    fn apply(&self, s: &mut Shape) {
        if let Some(v) = &self.fill {
            // Lines have no fill.
            if !s.is_line() {
                s.style.fill = v.clone();
            }
        }
        if let Some(v) = &self.stroke {
            s.style.stroke = v.clone();
        }
        if let Some(v) = self.stroke_width {
            s.style.stroke_width = v;
        }
        if let Some(v) = self.dash {
            s.style.dash = v;
        }
        if let Some(v) = self.opacity {
            s.style.opacity = v;
        }
    }
}

/// The checked text changes of a [`TextInput`].
struct TextChanges {
    text: Option<String>,
    changes: Vec<StyleChange>,
    align: Option<HAlign>,
    valign: Option<VAlign>,
}

impl TextChanges {
    fn parse(input: &TextInput) -> OpResult<TextChanges> {
        if let Some(t) = &input.text {
            check_text("text", t)?;
        }
        let mut changes = Vec::new();
        if let Some(f) = &input.font {
            let f = f.trim();
            changes.push(StyleChange::Font((!f.is_empty()).then(|| f.to_string())));
        }
        if let Some(s) = input.size {
            changes.push(StyleChange::Size(check_size(s)?));
        }
        if let Some(b) = input.bold {
            changes.push(StyleChange::Bold(b));
        }
        if let Some(i) = input.italic {
            changes.push(StyleChange::Italic(i));
        }
        if let Some(u) = input.underline {
            changes.push(StyleChange::Underline(u));
        }
        if let Some(c) = &input.color {
            changes.push(StyleChange::Color(parse_text_color(c)?));
        }
        Ok(TextChanges {
            text: input.text.clone(),
            changes,
            align: input
                .align
                .as_deref()
                .map(|a| parse_name("align", a, &["left", "center", "right"]))
                .transpose()?,
            valign: input
                .valign
                .as_deref()
                .map(|a| parse_name("valign", a, &["top", "middle", "bottom"]))
                .transpose()?,
        })
    }

    /// Applies the changes to `shape`'s label (giving it one if needed).
    fn apply(&self, shape: &mut Shape) {
        let label = shape.label.get_or_insert_with(Label::new_shape_label);
        if let Some(text) = &self.text {
            let style = label.text.style_at(0);
            let align = label
                .text
                .paragraphs
                .first()
                .map(|p| p.align)
                .unwrap_or_default();
            label.text = notes::rich_text_plain(text, &style, align);
        }
        for c in &self.changes {
            label.text.apply_all(c);
        }
        if let Some(a) = self.align {
            let len = label.text.len();
            label.text.set_align(0..len, a);
        }
        if let Some(v) = self.valign {
            label.valign = v;
        }
    }
}

/// The libraries of `libs` with their shapes and templates.
pub fn list_libraries(libs: &LibraryRegistry) -> Vec<LibraryInfo> {
    libs.libraries()
        .iter()
        .map(|lib| LibraryInfo {
            id: lib.id.clone(),
            name: lib.name.clone(),
            builtin: lib.builtin,
            description: lib.description.clone(),
            items: lib
                .items
                .iter()
                .filter_map(|item| match item {
                    LibraryItem::Shape(i) => lib.shapes.get(*i).map(|d| LibraryItemInfo {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        kind: "shape",
                        default_size: d.default_size,
                        params: d
                            .params
                            .iter()
                            .map(|p| format!("{} ({}..{})", p.key, p.min, p.max))
                            .collect(),
                    }),
                    LibraryItem::Template(t) => Some(LibraryItemInfo {
                        id: t.id.clone(),
                        name: t.name.clone(),
                        kind: "template",
                        default_size: t.default_size,
                        params: Vec::new(),
                    }),
                })
                .collect(),
        })
        .collect()
}

impl Workspace {
    /// The installed libraries with their shapes and templates.
    pub fn list_libraries(&self) -> Vec<LibraryInfo> {
        list_libraries(self.libraries())
    }

    /// Places shape or template `item` of library `library` with its
    /// top-left corner at `(x, y)` on page `page`, `w × h` (else its
    /// default size), labelled `text`, in `style`. A template comes as
    /// several grouped shapes. Returns the new ids. Placed inside an
    /// artboard, the new shapes join it.
    #[allow(clippy::too_many_arguments)]
    pub fn add_shape(
        &mut self,
        page: Option<usize>,
        library: &str,
        item: &str,
        (x, y): (f32, f32),
        size: Option<(f32, f32)>,
        text: Option<&str>,
        style: Option<&StyleInput>,
    ) -> OpResult<Vec<Uuid>> {
        let page = self.page_index(page)?;
        let (x, y) = (check_number("x", x)?, check_number("y", y)?);
        if let Some(t) = text {
            check_text("text", t)?;
        }
        let style = style.map(StyleChanges::parse).transpose()?;
        let libs = self.libraries();
        if libs.library(library).is_none() {
            let ids: Vec<&str> = libs.libraries().iter().map(|l| l.id.as_str()).collect();
            return Err(OpError::invalid(format!(
                "No library `{library}`; installed: {}. Call list_libraries.",
                ids.join(", ")
            )));
        }
        let mut shapes = if let Some((lib, t)) = libs.template(library, item) {
            let (w, h) = size.unwrap_or(t.default_size);
            let (w, h) = (check_size_arg(w)?, check_size_arg(h)?);
            if t.is_line_only() {
                instantiate_line_template(lib, t, (x, y), (x + w, y))
            } else {
                instantiate_template(
                    lib,
                    t,
                    Frame {
                        x,
                        y,
                        w,
                        h,
                        ..Frame::default()
                    },
                )
            }
        } else if let Some(def) = libs
            .library(library)
            .and_then(|l| l.shapes.iter().find(|s| s.id == item))
        {
            let (w, h) = size.unwrap_or(def.default_size);
            let (w, h) = (check_size_arg(w)?, check_size_arg(h)?);
            let frame = if matches!(def.source, ShapeSource::Primitive(Primitive::Line)) {
                line_frame((x, y), (x + w, y + h))
            } else {
                Frame {
                    x,
                    y,
                    w,
                    h,
                    ..Frame::default()
                }
            };
            vec![Shape::new(def, library, frame)]
        } else {
            return Err(OpError::invalid(format!(
                "Library `{library}` has no shape or template `{item}`. Call list_libraries for the ids."
            )));
        };
        if shapes.is_empty() {
            return Err(OpError::invalid(format!(
                "Template `{item}` has no parts that can be placed."
            )));
        }
        let corner = style.as_ref().and_then(|s| s.corner_radius);
        for s in &mut shapes {
            if let Some(st) = &style {
                st.apply(s);
            }
            if let Some(r) = corner {
                if has_param(self, s, "corner_radius") {
                    s.set_param("corner_radius", r);
                }
            }
        }
        if let Some(text) = text {
            // A template's text goes on its first part that has a label,
            // else its largest part.
            let target = shapes
                .iter()
                .position(|s| s.label.is_some())
                .unwrap_or_else(|| largest(&shapes));
            let changes = TextChanges::parse(&TextInput {
                text: Some(text.to_string()),
                ..TextInput::default()
            })?;
            changes.apply(&mut shapes[target]);
        }
        let ids: Vec<Uuid> = shapes.iter().map(|s| s.id).collect();
        self.on_layer(page, None, |s| super::add_with_membership(s, shapes));
        Ok(ids)
    }

    /// Adds a text box saying `text` with its top-left corner at `(x, y)`
    /// on page `page`; with `width` it wraps at that width, else it grows
    /// with its text. Returns its id.
    pub fn add_text(
        &mut self,
        page: Option<usize>,
        (x, y): (f32, f32),
        text: &str,
        width: Option<f32>,
        look: Option<&TextInput>,
    ) -> OpResult<Uuid> {
        let page = self.page_index(page)?;
        let (x, y) = (check_number("x", x)?, check_number("y", y)?);
        check_text("text", text)?;
        if text.trim().is_empty() {
            return Err(OpError::invalid("A text box needs some text."));
        }
        let mut changes = TextChanges::parse(look.unwrap_or(&TextInput::default()))?;
        changes.text = Some(text.to_string());
        let mut shape = Shape::new_text_box((x, y));
        if let Some(w) = width {
            shape.frame.w = check_size_arg(w)?;
            if let Some(tb) = shape.text_box.as_mut() {
                tb.auto_width = false;
            }
        }
        changes.apply(&mut shape);
        let id = shape.id;
        self.on_layer(page, None, |s| super::add_with_membership(s, vec![shape]));
        self.fit(page, &[id]);
        Ok(id)
    }

    /// Draws a connector from object `from` to object `to`, attached at
    /// both ends so it follows them; returns its id.
    pub fn connect(&mut self, from: Uuid, to: Uuid, opts: &ConnectOptions) -> OpResult<Uuid> {
        if from == to {
            return Err(OpError::invalid("A connector joins two different objects."));
        }
        for id in [from, to] {
            let s = self.shape(id)?;
            if matches!(kind_of(s), ObjectKind::Line | ObjectKind::Relationship) {
                return Err(OpError::WrongKind {
                    id,
                    kind: kind_of(s).name(),
                    expected: "lines cannot be attached to; connect shapes, text or topics".into(),
                });
            }
        }
        let (page, layer) = self.common_layer(&[from, to])?;
        let centre =
            |id: Uuid| -> OpResult<(f32, f32)> { Ok(frame_aabb(&self.shape(id)?.frame).center()) };
        let (a, b) = (centre(from)?, centre(to)?);
        let mut shape = self.line_shape(a, b, opts, ArrowHead::Triangle)?;
        if let Some(l) = shape.line.as_mut() {
            l.start = Some(from);
            l.end = Some(to);
        }
        let id = shape.id;
        self.on_layer(page, Some(layer), |s| add_line(s, shape));
        Ok(id)
    }

    /// Draws a free line from `a` to `b` on page `page` (its ends attached
    /// to `start` and `end` when given); returns its id.
    pub fn add_line(
        &mut self,
        page: Option<usize>,
        a: (f32, f32),
        b: (f32, f32),
        opts: &ConnectOptions,
    ) -> OpResult<Uuid> {
        let page = self.page_index(page)?;
        for v in [a.0, a.1, b.0, b.1] {
            check_number("a line end", v)?;
        }
        let shape = self.line_shape(a, b, opts, ArrowHead::None)?;
        let id = shape.id;
        self.on_layer(page, None, |s| add_line(s, shape));
        Ok(id)
    }

    /// A new line from `a` to `b` looking like `opts` (`end_default` when
    /// it names no end head).
    fn line_shape(
        &self,
        a: (f32, f32),
        b: (f32, f32),
        opts: &ConnectOptions,
        end_default: ArrowHead,
    ) -> OpResult<Shape> {
        let def = self
            .libraries()
            .shape(LINE_LIBRARY, LINE_SHAPE)
            .ok_or_else(|| OpError::invalid("The Basic Shapes library is missing."))?;
        let mut shape = Shape::new(def, LINE_LIBRARY, line_frame(a, b));
        let style = StyleChanges::parse(&StyleInput {
            stroke: opts.stroke.clone(),
            stroke_width: opts.stroke_width,
            dash: opts.dash.clone(),
            ..StyleInput::default()
        })?;
        style.apply(&mut shape);
        let route: LineRoute = opts
            .route
            .as_deref()
            .map(|r| parse_name("route", r, ROUTES))
            .transpose()?
            .unwrap_or_default();
        shape.line = Some(LineProps {
            start_head: opts
                .start_head
                .as_deref()
                .map(parse_head)
                .transpose()?
                .unwrap_or_default(),
            end_head: opts
                .end_head
                .as_deref()
                .map(parse_head)
                .transpose()?
                .unwrap_or(end_default),
            route,
            ..LineProps::default()
        });
        if let Some(text) = opts.text.as_deref().filter(|t| !t.is_empty()) {
            TextChanges::parse(&TextInput {
                text: Some(text.to_string()),
                ..TextInput::default()
            })?
            .apply(&mut shape);
        }
        Ok(shape)
    }

    /// Groups objects `ids` (at least two, on one layer); returns the
    /// group id.
    pub fn group(&mut self, ids: &[Uuid]) -> OpResult<Uuid> {
        check_batch("object", ids.len())?;
        let (page, layer) = self.common_layer(ids)?;
        for id in ids {
            let k = kind_of(self.shape(*id)?);
            if is_topic_kind(k) || k == ObjectKind::Boundary {
                return Err(OpError::invalid(
                    "Mind map topics are arranged by their map and cannot be grouped.",
                ));
            }
        }
        self.on_layer(page, Some(layer), |s| s.group_shapes(ids))
            .ok_or_else(|| OpError::invalid("Group at least two objects (or groups)."))
    }

    /// Takes objects `ids` out of their outermost group.
    pub fn ungroup(&mut self, ids: &[Uuid]) -> OpResult<()> {
        check_batch("object", ids.len())?;
        let (page, layer) = self.common_layer(ids)?;
        if ids
            .iter()
            .all(|id| self.shape(*id).is_ok_and(|s| s.groups.is_empty()))
        {
            return Err(OpError::invalid("None of these objects is in a group."));
        }
        self.on_layer(page, Some(layer), |s| s.ungroup_shapes(ids));
        Ok(())
    }

    /// Changes the style of objects `ids`.
    pub fn set_style(&mut self, ids: &[Uuid], input: &StyleInput) -> OpResult<()> {
        check_batch("object", ids.len())?;
        let changes = StyleChanges::parse(input)?;
        let by_layer = self.by_layer(ids)?;
        let corner: Vec<Uuid> = match changes.corner_radius {
            Some(_) => ids
                .iter()
                .copied()
                .filter(|id| {
                    self.shape(*id)
                        .is_ok_and(|s| has_param(self, s, "corner_radius"))
                })
                .collect(),
            None => Vec::new(),
        };
        if changes.corner_radius.is_some() && corner.is_empty() {
            return Err(OpError::invalid(
                "None of these objects has a corner radius (rectangles do).",
            ));
        }
        for ((page, layer), group) in by_layer {
            self.on_layer(page, Some(layer), |s| {
                s.update_shapes(&group, |shape| {
                    changes.apply(shape);
                    if let (Some(r), true) = (changes.corner_radius, corner.contains(&shape.id)) {
                        shape.set_param("corner_radius", r);
                    }
                })
            });
        }
        Ok(())
    }

    /// Changes the text of object `id` (a shape's label, a text box's
    /// text, a topic's or a line's label) and how it looks.
    pub fn set_text(&mut self, id: Uuid, input: &TextInput) -> OpResult<()> {
        let changes = TextChanges::parse(input)?;
        let (page, layer, shape) = self.locate(id)?;
        if shape.is_boundary() || shape.is_artboard() || shape.is_image() {
            return Err(OpError::WrongKind {
                id,
                kind: kind_of(shape).name(),
                expected: "it has no text (rename an artboard with set_name)".into(),
            });
        }
        self.on_layer(page, Some(layer), |s| {
            s.update_shapes(&[id], |shape| changes.apply(shape))
        });
        self.fit(page, &[id]);
        Ok(())
    }

    /// Names object `id`: an artboard's title, else the name the app
    /// shows in place of a missing library shape.
    pub fn set_name(&mut self, id: Uuid, name: &str) -> OpResult<()> {
        check_text("name", name)?;
        let (page, layer, _) = self.locate(id)?;
        let name = name.trim().to_string();
        self.on_layer(page, Some(layer), |s| {
            s.update_shapes(&[id], |shape| {
                shape.name = (!name.is_empty()).then(|| name.clone());
            })
        });
        Ok(())
    }

    /// `ids` sorted by page and layer.
    fn by_layer(&self, ids: &[Uuid]) -> OpResult<ByLayer> {
        let mut out: ByLayer = Vec::new();
        for id in ids {
            let (p, l, _) = self.locate(*id)?;
            match out.iter_mut().find(|(k, _)| *k == (p, l)) {
                Some((_, v)) => v.push(*id),
                None => out.push(((p, l), vec![*id])),
            }
        }
        Ok(out)
    }

    /// Moves objects `ids` by `(dx, dy)`: artboards with their members,
    /// whole mind maps when a central topic is among them, other topics
    /// with their subtree (offset from where the layout puts them).
    /// Objects moved into or out of an artboard change membership.
    pub fn move_objects(&mut self, ids: &[Uuid], dx: f32, dy: f32) -> OpResult<()> {
        check_batch("object", ids.len())?;
        let (dx, dy) = (check_number("dx", dx)?, check_number("dy", dy)?);
        for ((page, layer), group) in self.by_layer(ids)? {
            let shapes = self.doc().pages[page].layers[layer].objects.clone();
            let mut plain = Vec::new();
            let mut topics = Vec::new();
            for id in &group {
                let Some(s) = shapes.iter().find(|s| s.id == *id) else {
                    continue;
                };
                match &s.topic {
                    Some(t) if t.is_central() => {
                        // The whole map moves with its central topic.
                        plain.push(*id);
                    }
                    Some(t) => topics.push((*id, t.offset.unwrap_or((0.0, 0.0)))),
                    None if s.is_boundary() => {}
                    None => plain.push(*id),
                }
            }
            self.on_layer(page, Some(layer), |s| {
                s.begin_gesture();
                if !plain.is_empty() {
                    s.move_shapes(&plain, dx, dy);
                    settle_membership(s, &plain);
                }
                for (id, (ox, oy)) in topics {
                    s.set_topic_offset(id, Some((ox + dx, oy + dy)));
                }
                s.end_gesture();
            });
        }
        Ok(())
    }

    /// Moves object `id` so its top-left corner is at `(x, y)` and, when
    /// given, resizes it to `w × h` (a text box then wraps at `w`). Lines,
    /// boundaries and topics other than a central topic are placed by what
    /// they join; use move_objects or connect for them.
    pub fn set_frame(
        &mut self,
        id: Uuid,
        (x, y): (f32, f32),
        size: Option<(f32, f32)>,
    ) -> OpResult<()> {
        let (x, y) = (check_number("x", x)?, check_number("y", y)?);
        let size = size
            .map(|(w, h)| Ok::<_, OpError>((check_size_arg(w)?, check_size_arg(h)?)))
            .transpose()?;
        let (page, layer, shape) = self.locate(id)?;
        let kind = kind_of(shape);
        match kind {
            ObjectKind::Line | ObjectKind::Relationship | ObjectKind::Boundary | ObjectKind::Summary => {
                return Err(OpError::WrongKind {
                    id,
                    kind: kind.name(),
                    expected: "its frame follows what it joins; use move_objects".into(),
                })
            }
            ObjectKind::Topic if shape.topic.as_ref().is_some_and(|t| !t.is_central()) => {
                return Err(OpError::WrongKind {
                    id,
                    kind: kind.name(),
                    expected: "topics are placed by their map; move the central topic, or use move_objects to offset a branch".into(),
                })
            }
            _ => {}
        }
        let topic = kind == ObjectKind::Topic;
        if topic && size.is_some() {
            return Err(OpError::invalid(
                "Topics size themselves to their text; leave out w and h (change the text or its size with set_text).",
            ));
        }
        let (old_x, old_y) = (shape.frame.x, shape.frame.y);
        self.on_layer(page, Some(layer), |s| {
            s.begin_gesture();
            if topic || shape_is_artboard(s, id) {
                // Members and maps travel along.
                s.move_shapes(&[id], x - old_x, y - old_y);
            }
            s.update_shapes(&[id], |shape| {
                shape.frame.x = x;
                shape.frame.y = y;
                if let (Some((w, h)), false) = (size, topic) {
                    shape.frame.w = w;
                    shape.frame.h = h;
                    if let Some(tb) = shape.text_box.as_mut() {
                        tb.auto_width = false;
                    }
                }
            });
            settle_membership(s, &[id]);
            s.end_gesture();
        });
        self.fit(page, &[id]);
        Ok(())
    }

    /// Deletes objects `ids`: topics with their subtrees (and the
    /// relationships and summaries that go with them), artboards with
    /// their members. Lines attached to deleted objects stay, let go.
    /// Returns the ids of everything removed.
    pub fn delete(&mut self, ids: &[Uuid]) -> OpResult<Vec<Uuid>> {
        check_batch("object", ids.len())?;
        let mut removed = Vec::new();
        for ((page, layer), group) in self.by_layer(ids)? {
            let gone = self.on_layer(page, Some(layer), |s| s.delete_with_branches(&group));
            removed.extend(gone.into_iter().map(|s| s.id));
        }
        Ok(removed)
    }

    /// Moves objects `ids` in the stacking order of their layer.
    pub fn arrange(&mut self, ids: &[Uuid], how: ArrangeHow) -> OpResult<()> {
        check_batch("object", ids.len())?;
        let (page, layer) = self.common_layer(ids)?;
        let how = match how {
            ArrangeHow::Front => Arrange::ToFront,
            ArrangeHow::Back => Arrange::ToBack,
            ArrangeHow::Forward => Arrange::Forward,
            ArrangeHow::Backward => Arrange::Backward,
        };
        self.on_layer(page, Some(layer), |s| s.arrange_shapes(ids, how));
        Ok(())
    }
}

/// Whether `id` on the active layer is an artboard.
fn shape_is_artboard(s: &crate::session::DocumentSession, id: Uuid) -> bool {
    s.find_shape(id).is_some_and(Shape::is_artboard)
}

/// Checks a width or height.
fn check_size_arg(v: f32) -> OpResult<f32> {
    if v.is_finite() && (1.0..=super::MAX_COORD).contains(&v) {
        Ok(v)
    } else {
        Err(OpError::invalid(format!(
            "Width and height must be from 1 to {}.",
            super::MAX_COORD
        )))
    }
}

/// The index of the largest of `shapes` by area.
fn largest(shapes: &[Shape]) -> usize {
    shapes
        .iter()
        .enumerate()
        .max_by(|a, b| (a.1.frame.w * a.1.frame.h).total_cmp(&(b.1.frame.w * b.1.frame.h)))
        .map_or(0, |(i, _)| i)
}
