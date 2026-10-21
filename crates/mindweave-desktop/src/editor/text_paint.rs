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

use super::text_layout::{max_font_size, should_greek, CacheEntry, TextLayoutCache};
use gpui_kit::*;
use mindweave_core::geometry::{line_label_placement, Pt, Rect};
use mindweave_core::{auto_text_color, parse_hex_color, RichText, Shape, VAlign, Viewport};
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
    let opacity = shape.style.opacity;
    let centre = to_window((f.x + f.w / 2., f.y + f.h / 2.));
    let greek = should_greek(max_font_size(&label.text), zoom);
    if shape.is_line_like_label() {
        let color = auto_color(text.canvas, opacity);
        let (w, h) = text
            .layouts
            .size(window, shape.id, &label.text, None, color);
        let (mid, angle) = line_label_placement(f);
        let centre = to_window(mid);
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
        .map(|(x, y)| rotated(centre, (x * zoom, y * zoom), angle));
        let background = rgb_hsla(text.canvas).opacity(opacity);
        paint_polygon(window, &patch, background);
        let placement = Placement {
            centre,
            offset: (-w / 2. * zoom, -h / 2. * zoom),
            angle,
            clip: None,
            mask: None,
        };
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
    let background = text_background(shape.style.fill.as_deref(), text.canvas);
    let color = auto_color(background, opacity);
    if shape.is_text_box() {
        let pad = TEXT_BOX_PADDING;
        let placement = Placement {
            centre,
            offset: ((pad - f.w / 2.) * zoom, (pad - f.h / 2.) * zoom),
            angle: f.rotation,
            clip: None,
            mask: None,
        };
        let job = TextJob {
            id: shape.id,
            text: &label.text,
            wrap: text_box_wrap(shape),
            zoom,
            color,
            greek,
        };
        draw(window, cx, text, &job, &placement);
    } else {
        let (_, wrap) = label_origin(f.w, f.h, LABEL_PADDING, 0., label.valign);
        let th = if greek {
            // No layout just for bars: only one already cached.
            match text.layouts.cached(shape.id, &label.text, Some(wrap)) {
                Some(layout) => layout.size().1,
                None => return,
            }
        } else {
            let sized = text
                .layouts
                .size(window, shape.id, &label.text, Some(wrap), color);
            sized.1
        };
        let ((x, y), _) = label_origin(f.w, f.h, LABEL_PADDING, th, label.valign);
        let top_left = to_window((f.x, f.y));
        let placement = Placement {
            centre,
            offset: ((x - f.w / 2.) * zoom, (y - f.h / 2.) * zoom),
            angle: f.rotation,
            // The frame, from the text's top-left corner.
            clip: Some((-y * zoom, (f.h - y) * zoom)),
            mask: Some(Bounds::new(top_left, size(px(f.w * zoom), px(f.h * zoom)))),
        };
        let job = TextJob {
            id: shape.id,
            text: &label.text,
            wrap: Some(wrap),
            zoom,
            color,
            greek,
        };
        draw(window, cx, text, &job, &placement);
    }
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
/// from the layout already cached (nothing when there is none).
fn draw(window: &mut Window, cx: &mut App, text: &mut TextPaint, job: &TextJob, at: &Placement) {
    if job.greek {
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
    paint_layout(window, cx, entry, text.glyphs, at);
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
fn paint_polygon(window: &mut Window, corners: &[Point<Pixels>], color: Hsla) {
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
    use super::{canvas_background, label_origin, text_background};
    use mindweave_core::{auto_text_color, VAlign};

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
}
