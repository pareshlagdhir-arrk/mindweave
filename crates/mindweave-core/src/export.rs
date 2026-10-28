//! Picture export: a page, an artboard or some objects as a standalone
//! SVG document.
//!
//! The drawing follows the app closely enough to check a layout, not
//! pixel for pixel: shapes from their library outlines, path shapes,
//! lines with their heads, topics with their branch lines, summary
//! brackets and boundaries, artboards (clipping their members) with their
//! names, images and topic pictures as embedded data, and text as SVG
//! `<text>` laid out with the estimate of [`crate::text_measure`]. The
//! `auto` and `ink` colours resolve for a light or dark theme. Topic
//! equations are drawn by a caller-supplied renderer, else as their LaTeX
//! source; link and note badges are not drawn.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use base64::Engine as _;
use thiserror::Error;
use uuid::Uuid;

use crate::color::{auto_text_color, is_auto_color, is_ink_color, parse_hex_color};
use crate::document::{Document, Page};
use crate::geometry::{
    bounds, primitive_geometry, to_canvas, FillRole, Geometry, PathCmd, Primitive, Rect, SubPath,
};
use crate::image::{ImageData, ImageFormat};
use crate::library::LibraryRegistry;
use crate::line::{line_geometry, line_jumps, line_label_angle, line_midpoint};
use crate::mindmap::{map_decor, rounded_polygon};
use crate::shape::{Shape, VAlign};
use crate::text::{HAlign, RichText, RunStyle};
use crate::text_measure::{layout, LIST_INDENT};
use crate::transform::frame_aabb;

/// Font family list for text without a font of its own.
const DEFAULT_FONTS: &str = "Helvetica Neue, Helvetica, Arial, sans-serif";
/// Padding between a shape's edge and its label (as the app draws it).
const LABEL_PADDING: f32 = 8.0;
/// Padding between a text box's edge and its text.
const TEXT_BOX_PADDING: f32 = 4.0;
/// Padding of a line label's background patch.
const LINE_LABEL_PADDING: f32 = 2.0;
/// Size of an artboard's name above its corner.
const NAME_SIZE: f32 = 12.0;
/// Largest picture side exported.
pub const MAX_EXPORT_SIDE: f32 = 100_000.0;

/// What part of a page to export.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum ExportRegion {
    /// Everything on the page's visible layers.
    #[default]
    Page,
    /// An artboard and its members, cut to its frame.
    Artboard(Uuid),
    /// These objects (artboards with their members, central topics with
    /// their whole map).
    Objects(Vec<Uuid>),
}

/// Which theme `auto` and `ink` colours follow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    /// The theme's canvas and text colours.
    fn colors(self) -> (&'static str, &'static str) {
        match self {
            Theme::Light => ("#FFFFFF", "#1F2328"),
            Theme::Dark => ("#1E1E1E", "#E6E6E6"),
        }
    }
}

/// An equation drawn as SVG: the document and its size.
pub struct EquationSvg {
    pub svg: String,
    pub size: (f32, f32),
}

/// Renders LaTeX in a `#RRGGBB` colour at a font size.
pub type EquationRenderer<'a> = &'a dyn Fn(&str, &str, f32) -> Option<EquationSvg>;

/// How to export.
#[derive(Clone, Copy, Default)]
pub struct ExportOptions<'a> {
    pub theme: Theme,
    /// Space round what is exported (not for an artboard).
    pub padding: f32,
    /// The libraries shapes come from; without them library shapes are
    /// drawn as boxes.
    pub libs: Option<&'a LibraryRegistry>,
    /// Draws topic equations; without it their source is shown as text.
    pub equation: Option<EquationRenderer<'a>>,
}

/// An exported picture.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgExport {
    pub svg: String,
    /// Its size in canvas units.
    pub width: f32,
    pub height: f32,
}

/// Why an export failed.
#[derive(Debug, Error, PartialEq)]
pub enum ExportError {
    #[error("There is no page {0}.")]
    NoPage(usize),
    #[error("No object with id {0} on this page.")]
    NotFound(Uuid),
    #[error("Object {0} is not an artboard.")]
    NotArtboard(Uuid),
    #[error("The picture would be larger than {MAX_EXPORT_SIDE} units across.")]
    TooLarge,
}

/// A number for SVG: at most two decimals, no trailing zeros.
fn n(v: f32) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// `s` escaped for XML text and attributes.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// Path data of `cmds`.
fn path_data(cmds: &[PathCmd]) -> String {
    let mut d = String::new();
    for c in cmds {
        match *c {
            PathCmd::MoveTo(p) => {
                let _ = write!(d, "M{} {}", n(p.0), n(p.1));
            }
            PathCmd::LineTo(p) => {
                let _ = write!(d, "L{} {}", n(p.0), n(p.1));
            }
            PathCmd::QuadTo { ctrl, to } => {
                let _ = write!(d, "Q{} {} {} {}", n(ctrl.0), n(ctrl.1), n(to.0), n(to.1));
            }
            PathCmd::CubicTo { c1, c2, to } => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {}",
                    n(c1.0),
                    n(c1.1),
                    n(c2.0),
                    n(c2.1),
                    n(to.0),
                    n(to.1)
                );
            }
            PathCmd::Close => d.push('Z'),
        }
    }
    d
}

/// The colours the export resolves style colours with.
#[derive(Clone, Copy)]
struct Palette {
    /// The page's canvas colour (what `auto` fills are).
    canvas: (u8, u8, u8),
    /// The theme's text colour (`auto` strokes and `ink`).
    ink: (u8, u8, u8),
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

impl Palette {
    /// A fill colour: `auto` is the canvas, `ink` the text colour.
    fn fill(&self, c: Option<&str>) -> Option<String> {
        if is_auto_color(c) {
            Some(hex(self.canvas))
        } else if is_ink_color(c) {
            Some(hex(self.ink))
        } else {
            c.and_then(parse_hex_color).map(hex)
        }
    }

    /// A stroke colour: `auto` and `ink` are the text colour.
    fn stroke(&self, c: Option<&str>) -> Option<String> {
        if is_auto_color(c) || is_ink_color(c) {
            Some(hex(self.ink))
        } else {
            c.and_then(parse_hex_color).map(hex)
        }
    }

    /// What auto-coloured text on `fill` contrasts with.
    fn behind(&self, fill: Option<&str>) -> (u8, u8, u8) {
        if is_ink_color(fill) {
            self.ink
        } else {
            fill.and_then(parse_hex_color).unwrap_or(self.canvas)
        }
    }
}

/// The closed box `0..w × 0..h`.
fn box_geometry(w: f32, h: f32, fill: FillRole) -> Geometry {
    Geometry {
        paths: vec![SubPath {
            cmds: vec![
                PathCmd::MoveTo((0.0, 0.0)),
                PathCmd::LineTo((w, 0.0)),
                PathCmd::LineTo((w, h)),
                PathCmd::LineTo((0.0, h)),
                PathCmd::Close,
            ],
            fill,
        }],
    }
}

/// A rounded rectangle `w × h` with corner radius `r`.
fn rounded_box(w: f32, h: f32, r: f32) -> Geometry {
    primitive_geometry(Primitive::Rect, w, h, &|k| {
        (k == "corner_radius").then_some(r)
    })
}

/// `shape`'s outline in canvas coordinates, and whether it is a
/// placeholder for a missing library shape. A line jumps at `jumps`
/// (lengths along its route).
fn outline(shape: &Shape, libs: Option<&LibraryRegistry>, jumps: &[f32]) -> (Geometry, bool) {
    let f = &shape.frame;
    let (local, placeholder) = if shape.is_text_box() || shape.is_image() {
        (box_geometry(f.w, f.h, FillRole::Shape), false)
    } else if let Some(b) = &shape.boundary {
        let g = if b.outline.len() >= 3 {
            rounded_polygon(&b.outline, 18.0)
        } else {
            rounded_box(f.w, f.h, 12.0)
        };
        (g, false)
    } else if shape.is_artboard() {
        (
            primitive_geometry(Primitive::Rect, f.w, f.h, &|k| shape.param(k)),
            false,
        )
    } else if let Some(p) = shape.path.as_ref().filter(|_| shape.is_path()) {
        (p.local_geometry(f.w, f.h), false)
    } else if let Some(t) = &shape.topic {
        (
            rounded_box(f.w, f.h, if t.is_central() { 12.0 } else { 6.0 }),
            false,
        )
    } else if let Some(l) = shape.line.as_ref().filter(|_| shape.is_line()) {
        let g = line_geometry(f, l, shape.style.stroke_width, shape.style.double, jumps);
        (g, false)
    } else {
        match libs.and_then(|l| l.shape(&shape.library, &shape.shape)) {
            Some(def) => (def.local_geometry(f.w, f.h, &|k| shape.param(k)), false),
            None => (box_geometry(f.w, f.h, FillRole::None), true),
        }
    };
    (to_canvas(&local, f), placeholder)
}

/// The export's state while writing.
struct Writer<'a> {
    out: String,
    defs: String,
    palette: Palette,
    opts: &'a ExportOptions<'a>,
    /// Clip path id by clipping artboard.
    clips: HashMap<Uuid, String>,
    /// How each task topic on the page is shown.
    tasks: HashMap<Uuid, crate::task::TaskView>,
    /// Where each line jumps the lines it crosses (see [`line_jumps`]).
    jumps: HashMap<Uuid, Vec<f32>>,
}

impl Writer<'_> {
    /// Writes `g` painted in `shape`'s style.
    fn paint(&mut self, g: &Geometry, shape: &Shape, placeholder: bool) {
        let st = &shape.style;
        let p = self.palette;
        let (fill, stroke) = if placeholder {
            (None, Some(hex(p.ink)))
        } else {
            (p.fill(st.fill.as_deref()), p.stroke(st.stroke.as_deref()))
        };
        let width = st.stroke_width;
        let dash = if placeholder {
            Some([4.0, 3.0])
        } else {
            st.dash.pattern(width)
        };
        let boundary = shape.is_boundary();
        for sub in &g.paths {
            let d = path_data(&sub.cmds);
            if d.is_empty() {
                continue;
            }
            let (f, s, rule) = match sub.fill {
                FillRole::Shape => (fill.clone(), stroke.clone(), "evenodd"),
                FillRole::NonZero => (fill.clone(), stroke.clone(), "nonzero"),
                // Even-odd, so a hollow head's ring keeps its hole.
                FillRole::StrokeColor => (stroke.clone(), None, "evenodd"),
                FillRole::None => (None, stroke.clone(), "evenodd"),
                FillRole::FillOnly => (fill.clone(), None, "evenodd"),
                FillRole::Canvas => (Some(hex(p.canvas)), None, "nonzero"),
            };
            if f.is_none() && (s.is_none() || width <= 0.0) {
                continue;
            }
            let _ = write!(self.out, "<path d=\"{d}\" fill-rule=\"{rule}\"");
            match &f {
                Some(c) => {
                    let _ = write!(self.out, " fill=\"{c}\"");
                    if boundary {
                        self.out.push_str(" fill-opacity=\"0.1\"");
                    }
                }
                None => self.out.push_str(" fill=\"none\""),
            }
            if let Some(c) = s.as_ref().filter(|_| width > 0.0) {
                let _ = write!(
                    self.out,
                    " stroke=\"{c}\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"",
                    n(width)
                );
                if let Some([a, b]) = dash {
                    let _ = write!(self.out, " stroke-dasharray=\"{} {}\"", n(a), n(b));
                }
                if boundary {
                    self.out.push_str(" stroke-opacity=\"0.8\"");
                }
            }
            self.out.push_str("/>");
        }
    }

    /// Writes `shape` with its text, pictures and equation.
    fn shape(&mut self, shape: &Shape) {
        let clip = shape.artboard.and_then(|a| self.clips.get(&a).cloned());
        let opacity = shape.style.opacity;
        let _ = write!(self.out, "<g");
        if opacity < 1.0 {
            let _ = write!(self.out, " opacity=\"{}\"", n(opacity));
        }
        if let Some(c) = clip {
            let _ = write!(self.out, " clip-path=\"url(#{c})\"");
        }
        self.out.push('>');
        let jumps = self.jumps.get(&shape.id).map_or(&[][..], Vec::as_slice);
        let (g, placeholder) = outline(shape, self.opts.libs, jumps);
        if let Some(data) = shape.image.as_ref().filter(|_| shape.is_image()) {
            let f = shape.frame;
            self.image(
                data,
                Rect {
                    x: f.x,
                    y: f.y,
                    w: f.w,
                    h: f.h,
                },
            );
            self.paint(&g, shape, false);
        } else {
            self.paint(&g, shape, placeholder);
        }
        if shape.is_line() {
            self.line_label(shape);
        } else {
            self.rotated(shape, |w| {
                w.topic_extras(shape);
                w.label(shape, placeholder);
            });
        }
        self.out.push_str("</g>");
    }

    /// Runs `f` inside a group turned like `shape`'s frame.
    fn rotated(&mut self, shape: &Shape, f: impl FnOnce(&mut Self)) {
        let fr = &shape.frame;
        let turned = fr.rotation != 0.0;
        if turned {
            let (cx, cy) = (fr.x + fr.w / 2.0, fr.y + fr.h / 2.0);
            let _ = write!(
                self.out,
                "<g transform=\"rotate({} {} {})\">",
                n(fr.rotation),
                n(cx),
                n(cy)
            );
        }
        f(self);
        if turned {
            self.out.push_str("</g>");
        }
    }

    /// Writes picture `data` fitted into `r`.
    fn image(&mut self, data: &ImageData, r: Rect) {
        if data.bytes.is_empty() {
            return;
        }
        let mime = match data.format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Webp => "image/webp",
            ImageFormat::Bmp => "image/bmp",
            ImageFormat::Tiff => "image/tiff",
            ImageFormat::Svg => "image/svg+xml",
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(data.bytes.as_slice());
        let _ = write!(
            self.out,
            "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"xMidYMid meet\" href=\"data:{mime};base64,{b64}\"/>",
            n(r.x),
            n(r.y),
            n(r.w),
            n(r.h)
        );
    }

    /// A task badge in `r` showing `view`: a ring with the progress as a
    /// wedge, or a ticked disc when done.
    fn task_badge(&mut self, r: Rect, view: &crate::task::TaskView) {
        use crate::task::{task_pie, task_tick, TaskStatus};
        let color = view.status.color();
        let (cx, cy, rad) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0);
        if view.status == TaskStatus::Done {
            let _ = write!(
                self.out,
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{color}\"/>",
                n(cx),
                n(cy),
                n(rad)
            );
            let tick: Vec<String> = task_tick((r.x, r.y), r.w)
                .iter()
                .map(|(x, y)| format!("{},{}", n(*x), n(*y)))
                .collect();
            let _ = write!(
                self.out,
                "<polyline points=\"{}\" fill=\"none\" stroke=\"#FFFFFF\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                tick.join(" "),
                n(r.w * 0.12)
            );
            return;
        }
        let stroke = r.w * 0.1;
        let _ = write!(
            self.out,
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\"/>",
            n(cx),
            n(cy),
            n(rad - stroke / 2.0),
            n(stroke)
        );
        let pie = task_pie((cx, cy), rad - stroke * 1.5, view.progress);
        if !pie.is_empty() {
            let pts: Vec<String> = pie
                .iter()
                .map(|(x, y)| format!("{},{}", n(*x), n(*y)))
                .collect();
            let _ = write!(
                self.out,
                "<polygon points=\"{}\" fill=\"{color}\"/>",
                pts.join(" ")
            );
        }
    }

    /// A topic's picture, task badge and equation, unrotated (inside
    /// [`Self::rotated`]).
    fn topic_extras(&mut self, shape: &Shape) {
        let Some(t) = &shape.topic else { return };
        let f = shape.frame;
        let parts = t.parts(f.w, f.h);
        let at = |r: Rect| Rect {
            x: f.x + r.x,
            y: f.y + r.y,
            ..r
        };
        if let (Some(pic), Some(r)) = (&t.picture, parts.picture) {
            self.image(pic, at(r));
        }
        if let (Some(view), Some(r)) = (self.tasks.get(&shape.id).cloned(), parts.task) {
            self.task_badge(at(r), &view);
        }
        let (Some(eq), Some(r)) = (&t.equation, parts.equation) else {
            return;
        };
        let r = at(r);
        let color = self.text_color(shape, &RunStyle::default());
        let size = shape
            .label
            .as_ref()
            .map_or(crate::text::DEFAULT_FONT_SIZE, |l| max_size(&l.text));
        let rendered = self
            .opts
            .equation
            .and_then(|render| render(&eq.latex, &color, size));
        match rendered {
            Some(svg) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(svg.svg.as_bytes());
                let _ = write!(
                    self.out,
                    "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"data:image/svg+xml;base64,{b64}\"/>",
                    n(r.x),
                    n(r.y),
                    n(r.w),
                    n(r.h)
                );
            }
            None => {
                let _ = write!(
                    self.out,
                    "<text x=\"{}\" y=\"{}\" font-family=\"{DEFAULT_FONTS}\" font-size=\"{}\" font-style=\"italic\" fill=\"{color}\" text-anchor=\"middle\" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\">{}</text>",
                    n(r.x + r.w / 2.0),
                    n(r.y + r.h * 0.75),
                    n((r.h * 0.8).min(size)),
                    n(r.w),
                    esc(&eq.latex)
                );
            }
        }
    }

    /// The colour of text in `style` on `shape`.
    fn text_color(&self, shape: &Shape, style: &RunStyle) -> String {
        match style.color.as_deref().and_then(parse_hex_color) {
            Some(c) => hex(c),
            None => hex(auto_text_color(
                self.palette.behind(shape.style.fill.as_deref()),
            )),
        }
    }

    /// Writes `shape`'s label in its frame (unrotated, see
    /// [`Self::rotated`]); a placeholder shows its library and shape.
    fn label(&mut self, shape: &Shape, placeholder: bool) {
        let f = shape.frame;
        if placeholder {
            let name = shape
                .name
                .clone()
                .unwrap_or_else(|| format!("{}/{}", shape.library, shape.shape));
            let text = crate::ops::rich_text_plain(
                &name,
                &RunStyle {
                    size: 11.0,
                    ..RunStyle::default()
                },
                HAlign::Center,
            );
            let wrap = (f.w - 2.0 * LABEL_PADDING).max(1.0);
            self.text_block(
                shape,
                &text,
                (f.x + LABEL_PADDING, f.y + LABEL_PADDING),
                Some(wrap),
                wrap,
            );
            return;
        }
        let Some(label) = shape.label.as_ref().filter(|l| !l.text.is_empty()) else {
            return;
        };
        if shape.is_text_box() {
            let wrap = (!shape.auto_width()).then(|| (f.w - 2.0 * TEXT_BOX_PADDING).max(1.0));
            let origin = (f.x + TEXT_BOX_PADDING, f.y + TEXT_BOX_PADDING);
            let width = (f.w - 2.0 * TEXT_BOX_PADDING).max(1.0);
            self.text_block(shape, &label.text, origin, wrap, width);
            return;
        }
        let area = shape.label_area();
        let wrap = (area.w - 2.0 * LABEL_PADDING).max(1.0);
        let (_, th) = crate::text_measure::measure(&label.text, Some(wrap));
        let y = match label.valign {
            VAlign::Top => LABEL_PADDING,
            VAlign::Middle => (area.h - th) / 2.0,
            VAlign::Bottom => area.h - LABEL_PADDING - th,
        };
        self.text_block(
            shape,
            &label.text,
            (f.x + area.x + LABEL_PADDING, f.y + area.y + y),
            Some(wrap),
            wrap,
        );
    }

    /// Writes `text` with its top-left corner at `origin`, wrapped at
    /// `wrap` (when given) and aligned within `aw`.
    fn text_block(
        &mut self,
        shape: &Shape,
        text: &RichText,
        origin: (f32, f32),
        wrap: Option<f32>,
        aw: f32,
    ) {
        let lines = layout(text, wrap);
        let mut y = origin.1;
        for line in &lines {
            let baseline = y + (line.height - line.size) / 2.0 + line.size * 0.8;
            let free = (aw - line.indent - line.width).max(0.0);
            let x = origin.0
                + line.indent
                + match line.align {
                    HAlign::Left => 0.0,
                    HAlign::Center => free / 2.0,
                    HAlign::Right => free,
                };
            if let Some(m) = &line.marker {
                let style = line
                    .segments
                    .first()
                    .map(|s| s.style.clone())
                    .unwrap_or_default();
                let _ = write!(
                    self.out,
                    "<text x=\"{}\" y=\"{}\" font-family=\"{DEFAULT_FONTS}\" font-size=\"{}\" fill=\"{}\">{}</text>",
                    n(origin.0 + line.indent - LIST_INDENT + 4.0),
                    n(baseline),
                    n(style.size),
                    self.text_color(shape, &style),
                    esc(m)
                );
            }
            if !line.segments.is_empty() {
                let _ = write!(
                    self.out,
                    "<text x=\"{}\" y=\"{}\" xml:space=\"preserve\">",
                    n(x),
                    n(baseline)
                );
                for seg in &line.segments {
                    self.tspan(shape, &seg.text, &seg.style);
                }
                self.out.push_str("</text>");
            }
            y += line.height;
        }
    }

    /// Writes one run of text.
    fn tspan(&mut self, shape: &Shape, text: &str, style: &RunStyle) {
        let family = style
            .font
            .as_ref()
            .map(|f| format!("{}, {DEFAULT_FONTS}", esc(f)))
            .unwrap_or_else(|| DEFAULT_FONTS.to_string());
        let _ = write!(
            self.out,
            "<tspan font-family=\"{family}\" font-size=\"{}\" fill=\"{}\"",
            n(style.size),
            self.text_color(shape, style)
        );
        if style.bold {
            self.out.push_str(" font-weight=\"bold\"");
        }
        if style.italic {
            self.out.push_str(" font-style=\"italic\"");
        }
        match (style.underline, style.strike) {
            (true, true) => self
                .out
                .push_str(" text-decoration=\"underline line-through\""),
            (true, false) => self.out.push_str(" text-decoration=\"underline\""),
            (false, true) => self.out.push_str(" text-decoration=\"line-through\""),
            _ => {}
        }
        let _ = write!(self.out, ">{}</tspan>", esc(text));
    }

    /// Writes a line's label on a patch of canvas, centred on the line's
    /// middle and turned along a straight line.
    fn line_label(&mut self, shape: &Shape) {
        let (Some(label), Some(props)) = (shape.label.as_ref(), shape.line.as_ref()) else {
            return;
        };
        if label.text.is_empty() {
            return;
        }
        let centre = line_midpoint(&shape.frame, props);
        let (_, straight) = crate::geometry::line_label_placement(&shape.frame);
        let angle = line_label_angle(props.route, straight);
        let (w, h) = crate::text_measure::measure(&label.text, None);
        let pad = LINE_LABEL_PADDING;
        let _ = write!(
            self.out,
            "<g transform=\"rotate({} {} {})\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
            n(angle),
            n(centre.0),
            n(centre.1),
            n(centre.0 - w / 2.0 - pad),
            n(centre.1 - h / 2.0 - pad),
            n(w + 2.0 * pad),
            n(h + 2.0 * pad),
            hex(self.palette.canvas)
        );
        let plain = Shape {
            style: crate::shape::Style::default(),
            ..shape.clone()
        };
        self.text_block(
            &plain,
            &label.text,
            (centre.0 - w / 2.0, centre.1 - h / 2.0),
            None,
            w,
        );
        self.out.push_str("</g>");
    }

    /// Writes an artboard's name above its top-left corner.
    fn artboard_name(&mut self, shape: &Shape) {
        let Some(name) = shape.name.as_deref().filter(|n| !n.trim().is_empty()) else {
            return;
        };
        let muted = hex(mix(self.palette.ink, self.palette.canvas));
        let _ = write!(
            self.out,
            "<text x=\"{}\" y=\"{}\" font-family=\"{DEFAULT_FONTS}\" font-size=\"{}\" fill=\"{muted}\">{}</text>",
            n(shape.frame.x),
            n(shape.frame.y - 6.0),
            n(NAME_SIZE),
            esc(name)
        );
    }

    /// Writes branch or bracket geometry `g` in topic `topic`'s stroke.
    fn decor(&mut self, g: &Geometry, topic: &Shape) {
        let stroke = self
            .palette
            .stroke(topic.style.stroke.as_deref())
            .unwrap_or_else(|| hex(self.palette.ink));
        let width = topic.style.stroke_width.max(1.0);
        for sub in &g.paths {
            let d = path_data(&sub.cmds);
            if sub.fill == FillRole::StrokeColor {
                let _ = write!(self.out, "<path d=\"{d}\" fill=\"{stroke}\"/>");
            } else {
                let _ = write!(
                    self.out,
                    "<path d=\"{d}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                    n(width)
                );
            }
        }
    }
}

/// Halfway between two colours.
fn mix(a: (u8, u8, u8), b: (u8, u8, u8)) -> (u8, u8, u8) {
    let m = |x: u8, y: u8| ((u16::from(x) + u16::from(y)) / 2) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// The largest font size in `text`.
fn max_size(text: &RichText) -> f32 {
    text.paragraphs
        .iter()
        .flat_map(|p| p.runs.iter().map(|r| r.style.size).chain([p.style.size]))
        .fold(0.0, f32::max)
}

/// The ids `region` exports from `page`'s visible layers, and the box to
/// show (`None` for the whole of what is drawn).
fn selection(
    page: &Page,
    region: &ExportRegion,
) -> Result<(HashSet<Uuid>, Option<Rect>), ExportError> {
    let visible: Vec<&Shape> = page
        .layers
        .iter()
        .filter(|l| l.visible)
        .flat_map(|l| &l.objects)
        .collect();
    let all: Vec<&Shape> = page.layers.iter().flat_map(|l| &l.objects).collect();
    match region {
        ExportRegion::Page => Ok((visible.iter().map(|s| s.id).collect(), None)),
        ExportRegion::Artboard(id) => {
            let board = all
                .iter()
                .find(|s| s.id == *id)
                .ok_or(ExportError::NotFound(*id))?;
            if !board.is_artboard() {
                return Err(ExportError::NotArtboard(*id));
            }
            let mut ids: HashSet<Uuid> = visible
                .iter()
                .filter(|s| s.artboard == Some(*id))
                .map(|s| s.id)
                .collect();
            ids.insert(*id);
            Ok((ids, Some(frame_aabb(&board.frame))))
        }
        ExportRegion::Objects(wanted) => {
            for id in wanted {
                if !all.iter().any(|s| s.id == *id) {
                    return Err(ExportError::NotFound(*id));
                }
            }
            let picked: HashSet<Uuid> = wanted.iter().copied().collect();
            let maps: HashSet<Uuid> = all
                .iter()
                .filter(|s| picked.contains(&s.id))
                .filter_map(|s| s.topic.as_ref().filter(|t| t.is_central()).map(|t| t.map))
                .collect();
            let ids = visible
                .iter()
                .filter(|s| {
                    picked.contains(&s.id)
                        || s.artboard.is_some_and(|a| picked.contains(&a))
                        || s.topic.as_ref().is_some_and(|t| maps.contains(&t.map))
                })
                .map(|s| s.id)
                .collect();
            Ok((ids, None))
        }
    }
}

/// The box `shape` covers on the canvas.
fn shape_bounds(shape: &Shape, libs: Option<&LibraryRegistry>) -> Rect {
    if shape.is_line() {
        let (g, _) = outline(shape, libs, &[]);
        if let Some(b) = bounds(&g) {
            return b;
        }
    }
    frame_aabb(&shape.frame)
}

/// Exports page `page` of `doc` (see the module docs).
pub fn export_svg(
    doc: &Document,
    page: usize,
    region: &ExportRegion,
    opts: &ExportOptions,
) -> Result<SvgExport, ExportError> {
    let p = doc.pages.get(page).ok_or(ExportError::NoPage(page))?;
    let (ids, fixed) = selection(p, region)?;
    let shapes: Vec<&Shape> = p
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .filter(|s| ids.contains(&s.id))
        .collect();
    let view = match fixed {
        Some(r) => r,
        None => {
            let mut acc: Option<Rect> = None;
            for s in &shapes {
                let mut b = shape_bounds(s, opts.libs);
                if s.is_artboard() {
                    // Room for its name above.
                    b.y -= NAME_SIZE + 8.0;
                    b.h += NAME_SIZE + 8.0;
                }
                acc = Some(acc.map_or(b, |a| a.union(&b)));
            }
            let r = acc.unwrap_or(Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 120.0,
            });
            let pad = opts.padding.max(0.0);
            Rect {
                x: r.x - pad,
                y: r.y - pad,
                w: r.w + 2.0 * pad,
                h: r.h + 2.0 * pad,
            }
        }
    };
    let view = Rect {
        w: view.w.max(1.0),
        h: view.h.max(1.0),
        ..view
    };
    if view.w > MAX_EXPORT_SIDE || view.h > MAX_EXPORT_SIDE {
        return Err(ExportError::TooLarge);
    }
    let (theme_bg, theme_fg) = opts.theme.colors();
    let parse = |c: &str| parse_hex_color(c).unwrap_or((255, 255, 255));
    // A light or dark page, or one with its own background, keeps its
    // colours; otherwise the export's theme applies.
    let page_palette = p.canvas.palette();
    let palette = Palette {
        canvas: page_palette.background.unwrap_or_else(|| parse(theme_bg)),
        ink: page_palette.ink.unwrap_or_else(|| parse(theme_fg)),
    };
    let mut w = Writer {
        out: String::new(),
        defs: String::new(),
        palette,
        opts,
        clips: HashMap::new(),
        tasks: crate::task::task_views(p),
        jumps: line_jumps(p),
    };
    for (i, s) in shapes.iter().filter(|s| s.is_artboard()).enumerate() {
        if s.artboard_props.as_ref().is_some_and(|a| a.clip) {
            let id = format!("clip{i}");
            let r = frame_aabb(&s.frame);
            let _ = write!(
                w.defs,
                "<clipPath id=\"{id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>",
                n(r.x),
                n(r.y),
                n(r.w),
                n(r.h)
            );
            w.clips.insert(s.id, id);
        }
    }
    let by_id: HashMap<Uuid, &Shape> = shapes.iter().map(|s| (s.id, *s)).collect();
    let width_of = |id: Uuid| by_id.get(&id).map_or(1.0, |s| s.style.stroke_width);
    let decor = map_decor(p, width_of);
    // Bottom layer first; each layer's artboards, then its maps' branches
    // and brackets, then its other shapes (as the app paints).
    for layer in p.layers.iter().rev().filter(|l| l.visible) {
        let here: Vec<&Shape> = layer
            .objects
            .iter()
            .filter(|s| ids.contains(&s.id))
            .collect();
        for s in here.iter().filter(|s| s.is_artboard()) {
            w.shape(s);
        }
        let on_layer: HashSet<Uuid> = here.iter().map(|s| s.id).collect();
        for (id, g) in decor.branches.iter().chain(&decor.brackets) {
            let Some(topic) = by_id.get(id).filter(|_| on_layer.contains(id)) else {
                continue;
            };
            let parent_in = topic
                .topic
                .as_ref()
                .and_then(|t| t.parent)
                .is_some_and(|p| ids.contains(&p));
            if parent_in {
                w.decor(g, topic);
            }
        }
        for s in here.iter().filter(|s| !s.is_artboard()) {
            w.shape(s);
        }
    }
    for s in shapes.iter().filter(|s| s.is_artboard()) {
        if fixed.is_none() {
            w.artboard_name(s);
        }
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\"><defs>{}</defs><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>{}</svg>",
        n(view.w),
        n(view.h),
        n(view.x),
        n(view.y),
        n(view.w),
        n(view.h),
        w.defs,
        n(view.x),
        n(view.y),
        n(view.w),
        n(view.h),
        hex(palette.canvas),
        w.out
    );
    Ok(SvgExport {
        svg,
        width: view.w,
        height: view.h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::builtin_libraries;
    use crate::ops::{ArtboardSize, ConnectOptions, StyleInput, TopicPosition, Workspace};
    use std::sync::Arc;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let libs = Arc::new(LibraryRegistry::new(builtin_libraries(), None).0);
        let ws = Workspace::create(&dir.path().join("x.mindweave"), None, libs).unwrap();
        (dir, ws)
    }

    #[test]
    fn a_page_exports_shapes_text_lines_and_maps() {
        let (_dir, mut ws) = workspace();
        let a = ws
            .add_shape(
                None,
                "basic-shapes",
                "rectangle",
                (0.0, 0.0),
                None,
                Some("Box & co"),
                Some(&StyleInput {
                    fill: Some("#AABBCC".into()),
                    ..StyleInput::default()
                }),
            )
            .unwrap()[0];
        let b = ws
            .add_shape(
                None,
                "basic-shapes",
                "ellipse",
                (300.0, 0.0),
                None,
                None,
                None,
            )
            .unwrap()[0];
        ws.connect(a, b, &ConnectOptions::default()).unwrap();
        let map = ws
            .add_mind_map(None, "Central", Some((0.0, 400.0)), None)
            .unwrap();
        ws.add_topics(map, &["Left".into(), "Right".into()], TopicPosition::Child)
            .unwrap();
        let libs = ws.libraries().clone();
        let out = export_svg(
            ws.doc(),
            0,
            &ExportRegion::Page,
            &ExportOptions {
                padding: 20.0,
                libs: Some(&libs),
                ..ExportOptions::default()
            },
        )
        .unwrap();
        let svg = &out.svg;
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("fill=\"#AABBCC\""));
        assert!(svg.contains("Box &amp; co"));
        assert!(svg.contains(">Central</tspan>"));
        assert!(svg.contains(">Right</tspan>"));
        // Branch lines are filled tapered paths in branch colours.
        assert!(svg.contains(&format!("fill=\"{}\"", crate::mindmap::BRANCH_COLORS[0])));
        assert!(out.width > 300.0 && out.height > 400.0);
        // A valid document.
        roxmltree::Document::parse(svg).unwrap();
    }

    #[test]
    fn an_artboard_exports_cut_to_its_frame_in_the_dark_theme() {
        let (_dir, mut ws) = workspace();
        let board = ws
            .add_artboard(
                None,
                &ArtboardSize::Size {
                    width: 300.0,
                    height: 200.0,
                },
                Some("Card"),
                None,
            )
            .unwrap();
        ws.add_text(None, (20.0, 20.0), "Inside", None, None)
            .unwrap();
        ws.add_text(None, (900.0, 20.0), "Outside", None, None)
            .unwrap();
        let out = export_svg(
            ws.doc(),
            0,
            &ExportRegion::Artboard(board),
            &ExportOptions {
                theme: Theme::Dark,
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert_eq!((out.width, out.height), (300.0, 200.0));
        assert!(out.svg.contains("Inside"));
        assert!(!out.svg.contains("Outside"));
        assert!(out.svg.contains("#1E1E1E"));
        assert!(out.svg.contains("clip-path"));
        let err = export_svg(
            ws.doc(),
            0,
            &ExportRegion::Artboard(Uuid::new_v4()),
            &ExportOptions::default(),
        );
        assert!(matches!(err, Err(ExportError::NotFound(_))));
        assert_eq!(
            export_svg(ws.doc(), 4, &ExportRegion::Page, &ExportOptions::default()),
            Err(ExportError::NoPage(4))
        );
    }

    #[test]
    fn equations_use_the_renderer_or_show_their_source() {
        let (_dir, mut ws) = workspace();
        ws.set_equation_measure(Box::new(|_, _| Some((40.0, 20.0))));
        let map = ws.add_mind_map(None, "M", None, None).unwrap();
        ws.set_topic(
            map,
            &crate::ops::TopicEdit {
                equation: Some("x^2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let plain = export_svg(
            ws.doc(),
            0,
            &ExportRegion::Objects(vec![map]),
            &ExportOptions::default(),
        )
        .unwrap();
        assert!(plain.svg.contains(">x^2</text>"));
        let render = |_: &str, _: &str, _: f32| {
            Some(EquationSvg {
                svg: "<svg xmlns=\"http://www.w3.org/2000/svg\"/>".into(),
                size: (40.0, 20.0),
            })
        };
        let drawn = export_svg(
            ws.doc(),
            0,
            &ExportRegion::Objects(vec![map]),
            &ExportOptions {
                equation: Some(&render),
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert!(drawn.svg.contains("data:image/svg+xml;base64"));
    }

    #[test]
    fn hollow_heads_are_rings_over_the_canvas() {
        let (_dir, mut ws) = workspace();
        let a = ws
            .add_shape(
                None,
                "basic-shapes",
                "rectangle",
                (0.0, 0.0),
                None,
                None,
                None,
            )
            .unwrap()[0];
        let b = ws
            .add_shape(
                None,
                "basic-shapes",
                "rectangle",
                (400.0, 0.0),
                None,
                None,
                None,
            )
            .unwrap()[0];
        let line = ws
            .connect(
                a,
                b,
                &ConnectOptions {
                    end_head: Some("hollow-triangle".into()),
                    stroke: Some("#FF0000".into()),
                    ..ConnectOptions::default()
                },
            )
            .unwrap();
        let libs = ws.libraries().clone();
        let out = export_svg(
            ws.doc(),
            0,
            &ExportRegion::Objects(vec![line]),
            &ExportOptions {
                libs: Some(&libs),
                ..ExportOptions::default()
            },
        )
        .unwrap();
        let doc = roxmltree::Document::parse(&out.svg).unwrap();
        let paths: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("path"))
            .collect();
        // The interior is painted in the canvas colour.
        assert!(paths.iter().any(|p| p.attribute("fill") == Some("#FFFFFF")));
        // The red filled parts are rings (two contours, even-odd), never
        // a solid triangle.
        let red: Vec<_> = paths
            .iter()
            .filter(|p| p.attribute("fill") == Some("#FF0000"))
            .collect();
        assert!(!red.is_empty());
        for p in red {
            assert_eq!(p.attribute("fill-rule"), Some("evenodd"));
            assert_eq!(p.attribute("d").unwrap().matches('M').count(), 2);
        }
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(n(1.0), "1");
        assert_eq!(n(1.256), "1.26");
        assert_eq!(n(-0.001), "0");
    }
}
