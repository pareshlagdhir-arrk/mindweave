//! Paints shape labels, line labels and text boxes on the canvas.
//!
//! Upright text is painted with gpui's glyph sprites. gpui cannot paint a
//! glyph sprite rotated, so rotated text (labels of rotated shapes and
//! text boxes, line labels at an angle) is drawn as filled glyph outlines,
//! read with font-kit from the same font faces gpui shapes with. Glyphs
//! whose outline is not available (fallback fonts gpui picked for missing
//! characters, emoji) are painted upright at their rotated positions.

use std::collections::HashMap;
use std::rc::Rc;

use super::hex_to_hsla;
use super::text_layout::{
    canvas_default_family, installed_font_families, max_font_size, should_greek, text_run,
    CacheEntry, TextLayoutCache,
};
use gpui_kit::*;
use mindweave_core::geometry::{line_label_placement, Pt, Rect};
use mindweave_core::{
    auto_text_color, parse_hex_color, Mixed, RichText, RunStyle, Shape, VAlign, Viewport,
};
use uuid::Uuid;

/// Inner padding of a shape label, in canvas units.
pub const LABEL_PADDING: f32 = 8.;
/// Padding between a text box's edge and its text, in canvas units.
pub const TEXT_BOX_PADDING: f32 = 4.;
/// Padding of a line label's background patch, in canvas units.
pub const LINE_LABEL_PADDING: f32 = 2.;
/// Underline position below the baseline, as a fraction of the descent
/// (as gpui draws it).
const UNDERLINE_DESCENT: f32 = 0.618;
/// Strikethrough position above the baseline, as a fraction of the font
/// size.
const STRIKE_RISE: f32 = 0.3;
/// Opacity of the bars standing in for too-small text, times the text's.
const GREEK_OPACITY: f32 = 0.4;
/// Space between a list marker and its item's text, in canvas units.
const MARKER_GAP: f32 = 6.;

/// The colour links are drawn in: the theme's link colour, else (when the
/// theme leaves it transparent) its primary colour.
pub fn link_color(link: Hsla, primary: Hsla) -> Hsla {
    if link.a > 0. {
        link
    } else {
        primary
    }
}

/// What painting text needs besides the shape.
pub struct TextPaint<'a> {
    pub layouts: &'a mut TextLayoutCache,
    pub glyphs: &'a mut GlyphOutlines,
    /// The canvas background (see [`canvas_background`]).
    pub canvas: (u8, u8, u8),
}

/// Rotated glyph outlines of a cached layout, tessellated in pixels
/// around the rotation centre, so panning only moves them.
pub struct RotatedPaths {
    /// Angle, text origin (from the centre) and clip they were built for.
    key: [u32; 5],
    paths: Vec<(Path<Pixels>, Hsla)>,
    /// Glyphs without an outline: painted upright at their position.
    upright: Vec<UprightGlyph>,
}

struct UprightGlyph {
    /// Baseline origin, from the rotation centre.
    origin: Point<Pixels>,
    font_id: FontId,
    glyph_id: GlyphId,
    font_size: Pixels,
    color: Hsla,
    emoji: bool,
}

/// Where and how a layout is painted: its top-left corner `offset` pixels
/// from `centre` (window coordinates), turned by `angle` degrees about
/// `centre`. Lines outside `clip` (pixels from the top-left corner, top
/// and bottom) are skipped; when upright, the text is also masked to
/// `mask`.
struct Placement {
    centre: Point<Pixels>,
    offset: (f32, f32),
    angle: f32,
    clip: Option<(f32, f32)>,
    mask: Option<Bounds<Pixels>>,
}

/// Paints `shape`'s label, or a text box's text, after its outline.
/// `outline_visible` says whether the shape's outline reaches into the
/// canvas; labels and text boxes stay inside it.
pub fn paint_shape_text(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    vp: &Viewport,
    shape: &Shape,
    outline_visible: bool,
    text: &mut TextPaint,
) {
    let Some(label) = &shape.label else {
        return;
    };
    if label.text.is_empty() {
        return;
    }
    let zoom = vp.zoom;
    let f = &shape.frame;
    let to_window = |p: Pt| {
        let (x, y) = vp.canvas_to_screen(p);
        point(bounds.origin.x + px(x), bounds.origin.y + px(y))
    };
    let greek = should_greek(max_font_size(&label.text), zoom);
    let color = text_color(shape, text.canvas);
    // The placement of a layout `size` canvas units big, in pixels.
    let place = |size: (f32, f32), clip: Option<(f32, f32)>, mask: Option<Bounds<Pixels>>| {
        let frame = text_frame(shape, size);
        Placement {
            centre: to_window(frame.centre),
            offset: (frame.offset.0 * zoom, frame.offset.1 * zoom),
            angle: frame.angle,
            clip,
            mask,
        }
    };
    if shape.is_line_like_label() {
        let (w, h) = text
            .layouts
            .size(window, shape.id, &label.text, None, color);
        let placement = place((w, h), None, None);
        let centre = placement.centre;
        let reach = ((w * w + h * h).sqrt() / 2. + LINE_LABEL_PADDING) * zoom;
        let r = Bounds::new(
            point(centre.x - px(reach), centre.y - px(reach)),
            size(px(2. * reach), px(2. * reach)),
        );
        if !r.intersects(&bounds) {
            return;
        }
        let pad = LINE_LABEL_PADDING;
        let patch = [
            (-w / 2. - pad, -h / 2. - pad),
            (w / 2. + pad, -h / 2. - pad),
            (w / 2. + pad, h / 2. + pad),
            (-w / 2. - pad, h / 2. + pad),
        ]
        .map(|(x, y)| rotated(centre, (x * zoom, y * zoom), placement.angle));
        let background = rgb_hsla(text.canvas).opacity(shape.style.opacity);
        paint_polygon(window, &patch, background);
        let job = TextJob {
            id: shape.id,
            text: &label.text,
            wrap: None,
            zoom,
            color,
            greek,
        };
        draw(window, cx, text, &job, &placement);
        return;
    }
    if !outline_visible {
        text.layouts.keep(shape.id);
        return;
    }
    let wrap = text_wrap(shape);
    if shape.is_text_box() {
        // A text box's frame fits its text: the size is not needed.
        let placement = place((0., 0.), None, None);
        let job = TextJob {
            id: shape.id,
            text: &label.text,
            wrap,
            zoom,
            color,
            greek,
        };
        draw(window, cx, text, &job, &placement);
    } else {
        // Greeked too: the bars come from the zoom-1 layout, which a file
        // opened (or a label edited) while zoomed out has not had yet.
        let (_, th) = text
            .layouts
            .size(window, shape.id, &label.text, wrap, color);
        let area = label_area(shape);
        let ((_, y), _) = label_origin(area.w, area.h, LABEL_PADDING, th, label.valign);
        let y = y + area.y;
        let top_left = to_window((f.x, f.y));
        // The frame, from the text's top-left corner.
        let clip = Some((-y * zoom, (f.h - y) * zoom));
        let mask = Some(Bounds::new(top_left, size(px(f.w * zoom), px(f.h * zoom))));
        let placement = place((0., th), clip, mask);
        let job = TextJob {
            id: shape.id,
            text: &label.text,
            wrap,
            zoom,
            color,
            greek,
        };
        draw(window, cx, text, &job, &placement);
    }
}

/// Where an object's text sits on the canvas: its top-left corner
/// `offset` canvas units from `centre`, turned by `angle` degrees about
/// `centre`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFrame {
    pub centre: Pt,
    pub offset: Pt,
    pub angle: f32,
}

impl TextFrame {
    /// The canvas point of `p`, in canvas units from the text's top-left
    /// corner.
    pub fn to_canvas(self, (x, y): Pt) -> Pt {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let (x, y) = (x + self.offset.0, y + self.offset.1);
        (
            self.centre.0 + x * cos - y * sin,
            self.centre.1 + x * sin + y * cos,
        )
    }

    /// Canvas point `p` in canvas units from the text's top-left corner.
    pub fn to_local(self, (x, y): Pt) -> Pt {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let (dx, dy) = (x - self.centre.0, y - self.centre.1);
        (
            dx * cos + dy * sin - self.offset.0,
            -dx * sin + dy * cos - self.offset.1,
        )
    }
}

/// The width `shape`'s text wraps at, in canvas units: none for line
/// labels and growing text boxes.
pub fn text_wrap(shape: &Shape) -> Option<f32> {
    if shape.is_line_like_label() {
        None
    } else if shape.is_text_box() {
        text_box_wrap(shape)
    } else {
        let area = label_area(shape);
        Some(label_origin(area.w, area.h, LABEL_PADDING, 0., VAlign::Top).1)
    }
}

/// The part of `shape`'s box (local coordinates) its label goes in: all
/// of it, but for a topic the part beside its icon and badge and below
/// its image.
pub fn label_area(shape: &Shape) -> Rect {
    let f = &shape.frame;
    match &shape.topic {
        Some(t) => t.parts(f.w, f.h).text,
        None => Rect {
            x: 0.,
            y: 0.,
            w: f.w,
            h: f.h,
        },
    }
}

/// Where `shape`'s text, laid out `size` canvas units big, sits: a line
/// label centred on its line and along it, a text box's text inside its
/// padding, a shape label placed by its vertical alignment; all turned
/// with the shape.
pub fn text_frame(shape: &Shape, (w, h): (f32, f32)) -> TextFrame {
    let f = &shape.frame;
    if shape.is_line_like_label() {
        let (centre, angle) = line_label_placement(f);
        // On the middle of the route, which a bend moves.
        let centre = shape
            .line
            .as_ref()
            .map_or(centre, |l| mindweave_core::line_midpoint(f, l));
        let route = shape.line.as_ref().map(|l| l.route).unwrap_or_default();
        let bent = shape.line.as_ref().is_some_and(|l| l.bend.is_some());
        let angle = if bent {
            0.
        } else {
            mindweave_core::line::line_label_angle(route, angle)
        };
        return TextFrame {
            centre,
            offset: (-w / 2., -h / 2.),
            angle,
        };
    }
    let centre = (f.x + f.w / 2., f.y + f.h / 2.);
    let (x, y) = if shape.is_text_box() {
        (TEXT_BOX_PADDING, TEXT_BOX_PADDING)
    } else {
        let valign = shape.label.as_ref().map_or(VAlign::Middle, |l| l.valign);
        let area = label_area(shape);
        let (x, y) = label_origin(area.w, area.h, LABEL_PADDING, h, valign).0;
        (x + area.x, y + area.y)
    };
    TextFrame {
        centre,
        offset: (x - f.w / 2., y - f.h / 2.),
        angle: f.rotation,
    }
}

/// The colour of `shape`'s Auto text, with its opacity: contrasting with
/// the canvas `canvas` for line labels, else with its fill.
pub fn text_color(shape: &Shape, canvas: (u8, u8, u8)) -> Hsla {
    let background = if shape.is_line_like_label() {
        canvas
    } else {
        text_background(shape.style.fill.as_deref(), canvas)
    };
    auto_color(background, shape.style.opacity)
}

/// The colour topic `shape`'s equation is drawn in, with its opacity: its
/// label's colour when all its text shares one explicit colour, else its
/// Auto text colour. The fit pass and painting both use it, so they share
/// one render.
pub fn equation_color(shape: &Shape, canvas: (u8, u8, u8)) -> Hsla {
    let auto = text_color(shape, canvas);
    let explicit = shape
        .label
        .as_ref()
        .and_then(|l| match l.text.summary(0..l.text.len()).color {
            Mixed::Same(Some(hex)) => hex_to_hsla(&hex),
            _ => None,
        });
    explicit.map_or(auto, |c| c.opacity(auto.a))
}

/// One object's text to paint.
struct TextJob<'a> {
    id: Uuid,
    text: &'a RichText,
    wrap: Option<f32>,
    zoom: f32,
    /// Colour of Auto text, with the object's opacity.
    color: Hsla,
    /// Too small on screen for glyphs: bars stand in for the lines.
    greek: bool,
}

/// Paints `job` at `at`: its glyphs, or for greeked text a bar per line
/// from its layout at zoom 1 (laid out when not cached).
fn draw(window: &mut Window, cx: &mut App, text: &mut TextPaint, job: &TextJob, at: &Placement) {
    if job.greek {
        text.layouts
            .size(window, job.id, job.text, job.wrap, job.color);
        if let Some(layout) = text.layouts.cached(job.id, job.text, job.wrap) {
            let bars = layout.greek_bars();
            paint_greek(
                window,
                &bars,
                job.zoom,
                job.color.opacity(GREEK_OPACITY),
                at,
            );
        }
        return;
    }
    let entry = text
        .layouts
        .entry(window, job.id, job.text, job.wrap, job.zoom, job.color);
    let markers = entry.layout.list_markers();
    paint_layout(window, cx, entry, text.glyphs, at);
    paint_markers(window, cx, job, &markers, at);
}

/// Paints `text`, laid out for object `id` wrapped at `wrap`, upright at
/// zoom 1 with its top-left corner at `origin` (window coordinates), Auto
/// text in `color`: a note in its card.
#[allow(clippy::too_many_arguments)]
pub fn paint_upright_text(
    window: &mut Window,
    cx: &mut App,
    layouts: &mut TextLayoutCache,
    glyphs: &mut GlyphOutlines,
    id: Uuid,
    text: &RichText,
    wrap: Option<f32>,
    color: Hsla,
    origin: Point<Pixels>,
) {
    let job = TextJob {
        id,
        text,
        wrap,
        zoom: 1.,
        color,
        greek: false,
    };
    let at = Placement {
        centre: origin,
        offset: (0., 0.),
        angle: 0.,
        clip: None,
        mask: None,
    };
    let entry = layouts.entry(window, id, text, wrap, 1., color);
    let markers = entry.layout.list_markers();
    paint_layout(window, cx, entry, glyphs, &at);
    paint_markers(window, cx, &job, &markers, &at);
}

/// Paints `job`'s list `markers` (see `TextLayout::list_markers`) placed
/// by `at`, each ending [`MARKER_GAP`] before its item's text, in the
/// item's size, weight and colour. A turned text's markers sit at their
/// turned places, upright.
fn paint_markers(
    window: &mut Window,
    cx: &mut App,
    job: &TextJob,
    markers: &[(f32, f32, String)],
    at: &Placement,
) {
    if markers.is_empty() {
        return;
    }
    let family = canvas_default_family(
        &window.text_style().font().family,
        &installed_font_families(cx),
    );
    // One marker per list paragraph, in order.
    let items = job.text.paragraphs.iter().filter(|p| p.list.is_some());
    let zoom = job.zoom;
    let paint = |window: &mut Window, cx: &mut App| {
        for ((x, baseline, marker), p) in markers.iter().zip(items) {
            let (x, baseline) = (x * zoom, baseline * zoom);
            if at
                .clip
                .is_some_and(|(top, bottom)| baseline < top || baseline > bottom)
            {
                continue;
            }
            let style = RunStyle {
                size: p.style.size,
                bold: p.style.bold,
                color: p.style.color.clone(),
                ..RunStyle::default()
            };
            let run = text_run(&style, marker.len(), family.clone(), job.color, zoom);
            let shaped = window.text_system().shape_line(
                SharedString::from(marker.clone()),
                px(style.size * zoom),
                &[run],
                None,
            );
            let left = x - MARKER_GAP * zoom - f32::from(shaped.width());
            let origin = rotated(
                at.centre,
                (left + at.offset.0, baseline + at.offset.1),
                at.angle,
            );
            let top_left = point(origin.x, origin.y - shaped.ascent);
            let height = shaped.ascent + shaped.descent;
            shaped
                .paint(top_left, height, TextAlign::Left, None, window, cx)
                .ok();
        }
    };
    match at.mask {
        Some(bounds) => window.with_content_mask(Some(ContentMask { bounds }), |w| paint(w, cx)),
        None => paint(window, cx),
    }
}

/// Paints `bars` (canvas units from the text's top-left corner) placed by
/// `at`: quads when upright, polygons when turned.
fn paint_greek(window: &mut Window, bars: &[Rect], zoom: f32, color: Hsla, at: &Placement) {
    let paint = |window: &mut Window| {
        for bar in bars {
            let (x, y, w, h) = (bar.x * zoom, bar.y * zoom, bar.w * zoom, bar.h * zoom);
            if let Some((top, bottom)) = at.clip {
                if y + h <= top || y >= bottom {
                    continue;
                }
            }
            let (x, y) = (x + at.offset.0, y + at.offset.1);
            if at.angle.rem_euclid(360.) == 0. {
                let origin = point(at.centre.x + px(x), at.centre.y + px(y));
                window.paint_quad(fill(Bounds::new(origin, size(px(w), px(h))), color));
            } else {
                let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
                    .map(|p| rotated(at.centre, p, at.angle));
                paint_polygon(window, &corners, color);
            }
        }
    };
    match at.mask {
        Some(bounds) => window.with_content_mask(Some(ContentMask { bounds }), paint),
        None => paint(window),
    }
}

/// The width a text box's text wraps at: none while it grows with its
/// text, else its width inside the padding.
pub fn text_box_wrap(shape: &Shape) -> Option<f32> {
    (!shape.auto_width()).then(|| (shape.frame.w - 2. * TEXT_BOX_PADDING).max(1.))
}

/// The colour of Auto text on `background`, at `opacity`.
pub fn auto_color(background: (u8, u8, u8), opacity: f32) -> Hsla {
    rgb_hsla(auto_text_color(background)).opacity(opacity)
}

fn rgb_hsla((r, g, b): (u8, u8, u8)) -> Hsla {
    rgb(u32::from_be_bytes([0, r, g, b])).into()
}

/// `p` (pixels from `centre`) turned by `angle` degrees about `centre`.
fn rotated(centre: Point<Pixels>, (x, y): (f32, f32), angle: f32) -> Point<Pixels> {
    let (sin, cos) = angle.to_radians().sin_cos();
    point(
        centre.x + px(x * cos - y * sin),
        centre.y + px(x * sin + y * cos),
    )
}

/// Fills the polygon `corners` (window coordinates) with `color`.
pub(super) fn paint_polygon(window: &mut Window, corners: &[Point<Pixels>], color: Hsla) {
    let mut builder = PathBuilder::fill();
    builder.add_polygon(corners, true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Paints `entry`'s layout at `at`: upright with gpui's glyph sprites, or
/// rotated from glyph outlines.
fn paint_layout(
    window: &mut Window,
    cx: &mut App,
    entry: &mut CacheEntry,
    glyphs: &mut GlyphOutlines,
    at: &Placement,
) {
    if at.angle.rem_euclid(360.) == 0. {
        let origin = point(at.centre.x + px(at.offset.0), at.centre.y + px(at.offset.1));
        let paint = |window: &mut Window, cx: &mut App| {
            for line in entry.layout.lines() {
                if let Some((top, bottom)) = at.clip {
                    if line.y + line.height <= top || line.y >= bottom {
                        continue;
                    }
                }
                let baseline = origin.y + px(line.y + line.baseline);
                for piece in &line.pieces {
                    let shaped = &piece.shaped;
                    let piece_origin =
                        point(origin.x + px(line.x + piece.x), baseline - shaped.ascent);
                    shaped
                        .paint(
                            piece_origin,
                            shaped.ascent + shaped.descent,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                        .ok();
                }
            }
        };
        match at.mask {
            Some(bounds) => {
                window.with_content_mask(Some(ContentMask { bounds }), |w| paint(w, cx))
            }
            None => paint(window, cx),
        }
        return;
    }
    let key = [
        at.angle.to_bits(),
        at.offset.0.to_bits(),
        at.offset.1.to_bits(),
        at.clip.map_or(0, |c| c.0.to_bits()),
        at.clip.map_or(0, |c| c.1.to_bits()),
    ];
    if entry.rotated.as_ref().is_none_or(|r| r.key != key) {
        let rotated = build_rotated(window, entry, glyphs, at, key);
        entry.rotated = Some(rotated);
    }
    let Some(rotated) = &entry.rotated else {
        return;
    };
    let offset = at.centre;
    for (path, color) in &rotated.paths {
        let mut path = path.clone();
        for v in &mut path.vertices {
            v.xy_position += offset;
        }
        path.bounds.origin += offset;
        window.paint_path(path, *color);
    }
    for g in &rotated.upright {
        let origin = offset + g.origin;
        if g.emoji {
            window
                .paint_emoji(origin, g.font_id, g.glyph_id, g.font_size)
                .ok();
        } else {
            window
                .paint_glyph(origin, g.font_id, g.glyph_id, g.font_size, g.color)
                .ok();
        }
    }
}

/// Glyph outlines and decorations of `entry`'s layout placed by `at`,
/// relative to its rotation centre.
fn build_rotated(
    window: &mut Window,
    entry: &CacheEntry,
    glyphs: &mut GlyphOutlines,
    at: &Placement,
    key: [u32; 5],
) -> RotatedPaths {
    let text_system = window.text_system().clone();
    let (sin, cos) = at.angle.to_radians().sin_cos();
    let turn = |(x, y): (f32, f32)| {
        let (x, y) = (x + at.offset.0, y + at.offset.1);
        (x * cos - y * sin, x * sin + y * cos)
    };
    let to_point = |(x, y): (f32, f32)| point(px(x), px(y));
    // One fill per colour, so a line of text is a single path.
    let mut fills: Vec<(Hsla, PathBuilder)> = Vec::new();
    let fill_for = |color: Hsla, fills: &mut Vec<(Hsla, PathBuilder)>| -> usize {
        if let Some(i) = fills.iter().position(|(c, _)| *c == color) {
            return i;
        }
        let style = PathStyle::Fill(FillOptions::non_zero());
        fills.push((color, PathBuilder::fill().with_style(style)));
        fills.len() - 1
    };
    // Underlines and strikethroughs get paths of their own: under the
    // non-zero rule, their winding could cancel a glyph's and punch holes.
    let mut bars: Vec<(Hsla, PathBuilder)> = Vec::new();
    let mut upright = Vec::new();
    for line in entry.layout.lines() {
        if let Some((top, bottom)) = at.clip {
            if line.y + line.height <= top || line.y >= bottom {
                continue;
            }
        }
        let baseline = line.y + line.baseline;
        for piece in &line.pieces {
            let shaped = &piece.shaped;
            let fonts: Vec<(FontId, &Font)> = piece
                .runs
                .iter()
                .map(|r| (text_system.resolve_font(&r.font), &r.font))
                .collect();
            let size = f32::from(shaped.font_size);
            let left = line.x + piece.x;
            for run in shaped.runs.iter() {
                let font = fonts.iter().find(|(id, _)| *id == run.font_id).map(|f| f.1);
                for glyph in &run.glyphs {
                    let color =
                        run_at(&piece.runs, glyph.index).map_or(gpui_kit::black(), |r| r.color);
                    let gx = left + f32::from(glyph.position.x);
                    let outline = match font {
                        Some(font) if !glyph.is_emoji => {
                            glyphs.outline(run.font_id, font, glyph.id.0)
                        }
                        _ => None,
                    };
                    let Some(outline) = outline else {
                        upright.push(UprightGlyph {
                            origin: to_point(turn((gx, baseline))),
                            font_id: run.font_id,
                            glyph_id: glyph.id,
                            font_size: shaped.font_size,
                            color,
                            emoji: glyph.is_emoji,
                        });
                        continue;
                    };
                    let i = fill_for(color, &mut fills);
                    let builder = &mut fills[i].1;
                    // Outlines are in ems with y up.
                    let place = |(x, y): Pt| to_point(turn((gx + x * size, baseline - y * size)));
                    for cmd in outline.iter() {
                        match *cmd {
                            GlyphCmd::Move(p) => builder.move_to(place(p)),
                            GlyphCmd::Line(p) => builder.line_to(place(p)),
                            GlyphCmd::Quad(c, p) => builder.curve_to(place(p), place(c)),
                            GlyphCmd::Cubic(c1, c2, p) => {
                                builder.cubic_bezier_to(place(p), place(c1), place(c2))
                            }
                            GlyphCmd::Close => builder.close(),
                        }
                    }
                }
            }
            // Underlines and strikethroughs, as thin rotated boxes.
            let mut start = piece.start;
            for run in &piece.runs {
                let end = start + run.len;
                let x0 = line.x + piece.x_of(start);
                let x1 = line.x + piece.x_of(end);
                let mut bar = |y: f32, thickness: Pixels| {
                    let t = f32::from(thickness);
                    let corners =
                        [(x0, y), (x1, y), (x1, y + t), (x0, y + t)].map(|p| to_point(turn(p)));
                    let i = fill_for(run.color, &mut bars);
                    bars[i].1.add_polygon(&corners, true);
                };
                if let Some(u) = &run.underline {
                    let y = baseline + f32::from(shaped.descent) * UNDERLINE_DESCENT;
                    bar(y, u.thickness);
                }
                if let Some(s) = &run.strikethrough {
                    bar(baseline - size * STRIKE_RISE, s.thickness);
                }
                start = end;
            }
        }
    }
    let paths = fills
        .into_iter()
        .chain(bars)
        .filter_map(|(color, builder)| {
            let path = builder.build().ok()?;
            (!path.vertices.is_empty()).then_some((path, color))
        })
        .collect();
    RotatedPaths {
        key,
        paths,
        upright,
    }
}

/// The run covering byte `index` of runs laid end to end.
fn run_at(runs: &[TextRun], index: usize) -> Option<&TextRun> {
    let mut start = 0;
    runs.iter().find(|r| {
        let end = start + r.len;
        let hit = index < end;
        start = end;
        hit
    })
}

/// One step of a glyph outline, in ems with y up.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GlyphCmd {
    Move(Pt),
    Line(Pt),
    Quad(Pt, Pt),
    Cubic(Pt, Pt, Pt),
    Close,
}

/// Glyph outlines by font and glyph, read once from the font files.
#[derive(Default)]
pub struct GlyphOutlines {
    #[cfg(target_os = "macos")]
    faces: HashMap<FontId, Option<font_kit::font::Font>>,
    outlines: HashMap<(FontId, u32), Option<Rc<[GlyphCmd]>>>,
}

impl GlyphOutlines {
    /// The outline of glyph `glyph` of `font` (which gpui resolved to
    /// `id`), or `None` when it cannot be read.
    fn outline(&mut self, id: FontId, font: &Font, glyph: u32) -> Option<Rc<[GlyphCmd]>> {
        if let Some(outline) = self.outlines.get(&(id, glyph)) {
            return outline.clone();
        }
        let outline = self.read_outline(id, font, glyph);
        self.outlines.insert((id, glyph), outline.clone());
        outline
    }

    #[cfg(target_os = "macos")]
    fn read_outline(&mut self, id: FontId, font: &Font, glyph: u32) -> Option<Rc<[GlyphCmd]>> {
        use font_kit::hinting::HintingOptions;
        use font_kit::outline::{OutlineBuilder, PointFlags};
        let face = self
            .faces
            .entry(id)
            .or_insert_with(|| load_face(font))
            .as_ref()?;
        let mut builder = OutlineBuilder::new();
        face.outline(glyph, HintingOptions::None, &mut builder)
            .ok()?;
        let em = face.metrics().units_per_em as f32;
        let mut cmds = Vec::new();
        for contour in builder.into_outline().contours {
            let p = |i: usize| {
                let v = contour.positions[i];
                (v.x() / em, v.y() / em)
            };
            let flags = &contour.flags;
            let Some(_) = flags.first() else {
                continue;
            };
            cmds.push(GlyphCmd::Move(p(0)));
            let mut i = 1;
            while i < flags.len() {
                if flags[i].contains(PointFlags::CONTROL_POINT_0) {
                    if i + 2 < flags.len() && flags[i + 1].contains(PointFlags::CONTROL_POINT_1) {
                        cmds.push(GlyphCmd::Cubic(p(i), p(i + 1), p(i + 2)));
                        i += 3;
                    } else if i + 1 < flags.len() {
                        cmds.push(GlyphCmd::Quad(p(i), p(i + 1)));
                        i += 2;
                    } else {
                        break;
                    }
                } else {
                    cmds.push(GlyphCmd::Line(p(i)));
                    i += 1;
                }
            }
            cmds.push(GlyphCmd::Close);
        }
        Some(cmds.into())
    }

    #[cfg(not(target_os = "macos"))]
    fn read_outline(&mut self, _: FontId, _: &Font, _: u32) -> Option<Rc<[GlyphCmd]>> {
        None
    }
}

/// The face of `font` that gpui's macOS text system picks: the family
/// (the system UI font for gpui's alias) loaded through font-kit, faces
/// without an "m" or with a repeated PostScript name skipped, and the best
/// match for the weight and style.
#[cfg(target_os = "macos")]
fn load_face(font: &Font) -> Option<font_kit::font::Font> {
    use font_kit::matching::find_best_match;
    use font_kit::properties::{Properties, Stretch, Style, Weight};
    use font_kit::source::SystemSource;
    let name = font_name_with_fallbacks(&font.family, ".AppleSystemUIFont");
    let family = SystemSource::new().select_family_by_name(name).ok()?;
    let mut seen = std::collections::HashSet::new();
    let faces: Vec<font_kit::font::Font> = family
        .fonts()
        .iter()
        .filter_map(|handle| handle.load().ok())
        .filter(|face| face.glyph_for_char('m').is_some())
        .filter(|face| face.postscript_name().is_some_and(|n| seen.insert(n)))
        .collect();
    let properties: Vec<Properties> = faces.iter().map(|f| f.properties()).collect();
    let query = Properties {
        style: match font.style {
            FontStyle::Normal => Style::Normal,
            FontStyle::Italic => Style::Italic,
            FontStyle::Oblique => Style::Oblique,
        },
        weight: Weight(font.weight.0),
        stretch: Stretch::NORMAL,
    };
    let ix = find_best_match(&properties, &query).ok()?;
    faces.into_iter().nth(ix)
}

/// The canvas background as RGB: the page's `background` colour, else the
/// theme's.
pub fn canvas_background(page_background: Option<&str>, theme_background: Hsla) -> (u8, u8, u8) {
    page_background
        .and_then(parse_hex_color)
        .unwrap_or_else(|| hsla_to_rgb(theme_background))
}

/// `color` as 8-bit RGB; alpha is dropped.
fn hsla_to_rgb(color: Hsla) -> (u8, u8, u8) {
    let c = Rgba::from(color);
    let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    (channel(c.r), channel(c.g), channel(c.b))
}

/// What Auto-coloured text is drawn on: the object's `fill` (a shape's
/// fill for labels, a text box's own fill), else the canvas background.
pub fn text_background(fill: Option<&str>, canvas: (u8, u8, u8)) -> (u8, u8, u8) {
    fill.and_then(parse_hex_color).unwrap_or(canvas)
}

/// Where a label `text_height` tall starts inside a `w` × `h` frame, in the
/// frame's local coordinates: inset by `padding` on the left, and placed
/// vertically by `valign` (top and bottom inset by `padding`, middle
/// centred). Also returns the wrap width, the frame's width minus the
/// padding on both sides (at least 1).
pub fn label_origin(w: f32, h: f32, padding: f32, text_height: f32, valign: VAlign) -> (Pt, f32) {
    let y = match valign {
        VAlign::Top => padding,
        VAlign::Middle => (h - text_height) / 2.,
        VAlign::Bottom => h - padding - text_height,
    };
    ((padding, y), (w - 2. * padding).max(1.))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        canvas_background, equation_color, label_origin, text_background, text_color, text_frame,
        text_wrap,
    };
    use crate::editor::hex_to_hsla;
    use mindweave_core::geometry::Frame;
    use mindweave_core::{
        auto_text_color, builtin_libraries, Label, LibraryRegistry, RichText, Shape, StyleChange,
        VAlign,
    };

    #[test]
    fn equations_take_their_topics_one_text_colour() {
        let canvas = (255, 255, 255);
        let mut topic = Shape::new_topic(Frame {
            w: 120.,
            h: 40.,
            ..Frame::default()
        });
        topic.style.opacity = 0.5;
        let auto = text_color(&topic, canvas);
        // Auto text: the Auto colour.
        assert_eq!(equation_color(&topic, canvas), auto);
        // All the text one colour: that colour, with the topic's opacity.
        let text = &mut topic.label.as_mut().unwrap().text;
        let len = text.len();
        text.apply(0..len, &StyleChange::Color(Some("#FF0000".into())));
        let red = hex_to_hsla("#FF0000").unwrap().opacity(0.5);
        assert_eq!(equation_color(&topic, canvas), red);
        // Two colours: Auto again.
        let text = &mut topic.label.as_mut().unwrap().text;
        text.apply(0..3, &StyleChange::Color(Some("#0000FF".into())));
        assert_eq!(equation_color(&topic, canvas), auto);
    }

    #[test]
    fn auto_colour_backgrounds() {
        let theme = gpui_kit::white();
        assert_eq!(
            canvas_background(Some("#102030"), theme),
            (0x10, 0x20, 0x30)
        );
        assert_eq!(canvas_background(None, theme), (255, 255, 255));
        assert_eq!(canvas_background(Some("bogus"), theme), (255, 255, 255));
        let dark = canvas_background(None, gpui_kit::black());
        assert_eq!(dark, (0, 0, 0));

        // A label on a dark fill is white even on a light canvas.
        let canvas = (255, 255, 255);
        let bg = text_background(Some("#202020"), canvas);
        assert_eq!(bg, (0x20, 0x20, 0x20));
        assert_eq!(auto_text_color(bg), (255, 255, 255));
        // No fill (or an unreadable one): the canvas shows through.
        assert_eq!(text_background(None, (0, 0, 0)), (0, 0, 0));
        assert_eq!(text_background(Some("x"), canvas), canvas);
    }

    #[test]
    fn labels_sit_inside_the_padded_frame() {
        let (origin, wrap) = label_origin(100., 60., 8., 20., VAlign::Top);
        assert_eq!((origin, wrap), ((8., 8.), 84.));
        let (origin, _) = label_origin(100., 60., 8., 20., VAlign::Middle);
        assert_eq!(origin, (8., 20.));
        let (origin, _) = label_origin(100., 60., 8., 20., VAlign::Bottom);
        assert_eq!(origin, (8., 32.));
        // Too narrow for the padding: still a positive wrap width.
        let (_, wrap) = label_origin(10., 60., 8., 20., VAlign::Top);
        assert_eq!(wrap, 1.);
        // Taller than the frame: middle still centres (and overflows
        // evenly, to be clipped).
        let (origin, _) = label_origin(100., 20., 8., 40., VAlign::Middle);
        assert_eq!(origin, (8., -10.));
    }

    #[test]
    fn text_frames_place_labels_and_line_labels() {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let frame = Frame {
            x: 10.,
            y: 20.,
            w: 100.,
            h: 60.,
            ..Frame::default()
        };
        let rect = libs.shape("basic-shapes", "rectangle").unwrap();
        let mut label = Shape::new(rect, "basic-shapes", frame);
        label.label = Some(Label {
            text: RichText::new(Default::default(), Default::default()),
            valign: VAlign::Middle,
        });
        // Inside the padding, centred vertically, from the frame's centre.
        let f = text_frame(&label, (50., 20.));
        assert_eq!(
            (f.centre, f.offset, f.angle),
            ((60., 50.), (-42., -10.), 0.)
        );
        assert_eq!(text_wrap(&label), Some(84.));
        assert_eq!(f.to_canvas((0., 0.)), (18., 40.));

        // A line label is centred on the line's middle, along it.
        let line = libs.shape("basic-shapes", "line").unwrap();
        let line = Shape::new(line, "basic-shapes", frame);
        let f = text_frame(&line, (40., 16.));
        assert_eq!(f.offset, (-20., -8.));
        assert_eq!(f.centre, (60., 50.));
        assert!((f.angle - 30.963_757).abs() < 1e-3);
        assert_eq!(text_wrap(&line), None);
        let p = f.to_canvas((3., 4.));
        let back = f.to_local(p);
        assert!((back.0 - 3.).abs() < 1e-3 && (back.1 - 4.).abs() < 1e-3);
    }
}
