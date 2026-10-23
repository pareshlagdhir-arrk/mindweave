//! Importing an SVG file as editable shapes: each filled or stroked path
//! becomes a path shape, each text a text box (see [`import_svg`]); then
//! [`place_import`] fits them to the view as one group.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

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
    /// Elements left out: embedded or linked pictures.
    pub skipped: usize,
}

/// Why an SVG file could not be imported.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SvgImportError {
    #[error("The SVG file is larger than 10 MB.")]
    TooLarge,
    #[error("The SVG has more than 5,000 elements to import.")]
    TooManyObjects,
    #[error("The file is not an SVG Mindweave can read: {0}")]
    Invalid(String),
}

/// Reads `bytes` (an SVG file) as shapes: each visible filled or stroked
/// path (basic shapes included, `<use>` copies resolved, transforms
/// applied) becomes a path shape, each text a text box. Pictures are left
/// out and counted; nothing is read from other files. Text is laid out
/// with the system's fonts; text no installed font can show is lost.
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
    let pictures = Arc::new(AtomicUsize::new(0));
    let opt = usvg::Options {
        image_href_resolver: skipping_pictures(&pictures),
        fontdb: system_fonts(),
        ..usvg::Options::default()
    };
    let tree =
        usvg::Tree::from_str(src, &opt).map_err(|e| SvgImportError::Invalid(e.to_string()))?;
    let mut shapes = Vec::new();
    let mut skipped = 0;
    collect(tree.root(), 1.0, &mut shapes, &mut skipped)?;
    Ok(SvgImport {
        shapes,
        skipped: skipped + pictures.load(Ordering::Relaxed),
    })
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

/// Appends the shapes of `group`'s children, back to front, to `out`;
/// `opacity` is the product of the enclosing groups' opacities. Pictures
/// are counted in `skipped`.
fn collect(
    group: &usvg::Group,
    opacity: f32,
    out: &mut Vec<Shape>,
    skipped: &mut usize,
) -> Result<(), SvgImportError> {
    for node in group.children() {
        match node {
            Node::Group(g) => collect(g, opacity * g.opacity().get(), out, skipped)?,
            Node::Path(p) => out.extend(path_shape(p, opacity)),
            Node::Text(t) => out.extend(text_box(t, opacity)),
            Node::Image(_) => *skipped += 1,
        }
        if out.len() > MAX_IMPORT_OBJECTS {
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
    Some(Shape::new_path(props, Frame::from_rect(&bounds), style))
}

/// The text box of `text`: its plain text (a new line wherever a chunk
/// starts lower down) in its first span's style, over its laid-out
/// bounds, or above its anchor point when it has none.
fn text_box(text: &usvg::Text, opacity: f32) -> Option<Shape> {
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
        return None;
    }
    let first = chunks.first()?;
    let span = first.spans().first()?;
    let t = text.abs_transform();
    let font = span.font();
    let style = RunStyle {
        font: font.families().iter().find_map(|f| match f {
            FontFamily::Named(name) => Some(name.clone()),
            _ => None,
        }),
        size: span.font_size().get() * transform_scale(t),
        bold: font.weight() >= 600,
        italic: font.style() != FontStyle::Normal,
        color: span.fill().and_then(|f| paint_color(f.paint())),
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
    shape.style.opacity = opacity;
    let mut rich = RichText::new(style.clone(), align);
    rich.insert(0, &plain, &style);
    shape.label.as_mut()?.text = rich;
    shape.normalize();
    Some(shape)
}

/// `shapes` (from [`import_svg`]) centred in `view`, scaled down to fit
/// 80% of it when larger (never up), in one new group when there are
/// several. Stroke widths and font sizes scale with them.
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
        if let Some(label) = &mut s.label {
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
}
