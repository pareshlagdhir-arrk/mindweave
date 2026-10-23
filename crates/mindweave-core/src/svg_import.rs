//! Importing an SVG file as editable shapes: each filled or stroked path
//! becomes a path shape, each text a text box (see [`import_svg`]); then
//! [`place_import`] fits them to the view as one group.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use roxmltree::NodeId;

use thiserror::Error;
use usvg::tiny_skia_path::{self, PathSegment};
use usvg::{fontdb, FontFamily, FontStyle, ImageHrefResolver, Node, Paint, TextAnchor};
use uuid::Uuid;

use crate::color::format_hex_color;
use crate::geometry::{Frame, PathCmd, Pt, Rect};
use crate::line::LineDash;
use crate::path_shape::PathProps;
use crate::shape::{Shape, Style};
use crate::svg::{nesting_exceeds, MAX_NESTING};
use crate::text::{HAlign, RichText, RunStyle};
use crate::transform::selection_bounds;

/// Largest SVG file imported, in bytes.
pub const MAX_IMPORT_BYTES: usize = 10 * 1024 * 1024;
/// Most objects one import creates.
pub const MAX_IMPORT_OBJECTS: usize = 5_000;
/// Most elements an SVG may expand to (its `<use>` copies included)
/// before conversion; more is refused before any work is done.
const MAX_EXPANDED_ELEMENTS: u64 = 10 * MAX_IMPORT_OBJECTS as u64;
/// Most path data and text (bytes) an SVG may expand to.
const MAX_EXPANDED_DATA: u64 = 20 * 1024 * 1024;
/// Longest chain of `<use>` references followed.
const MAX_USE_DEPTH: usize = 256;
/// Elements whose content is drawn only where referenced.
const NOT_RENDERED: [&str; 10] = [
    "defs",
    "symbol",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "linearGradient",
    "radialGradient",
    "filter",
    "style",
];
/// The share of the view an import is fitted to.
const FIT_SHARE: f32 = 0.8;
/// Room a text box keeps round its text (the desktop's text box padding),
/// so imported text is drawn where the SVG drew it.
const TEXT_INSET: f32 = 4.0;

/// The shapes read from an SVG file, in canvas units at 1:1 with the
/// document's origin at `(0, 0)`, back to front.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgImport {
    pub shapes: Vec<Shape>,
    /// Elements left out: embedded or linked pictures, and text no font
    /// could lay out.
    pub skipped: usize,
}

/// Why an SVG file could not be imported.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SvgImportError {
    #[error("The SVG file is larger than 10 MB.")]
    TooLarge,
    #[error("The SVG has more than 5,000 elements to import.")]
    TooManyObjects,
    #[error(
        "The SVG is too complex to import: its elements expand to too many shapes or too much path data."
    )]
    TooComplex,
    #[error("The file is not an SVG Mindweave can read: {0}")]
    Invalid(String),
}

/// Reads `bytes` (an SVG file) as shapes: each visible filled or stroked
/// path (basic shapes included, `<use>` copies resolved, transforms
/// applied) becomes a path shape, each text a text box. Pictures are left
/// out and counted; nothing is read from other files. Text is laid out
/// with the system's fonts; text no installed font can show is left out
/// and counted. Files whose `<use>` copies would expand past the limits
/// are refused before any copy is made.
pub fn import_svg(bytes: &[u8]) -> Result<SvgImport, SvgImportError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(SvgImportError::TooLarge);
    }
    let src =
        std::str::from_utf8(bytes).map_err(|_| SvgImportError::Invalid("it is not text".into()))?;
    // The XML parser recurses once per nesting level.
    if nesting_exceeds(src, MAX_NESTING) {
        return Err(SvgImportError::Invalid(
            "elements are nested too deeply".into(),
        ));
    }
    // usvg expands every `<use>` first: weigh the copies before that.
    let expected = expanded_weight(src)?;
    let pictures = Arc::new(AtomicUsize::new(0));
    let opt = usvg::Options {
        image_href_resolver: skipping_pictures(&pictures),
        fontdb: system_fonts(),
        ..usvg::Options::default()
    };
    let tree =
        usvg::Tree::from_str(src, &opt).map_err(|e| SvgImportError::Invalid(e.to_string()))?;
    let mut walk = Walk::default();
    collect(tree.root(), 1.0, &mut walk)?;
    // Text no font could lay out never reaches the tree.
    let lost_texts = (expected.texts as usize).saturating_sub(walk.texts);
    Ok(SvgImport {
        shapes: walk.shapes,
        skipped: walk.skipped + lost_texts + pictures.load(Ordering::Relaxed),
    })
}

/// What an SVG expands to once its `<use>` copies are made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Weight {
    /// Elements, copies included.
    elements: u64,
    /// Bytes of path data (`d`, `points`) and text.
    data: u64,
    /// Texts drawn.
    texts: u64,
}

impl Weight {
    fn add(&mut self, other: Weight, rendered: bool) {
        self.elements = self.elements.saturating_add(other.elements);
        self.data = self.data.saturating_add(other.data);
        if rendered {
            self.texts = self.texts.saturating_add(other.texts);
        }
    }

    fn too_heavy(&self) -> bool {
        self.elements > MAX_EXPANDED_ELEMENTS || self.data > MAX_EXPANDED_DATA
    }
}

/// The [`Weight`] of SVG `src`, worked out on the XML before usvg makes
/// any copies; [`SvgImportError::TooComplex`] when over the limits.
fn expanded_weight(src: &str) -> Result<Weight, SvgImportError> {
    let opt = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(src, opt)
        .map_err(|e| SvgImportError::Invalid(e.to_string()))?;
    let ids: HashMap<&str, roxmltree::Node> = doc
        .descendants()
        .filter(|n| n.is_element())
        .filter_map(|n| Some((n.attribute("id")?, n)))
        .collect();
    let mut weigh = Weigher {
        ids,
        memo: HashMap::new(),
        visiting: HashSet::new(),
    };
    let w = weigh.weight(doc.root_element(), 0)?;
    if w.too_heavy() {
        return Err(SvgImportError::TooComplex);
    }
    Ok(w)
}

/// Works out element weights, each once, following `<use>` references.
struct Weigher<'a, 'i> {
    ids: HashMap<&'a str, roxmltree::Node<'a, 'i>>,
    memo: HashMap<NodeId, Weight>,
    /// Elements being weighed: a reference back to one is a cycle, which
    /// usvg leaves out.
    visiting: HashSet<NodeId>,
}

impl<'a, 'i> Weigher<'a, 'i> {
    /// The weight of `node` and what it draws; `depth` counts nesting and
    /// `<use>` references followed.
    fn weight(
        &mut self,
        node: roxmltree::Node<'a, 'i>,
        depth: usize,
    ) -> Result<Weight, SvgImportError> {
        if let Some(w) = self.memo.get(&node.id()) {
            return Ok(*w);
        }
        if depth > MAX_USE_DEPTH {
            return Err(SvgImportError::TooComplex);
        }
        if !self.visiting.insert(node.id()) {
            return Ok(Weight::default());
        }
        let name = node.tag_name().name();
        let mut w = Weight {
            elements: 1,
            data: ["d", "points"]
                .iter()
                .filter_map(|a| node.attribute(*a))
                .map(|v| v.len() as u64)
                .sum(),
            texts: u64::from(name == "text" && node.attribute("display") != Some("none")),
        };
        for child in node.children() {
            if child.is_text() {
                w.data = w
                    .data
                    .saturating_add(child.text().map_or(0, str::len) as u64);
            } else if child.is_element() {
                let rendered = !NOT_RENDERED.contains(&child.tag_name().name())
                    && child.attribute("display") != Some("none");
                // A text's texts are itself.
                let rendered = rendered && name != "text";
                w.add(self.weight(child, depth + 1)?, rendered);
            }
            if w.too_heavy() {
                return Err(SvgImportError::TooComplex);
            }
        }
        if name == "use" {
            let href = node
                .attribute("href")
                .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")));
            let target = href
                .and_then(|h| h.strip_prefix('#'))
                .and_then(|id| self.ids.get(id).copied());
            if let Some(target) = target {
                w.add(self.weight(target, depth + 1)?, true);
            }
        }
        self.visiting.remove(&node.id());
        if w.too_heavy() {
            return Err(SvgImportError::TooComplex);
        }
        self.memo.insert(node.id(), w);
        Ok(w)
    }
}

/// What walking a converted tree gives.
#[derive(Default)]
struct Walk {
    shapes: Vec<Shape>,
    /// Pictures and texts left out.
    skipped: usize,
    /// Texts met, imported or blank.
    texts: usize,
}

/// An image reference resolver that loads nothing and counts the
/// pictures asked for in `count`.
fn skipping_pictures(count: &Arc<AtomicUsize>) -> ImageHrefResolver<'static> {
    let (data, string) = (count.clone(), count.clone());
    ImageHrefResolver {
        resolve_data: Box::new(move |_, _, _| {
            data.fetch_add(1, Ordering::Relaxed);
            None
        }),
        resolve_string: Box::new(move |_, _| {
            string.fetch_add(1, Ordering::Relaxed);
            None
        }),
    }
}

/// The system's fonts, loaded once.
fn system_fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Adds the shapes of `group`'s children, back to front, to `walk`;
/// `opacity` is the product of the enclosing groups' opacities.
fn collect(group: &usvg::Group, opacity: f32, walk: &mut Walk) -> Result<(), SvgImportError> {
    for node in group.children() {
        match node {
            Node::Group(g) => collect(g, opacity * g.opacity().get(), walk)?,
            Node::Path(p) => walk.shapes.extend(path_shape(p, opacity)),
            Node::Text(t) => {
                walk.texts += 1;
                match text_box(t, opacity) {
                    Some(TextOutcome::Box(s)) => walk.shapes.push(*s),
                    Some(TextOutcome::Blank) => {}
                    None => walk.skipped += 1,
                }
            }
            Node::Image(_) => walk.skipped += 1,
        }
        if walk.shapes.len() > MAX_IMPORT_OBJECTS {
            return Err(SvgImportError::TooManyObjects);
        }
    }
    Ok(())
}

/// How much `t` scales lengths (the square root of its area factor).
fn transform_scale(t: usvg::Transform) -> f32 {
    (t.sx * t.sy - t.kx * t.ky).abs().sqrt()
}

/// `p` mapped by `t`.
fn apply(t: usvg::Transform, (x, y): Pt) -> Pt {
    (t.sx * x + t.kx * y + t.tx, t.ky * x + t.sy * y + t.ty)
}

/// The colour `paint` is shown in: a gradient's first stop; `None` for a
/// pattern.
fn paint_color(paint: &Paint) -> Option<String> {
    let c = match paint {
        Paint::Color(c) => *c,
        Paint::LinearGradient(g) => g.stops().first()?.color(),
        Paint::RadialGradient(g) => g.stops().first()?.color(),
        Paint::Pattern(_) => return None,
    };
    Some(format_hex_color((c.red, c.green, c.blue)))
}

/// `path` as commands.
fn path_cmds(path: &tiny_skia_path::Path) -> Vec<PathCmd> {
    let pt = |p: tiny_skia_path::Point| (p.x, p.y);
    path.segments()
        .map(|seg| match seg {
            PathSegment::MoveTo(p) => PathCmd::MoveTo(pt(p)),
            PathSegment::LineTo(p) => PathCmd::LineTo(pt(p)),
            PathSegment::QuadTo(c, p) => PathCmd::QuadTo {
                ctrl: pt(c),
                to: pt(p),
            },
            PathSegment::CubicTo(c1, c2, p) => PathCmd::CubicTo {
                c1: pt(c1),
                c2: pt(c2),
                to: pt(p),
            },
            PathSegment::Close => PathCmd::Close,
        })
        .collect()
}

/// The path shape of a visible, filled or stroked `path`, its frame
/// tight round its outline in canvas coordinates.
fn path_shape(path: &usvg::Path, opacity: f32) -> Option<Shape> {
    let (fill, stroke) = (path.fill(), path.stroke());
    if !path.is_visible() || (fill.is_none() && stroke.is_none()) {
        return None;
    }
    let t = path.abs_transform();
    let data = path.data().clone().transform(t)?;
    let b = data.compute_tight_bounds()?;
    let bounds = Rect {
        x: b.x(),
        y: b.y(),
        w: b.width(),
        h: b.height(),
    };
    let even_odd = fill.is_some_and(|f| f.rule() == usvg::FillRule::EvenOdd);
    let props = PathProps::from_cmds(&path_cmds(&data), &bounds, even_odd);
    let paint_opacity = fill
        .map(|f| f.opacity().get())
        .or(stroke.map(|s| s.opacity().get()))
        .unwrap_or(1.0);
    let style = Style {
        fill: fill.and_then(|f| paint_color(f.paint())),
        stroke: stroke.and_then(|s| paint_color(s.paint())),
        stroke_width: stroke.map_or(1.0, |s| s.width().get() * transform_scale(t)),
        opacity: opacity * paint_opacity,
        dash: if stroke.is_some_and(|s| s.dasharray().is_some()) {
            LineDash::Dashed
        } else {
            LineDash::Solid
        },
        double: false,
    };
    let width = style.stroke_width;
    let mut shape = Shape::new_path(props, Frame::from_rect(&bounds), style);
    // Unclamped: [`place_import`] clamps it once scaled.
    shape.style.stroke_width = width;
    Some(shape)
}

/// What a text gives.
enum TextOutcome {
    Box(Box<Shape>),
    /// Nothing to show.
    Blank,
}

/// The text box of `text`: its plain text (a new line wherever a chunk
/// starts lower down) in its first span's style (its fill, or else its
/// stroke, colour and opacity), over its laid-out bounds, or above its
/// anchor point when it has none. `None` when it cannot be read.
fn text_box(text: &usvg::Text, opacity: f32) -> Option<TextOutcome> {
    let chunks = text.chunks();
    let mut plain = String::new();
    let mut line_y = None;
    for (i, chunk) in chunks.iter().enumerate() {
        if i > 0 && chunk.y().is_some() && chunk.y() != line_y {
            plain.push('\n');
        }
        line_y = chunk.y().or(line_y);
        plain.push_str(chunk.text());
    }
    if plain.trim().is_empty() {
        return Some(TextOutcome::Blank);
    }
    let first = chunks.first()?;
    let span = first.spans().first()?;
    let t = text.abs_transform();
    let font = span.font();
    let (color, paint_opacity) = match (span.fill(), span.stroke()) {
        (Some(f), _) => (paint_color(f.paint()), f.opacity().get()),
        (None, Some(s)) => (paint_color(s.paint()), s.opacity().get()),
        (None, None) => (None, 1.0),
    };
    let style = RunStyle {
        font: font.families().iter().find_map(|f| match f {
            FontFamily::Named(name) => Some(name.clone()),
            _ => None,
        }),
        size: span.font_size().get() * transform_scale(t),
        bold: font.weight() >= 600,
        italic: font.style() != FontStyle::Normal,
        color,
        ..RunStyle::default()
    };
    let align = match first.anchor() {
        TextAnchor::Start => HAlign::Left,
        TextAnchor::Middle => HAlign::Center,
        TextAnchor::End => HAlign::Right,
    };
    let b = text.abs_bounding_box();
    let laid_out = b.width() > 0.0 && b.height() > 0.0;
    let (x, y, w, h) = if laid_out {
        (b.x(), b.y(), b.width(), b.height())
    } else {
        let anchor = apply(t, (first.x().unwrap_or(0.0), first.y().unwrap_or(0.0)));
        (anchor.0, anchor.1 - style.size, 0.0, style.size)
    };
    let mut shape = Shape::new_text_box((x - TEXT_INSET, y - TEXT_INSET));
    // Its laid-out size until the editor measures the text.
    (shape.frame.w, shape.frame.h) = (w + 2.0 * TEXT_INSET, h + 2.0 * TEXT_INSET);
    shape.style.opacity = opacity * paint_opacity;
    let mut rich = RichText::new(style.clone(), align);
    rich.insert(0, &plain, &style);
    shape.label.as_mut()?.text = rich;
    shape.normalize();
    Some(TextOutcome::Box(Box::new(shape)))
}

/// `shapes` (from [`import_svg`]) centred in `view`, scaled down to fit
/// 80% of it when larger (never up), in one new group when there are
/// several. Stroke widths and text boxes' font sizes scale with them, and
/// are then clamped.
pub fn place_import(mut shapes: Vec<Shape>, view: &Rect) -> Vec<Shape> {
    let frames: Vec<Frame> = shapes.iter().map(|s| s.frame).collect();
    let Some(b) = selection_bounds(&frames) else {
        return shapes;
    };
    let fit = |room: f32, len: f32| {
        if len > 0.0 {
            room * FIT_SHARE / len
        } else {
            f32::INFINITY
        }
    };
    let scale = fit(view.w, b.w).min(fit(view.h, b.h)).min(1.0);
    let from = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let to = (view.x + view.w / 2.0, view.y + view.h / 2.0);
    let group = (shapes.len() > 1).then(Uuid::new_v4);
    for s in &mut shapes {
        let f = &mut s.frame;
        f.x = to.0 + (f.x - from.0) * scale;
        f.y = to.1 + (f.y - from.1) * scale;
        f.w *= scale;
        f.h *= scale;
        s.style.stroke_width *= scale;
        let text_box = s.is_text_box();
        if let Some(label) = s.label.as_mut().filter(|_| text_box) {
            scale_font_sizes(&mut label.text, scale);
        }
        s.groups.extend(group);
        s.normalize();
    }
    shapes
}

/// Multiplies every font size in `text` by `scale`.
fn scale_font_sizes(text: &mut RichText, scale: f32) {
    for p in &mut text.paragraphs {
        p.style.size *= scale;
        for run in &mut p.runs {
            run.style.size *= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{bounds, to_canvas, Frame};
    use crate::line::LineDash;
    use crate::transform::frame_aabb;

    fn import(body: &str) -> SvgImport {
        let src = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300">{body}</svg>"#
        );
        import_svg(src.as_bytes()).unwrap()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn frame_is(f: &Frame, x: f32, y: f32, w: f32, h: f32) -> bool {
        close(f.x, x) && close(f.y, y) && close(f.w, w) && close(f.h, h)
    }

    #[test]
    fn basic_shapes_become_path_shapes_with_their_colours() {
        let out = import(
            r##"<rect x="10" y="20" width="30" height="40" fill="#ff0000" stroke="#0000ff" stroke-width="2"/>
            <circle cx="100" cy="50" r="10" fill="lime"/>
            <path d="M 200 100 L 260 100 L 260 130 Z" fill="none" stroke="#123456"/>"##,
        );
        assert_eq!(out.skipped, 0);
        let s = &out.shapes;
        assert_eq!(s.len(), 3);
        assert!(s.iter().all(|s| s.is_path() && s.path.is_some()));
        assert!(
            frame_is(&s[0].frame, 10.0, 20.0, 30.0, 40.0),
            "{:?}",
            s[0].frame
        );
        assert_eq!(s[0].style.fill.as_deref(), Some("#FF0000"));
        assert_eq!(s[0].style.stroke.as_deref(), Some("#0000FF"));
        assert_eq!(s[0].style.stroke_width, 2.0);
        assert!(
            frame_is(&s[1].frame, 90.0, 40.0, 20.0, 20.0),
            "{:?}",
            s[1].frame
        );
        assert_eq!(s[1].style.fill.as_deref(), Some("#00FF00"));
        assert_eq!(s[1].style.stroke, None);
        assert!(frame_is(&s[2].frame, 200.0, 100.0, 60.0, 30.0));
        assert_eq!(s[2].style.fill, None);
        assert_eq!(s[2].style.stroke.as_deref(), Some("#123456"));
        // The outline drawn in the frame is the original one.
        let g = to_canvas(
            &s[2].path.as_ref().unwrap().local_geometry(60.0, 30.0),
            &s[2].frame,
        );
        let b = bounds(&g).unwrap();
        assert!(close(b.x, 200.0) && close(b.w, 60.0) && close(b.h, 30.0));
    }

    #[test]
    fn transforms_are_applied_to_frames_and_stroke_widths() {
        let out = import(
            r#"<g transform="translate(100 50) scale(2)">
                <rect x="5" y="0" width="10" height="5" fill="black" stroke="red" stroke-width="3"/>
            </g>"#,
        );
        let s = &out.shapes[0];
        assert!(frame_is(&s.frame, 110.0, 50.0, 20.0, 10.0), "{:?}", s.frame);
        assert_eq!(s.style.stroke_width, 6.0);
    }

    #[test]
    fn dashes_fill_rules_and_opacity_are_kept() {
        let out = import(
            r#"<g opacity="0.5">
                <path d="M0 0 H50 V50 H0 Z M10 10 H40 V40 H10 Z" fill="red" fill-rule="evenodd"
                    fill-opacity="0.5" stroke="black" stroke-dasharray="4 2"/>
            </g>
            <rect width="5" height="5" fill="blue"/>"#,
        );
        let (a, b) = (&out.shapes[0], &out.shapes[1]);
        assert_eq!(a.style.dash, LineDash::Dashed);
        assert!(a.path.as_ref().unwrap().even_odd);
        assert!(close(a.style.opacity, 0.25), "{}", a.style.opacity);
        assert_eq!(b.style.dash, LineDash::Solid);
        assert!(!b.path.as_ref().unwrap().even_odd);
        assert_eq!(b.style.opacity, 1.0);
    }

    #[test]
    fn gradients_fill_with_their_first_stop() {
        let out = import(
            r##"<defs><linearGradient id="g">
                <stop offset="0" stop-color="#123456"/><stop offset="1" stop-color="#ffffff"/>
            </linearGradient></defs>
            <rect width="50" height="20" fill="url(#g)"/>"##,
        );
        assert_eq!(out.shapes[0].style.fill.as_deref(), Some("#123456"));
    }

    #[test]
    fn text_becomes_a_text_box() {
        let out = import(
            r##"<text x="20" y="60" font-size="24" fill="#ff0000" font-weight="bold">Hello</text>"##,
        );
        assert_eq!(out.shapes.len(), 1);
        let t = &out.shapes[0];
        assert!(t.is_text_box());
        let text = &t.label.as_ref().unwrap().text;
        assert_eq!(text.plain(), "Hello");
        let style = text.style_at(0);
        assert_eq!(style.size, 24.0);
        assert!(style.bold);
        assert_eq!(style.color.as_deref(), Some("#FF0000"));
        // Near where the text is drawn: its top is above the baseline.
        assert!(t.frame.x > 10.0 && t.frame.x < 25.0, "{:?}", t.frame);
        assert!(t.frame.y > 25.0 && t.frame.y < 60.0, "{:?}", t.frame);
    }

    #[test]
    fn text_lines_are_kept() {
        let out = import(
            r#"<text x="0" y="20" font-size="10"><tspan x="0" y="20">One</tspan><tspan x="0" y="40">Two</tspan></text>"#,
        );
        let text = &out.shapes[0].label.as_ref().unwrap().text;
        assert_eq!(text.plain(), "One\nTwo");
    }

    #[test]
    fn pictures_are_skipped_and_counted() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        let out = import(&format!(
            r#"<image width="10" height="10" href="data:image/png;base64,{png}"/>
            <image width="10" height="10" href="/etc/passwd"/>
            <rect width="5" height="5"/>"#
        ));
        assert_eq!(out.shapes.len(), 1);
        assert_eq!(out.skipped, 2);
    }

    #[test]
    fn oversized_crowded_and_invalid_files_are_refused() {
        let big = vec![b' '; MAX_IMPORT_BYTES + 1];
        assert_eq!(import_svg(&big), Err(SvgImportError::TooLarge));
        let rects = r#"<rect width="1" height="1"/>"#.repeat(MAX_IMPORT_OBJECTS + 1);
        let src = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{rects}</svg>"#);
        assert_eq!(
            import_svg(src.as_bytes()),
            Err(SvgImportError::TooManyObjects)
        );
        assert!(matches!(
            import_svg(b"not an svg"),
            Err(SvgImportError::Invalid(_))
        ));
        assert!(matches!(
            import_svg(&[0xff, 0xfe, 0x00]),
            Err(SvgImportError::Invalid(_))
        ));
    }

    #[test]
    fn path_shapes_survive_save_and_load_and_scale_with_their_frame() {
        let out = import(r#"<rect x="10" y="10" width="20" height="10" fill="red"/>"#);
        let shape = out.shapes[0].clone();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mindweave");
        let mut doc = crate::document::Document::new();
        doc.pages[0].layers[0].objects.push(shape.clone());
        crate::format::save(&doc, &file).unwrap();
        let back = crate::format::load(&file).unwrap();
        assert_eq!(back.pages[0].layers[0].objects[0], shape);
        // The outline spans whatever frame it is given.
        let g = shape.path.as_ref().unwrap().local_geometry(60.0, 10.0);
        assert!(close(bounds(&g).unwrap().w, 60.0));
        // Only path shapes carry a path.
        let mut text = Shape::new_text_box((0.0, 0.0));
        text.path = shape.path.clone();
        text.normalize();
        assert!(text.path.is_none());
    }

    #[test]
    fn placing_fits_centres_and_groups() {
        let out = import(
            r#"<rect x="0" y="0" width="1000" height="500" fill="red" stroke="black" stroke-width="10"/>
            <text x="100" y="100" font-size="40">Hi</text>"#,
        );
        let view = Rect {
            x: -100.0,
            y: -100.0,
            w: 500.0,
            h: 500.0,
        };
        let placed = place_import(out.shapes, &view);
        assert_eq!(placed.len(), 2);
        let group = placed[0].groups.clone();
        assert_eq!(group.len(), 1);
        assert_eq!(placed[1].groups, group);
        // 1000 wide into 80% of 500: scale 0.4, centred on (150, 150).
        let f = &placed[0].frame;
        assert!(frame_is(f, -50.0, 50.0, 400.0, 200.0), "{f:?}");
        assert!(close(placed[0].style.stroke_width, 4.0));
        let text = &placed[1].label.as_ref().unwrap().text;
        assert!(close(text.style_at(0).size, 16.0));
        // Small imports keep their size; a lone shape is not grouped.
        let one = import(r#"<rect x="0" y="0" width="10" height="20"/>"#);
        let placed = place_import(one.shapes, &view);
        assert!(placed[0].groups.is_empty());
        let f = frame_aabb(&placed[0].frame);
        assert!(
            frame_is(&placed[0].frame, 145.0, 140.0, 10.0, 20.0),
            "{f:?}"
        );
        assert!(place_import(Vec::new(), &view).is_empty());
    }

    /// `levels` groups, each holding `fan` copies of the one below, over
    /// `leaf`, all used once.
    fn fan_out(leaf: &str, levels: usize, fan: usize) -> String {
        let mut defs = format!(r#"<g id="l0">{leaf}</g>"#);
        for i in 1..=levels {
            let uses = format!(r##"<use href="#l{}"/>"##, i - 1).repeat(fan);
            defs.push_str(&format!(r#"<g id="l{i}">{uses}</g>"#));
        }
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg"><defs>{defs}</defs><use href="#l{levels}"/></svg>"##
        )
    }

    #[test]
    fn exponential_use_copies_are_refused_before_conversion() {
        let start = std::time::Instant::now();
        let src = fan_out(r#"<rect width="1" height="1"/>"#, 6, 10);
        assert_eq!(import_svg(src.as_bytes()), Err(SvgImportError::TooComplex));
        assert!(start.elapsed().as_secs() < 2);
        // Few elements, but a long path copied a hundred times.
        let d = "M0 0 L1 1 ".repeat(30_000);
        let src = fan_out(&format!(r#"<path d="{d}" fill="red"/>"#), 2, 10);
        assert_eq!(import_svg(src.as_bytes()), Err(SvgImportError::TooComplex));
        // A modest fan-out imports; a reference cycle is harmless.
        let src = fan_out(r#"<rect width="1" height="1"/>"#, 2, 3);
        assert_eq!(import_svg(src.as_bytes()).unwrap().shapes.len(), 9);
        let cycle = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <g id="a"><rect width="1" height="1"/><use href="#a"/></g></svg>"##;
        assert!(import_svg(cycle.as_bytes()).is_ok());
    }

    #[test]
    fn texts_are_counted_through_their_copies() {
        let src = fan_out(r#"<text>Hi</text>"#, 2, 3);
        let w = expanded_weight(&src).unwrap();
        assert_eq!(w.texts, 9);
        // Text in definitions or hidden is not drawn.
        let w = expanded_weight(
            r#"<svg><defs><text>a</text></defs><text display="none">b</text><text>c</text></svg>"#,
        )
        .unwrap();
        assert_eq!(w.texts, 1);
        // All drawn text was imported: nothing skipped.
        assert_eq!(import_svg(src.as_bytes()).unwrap().skipped, 0);
    }

    #[test]
    fn stroke_widths_are_clamped_after_scaling() {
        let out = import(
            r#"<rect width="1000" height="1000" fill="red" stroke="black" stroke-width="250"/>"#,
        );
        let view = Rect {
            x: 0.0,
            y: 0.0,
            w: 125.0,
            h: 125.0,
        };
        let placed = place_import(out.shapes, &view);
        assert!(close(placed[0].style.stroke_width, 25.0));
        // A path shape's empty label keeps its size.
        let label = placed[0].label.as_ref().unwrap();
        assert_eq!(label.text.style_at(0).size, crate::text::LABEL_FONT_SIZE);
    }

    #[test]
    fn text_takes_its_fill_opacity_or_its_stroke_colour() {
        let out = import(
            r##"<g opacity="0.5"><text y="20" fill="red" fill-opacity="0.5">A</text></g>
            <text y="60" fill="none" stroke="#00ff00" stroke-opacity="0.4">B</text>"##,
        );
        let (a, b) = (&out.shapes[0], &out.shapes[1]);
        assert!(close(a.style.opacity, 0.25), "{}", a.style.opacity);
        let color = |s: &Shape| s.label.as_ref().unwrap().text.style_at(0).color;
        assert_eq!(color(a).as_deref(), Some("#FF0000"));
        assert_eq!(color(b).as_deref(), Some("#00FF00"));
        assert!(close(b.style.opacity, 0.4));
    }

    #[test]
    fn path_shapes_have_an_empty_shape_label() {
        let out = import(r#"<rect width="5" height="5"/>"#);
        assert_eq!(
            out.shapes[0].label,
            Some(crate::shape::Label::new_shape_label())
        );
    }
}
