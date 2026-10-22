//! Paints a page's shapes on the canvas: library outlines filled and
//! stroked with each shape's style, and a dashed placeholder for shapes
//! whose library or shape is not installed.

use std::collections::{BTreeMap, HashMap};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mindweave_core::geometry::{
    bounds, primitive_geometry, to_canvas, FillRole, Frame, Geometry, PathCmd, Primitive, Pt, Rect,
};
use mindweave_core::{
    double_outline, frame_aabb, handle_positions, is_auto_color, line_geometry, line_jumps,
    map_decor, rotate_handle_position, rounded_polygon, Handle, LibraryRegistry, LineDash,
    LineProps, Page, Shape, Style, Viewport,
};
use uuid::Uuid;

use super::hex_to_hsla;
use super::images::{contrast_plate, hsla_luminance, ImageCache};
use super::text_paint::{paint_shape_text, TextPaint};

/// Outline colour of a placeholder.
const PLACEHOLDER_COLOR: &str = "#7F8EA3";
/// Dash and gap lengths of a placeholder outline, in pixels.
const PLACEHOLDER_DASH: [f32; 2] = [4., 3.];
const PLACEHOLDER_TEXT_SIZE: f32 = 11.;
/// Thinnest stroke painted, in pixels, so zoomed-out outlines stay visible.
const MIN_STROKE_PX: f32 = 0.5;
/// Side of a square resize handle, and diameter of the rotate handle, in
/// pixels.
pub const HANDLE_SIZE_PX: f32 = 6.;
/// Fill opacity of the marquee, times the accent colour.
const MARQUEE_FILL_OPACITY: f32 = 0.08;

/// A shape's outline in canvas coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasGeometry {
    pub geometry: Geometry,
    /// Axis-aligned bounds of `geometry` (control points included).
    pub aabb: Rect,
    /// The library or shape is missing: `geometry` is the frame's box.
    pub placeholder: bool,
    /// Dash and gap lengths of the stroke, in canvas units; `None` when
    /// solid.
    pub dash: Option<[f32; 2]>,
}

/// The canvas outline of `shape`: a text box's filled box, or its library
/// shape, or its frame's box when that is not installed. Lines are drawn
/// without jumps (see [`shape_geometry_with`]).
pub fn shape_geometry(shape: &Shape, libs: &LibraryRegistry) -> CanvasGeometry {
    shape_geometry_with(shape, libs, &[])
}

/// [`shape_geometry`], a line jumping at `jumps` (lengths along it). A
/// double outline is stroked twice; a placeholder ignores the style.
pub fn shape_geometry_with(shape: &Shape, libs: &LibraryRegistry, jumps: &[f32]) -> CanvasGeometry {
    let f = &shape.frame;
    let style = &shape.style;
    let line = shape.line.as_ref().filter(|_| shape.is_line());
    let (local, placeholder) = if shape.is_text_box() || shape.is_image() {
        (box_geometry(f.w, f.h, FillRole::Shape), false)
    } else if let Some(b) = &shape.boundary {
        // Round its members' hull, once laid out.
        let g = if b.outline.len() >= 3 {
            rounded_polygon(&b.outline, 18.)
        } else {
            primitive_geometry(Primitive::Rect, f.w, f.h, &|k| {
                (k == "corner_radius").then_some(12.)
            })
        };
        (g, false)
    } else if let Some(t) = &shape.topic {
        let radius = if t.is_central() { 12. } else { 6. };
        let g = primitive_geometry(Primitive::Rect, f.w, f.h, &|k| {
            (k == "corner_radius").then_some(radius)
        });
        (g, false)
    } else if let Some(props) = line {
        (
            line_geometry(f, props, style.stroke_width, style.double, jumps),
            false,
        )
    } else {
        match libs.shape(&shape.library, &shape.shape) {
            Some(def) => (def.local_geometry(f.w, f.h, &|k| shape.param(k)), false),
            None => (box_geometry(f.w, f.h, FillRole::None), true),
        }
    };
    let local = if style.double && line.is_none() && !placeholder {
        double_outline(&local, style.stroke_width)
    } else {
        local
    };
    let geometry = to_canvas(&local, f);
    let aabb = bounds(&geometry).unwrap_or_else(|| frame_aabb(f));
    CanvasGeometry {
        geometry,
        aabb,
        placeholder,
        dash: (!placeholder)
            .then(|| style.dash.pattern(style.stroke_width))
            .flatten(),
    }
}

/// The closed box `0..w × 0..h`, filled as `fill`.
fn box_geometry(w: f32, h: f32, fill: FillRole) -> Geometry {
    use mindweave_core::geometry::SubPath;
    Geometry {
        paths: vec![SubPath {
            cmds: vec![
                PathCmd::MoveTo((0., 0.)),
                PathCmd::LineTo((w, 0.)),
                PathCmd::LineTo((w, h)),
                PathCmd::LineTo((0., h)),
                PathCmd::Close,
            ],
            fill,
        }],
    }
}

/// What a cached outline was built from.
struct CacheEntry {
    library: String,
    shape: String,
    frame: Frame,
    params: BTreeMap<String, serde_json::Value>,
    /// What else shapes the outline: see [`GeomKey`].
    key: GeomKey,
    outline: CanvasGeometry,
    /// The outline tessellated at the last zoom it was painted at.
    tessellation: Option<Tessellation>,
    /// The frame in which the entry was last used.
    used: u64,
}

/// Painted paths of an outline at one zoom and style, in canvas
/// coordinates times the zoom; panning only moves them.
struct Tessellation {
    zoom: f32,
    style: Style,
    /// What "auto" colours were painted as (they follow the theme).
    auto: AutoColors,
    paths: Vec<(Path<Pixels>, Hsla)>,
}

impl CacheEntry {
    fn matches(&self, s: &Shape) -> bool {
        self.frame == s.frame
            && self.library == s.library
            && self.shape == s.shape
            && self.params == s.params
    }
}

/// Besides the frame and library shape, an outline depends on a line's
/// properties and jumps, the stroke width (heads, double outlines, dash
/// lengths), whether the outline is double and its dash pattern.
#[derive(Clone, Debug, PartialEq)]
struct GeomKey {
    line: Option<LineProps>,
    /// A boundary's outline.
    outline: Option<Vec<Pt>>,
    width: f32,
    double: bool,
    dash: LineDash,
    jumps: Vec<f32>,
}

fn geom_key(s: &Shape, jumps: &[f32]) -> GeomKey {
    GeomKey {
        line: s.line.clone(),
        outline: s.boundary.as_ref().map(|b| b.outline.clone()),
        width: s.style.stroke_width,
        double: s.style.double,
        dash: s.style.dash,
        jumps: jumps.to_vec(),
    }
}

/// Canvas outlines of shapes, by shape id, reused across frames while the
/// shape's frame, parameters and library shape stay the same.
#[derive(Default)]
pub struct GeometryCache {
    entries: HashMap<Uuid, CacheEntry>,
    frame: u64,
    /// Where the page's lines jump, and a fingerprint of the lines it was
    /// worked out from.
    jumps: HashMap<Uuid, Vec<f32>>,
    jumps_of: u64,
}

impl GeometryCache {
    /// Starts a frame; entries not used by [`Self::end_frame`] are dropped.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Works out where `page`'s lines jump, unless its lines are as they
    /// were last time.
    pub fn update_jumps(&mut self, page: &Page) {
        let fingerprint = lines_fingerprint(page);
        if fingerprint != self.jumps_of {
            self.jumps = line_jumps(page);
            self.jumps_of = fingerprint;
        }
    }

    /// Drops the entries of shapes not painted since [`Self::begin_frame`].
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.entries.retain(|_, e| e.used == frame);
    }

    /// The canvas outline of `shape`, built on a miss or a change. Hit
    /// tests use it so a pointer move reuses the painted outlines.
    pub fn outline(&mut self, shape: &Shape, libs: &LibraryRegistry) -> &CanvasGeometry {
        &self.entry(shape, libs).outline
    }

    fn entry(&mut self, shape: &Shape, libs: &LibraryRegistry) -> &mut CacheEntry {
        let frame = self.frame;
        let jumps = self.jumps.get(&shape.id).map_or(&[][..], Vec::as_slice);
        let entry = self
            .entries
            .entry(shape.id)
            .and_modify(|e| {
                if !e.matches(shape) || e.key != geom_key(shape, jumps) {
                    *e = new_entry(shape, libs, jumps, frame);
                }
            })
            .or_insert_with(|| new_entry(shape, libs, jumps, frame));
        entry.used = frame;
        entry
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// A hash of everything about `page`'s lines that decides where they
/// jump: their frames, routes and jump styles, and the layers shown.
fn lines_fingerprint(page: &Page) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for layer in &page.layers {
        layer.visible.hash(&mut h);
        for s in &layer.objects {
            let Some(p) = &s.line else { continue };
            s.id.hash(&mut h);
            let f = &s.frame;
            for v in [f.x, f.y, f.w, f.h, f.rotation, p.jump_size] {
                v.to_bits().hash(&mut h);
            }
            (f.flip_x, f.flip_y, p.vertical).hash(&mut h);
            (p.route as u8, p.jump as u8).hash(&mut h);
        }
    }
    h.finish()
}

fn new_entry(shape: &Shape, libs: &LibraryRegistry, jumps: &[f32], used: u64) -> CacheEntry {
    CacheEntry {
        library: shape.library.clone(),
        shape: shape.shape.clone(),
        frame: shape.frame,
        params: shape.params.clone(),
        key: geom_key(shape, jumps),
        outline: shape_geometry_with(shape, libs, jumps),
        tessellation: None,
        used,
    }
}

/// The painted stroke width for `width` canvas pixels at `zoom`: at least
/// [`MIN_STROKE_PX`]; `None` when the shape has no stroke width.
pub fn screen_stroke_width(width: f32, zoom: f32) -> Option<f32> {
    (width > 0.).then(|| (width * zoom).max(MIN_STROKE_PX))
}

/// `r` (canvas coordinates) on screen, relative to the canvas origin, grown
/// by `pad` pixels on every side.
pub fn screen_rect(r: &Rect, vp: &Viewport, pad: f32) -> Rect {
    let (x, y) = vp.canvas_to_screen((r.x, r.y));
    Rect {
        x: x - pad,
        y: y - pad,
        w: r.w * vp.zoom + 2. * pad,
        h: r.h * vp.zoom + 2. * pad,
    }
}

/// Whether `r` (relative to the canvas origin) overlaps a canvas of size
/// `extent`.
pub fn overlaps_canvas(r: &Rect, extent: (f32, f32)) -> bool {
    r.x <= extent.0 && r.y <= extent.1 && r.x + r.w >= 0. && r.y + r.h >= 0.
}

/// The text shown in a placeholder: the shape's name, else
/// `library/shape`.
pub fn placeholder_label(shape: &Shape) -> String {
    match &shape.name {
        Some(name) if !name.trim().is_empty() => name.clone(),
        _ => format!("{}/{}", shape.library, shape.shape),
    }
}

/// A style colour with the shape's opacity applied; "auto" is `auto`.
fn style_color(hex: Option<&str>, auto: Hsla, opacity: f32) -> Option<Hsla> {
    let color = if is_auto_color(hex) {
        Some(auto)
    } else {
        hex.and_then(hex_to_hsla)
    };
    color.map(|c| c.opacity(opacity))
}

/// The colours "auto" stands for on a page with background
/// `page_background`: fills in the canvas colour, strokes in the theme's
/// text colour (bright on dark themes, dark on light ones).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoColors {
    pub fill: Hsla,
    pub stroke: Hsla,
}

impl AutoColors {
    pub fn new(page_background: Option<&str>, cx: &App) -> Self {
        let theme = cx.theme();
        AutoColors {
            fill: page_background
                .and_then(hex_to_hsla)
                .unwrap_or(theme.background),
            stroke: theme.foreground,
        }
    }
}

/// Paints the visible layers of `page`, bottom layer first and each
/// layer's shapes back to front (each with its text), skipping shapes
/// outside `bounds`.
#[allow(clippy::too_many_arguments)]
pub fn paint_shapes(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    page: &Page,
    libs: &LibraryRegistry,
    vp: &Viewport,
    cache: &mut GeometryCache,
    images: &mut ImageCache,
    text: &mut TextPaint,
) {
    let auto = AutoColors::new(page.canvas.background.as_deref(), cx);
    let painter = ShapePainter::new(bounds, vp, 1., auto, cx);
    cache.begin_frame();
    cache.update_jumps(page);
    images.begin_frame();
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        paint_map_decor(window, page, &painter);
        // Layers are stored top first.
        for layer in page.layers.iter().rev().filter(|l| l.visible) {
            for shape in &layer.objects {
                let entry = cache.entry(shape, libs);
                let visible = painter.visible(&shape.style, &entry.outline);
                if visible {
                    if let Some(data) = shape.image.as_ref().filter(|_| shape.is_image()) {
                        let r = screen_rect(&frame_aabb(&shape.frame), vp, 0.);
                        let shown = r.w.max(r.h) * window.scale_factor();
                        if let Some(image) = images.get(data, shown, cx) {
                            let at = Bounds::new(
                                bounds.origin + point(px(r.x), px(r.y)),
                                size_of(r.w, r.h),
                            );
                            // A picture as dark (or light) as the canvas, with
                            // no fill of its own, sits on a contrasting plate.
                            let plate = shape
                                .style
                                .fill
                                .is_none()
                                .then(|| images.luminance(data, cx))
                                .flatten()
                                .and_then(|l| contrast_plate(l, hsla_luminance(auto.fill)));
                            if let Some(color) = plate {
                                let radius = (r.w.min(r.h) * 0.12).min(8. * vp.zoom);
                                window.paint_quad(
                                    fill(at, color.opacity(shape.style.opacity))
                                        .corner_radii(px(radius)),
                                );
                            }
                            window
                                .paint_image(bounds, at, Corners::default(), image, 0, false)
                                .ok();
                        }
                    }
                    let stale = entry.tessellation.as_ref().is_none_or(|t| {
                        t.zoom != vp.zoom || t.style != shape.style || t.auto != auto
                    });
                    if stale {
                        entry.tessellation = Some(Tessellation {
                            zoom: vp.zoom,
                            style: shape.style.clone(),
                            auto,
                            paths: if shape.is_boundary() {
                                boundary_paths(&painter, &shape.style, &entry.outline)
                            } else {
                                painter.tessellate(&shape.style, &entry.outline)
                            },
                        });
                    }
                    if let Some(t) = &entry.tessellation {
                        painter.paint_paths(&t.paths, window);
                    }
                    if shape.topic.is_some() {
                        paint_topic_extras(window, cx, bounds, vp, shape, images, auto);
                    }
                    if entry.outline.placeholder {
                        painter.paint_label(shape, &entry.outline, window, cx);
                    }
                } else {
                    // Out of sight: its pictures stay decoded for when it
                    // comes back.
                    let topic_picture = shape.topic.as_ref().and_then(|t| t.picture.as_ref());
                    for data in shape.image.iter().chain(topic_picture) {
                        images.keep(data);
                    }
                }
                paint_shape_text(window, cx, bounds, vp, shape, visible, text);
            }
        }
    });
    cache.end_frame();
    images.end_frame(window);
}

/// Paints the branch lines and summary brackets of `page`'s mind maps,
/// beneath every shape, each in the stroke of the topic it leads to.
fn paint_map_decor(window: &mut Window, page: &Page, painter: &ShapePainter) {
    let shapes: HashMap<Uuid, &Shape> = page
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .filter(|s| s.topic.is_some())
        .map(|s| (s.id, s))
        .collect();
    if shapes.is_empty() {
        return;
    }
    let width = |id: Uuid| shapes.get(&id).map_or(2., |s| s.style.stroke_width.max(1.));
    let decor = map_decor(page, width);
    let outline = |geometry: Geometry, dash: Option<[f32; 2]>| {
        let aabb = bounds(&geometry).unwrap_or_default();
        CanvasGeometry {
            geometry,
            aabb,
            placeholder: false,
            dash,
        }
    };
    let line_style = |s: &Shape| Style {
        fill: None,
        stroke: s.style.stroke.clone(),
        stroke_width: s.style.stroke_width.max(1.),
        opacity: s.style.opacity,
        ..Style::default()
    };
    for (id, g) in decor.branches.into_iter().chain(decor.brackets) {
        let Some(s) = shapes.get(&id) else { continue };
        let o = outline(g, None);
        if painter.visible(&line_style(s), &o) {
            painter.paint_paths(&painter.tessellate(&line_style(s), &o), window);
        }
    }
}

/// A boundary's paths: a light tint of its fill colour and its outline
/// (dashed by default) a little lighter than its stroke colour.
fn boundary_paths(
    painter: &ShapePainter,
    style: &Style,
    outline: &CanvasGeometry,
) -> Vec<(Path<Pixels>, Hsla)> {
    let tint = Style {
        stroke: None,
        opacity: 0.1 * style.opacity,
        ..style.clone()
    };
    let edge = Style {
        fill: None,
        opacity: 0.8 * style.opacity,
        ..style.clone()
    };
    let mut paths = painter.tessellate(&tint, outline);
    paths.extend(painter.tessellate(&edge, outline));
    paths
}

/// Paints topic `shape`'s picture and link badge in their places.
fn paint_topic_extras(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    vp: &Viewport,
    shape: &Shape,
    images: &mut ImageCache,
    auto: AutoColors,
) {
    let Some(t) = &shape.topic else { return };
    let f = &shape.frame;
    let parts = t.parts(f.w, f.h);
    let on_screen = |r: &Rect| {
        let r = screen_rect(
            &Rect {
                x: f.x + r.x,
                y: f.y + r.y,
                w: r.w,
                h: r.h,
            },
            vp,
            0.,
        );
        Bounds::new(bounds.origin + point(px(r.x), px(r.y)), size_of(r.w, r.h))
    };
    if let (Some(data), Some(rect)) = (&t.picture, parts.picture) {
        let at = on_screen(&rect);
        let shown = f32::from(at.size.width.max(at.size.height)) * window.scale_factor();
        if let Some(image) = images.get(data, shown, cx) {
            // Fit, keeping the picture's proportions.
            let s = image.size(0);
            let (iw, ih) = (s.width.0.max(1) as f32, s.height.0.max(1) as f32);
            let (bw, bh) = (f32::from(at.size.width), f32::from(at.size.height));
            let k = (bw / iw).min(bh / ih);
            let (w, h) = (iw * k, ih * k);
            let fit = Bounds::new(
                at.origin + point(px((bw - w) / 2.), px((bh - h) / 2.)),
                size_of(w, h),
            );
            window
                .paint_image(bounds, fit, Corners::default(), image, 0, false)
                .ok();
        }
    }
    if let Some(rect) = parts.badge {
        let color =
            style_color(shape.style.stroke.as_deref(), auto.stroke, 1.).unwrap_or(auto.stroke);
        window
            .paint_svg(
                on_screen(&rect),
                crate::libraries::lucide_icon_path("link", cx),
                None,
                TransformationMatrix::unit(),
                color,
                cx,
            )
            .ok();
    }
}

/// Paints `shape` as a placement preview, at `opacity` times its own.
#[allow(clippy::too_many_arguments)]
pub fn paint_preview(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    shape: &Shape,
    libs: &LibraryRegistry,
    vp: &Viewport,
    opacity: f32,
    auto: AutoColors,
) {
    let outline = shape_geometry(shape, libs);
    let painter = ShapePainter::new(bounds, vp, opacity, auto, cx);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let paths = painter.tessellate(&shape.style, &outline);
        painter.paint_paths(&paths, window);
        if outline.placeholder {
            painter.paint_label(shape, &outline, window, cx);
        }
    });
}

/// Paints outlines on the canvas at `bounds` through viewport `vp`.
struct ShapePainter<'a> {
    bounds: Bounds<Pixels>,
    vp: &'a Viewport,
    /// Multiplies each shape's opacity.
    opacity: f32,
    label_color: Hsla,
    auto: AutoColors,
}

impl<'a> ShapePainter<'a> {
    fn new(
        bounds: Bounds<Pixels>,
        vp: &'a Viewport,
        opacity: f32,
        auto: AutoColors,
        cx: &App,
    ) -> Self {
        ShapePainter {
            bounds,
            vp,
            opacity,
            label_color: cx.theme().muted_foreground,
            auto,
        }
    }

    fn extent(&self) -> (f32, f32) {
        (
            f32::from(self.bounds.size.width),
            f32::from(self.bounds.size.height),
        )
    }

    /// Whether the outline, with its stroke, reaches into the canvas.
    fn visible(&self, style: &Style, outline: &CanvasGeometry) -> bool {
        let pad = if outline.placeholder {
            1.
        } else if style.stroke.is_some() {
            // Miter joins can reach past half the stroke width.
            screen_stroke_width(style.stroke_width, self.vp.zoom).map_or(0., |w| w * 2.)
        } else {
            0.
        };
        overlaps_canvas(&screen_rect(&outline.aabb, self.vp, pad), self.extent())
    }

    /// A canvas point scaled by the zoom; [`Self::paint_paths`] moves it
    /// into place.
    fn zoomed(&self, p: Pt) -> Point<Pixels> {
        point(px(p.0 * self.vp.zoom), px(p.1 * self.vp.zoom))
    }

    /// Where the canvas origin is painted, in window coordinates.
    fn offset(&self) -> Point<Pixels> {
        let (x, y) = self.vp.canvas_to_screen((0., 0.));
        point(self.bounds.origin.x + px(x), self.bounds.origin.y + px(y))
    }

    /// Fills then stroke of `outline` with `style`, or the dashed box of a
    /// placeholder, in zoomed canvas coordinates.
    fn tessellate(&self, style: &Style, outline: &CanvasGeometry) -> Vec<(Path<Pixels>, Hsla)> {
        let mut paths = Vec::new();
        if outline.placeholder {
            // Placeholders ignore the shape's style, opacity included, so a
            // missing shape stays visible; only the preview factor applies.
            if let Some(color) =
                style_color(Some(PLACEHOLDER_COLOR), self.auto.stroke, self.opacity)
            {
                let mut builder = PathBuilder::stroke(px(1.)).dash_array(&PLACEHOLDER_DASH.map(px));
                for sub in &outline.geometry.paths {
                    self.append(&mut builder, &sub.cmds);
                }
                push_built(builder, color, &mut paths);
            }
            return paths;
        }
        let opacity = style.opacity * self.opacity;
        let fill = style_color(style.fill.as_deref(), self.auto.fill, opacity);
        let stroke_color = style_color(style.stroke.as_deref(), self.auto.stroke, opacity);
        for sub in &outline.geometry.paths {
            let color = match sub.fill {
                FillRole::Shape | FillRole::FillOnly => fill,
                FillRole::StrokeColor => stroke_color,
                FillRole::None => None,
            };
            if let Some(color) = color {
                let mut builder = PathBuilder::fill();
                self.append(&mut builder, &sub.cmds);
                push_built(builder, color, &mut paths);
            }
        }
        let width = screen_stroke_width(style.stroke_width, self.vp.zoom);
        if let Some((color, width)) = stroke_color.zip(width) {
            let mut builder = PathBuilder::stroke(px(width));
            if let Some(dash) = outline.dash {
                builder = builder.dash_array(&dash.map(|d| px((d * self.vp.zoom).max(1.))));
            }
            // Arrowheads are already filled with the stroke colour.
            for sub in outline
                .geometry
                .paths
                .iter()
                .filter(|s| !matches!(s.fill, FillRole::StrokeColor | FillRole::FillOnly))
            {
                self.append(&mut builder, &sub.cmds);
            }
            push_built(builder, color, &mut paths);
        }
        paths
    }

    /// Paints tessellated `paths`, moved to where the viewport shows them.
    fn paint_paths(&self, paths: &[(Path<Pixels>, Hsla)], window: &mut Window) {
        let offset = self.offset();
        for (path, color) in paths {
            let mut path = path.clone();
            for v in &mut path.vertices {
                v.xy_position += offset;
            }
            path.bounds.origin += offset;
            window.paint_path(path, *color);
        }
    }

    /// A placeholder's name, centred in and clipped to its box.
    fn paint_label(
        &self,
        shape: &Shape,
        outline: &CanvasGeometry,
        window: &mut Window,
        cx: &mut App,
    ) {
        let rect = screen_rect(&outline.aabb, self.vp, 0.);
        let text: SharedString = placeholder_label(shape).into();
        let run = TextRun {
            len: text.len(),
            font: window.text_style().font(),
            color: self.label_color.opacity(self.opacity),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let size = px(PLACEHOLDER_TEXT_SIZE);
        let line = window.text_system().shape_line(text, size, &[run], None);
        let line_height = size * 1.3;
        let origin = self.bounds.origin + point(px(rect.x), px(rect.y));
        let clip = Bounds::new(origin, size_of(rect.w, rect.h));
        let centre = clip.center();
        let text_origin = point(centre.x - line.width / 2., centre.y - line_height / 2.);
        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            line.paint(text_origin, line_height, TextAlign::Left, None, window, cx)
                .ok();
        });
    }

    /// Adds `cmds`, in zoomed canvas coordinates, to `builder`.
    fn append(&self, builder: &mut PathBuilder, cmds: &[PathCmd]) {
        for cmd in cmds {
            match *cmd {
                PathCmd::MoveTo(p) => builder.move_to(self.zoomed(p)),
                PathCmd::LineTo(p) => builder.line_to(self.zoomed(p)),
                PathCmd::QuadTo { ctrl, to } => {
                    builder.curve_to(self.zoomed(to), self.zoomed(ctrl))
                }
                PathCmd::CubicTo { c1, c2, to } => {
                    builder.cubic_bezier_to(self.zoomed(to), self.zoomed(c1), self.zoomed(c2))
                }
                PathCmd::Close => builder.close(),
            }
        }
    }
}

/// Paints the selection chrome on `chrome` (canvas coordinates): a 1px
/// `accent` box, the square `resize_handles` and the rotate handle
/// `rotate_offset` pixels above the top edge (none for `None`), all white
/// with an accent border.
pub fn paint_selection(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    vp: &Viewport,
    chrome: &Frame,
    rotate_offset: Option<f32>,
    resize_handles: &[(Handle, Pt)],
    accent: Hsla,
) {
    let to_window = |p: Pt| {
        let (x, y) = vp.canvas_to_screen(p);
        point(bounds.origin.x + px(x), bounds.origin.y + px(y))
    };
    let handles = handle_positions(chrome);
    let corner = |h: Handle| {
        to_window(
            handles
                .iter()
                .find(|(k, _)| *k == h)
                .map_or((0., 0.), |(_, p)| *p),
        )
    };
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        if chrome.rotation == 0. {
            // Snapped to whole pixels so the 1px box stays crisp.
            let (a, b) = (corner(Handle::NW), corner(Handle::SE));
            let (x0, y0) = (f32::from(a.x).round(), f32::from(a.y).round());
            let (x1, y1) = (f32::from(b.x).round(), f32::from(b.y).round());
            let r = Bounds::new(
                point(px(x0), px(y0)),
                size_of((x1 - x0).max(1.), (y1 - y0).max(1.)),
            );
            window.paint_quad(outline(r, accent, BorderStyle::Solid));
        } else {
            let mut builder = PathBuilder::stroke(px(1.));
            builder.move_to(corner(Handle::NW));
            for h in [Handle::NE, Handle::SE, Handle::SW] {
                builder.line_to(corner(h));
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, accent);
            }
        }
        let rotate = rotate_offset.map(|o| to_window(rotate_handle_position(chrome, o / vp.zoom)));
        let half = HANDLE_SIZE_PX / 2.;
        let knob = |centre: Point<Pixels>, radius: f32| {
            let origin = point(
                px((f32::from(centre.x) - half).round()),
                px((f32::from(centre.y) - half).round()),
            );
            quad(
                Bounds::new(origin, size_of(HANDLE_SIZE_PX, HANDLE_SIZE_PX)),
                px(radius),
                gpui_kit::white(),
                px(1.),
                accent,
                BorderStyle::Solid,
            )
        };
        for &(_, p) in resize_handles {
            window.paint_quad(knob(to_window(p), 0.));
        }
        if let Some(rotate) = rotate {
            window.paint_quad(knob(rotate, half));
        }
    });
}

/// Paints round handles at a selected line's `ends` (canvas coordinates).
pub fn paint_line_ends(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    vp: &Viewport,
    ends: &[Pt],
    accent: Hsla,
) {
    let size = HANDLE_SIZE_PX + 2.;
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for &p in ends {
            let (x, y) = vp.canvas_to_screen(p);
            let origin = point(
                bounds.origin.x + px((x - size / 2.).round()),
                bounds.origin.y + px((y - size / 2.).round()),
            );
            window.paint_quad(quad(
                Bounds::new(origin, size_of(size, size)),
                px(size / 2.),
                gpui_kit::white(),
                px(1.5),
                accent,
                BorderStyle::Solid,
            ));
        }
    });
}

/// Outlines `outline` (canvas coordinates) in `accent`: the shape a line
/// end will attach to.
pub fn paint_highlight(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    vp: &Viewport,
    outline: &Geometry,
    accent: Hsla,
) {
    let (ox, oy) = vp.canvas_to_screen((0., 0.));
    let offset = (
        f32::from(bounds.origin.x) + ox,
        f32::from(bounds.origin.y) + oy,
    );
    let at = |p: Pt| point(px(offset.0 + p.0 * vp.zoom), px(offset.1 + p.1 * vp.zoom));
    let mut builder = PathBuilder::stroke(px(2.));
    for sub in &outline.paths {
        for cmd in &sub.cmds {
            match *cmd {
                PathCmd::MoveTo(p) => builder.move_to(at(p)),
                PathCmd::LineTo(p) => builder.line_to(at(p)),
                PathCmd::QuadTo { ctrl, to } => builder.curve_to(at(to), at(ctrl)),
                PathCmd::CubicTo { c1, c2, to } => builder.cubic_bezier_to(at(to), at(c1), at(c2)),
                PathCmd::Close => builder.close(),
            }
        }
    }
    if let Ok(path) = builder.build() {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_path(path, accent.opacity(0.8));
        });
    }
}

/// Paints the marquee `rect` (canvas coordinates): a 1px `accent` border
/// over an 8% accent fill.
pub fn paint_marquee(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    vp: &Viewport,
    rect: &Rect,
    accent: Hsla,
) {
    let r = screen_rect(rect, vp, 0.);
    let r = Bounds::new(
        point(
            bounds.origin.x + px(r.x.round()),
            bounds.origin.y + px(r.y.round()),
        ),
        size_of(r.w.round().max(1.), r.h.round().max(1.)),
    );
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        window.paint_quad(quad(
            r,
            px(0.),
            accent.opacity(MARQUEE_FILL_OPACITY),
            px(1.),
            accent,
            BorderStyle::Solid,
        ));
    });
}

fn size_of(w: f32, h: f32) -> Size<Pixels> {
    size(px(w), px(h))
}

/// Tessellates `builder` into `paths`; degenerate outlines add nothing.
fn push_built(builder: PathBuilder, color: Hsla, paths: &mut Vec<(Path<Pixels>, Hsla)>) {
    if let Ok(path) = builder.build() {
        if !path.vertices.is_empty() {
            paths.push((path, color));
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        overlaps_canvas, placeholder_label, screen_rect, screen_stroke_width, shape_geometry,
        GeometryCache,
    };
    use mindweave_core::geometry::{Frame, Rect};
    use mindweave_core::{builtin_libraries, LibraryRegistry, LineDash, Shape, Viewport};

    fn registry() -> LibraryRegistry {
        LibraryRegistry::new(builtin_libraries(), None).0
    }

    fn rectangle(libs: &LibraryRegistry, frame: Frame) -> Shape {
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        Shape::new(def, "basic-shapes", frame)
    }

    fn frame(x: f32, y: f32, w: f32, h: f32) -> Frame {
        Frame {
            x,
            y,
            w,
            h,
            ..Frame::default()
        }
    }

    #[test]
    fn auto_colours_follow_the_theme_and_hex_colours_do_not() {
        let auto = gpui_kit::hsla(0.5, 0.5, 0.9, 1.);
        assert_eq!(super::style_color(Some("auto"), auto, 1.), Some(auto));
        assert_eq!(
            super::style_color(Some("auto"), auto, 0.5),
            Some(auto.opacity(0.5))
        );
        assert_eq!(super::style_color(None, auto, 1.), None);
        let red = super::style_color(Some("#FF0000"), auto, 1.).unwrap();
        assert_ne!(red, auto);
    }

    #[test]
    fn stroke_width_scales_with_zoom_but_stays_visible() {
        assert_eq!(screen_stroke_width(2., 1.5), Some(3.));
        assert_eq!(screen_stroke_width(1., 0.1), Some(0.5));
        assert_eq!(screen_stroke_width(0., 2.), None);
    }

    #[test]
    fn screen_rect_maps_through_the_viewport_and_pads() {
        let vp = Viewport {
            x: 10.,
            y: 20.,
            zoom: 2.,
        };
        let r = screen_rect(
            &Rect {
                x: 10.,
                y: 20.,
                w: 5.,
                h: 4.,
            },
            &vp,
            1.,
        );
        assert_eq!(
            r,
            Rect {
                x: -1.,
                y: -1.,
                w: 12.,
                h: 10.
            }
        );
    }

    #[test]
    fn shapes_off_the_canvas_are_culled() {
        let extent = (100., 50.);
        let r = |x, y, w, h| Rect { x, y, w, h };
        assert!(overlaps_canvas(&r(10., 10., 5., 5.), extent));
        assert!(overlaps_canvas(&r(-10., -10., 11., 11.), extent));
        assert!(overlaps_canvas(&r(-10., 10., 200., 5.), extent));
        assert!(!overlaps_canvas(&r(-10., 10., 5., 5.), extent));
        assert!(!overlaps_canvas(&r(101., 10., 5., 5.), extent));
        assert!(!overlaps_canvas(&r(10., 51., 5., 5.), extent));
        assert!(!overlaps_canvas(&r(10., -20., 5., 5.), extent));
    }

    #[test]
    fn missing_library_shapes_become_placeholder_boxes() {
        let libs = registry();
        let mut shape = rectangle(&libs, frame(10., 20., 30., 40.));
        assert!(!shape_geometry(&shape, &libs).placeholder);

        shape.library = "no-such-library".into();
        let outline = shape_geometry(&shape, &libs);
        assert!(outline.placeholder);
        assert_eq!(
            outline.aabb,
            Rect {
                x: 10.,
                y: 20.,
                w: 30.,
                h: 40.
            }
        );
        assert_eq!(placeholder_label(&shape), "no-such-library/rectangle");
        shape.name = Some("Server".into());
        assert_eq!(placeholder_label(&shape), "Server");
        shape.name = Some("  ".into());
        assert_eq!(placeholder_label(&shape), "no-such-library/rectangle");
    }

    #[test]
    fn cache_rebuilds_on_change_and_drops_unpainted_shapes() {
        let libs = registry();
        let mut a = rectangle(&libs, frame(0., 0., 10., 10.));
        let b = rectangle(&libs, frame(50., 0., 10., 10.));
        let mut cache = GeometryCache::default();

        cache.begin_frame();
        assert_eq!(cache.outline(&a, &libs).aabb.w, 10.);
        cache.outline(&b, &libs);
        cache.end_frame();
        assert_eq!(cache.len(), 2);

        a.frame.w = 30.;
        cache.begin_frame();
        assert_eq!(cache.outline(&a, &libs).aabb.w, 30.);
        cache.end_frame();
        assert_eq!(cache.len(), 1);

        a.set_param("corner_radius", 4.);
        cache.begin_frame();
        let rebuilt = cache.outline(&a, &libs).clone();
        assert_eq!(rebuilt, shape_geometry(&a, &libs));
        cache.end_frame();

        // Every outline style change shows at once, the first included.
        for change in [
            |s: &mut Shape| s.style.dash = LineDash::Dotted,
            |s: &mut Shape| s.style.dash = LineDash::Dashed,
            |s: &mut Shape| s.style.double = true,
            |s: &mut Shape| s.style.stroke_width = 3.,
        ] {
            change(&mut a);
            cache.begin_frame();
            assert_eq!(cache.outline(&a, &libs), &shape_geometry(&a, &libs));
            cache.end_frame();
        }
        assert_eq!(cache.outline(&a, &libs).dash, LineDash::Dashed.pattern(3.));
    }
}
