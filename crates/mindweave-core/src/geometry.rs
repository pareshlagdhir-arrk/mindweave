//! Shape geometry: outlines as path commands, primitive shapes, frame
//! transforms, bounds, flattening and hit testing (pure f32, no gpui).
//!
//! Coordinates are screen-like: y grows downward. Rotations are in degrees,
//! clockwise on screen, about the centre of the shape's box.

use serde::{Deserialize, Serialize};

/// A 2D point `(x, y)`.
pub type Pt = (f32, f32);

/// One drawing command; points are absolute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCmd {
    MoveTo(Pt),
    LineTo(Pt),
    QuadTo { ctrl: Pt, to: Pt },
    CubicTo { c1: Pt, c2: Pt, to: Pt },
    Close,
}

/// How a sub-path is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRole {
    /// Filled with the shape's fill colour (closed outlines).
    #[default]
    Shape,
    /// Filled with the stroke colour (arrowheads).
    StrokeColor,
    /// Stroke only (lines).
    None,
}

/// A run of commands painted the same way. It may hold several contours
/// (each starting with `MoveTo`); fills use the even-odd rule.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SubPath {
    pub cmds: Vec<PathCmd>,
    pub fill: FillRole,
}

/// A shape outline: sub-paths drawn in order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Geometry {
    pub paths: Vec<SubPath>,
}

/// Built-in parametric shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Primitive {
    Rect,
    Ellipse,
    Line,
    Arrow,
    Polygon,
    Star,
}

impl Primitive {
    /// The primitive named `name` in a library file, if any.
    pub fn from_name(name: &str) -> Option<Primitive> {
        match name {
            "rect" => Some(Primitive::Rect),
            "ellipse" => Some(Primitive::Ellipse),
            "line" => Some(Primitive::Line),
            "arrow" => Some(Primitive::Arrow),
            "polygon" => Some(Primitive::Polygon),
            "star" => Some(Primitive::Star),
            _ => None,
        }
    }

    /// Parameter names this primitive reads.
    pub fn param_keys(self) -> &'static [&'static str] {
        match self {
            Primitive::Rect => &["corner_radius"],
            Primitive::Arrow => &["head_size"],
            Primitive::Polygon => &["sides"],
            Primitive::Star => &["points", "inner_ratio"],
            Primitive::Ellipse | Primitive::Line => &[],
        }
    }
}

/// Default arrowhead length.
const DEFAULT_HEAD_SIZE: f32 = 12.0;

/// Cubic Bézier handle length for a quarter circle of radius 1.
const KAPPA: f32 = 0.552_284_8;

/// Flattening tolerance used by [`hit_test`].
const HIT_TOLERANCE: f32 = 0.25;

/// Upper bound on segments per flattened curve.
const MAX_CURVE_SEGMENTS: usize = 1_000;

/// Outline of `kind` in the local box `0..w × 0..h`. `param` supplies
/// values for [`Primitive::param_keys`]; missing or non-finite values use
/// the defaults, and out-of-range values are clamped.
pub fn primitive_geometry(
    kind: Primitive,
    w: f32,
    h: f32,
    param: &dyn Fn(&str) -> Option<f32>,
) -> Geometry {
    let param = |key: &str| param(key).filter(|v| v.is_finite());
    match kind {
        Primitive::Rect => {
            let radius = param("corner_radius")
                .unwrap_or(0.0)
                .clamp(0.0, (w.min(h) / 2.0).max(0.0));
            Geometry {
                paths: vec![SubPath {
                    cmds: rounded_rect_cmds(0.0, 0.0, w, h, radius, radius),
                    fill: FillRole::Shape,
                }],
            }
        }
        Primitive::Ellipse => Geometry {
            paths: vec![SubPath {
                cmds: ellipse_cmds((w / 2.0, h / 2.0), w / 2.0, h / 2.0),
                fill: FillRole::Shape,
            }],
        },
        Primitive::Line => Geometry {
            paths: vec![line_path(w, h)],
        },
        Primitive::Arrow => {
            let size = param("head_size").unwrap_or(DEFAULT_HEAD_SIZE).max(0.0);
            let len = (w * w + h * h).sqrt();
            let dir = if len > 0.0 {
                (w / len, h / len)
            } else {
                (1.0, 0.0)
            };
            let normal = (-dir.1, dir.0);
            let base = (w - dir.0 * size, h - dir.1 * size);
            let half = size / 2.0;
            let head = SubPath {
                cmds: vec![
                    PathCmd::MoveTo((w, h)),
                    PathCmd::LineTo((base.0 + normal.0 * half, base.1 + normal.1 * half)),
                    PathCmd::LineTo((base.0 - normal.0 * half, base.1 - normal.1 * half)),
                    PathCmd::Close,
                ],
                fill: FillRole::StrokeColor,
            };
            Geometry {
                paths: vec![line_path(w, h), head],
            }
        }
        Primitive::Polygon => {
            let sides = param("sides").unwrap_or(3.0).round().clamp(3.0, 12.0) as usize;
            let (rx, ry) = (w / 2.0, h / 2.0);
            Geometry {
                paths: vec![radial_polygon(w, h, sides, |_| (rx, ry))],
            }
        }
        Primitive::Star => {
            let points = param("points").unwrap_or(5.0).round().clamp(3.0, 20.0) as usize;
            let ratio = param("inner_ratio").unwrap_or(0.5).clamp(0.1, 0.9);
            let (rx, ry) = (w / 2.0, h / 2.0);
            Geometry {
                paths: vec![radial_polygon(w, h, points * 2, |i| {
                    if i % 2 == 0 {
                        (rx, ry)
                    } else {
                        (rx * ratio, ry * ratio)
                    }
                })],
            }
        }
    }
}

fn line_path(w: f32, h: f32) -> SubPath {
    SubPath {
        cmds: vec![PathCmd::MoveTo((0.0, 0.0)), PathCmd::LineTo((w, h))],
        fill: FillRole::None,
    }
}

/// Closed polygon of `count` vertices around the box centre, the first at
/// top-centre, going clockwise; `radii(i)` gives vertex `i`'s radii.
fn radial_polygon(w: f32, h: f32, count: usize, radii: impl Fn(usize) -> (f32, f32)) -> SubPath {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let mut cmds = Vec::with_capacity(count + 1);
    for i in 0..count {
        let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / count as f32;
        let (rx, ry) = radii(i);
        let p = (cx + rx * angle.cos(), cy + ry * angle.sin());
        cmds.push(if i == 0 {
            PathCmd::MoveTo(p)
        } else {
            PathCmd::LineTo(p)
        });
    }
    cmds.push(PathCmd::Close);
    SubPath {
        cmds,
        fill: FillRole::Shape,
    }
}

/// Closed rectangle at `(x, y)` sized `w × h` with corner radii `rx`, `ry`
/// (clamped to `0..=w/2` and `0..=h/2` independently, as SVG does;
/// corners drawn as cubics).
pub(crate) fn rounded_rect_cmds(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Vec<PathCmd> {
    let rx = rx.clamp(0.0, (w / 2.0).max(0.0));
    let ry = ry.clamp(0.0, (h / 2.0).max(0.0));
    let (r, b) = (x + w, y + h);
    if rx == 0.0 || ry == 0.0 {
        return vec![
            PathCmd::MoveTo((x, y)),
            PathCmd::LineTo((r, y)),
            PathCmd::LineTo((r, b)),
            PathCmd::LineTo((x, b)),
            PathCmd::Close,
        ];
    }
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    vec![
        PathCmd::MoveTo((x + rx, y)),
        PathCmd::LineTo((r - rx, y)),
        PathCmd::CubicTo {
            c1: (r - rx + kx, y),
            c2: (r, y + ry - ky),
            to: (r, y + ry),
        },
        PathCmd::LineTo((r, b - ry)),
        PathCmd::CubicTo {
            c1: (r, b - ry + ky),
            c2: (r - rx + kx, b),
            to: (r - rx, b),
        },
        PathCmd::LineTo((x + rx, b)),
        PathCmd::CubicTo {
            c1: (x + rx - kx, b),
            c2: (x, b - ry + ky),
            to: (x, b - ry),
        },
        PathCmd::LineTo((x, y + ry)),
        PathCmd::CubicTo {
            c1: (x, y + ry - ky),
            c2: (x + rx - kx, y),
            to: (x + rx, y),
        },
        PathCmd::Close,
    ]
}

/// Closed ellipse centred at `c` with radii `rx`, `ry`, as four cubics.
pub(crate) fn ellipse_cmds(c: Pt, rx: f32, ry: f32) -> Vec<PathCmd> {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let (cx, cy) = c;
    vec![
        PathCmd::MoveTo((cx + rx, cy)),
        PathCmd::CubicTo {
            c1: (cx + rx, cy + ky),
            c2: (cx + kx, cy + ry),
            to: (cx, cy + ry),
        },
        PathCmd::CubicTo {
            c1: (cx - kx, cy + ry),
            c2: (cx - rx, cy + ky),
            to: (cx - rx, cy),
        },
        PathCmd::CubicTo {
            c1: (cx - rx, cy - ky),
            c2: (cx - kx, cy - ry),
            to: (cx, cy - ry),
        },
        PathCmd::CubicTo {
            c1: (cx + kx, cy - ry),
            c2: (cx + rx, cy - ky),
            to: (cx + rx, cy),
        },
        PathCmd::Close,
    ]
}

/// `g` with every point mapped through `f`.
pub(crate) fn map_points(g: &Geometry, f: impl Fn(Pt) -> Pt) -> Geometry {
    let map = |c: &PathCmd| match *c {
        PathCmd::MoveTo(p) => PathCmd::MoveTo(f(p)),
        PathCmd::LineTo(p) => PathCmd::LineTo(f(p)),
        PathCmd::QuadTo { ctrl, to } => PathCmd::QuadTo {
            ctrl: f(ctrl),
            to: f(to),
        },
        PathCmd::CubicTo { c1, c2, to } => PathCmd::CubicTo {
            c1: f(c1),
            c2: f(c2),
            to: f(to),
        },
        PathCmd::Close => PathCmd::Close,
    };
    Geometry {
        paths: g
            .paths
            .iter()
            .map(|sp| SubPath {
                cmds: sp.cmds.iter().map(map).collect(),
                fill: sp.fill,
            })
            .collect(),
    }
}

/// Every point (including control points) of `g`.
fn points(g: &Geometry) -> impl Iterator<Item = Pt> + '_ {
    g.paths.iter().flat_map(|sp| sp.cmds.iter()).flat_map(|c| {
        let pts: [Option<Pt>; 3] = match *c {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => [Some(p), None, None],
            PathCmd::QuadTo { ctrl, to } => [Some(ctrl), Some(to), None],
            PathCmd::CubicTo { c1, c2, to } => [Some(c1), Some(c2), Some(to)],
            PathCmd::Close => [None, None, None],
        };
        pts.into_iter().flatten()
    })
}

/// Maps a unit-box outline (`0..1 × 0..1`) onto the box `0..w × 0..h`.
pub fn scale_unit(g: &Geometry, w: f32, h: f32) -> Geometry {
    map_points(g, |(x, y)| (x * w, y * h))
}

/// Where a shape sits on the canvas: its box, rotation (degrees, clockwise
/// about the box centre) and mirroring within the box.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
}

impl Frame {
    /// An unrotated, unflipped frame covering `r`.
    pub fn from_rect(r: &Rect) -> Frame {
        Frame {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
            ..Frame::default()
        }
    }
}

/// Maps a local outline (box `0..w × 0..h` of `f`) to canvas coordinates:
/// flips within the box, moves it to `(f.x, f.y)`, then rotates it by
/// `f.rotation` degrees clockwise about the box centre.
pub fn to_canvas(local: &Geometry, f: &Frame) -> Geometry {
    let (sin, cos) = f.rotation.to_radians().sin_cos();
    let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
    map_points(local, |(lx, ly)| {
        let lx = if f.flip_x { f.w - lx } else { lx };
        let ly = if f.flip_y { f.h - ly } else { ly };
        let (dx, dy) = (f.x + lx - cx, f.y + ly - cy);
        (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
    })
}

/// An axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Whether `p` lies inside or on the edge.
    pub fn contains(&self, p: Pt) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }

    /// Whether `other` lies entirely inside (edges may touch).
    pub fn contains_rect(&self, other: &Rect) -> bool {
        self.contains((other.x, other.y)) && self.contains((other.x + other.w, other.y + other.h))
    }

    /// The smallest rectangle covering both.
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = (self.x + self.w).max(other.x + other.w);
        let b = (self.y + self.h).max(other.y + other.h);
        Rect {
            x,
            y,
            w: r - x,
            h: b - y,
        }
    }

    pub fn center(&self) -> Pt {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Bounding box of all points of `g`, control points included (so curves
/// may be slightly over-estimated); `None` when `g` has no points.
pub fn bounds(g: &Geometry) -> Option<Rect> {
    let mut pts = points(g);
    let first = pts.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for (x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Some(Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// Point on a cubic Bézier at parameter `t`.
fn cubic_point(p0: Pt, c1: Pt, c2: Pt, p3: Pt, t: f32) -> Pt {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0,
        a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1,
    )
}

/// Point on a quadratic Bézier at parameter `t`.
fn quad_point(p0: Pt, c: Pt, p2: Pt, t: f32) -> Pt {
    let u = 1.0 - t;
    let (a, b, d) = (u * u, 2.0 * u * t, t * t);
    (a * p0.0 + b * c.0 + d * p2.0, a * p0.1 + b * c.1 + d * p2.1)
}

fn len(p: Pt) -> f32 {
    (p.0 * p.0 + p.1 * p.1).sqrt()
}

/// Second difference `a - 2b + c` length.
fn second_diff(a: Pt, b: Pt, c: Pt) -> f32 {
    len((a.0 - 2.0 * b.0 + c.0, a.1 - 2.0 * b.1 + c.1))
}

/// Segments needed so a uniform subdivision deviates at most `tol`, given
/// the error bound `k * dd / n²`.
fn segment_count(k: f32, dd: f32, tol: f32) -> usize {
    let n = (k * dd / tol).sqrt().ceil();
    if n.is_finite() {
        (n as usize).clamp(1, MAX_CURVE_SEGMENTS)
    } else {
        MAX_CURVE_SEGMENTS
    }
}

/// Contours (polylines) of one sub-path; a closed contour ends with its
/// start point repeated.
fn flatten_subpath(sp: &SubPath, tol: f32) -> Vec<Vec<Pt>> {
    let mut contours = Vec::new();
    let mut current: Vec<Pt> = Vec::new();
    let mut start: Pt = (0.0, 0.0);
    let mut last: Pt = (0.0, 0.0);
    let finish = |current: &mut Vec<Pt>, contours: &mut Vec<Vec<Pt>>| {
        if !current.is_empty() {
            contours.push(std::mem::take(current));
        }
    };
    for cmd in &sp.cmds {
        if !matches!(cmd, PathCmd::MoveTo(_) | PathCmd::Close) && current.is_empty() {
            // Drawing after Close (or with no MoveTo) continues from `last`.
            start = last;
            current.push(last);
        }
        match *cmd {
            PathCmd::MoveTo(p) => {
                finish(&mut current, &mut contours);
                current.push(p);
                start = p;
                last = p;
            }
            PathCmd::LineTo(p) => {
                current.push(p);
                last = p;
            }
            PathCmd::QuadTo { ctrl, to } => {
                // Error ≤ |B''| / (8n²), |B''| = 2·dd.
                let n = segment_count(0.25, second_diff(last, ctrl, to), tol);
                let from = last;
                current.extend((1..=n).map(|i| quad_point(from, ctrl, to, i as f32 / n as f32)));
                last = to;
            }
            PathCmd::CubicTo { c1, c2, to } => {
                // Error ≤ |B''| / (8n²), |B''| ≤ 6·dd.
                let dd = second_diff(last, c1, c2).max(second_diff(c1, c2, to));
                let n = segment_count(0.75, dd, tol);
                let from = last;
                current.extend((1..=n).map(|i| cubic_point(from, c1, c2, to, i as f32 / n as f32)));
                last = to;
            }
            PathCmd::Close => {
                if !current.is_empty() {
                    if current.last() != Some(&start) {
                        current.push(start);
                    }
                    finish(&mut current, &mut contours);
                }
                last = start;
            }
        }
    }
    finish(&mut current, &mut contours);
    contours
}

/// Flattens curves into polylines deviating at most `tolerance` from the
/// true outline. Each contour comes with its sub-path's fill role; closed
/// contours end with their start point repeated. A non-positive tolerance
/// is treated as a small positive one.
pub fn flatten(g: &Geometry, tolerance: f32) -> Vec<(Vec<Pt>, FillRole)> {
    let tol = sanitize_tolerance(tolerance);
    g.paths
        .iter()
        .flat_map(|sp| {
            flatten_subpath(sp, tol)
                .into_iter()
                .map(move |c| (c, sp.fill))
        })
        .collect()
}

fn sanitize_tolerance(tolerance: f32) -> f32 {
    if tolerance.is_finite() && tolerance > 0.0 {
        tolerance.max(1e-3)
    } else {
        1e-3
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Pt, a: Pt, b: Pt) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    len((p.0 - (a.0 + t * abx), p.1 - (a.1 + t * aby)))
}

/// Whether `p` is inside a filled sub-path (even-odd rule, contours closed
/// implicitly) or within `stroke_tolerance` of any segment of `g`.
pub fn hit_test(g: &Geometry, p: Pt, stroke_tolerance: f32) -> bool {
    g.paths.iter().any(|sp| {
        let contours = flatten_subpath(sp, HIT_TOLERANCE);
        let near_stroke = contours.iter().any(|c| {
            c.windows(2)
                .any(|s| segment_distance(p, s[0], s[1]) <= stroke_tolerance)
                || (c.len() == 1 && segment_distance(p, c[0], c[0]) <= stroke_tolerance)
        });
        near_stroke || (sp.fill != FillRole::None && inside_even_odd(&contours, p))
    })
}

/// Even-odd point-in-polygon test over all `contours`.
fn inside_even_odd(contours: &[Vec<Pt>], p: Pt) -> bool {
    let mut inside = false;
    for c in contours {
        let n = c.len();
        for i in 0..n {
            let (a, b) = (c[i], c[(i + 1) % n]);
            if (a.1 > p.1) != (b.1 > p.1) {
                let x = a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0);
                if p.0 < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_params(_: &str) -> Option<f32> {
        None
    }

    fn vertices(g: &Geometry, index: usize) -> Vec<Pt> {
        g.paths[index]
            .cmds
            .iter()
            .filter_map(|c| match c {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    fn close_to(a: Pt, b: Pt) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn rect_without_radius_is_four_lines() {
        let g = primitive_geometry(Primitive::Rect, 100.0, 50.0, &no_params);
        assert_eq!(g.paths.len(), 1);
        assert_eq!(g.paths[0].fill, FillRole::Shape);
        let cmds = &g.paths[0].cmds;
        assert_eq!(cmds.len(), 5);
        assert!(matches!(cmds[0], PathCmd::MoveTo(_)));
        assert!(cmds[1..4].iter().all(|c| matches!(c, PathCmd::LineTo(_))));
        assert_eq!(cmds[4], PathCmd::Close);
        assert_eq!(
            bounds(&g),
            Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 50.0
            })
        );
    }

    #[test]
    fn rect_radius_is_clamped_to_half_min_side() {
        let param = |k: &str| (k == "corner_radius").then_some(999.0);
        let g = primitive_geometry(Primitive::Rect, 100.0, 50.0, &param);
        assert_eq!(
            bounds(&g),
            Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 50.0
            })
        );
        let cmds = &g.paths[0].cmds;
        let segments = cmds
            .iter()
            .filter(|c| matches!(c, PathCmd::LineTo(_) | PathCmd::CubicTo { .. }))
            .count();
        assert_eq!(segments, 8);
        let cubics = cmds
            .iter()
            .filter(|c| matches!(c, PathCmd::CubicTo { .. }))
            .count();
        assert_eq!(cubics, 4);
        // The top edge starts at the clamped radius (25), not 999.
        assert_eq!(cmds[0], PathCmd::MoveTo((25.0, 0.0)));
    }

    #[test]
    fn ellipse_bounds_match_box() {
        let g = primitive_geometry(Primitive::Ellipse, 80.0, 40.0, &no_params);
        let b = bounds(&g).unwrap();
        assert!(close_to((b.x, b.y), (0.0, 0.0)));
        assert!(close_to((b.w, b.h), (80.0, 40.0)));
        let cubics = g.paths[0]
            .cmds
            .iter()
            .filter(|c| matches!(c, PathCmd::CubicTo { .. }))
            .count();
        assert_eq!(cubics, 4);
    }

    #[test]
    fn line_is_stroke_only() {
        let g = primitive_geometry(Primitive::Line, 30.0, 20.0, &no_params);
        assert_eq!(g.paths.len(), 1);
        assert_eq!(g.paths[0].fill, FillRole::None);
        assert_eq!(
            g.paths[0].cmds,
            vec![PathCmd::MoveTo((0.0, 0.0)), PathCmd::LineTo((30.0, 20.0))]
        );
    }

    #[test]
    fn arrow_has_filled_head_at_end() {
        let g = primitive_geometry(Primitive::Arrow, 100.0, 0.0, &no_params);
        assert_eq!(g.paths.len(), 2);
        assert_eq!(g.paths[0].fill, FillRole::None);
        assert_eq!(g.paths[1].fill, FillRole::StrokeColor);
        let head = vertices(&g, 1);
        assert_eq!(head.len(), 3);
        assert!(head.iter().any(|&p| close_to(p, (100.0, 0.0))));
        // Default head length 12 along the line.
        assert!(head.iter().any(|&p| close_to(p, (88.0, 6.0))));
        assert_eq!(g.paths[1].cmds.last(), Some(&PathCmd::Close));
    }

    #[test]
    fn polygon_sides_are_clamped() {
        let sides = |n: f32| move |k: &str| (k == "sides").then_some(n);
        let g = primitive_geometry(Primitive::Polygon, 10.0, 10.0, &sides(1.0));
        assert_eq!(vertices(&g, 0).len(), 3);
        let g = primitive_geometry(Primitive::Polygon, 10.0, 10.0, &sides(50.0));
        assert_eq!(vertices(&g, 0).len(), 12);
        let g = primitive_geometry(Primitive::Polygon, 10.0, 10.0, &no_params);
        let v = vertices(&g, 0);
        assert_eq!(v.len(), 3);
        assert!(close_to(v[0], (5.0, 0.0)));
    }

    #[test]
    fn star_has_twice_points_vertices() {
        let g = primitive_geometry(Primitive::Star, 10.0, 10.0, &no_params);
        let v = vertices(&g, 0);
        assert_eq!(v.len(), 10);
        assert!(close_to(v[0], (5.0, 0.0)));
        let param = |k: &str| match k {
            "points" => Some(100.0),
            "inner_ratio" => Some(5.0),
            _ => None,
        };
        let g = primitive_geometry(Primitive::Star, 10.0, 10.0, &param);
        let v = vertices(&g, 0);
        assert_eq!(v.len(), 40);
        // inner_ratio clamped to 0.9: inner vertex at 0.9 × radius 5.
        let (dx, dy) = (v[1].0 - 5.0, v[1].1 - 5.0);
        assert!(((dx * dx + dy * dy).sqrt() - 4.5).abs() < 1e-3);
    }

    #[test]
    fn to_canvas_flips_and_rotates() {
        let local = Geometry {
            paths: vec![SubPath {
                cmds: vec![PathCmd::MoveTo((0.0, 0.0))],
                fill: FillRole::None,
            }],
        };
        let first = |g: &Geometry| match g.paths[0].cmds[0] {
            PathCmd::MoveTo(p) => p,
            _ => unreachable!(),
        };
        let flipped = Frame {
            x: 100.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            flip_x: true,
            ..Frame::default()
        };
        assert!(close_to(first(&to_canvas(&local, &flipped)), (110.0, 0.0)));
        let rotated = Frame {
            x: 100.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            rotation: 90.0,
            ..Frame::default()
        };
        assert!(close_to(first(&to_canvas(&local, &rotated)), (110.0, 0.0)));
        let plain = Frame::from_rect(&Rect {
            x: 100.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        });
        assert!(close_to(first(&to_canvas(&local, &plain)), (100.0, 0.0)));
    }

    #[test]
    fn hit_test_inside_rect_and_near_line() {
        let rect = primitive_geometry(Primitive::Rect, 100.0, 50.0, &no_params);
        assert!(hit_test(&rect, (50.0, 25.0), 3.0));
        assert!(!hit_test(&rect, (150.0, 25.0), 3.0));
        let line = primitive_geometry(Primitive::Line, 100.0, 0.0, &no_params);
        assert!(hit_test(&line, (50.0, 1.0), 3.0));
        assert!(!hit_test(&line, (50.0, 10.0), 3.0));
        assert!(!hit_test(&line, (200.0, 0.0), 3.0));
    }

    #[test]
    fn hit_test_even_odd_hole() {
        let square = |a: f32, b: f32| {
            vec![
                PathCmd::MoveTo((a, a)),
                PathCmd::LineTo((b, a)),
                PathCmd::LineTo((b, b)),
                PathCmd::LineTo((a, b)),
                PathCmd::Close,
            ]
        };
        let mut cmds = square(0.0, 100.0);
        cmds.extend(square(25.0, 75.0));
        let g = Geometry {
            paths: vec![SubPath {
                cmds,
                fill: FillRole::Shape,
            }],
        };
        assert!(hit_test(&g, (10.0, 10.0), 0.0));
        assert!(!hit_test(&g, (50.0, 50.0), 0.0));
        assert!(hit_test(&g, (50.0, 26.0), 2.0), "near the hole's edge");
    }

    #[test]
    fn hit_test_stroke_tolerance_on_curve() {
        // Apex of this cubic is (50, 75).
        let g = Geometry {
            paths: vec![SubPath {
                cmds: vec![
                    PathCmd::MoveTo((0.0, 0.0)),
                    PathCmd::CubicTo {
                        c1: (0.0, 100.0),
                        c2: (100.0, 100.0),
                        to: (100.0, 0.0),
                    },
                ],
                fill: FillRole::None,
            }],
        };
        assert!(hit_test(&g, (50.0, 77.0), 3.0));
        assert!(!hit_test(&g, (50.0, 85.0), 3.0));
        assert!(
            !hit_test(&g, (50.0, 50.0), 3.0),
            "stroke-only is not filled"
        );
    }

    #[test]
    fn flatten_curve_respects_tolerance() {
        let (p0, c1, c2, to) = ((0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0));
        let g = Geometry {
            paths: vec![SubPath {
                cmds: vec![PathCmd::MoveTo(p0), PathCmd::CubicTo { c1, c2, to }],
                fill: FillRole::None,
            }],
        };
        let coarse = flatten(&g, 1.0);
        let fine = flatten(&g, 0.05);
        assert_eq!(coarse.len(), 1);
        assert!(fine[0].0.len() > coarse[0].0.len());
        let poly = &coarse[0].0;
        assert_eq!(poly.first(), Some(&p0));
        assert_eq!(poly.last(), Some(&to));
        for i in 0..=200 {
            let t = i as f32 / 200.0;
            let p = cubic_point(p0, c1, c2, to, t);
            let d = poly
                .windows(2)
                .map(|s| segment_distance(p, s[0], s[1]))
                .fold(f32::INFINITY, f32::min);
            assert!(d <= 1.0, "t={t} is {d} from the polyline");
        }
    }

    #[test]
    fn scale_unit_and_rect_helpers() {
        let unit = Geometry {
            paths: vec![SubPath {
                cmds: vec![PathCmd::MoveTo((0.5, 1.0))],
                fill: FillRole::Shape,
            }],
        };
        assert_eq!(
            scale_unit(&unit, 20.0, 10.0).paths[0].cmds[0],
            PathCmd::MoveTo((10.0, 10.0))
        );
        let a = Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        let b = Rect {
            x: 5.0,
            y: 5.0,
            w: 10.0,
            h: 2.0,
        };
        assert!(a.contains((10.0, 10.0)));
        assert!(!a.contains((10.1, 5.0)));
        assert!(!a.contains_rect(&b));
        let u = a.union(&b);
        assert_eq!(
            u,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 15.0,
                h: 10.0
            }
        );
        assert!(u.contains_rect(&b));
        assert_eq!(a.center(), (5.0, 5.0));
    }

    #[test]
    fn primitive_names_and_params() {
        assert_eq!(Primitive::from_name("star"), Some(Primitive::Star));
        assert_eq!(Primitive::from_name("hexagon"), None);
        assert_eq!(Primitive::Star.param_keys(), &["points", "inner_ratio"]);
        assert!(Primitive::Line.param_keys().is_empty());
    }
}
