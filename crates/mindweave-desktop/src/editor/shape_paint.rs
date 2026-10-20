//! Paints a page's shapes on the canvas: library outlines filled and
//! stroked with each shape's style, and a dashed placeholder for shapes
//! whose library or shape is not installed.

use std::collections::{BTreeMap, HashMap};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mindweave_core::geometry::{bounds, to_canvas, FillRole, Frame, Geometry, PathCmd, Pt, Rect};
use mindweave_core::{frame_aabb, LibraryRegistry, Page, Shape, Style, Viewport};
use uuid::Uuid;

use super::hex_to_hsla;

/// Outline colour of a placeholder.
const PLACEHOLDER_COLOR: &str = "#7F8EA3";
/// Dash and gap lengths of a placeholder outline, in pixels.
const PLACEHOLDER_DASH: [f32; 2] = [4., 3.];
const PLACEHOLDER_TEXT_SIZE: f32 = 11.;
/// Thinnest stroke painted, in pixels, so zoomed-out outlines stay visible.
const MIN_STROKE_PX: f32 = 0.5;

/// A shape's outline in canvas coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasGeometry {
    pub geometry: Geometry,
    /// Axis-aligned bounds of `geometry` (control points included).
    pub aabb: Rect,
    /// The library or shape is missing: `geometry` is the frame's box.
    pub placeholder: bool,
}

/// The canvas outline of `shape`, or its frame's box when its library
/// shape is not installed.
pub fn shape_geometry(shape: &Shape, libs: &LibraryRegistry) -> CanvasGeometry {
    let f = &shape.frame;
    let (local, placeholder) = match libs.shape(&shape.library, &shape.shape) {
        Some(def) => (def.local_geometry(f.w, f.h, &|k| shape.param(k)), false),
        None => (box_geometry(f.w, f.h), true),
    };
    let geometry = to_canvas(&local, f);
    let aabb = bounds(&geometry).unwrap_or_else(|| frame_aabb(f));
    CanvasGeometry {
        geometry,
        aabb,
        placeholder,
    }
}

/// The closed, unfilled box `0..w × 0..h`.
fn box_geometry(w: f32, h: f32) -> Geometry {
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
            fill: FillRole::None,
        }],
    }
}

/// What a cached outline was built from.
struct CacheEntry {
    library: String,
    shape: String,
    frame: Frame,
    params: BTreeMap<String, serde_json::Value>,
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

/// Canvas outlines of shapes, by shape id, reused across frames while the
/// shape's frame, parameters and library shape stay the same.
#[derive(Default)]
pub struct GeometryCache {
    entries: HashMap<Uuid, CacheEntry>,
    frame: u64,
}

impl GeometryCache {
    /// Starts a frame; entries not used by [`Self::end_frame`] are dropped.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Drops the entries of shapes not painted since [`Self::begin_frame`].
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        self.entries.retain(|_, e| e.used == frame);
    }

    /// The canvas outline of `shape`, built on a miss or a change.
    #[cfg(test)]
    fn get(&mut self, shape: &Shape, libs: &LibraryRegistry) -> &CanvasGeometry {
        &self.entry(shape, libs).outline
    }

    fn entry(&mut self, shape: &Shape, libs: &LibraryRegistry) -> &mut CacheEntry {
        let frame = self.frame;
        let entry = self
            .entries
            .entry(shape.id)
            .and_modify(|e| {
                if !e.matches(shape) {
                    *e = new_entry(shape, libs, frame);
                }
            })
            .or_insert_with(|| new_entry(shape, libs, frame));
        entry.used = frame;
        entry
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

fn new_entry(shape: &Shape, libs: &LibraryRegistry, used: u64) -> CacheEntry {
    CacheEntry {
        library: shape.library.clone(),
        shape: shape.shape.clone(),
        frame: shape.frame,
        params: shape.params.clone(),
        outline: shape_geometry(shape, libs),
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

/// A style colour with the shape's opacity applied.
fn style_color(hex: Option<&str>, opacity: f32) -> Option<Hsla> {
    hex.and_then(hex_to_hsla).map(|c| c.opacity(opacity))
}

/// Paints the visible layers of `page`, bottom layer first and each
/// layer's shapes back to front, skipping shapes outside `bounds`.
pub fn paint_shapes(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    page: &Page,
    libs: &LibraryRegistry,
    vp: &Viewport,
    cache: &mut GeometryCache,
) {
    let painter = ShapePainter::new(bounds, vp, 1., cx);
    cache.begin_frame();
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        // Layers are stored top first.
        for layer in page.layers.iter().rev().filter(|l| l.visible) {
            for shape in &layer.objects {
                let entry = cache.entry(shape, libs);
                if !painter.visible(&shape.style, &entry.outline) {
                    continue;
                }
                let stale = entry
                    .tessellation
                    .as_ref()
                    .is_none_or(|t| t.zoom != vp.zoom || t.style != shape.style);
                if stale {
                    entry.tessellation = Some(Tessellation {
                        zoom: vp.zoom,
                        style: shape.style.clone(),
                        paths: painter.tessellate(&shape.style, &entry.outline),
                    });
                }
                if let Some(t) = &entry.tessellation {
                    painter.paint_paths(&t.paths, window);
                }
                if entry.outline.placeholder {
                    painter.paint_label(shape, &entry.outline, window, cx);
                }
            }
        }
    });
    cache.end_frame();
}

/// Paints `shape` as a placement preview, at `opacity` times its own.
pub fn paint_preview(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    shape: &Shape,
    libs: &LibraryRegistry,
    vp: &Viewport,
    opacity: f32,
) {
    let outline = shape_geometry(shape, libs);
    let painter = ShapePainter::new(bounds, vp, opacity, cx);
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
}

impl<'a> ShapePainter<'a> {
    fn new(bounds: Bounds<Pixels>, vp: &'a Viewport, opacity: f32, cx: &App) -> Self {
        ShapePainter {
            bounds,
            vp,
            opacity,
            label_color: cx.theme().muted_foreground,
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
            if let Some(color) = style_color(Some(PLACEHOLDER_COLOR), self.opacity) {
                let mut builder = PathBuilder::stroke(px(1.)).dash_array(&PLACEHOLDER_DASH.map(px));
                for sub in &outline.geometry.paths {
                    self.append(&mut builder, &sub.cmds);
                }
                push_built(builder, color, &mut paths);
            }
            return paths;
        }
        let opacity = style.opacity * self.opacity;
        let fill = style_color(style.fill.as_deref(), opacity);
        let stroke_color = style_color(style.stroke.as_deref(), opacity);
        for sub in &outline.geometry.paths {
            let color = match sub.fill {
                FillRole::Shape => fill,
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
            // Arrowheads are already filled with the stroke colour.
            for sub in outline
                .geometry
                .paths
                .iter()
                .filter(|s| s.fill != FillRole::StrokeColor)
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
    use mindweave_core::{builtin_libraries, LibraryRegistry, Shape, Viewport};

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
        assert_eq!(cache.get(&a, &libs).aabb.w, 10.);
        cache.get(&b, &libs);
        cache.end_frame();
        assert_eq!(cache.len(), 2);

        a.frame.w = 30.;
        cache.begin_frame();
        assert_eq!(cache.get(&a, &libs).aabb.w, 30.);
        cache.end_frame();
        assert_eq!(cache.len(), 1);

        a.set_param("corner_radius", 4.);
        cache.begin_frame();
        let rebuilt = cache.get(&a, &libs).clone();
        assert_eq!(rebuilt, shape_geometry(&a, &libs));
        cache.end_frame();
    }
}
