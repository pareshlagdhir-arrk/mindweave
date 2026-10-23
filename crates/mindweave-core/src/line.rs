//! Lines: the Line tool's shape drawn from its start to its end, with a
//! head at either end, a dash pattern, an optional double stroke and a
//! route. Either end may be attached to another shape; it then meets that
//! shape's outline and follows it when the shape moves.
//!
//! A line's frame is the box between its ends (see [`line_frame`]), so
//! selection, bounds and hit testing work as for other shapes; its
//! outline is drawn inside that box by [`line_geometry`].

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::collections::HashMap;

use crate::document::Page;
use crate::geometry::{
    flatten, line_endpoints, to_canvas, FillRole, Frame, Geometry, PathCmd, Pt, SubPath,
};
use crate::library::LibraryRegistry;
use crate::shape::Shape;

/// Library and shape id of the Line tool's shape.
pub const LINE_LIBRARY: &str = "basic-shapes";
pub const LINE_SHAPE: &str = "line";
/// The Arrow shape of earlier versions, opened as a line with an end head.
pub const OLD_ARROW_SHAPE: &str = "arrow";

/// Largest corner radius of a rounded route.
const ROUND_RADIUS: f32 = 12.0;
/// Flattening tolerance for curved and rounded routes.
const ROUTE_TOLERANCE: f32 = 0.1;
/// How far an attachment ray reaches past a shape's centre.
const RAY_LENGTH: f32 = 1e7;
/// Segments of a circle head.
const CIRCLE_SEGMENTS: usize = 16;

/// The mark drawn at one end of a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArrowHead {
    #[default]
    None,
    /// An open V.
    Arrow,
    /// A filled triangle.
    Triangle,
    Diamond,
    Circle,
    /// A triangle outlined in the line's colour, filled with the canvas
    /// colour.
    #[serde(rename = "hollow-triangle")]
    HollowTriangle,
    /// A diamond outlined in the line's colour, filled with the canvas
    /// colour.
    #[serde(rename = "hollow-diamond")]
    HollowDiamond,
}

impl ArrowHead {
    pub const ALL: [ArrowHead; 7] = [
        ArrowHead::None,
        ArrowHead::Arrow,
        ArrowHead::Triangle,
        ArrowHead::Diamond,
        ArrowHead::Circle,
        ArrowHead::HollowTriangle,
        ArrowHead::HollowDiamond,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ArrowHead::None => "None",
            ArrowHead::Arrow => "Arrow",
            ArrowHead::Triangle => "Triangle",
            ArrowHead::Diamond => "Diamond",
            ArrowHead::Circle => "Circle",
            ArrowHead::HollowTriangle => "Hollow triangle",
            ArrowHead::HollowDiamond => "Hollow diamond",
        }
    }
}

/// How a line's stroke is broken up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineDash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl LineDash {
    pub const ALL: [LineDash; 3] = [LineDash::Solid, LineDash::Dashed, LineDash::Dotted];

    pub fn name(self) -> &'static str {
        match self {
            LineDash::Solid => "Solid",
            LineDash::Dashed => "Dashed",
            LineDash::Dotted => "Dotted",
        }
    }

    /// Dash and gap lengths for a stroke `width` wide; `None` when solid.
    pub fn pattern(self, width: f32) -> Option<[f32; 2]> {
        let w = width.max(1.0);
        match self {
            LineDash::Solid => None,
            LineDash::Dashed => Some([4.0 * w, 2.5 * w]),
            LineDash::Dotted => Some([w, 1.5 * w]),
        }
    }
}

/// The path a line takes from its start to its end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineRoute {
    #[default]
    Straight,
    /// Horizontal and vertical segments with sharp corners.
    Angular,
    /// Like angular, with rounded corners.
    Rounded,
    /// A smooth S curve.
    Curved,
}

impl LineRoute {
    pub const ALL: [LineRoute; 4] = [
        LineRoute::Straight,
        LineRoute::Angular,
        LineRoute::Rounded,
        LineRoute::Curved,
    ];

    pub fn name(self) -> &'static str {
        match self {
            LineRoute::Straight => "Straight",
            LineRoute::Angular => "Angular",
            LineRoute::Rounded => "Rounded",
            LineRoute::Curved => "Curved",
        }
    }
}

/// How a line passes over the lines beneath it where they cross.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineJump {
    /// Straight across.
    #[default]
    None,
    /// A half-circle hop.
    Arc,
    /// A break in the line.
    Gap,
    /// A pointed hop.
    Sharp,
    /// A square hop.
    Line,
}

impl LineJump {
    pub const ALL: [LineJump; 5] = [
        LineJump::None,
        LineJump::Arc,
        LineJump::Gap,
        LineJump::Sharp,
        LineJump::Line,
    ];

    pub fn name(self) -> &'static str {
        match self {
            LineJump::None => "None",
            LineJump::Arc => "Arc",
            LineJump::Gap => "Gap",
            LineJump::Sharp => "Sharp",
            LineJump::Line => "Line",
        }
    }
}

/// Jump sizes offered, in canvas units; the first is the default.
pub const JUMP_SIZES: [f32; 5] = [8.0, 6.0, 10.0, 12.0, 16.0];

fn default_jump_size() -> f32 {
    JUMP_SIZES[0]
}

/// Which end of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineEnd {
    Start,
    End,
}

/// Properties only lines have. The dash pattern and double stroke are in
/// the shape's [`crate::shape::Style`], as every shape has them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineProps {
    pub start_head: ArrowHead,
    pub end_head: ArrowHead,
    pub route: LineRoute,
    pub jump: LineJump,
    /// Length of a jump along the line, in canvas units.
    pub jump_size: f32,
    /// The shape the start is attached to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Uuid>,
    /// The shape the end is attached to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Uuid>,
    /// How far the middle of the route was dragged, in canvas units: a
    /// straight or curved line bows through it, an elbow's middle segment
    /// moves across. `None` for the plain route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bend: Option<(f32, f32)>,
    /// A mind map relationship: removed with either topic it joins.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub relationship: bool,
    /// Angular, rounded and curved routes leave and arrive vertically
    /// rather than horizontally. Set by [`route_line`].
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub vertical: bool,
}

impl Default for LineProps {
    fn default() -> Self {
        LineProps {
            start_head: ArrowHead::None,
            end_head: ArrowHead::None,
            route: LineRoute::Straight,
            jump: LineJump::None,
            jump_size: default_jump_size(),
            start: None,
            end: None,
            bend: None,
            relationship: false,
            vertical: false,
        }
    }
}

impl LineProps {
    /// The shape `end` is attached to.
    pub fn attached(&self, end: LineEnd) -> Option<Uuid> {
        match end {
            LineEnd::Start => self.start,
            LineEnd::End => self.end,
        }
    }

    pub fn set_attached(&mut self, end: LineEnd, id: Option<Uuid>) {
        match end {
            LineEnd::Start => self.start = id,
            LineEnd::End => self.end = id,
        }
    }
}

/// The frame of a line from `a` to `b`: the box between them, flipped to
/// record which corner is the start, unrotated.
pub fn line_frame(a: Pt, b: Pt) -> Frame {
    Frame {
        x: a.0.min(b.0),
        y: a.1.min(b.1),
        w: (b.0 - a.0).abs(),
        h: (b.1 - a.1).abs(),
        rotation: 0.0,
        flip_x: b.0 < a.0,
        flip_y: b.1 < a.1,
    }
}

/// A shape a line end is attached to: its centre and its outline, both in
/// canvas coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub centre: Pt,
    pub outline: Geometry,
}

/// Where a ray from `t`'s centre towards `toward` leaves its outline: the
/// farthest crossing, or the centre when the ray crosses nothing.
pub fn edge_point(t: &Target, toward: Pt) -> Pt {
    let c = t.centre;
    let (dx, dy) = (toward.0 - c.0, toward.1 - c.1);
    let len = dx.hypot(dy);
    if len.is_nan() || len <= 1e-6 {
        return c;
    }
    let far = (c.0 + dx / len * RAY_LENGTH, c.1 + dy / len * RAY_LENGTH);
    let mut best: Option<f32> = None;
    for (contour, _) in flatten(&t.outline, ROUTE_TOLERANCE) {
        for seg in contour.windows(2) {
            if let Some(s) = segment_crossing(c, far, seg[0], seg[1]) {
                best = Some(best.map_or(s, |b: f32| b.max(s)));
            }
        }
    }
    match best {
        Some(s) => (c.0 + (far.0 - c.0) * s, c.1 + (far.1 - c.1) * s),
        None => c,
    }
}

/// Where segment `p`–`q` crosses segment `a`–`b`, as a fraction along
/// `p`–`q`.
fn segment_crossing(p: Pt, q: Pt, a: Pt, b: Pt) -> Option<f32> {
    let r = (q.0 - p.0, q.1 - p.1);
    let s = (b.0 - a.0, b.1 - a.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() < 1e-12 {
        return None;
    }
    let ap = (a.0 - p.0, a.1 - p.1);
    let t = (ap.0 * s.1 - ap.1 * s.0) / denom;
    let u = (ap.0 * r.1 - ap.1 * r.0) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// Moves `line`'s attached ends onto their shapes' outlines (`start` and
/// `end` are the shapes its ends are attached to, if any) and sets which
/// way its route leaves. Free ends stay where they are. A straight line
/// aims each attached end at the other end; other routes leave
/// horizontally or vertically, whichever way the ends are farther apart.
pub fn route_line(
    frame: &mut Frame,
    props: &mut LineProps,
    start: Option<&Target>,
    end: Option<&Target>,
) {
    let (a, b) = line_endpoints(frame);
    let ca = start.map_or(a, |t| t.centre);
    let cb = end.map_or(b, |t| t.centre);
    let (dx, dy) = (cb.0 - ca.0, cb.1 - ca.1);
    props.vertical = dy.abs() > dx.abs();
    if start.is_none() && end.is_none() {
        return;
    }
    let (toward_a, toward_b) = if props.route == LineRoute::Straight {
        (cb, ca)
    } else if props.vertical {
        let s = if dy < 0.0 { -1.0 } else { 1.0 };
        ((ca.0, ca.1 + s), (cb.0, cb.1 - s))
    } else {
        let s = if dx < 0.0 { -1.0 } else { 1.0 };
        ((ca.0 + s, ca.1), (cb.0 - s, cb.1))
    };
    let a = start.map_or(a, |t| edge_point(t, toward_a));
    let b = end.map_or(b, |t| edge_point(t, toward_b));
    *frame = line_frame(a, b);
}

/// The angle (degrees) a line's label is drawn at, given the angle a
/// straight line's label would have: along a straight line, level on
/// other routes, whose middle segment the label's centre sits on.
pub fn line_label_angle(route: LineRoute, straight_angle: f32) -> f32 {
    if route == LineRoute::Straight {
        straight_angle
    } else {
        0.0
    }
}

/// The outline of `shape` in canvas coordinates as a line end can attach
/// to it: its library outline, or its box when that is unknown (no
/// `libs`, or a missing library). Lines cannot be attached to.
pub fn target_of(shape: &Shape, libs: Option<&LibraryRegistry>) -> Option<Target> {
    if shape.is_line() {
        return None;
    }
    let f = &shape.frame;
    let local = libs
        .filter(|_| !shape.is_text_box())
        .and_then(|l| l.shape(&shape.library, &shape.shape))
        .map(|def| def.local_geometry(f.w, f.h, &|k| shape.param(k)))
        .unwrap_or_else(|| {
            let cmds = vec![
                PathCmd::MoveTo((0.0, 0.0)),
                PathCmd::LineTo((f.w, 0.0)),
                PathCmd::LineTo((f.w, f.h)),
                PathCmd::LineTo((0.0, f.h)),
                PathCmd::Close,
            ];
            Geometry {
                paths: vec![SubPath {
                    cmds,
                    fill: FillRole::Shape,
                }],
            }
        });
    Some(Target {
        centre: (f.x + f.w / 2.0, f.y + f.h / 2.0),
        outline: to_canvas(&local, f),
    })
}

/// Re-routes every line of `page` (see [`route_line`]) after its shapes
/// changed. An end attached to a shape that is gone, or that cannot be
/// attached to, is let go where it is. Returns whether anything changed.
pub fn reroute_page(page: &mut Page, libs: Option<&LibraryRegistry>) -> bool {
    let mut targets: HashMap<uuid::Uuid, Option<Target>> = HashMap::new();
    let wanted: Vec<uuid::Uuid> = page
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .filter_map(|s| s.line.as_ref())
        .flat_map(|p| [p.start, p.end])
        .flatten()
        .collect();
    for shape in page.layers.iter().flat_map(|l| &l.objects) {
        if wanted.contains(&shape.id) {
            targets.insert(shape.id, target_of(shape, libs));
        }
    }
    let mut changed = false;
    for shape in page.layers.iter_mut().flat_map(|l| l.objects.iter_mut()) {
        let Some(props) = shape.line.as_ref() else {
            continue;
        };
        let mut props = props.clone();
        let mut frame = shape.frame;
        let target =
            |id: Option<uuid::Uuid>| id.and_then(|id| targets.get(&id).and_then(Option::as_ref));
        let (start, end) = (target(props.start), target(props.end));
        if start.is_none() {
            props.start = None;
        }
        if end.is_none() {
            props.end = None;
        }
        route_line(&mut frame, &mut props, start, end);
        if frame != shape.frame || Some(&props) != shape.line.as_ref() {
            shape.frame = frame;
            shape.line = Some(props);
            changed = true;
        }
    }
    changed
}

/// Head length for a stroke `width` wide.
fn head_length(width: f32) -> f32 {
    3.0 * width.max(1.0) + 6.0
}

/// The route's points in the local box `0..w × 0..h`, from `(0, 0)` to
/// `(w, h)`, curves flattened.
fn route_points(w: f32, h: f32, props: &LineProps, bend: Pt) -> Vec<Pt> {
    let v = props.vertical;
    let corners = if v {
        let y = h / 2.0 + bend.1;
        [(0.0, 0.0), (0.0, y), (w, y), (w, h)]
    } else {
        let x = w / 2.0 + bend.0;
        [(0.0, 0.0), (x, 0.0), (x, h), (w, h)]
    };
    let bent = bend != (0.0, 0.0);
    let pts = match props.route {
        // A cubic's middle moves by 3/4 of its control points' move, a
        // quadratic's by half of its control point's.
        LineRoute::Straight if bent => flatten_cmds(vec![
            PathCmd::MoveTo((0.0, 0.0)),
            PathCmd::QuadTo {
                ctrl: (w / 2.0 + 2.0 * bend.0, h / 2.0 + 2.0 * bend.1),
                to: (w, h),
            },
        ]),
        LineRoute::Straight => vec![(0.0, 0.0), (w, h)],
        LineRoute::Angular => corners.to_vec(),
        LineRoute::Rounded => {
            let mut cmds = vec![PathCmd::MoveTo(corners[0])];
            for i in 1..3 {
                let (p, c, n) = (corners[i - 1], corners[i], corners[i + 1]);
                let r = ROUND_RADIUS.min(dist(p, c) / 2.0).min(dist(c, n) / 2.0);
                cmds.push(PathCmd::LineTo(towards(c, p, r)));
                cmds.push(PathCmd::QuadTo {
                    ctrl: c,
                    to: towards(c, n, r),
                });
            }
            cmds.push(PathCmd::LineTo(corners[3]));
            flatten_cmds(cmds)
        }
        LineRoute::Curved => {
            let (c1, c2) = if v {
                ((0.0, h / 2.0), (w, h / 2.0))
            } else {
                ((w / 2.0, 0.0), (w / 2.0, h))
            };
            let k = 4.0 / 3.0;
            let (c1, c2) = (
                (c1.0 + bend.0 * k, c1.1 + bend.1 * k),
                (c2.0 + bend.0 * k, c2.1 + bend.1 * k),
            );
            flatten_cmds(vec![
                PathCmd::MoveTo((0.0, 0.0)),
                PathCmd::CubicTo { c1, c2, to: (w, h) },
            ])
        }
    };
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_none_or(|&q| dist(p, q) > 1e-4) {
            out.push(p);
        }
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

fn flatten_cmds(cmds: Vec<PathCmd>) -> Vec<Pt> {
    let g = Geometry {
        paths: vec![SubPath {
            cmds,
            fill: FillRole::None,
        }],
    };
    flatten(&g, ROUTE_TOLERANCE)
        .into_iter()
        .next()
        .map(|(c, _)| c)
        .unwrap_or_default()
}

fn dist(a: Pt, b: Pt) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The point `d` from `from` towards `to`.
fn towards(from: Pt, to: Pt, d: f32) -> Pt {
    let l = dist(from, to);
    if l <= 0.0 {
        return from;
    }
    (
        from.0 + (to.0 - from.0) * d / l,
        from.1 + (to.1 - from.1) * d / l,
    )
}

/// The unit direction a polyline arrives at its last point with, or
/// `None` when it has no length.
fn end_direction(pts: &[Pt]) -> Option<Pt> {
    let tip = *pts.last()?;
    let from = pts.iter().rev().find(|&&p| dist(p, tip) > 1e-4)?;
    let l = dist(*from, tip);
    Some(((tip.0 - from.0) / l, (tip.1 - from.1) / l))
}

/// Shortens the polyline at its last point by `d`, keeping at least its
/// first point.
fn trim_end(pts: &mut Vec<Pt>, mut d: f32) {
    while d > 0.0 && pts.len() >= 2 {
        let n = pts.len();
        let (a, b) = (pts[n - 2], pts[n - 1]);
        let l = dist(a, b);
        if l > d {
            pts[n - 1] = towards(b, a, d);
            return;
        }
        d -= l;
        pts.pop();
    }
}

/// How far a head at a line's end cuts the line back, for a head `len`
/// long on a stroke `width` wide.
fn head_inset(head: ArrowHead, len: f32, width: f32) -> f32 {
    match head {
        ArrowHead::None => 0.0,
        ArrowHead::Arrow => width / 2.0,
        ArrowHead::Triangle
        | ArrowHead::Diamond
        | ArrowHead::HollowTriangle
        | ArrowHead::HollowDiamond => len * 0.8,
        ArrowHead::Circle => len * 0.5,
    }
}

/// The filled sub-paths of `head` with its tip at `tip`, pointing along
/// unit direction `dir`, `len` long, for a stroke `width` wide. A hollow
/// head is its filled form's outline filled with the canvas colour, then
/// a ring `width` thick (at least `min_ring`) inside that outline in the
/// stroke colour, or the whole outline when the ring would fill it.
fn head_paths(
    head: ArrowHead,
    tip: Pt,
    dir: Pt,
    len: f32,
    width: f32,
    min_ring: f32,
) -> Vec<SubPath> {
    let n = (-dir.1, dir.0);
    let at = |along: f32, side: f32| {
        (
            tip.0 - dir.0 * along + n.0 * side,
            tip.1 - dir.1 * along + n.1 * side,
        )
    };
    let half = len * 0.45;
    let triangle = || vec![tip, at(len, half), at(len, -half)];
    let diamond = || {
        vec![
            tip,
            at(len * 0.8 / 2.0, half * 0.8),
            at(len * 0.8, 0.0),
            at(len * 0.8 / 2.0, -half * 0.8),
        ]
    };
    let polygon = |pts: Vec<Pt>| SubPath {
        cmds: polygon_cmds(&pts),
        fill: FillRole::StrokeColor,
    };
    let hollow = |pts: Vec<Pt>| {
        let outline = polygon_cmds(&pts);
        let mut ring = outline.clone();
        // Thick strokes keep a hole: at most most of the way to the middle,
        // unless the floor is thicker still.
        let inner = inradius(&pts);
        let thickness = width.max(1.0).min(0.7 * inner).max(min_ring);
        if thickness < inner {
            ring.extend(polygon_cmds(&inset_polygon(&pts, thickness)));
        }
        vec![
            SubPath {
                cmds: outline,
                fill: FillRole::Canvas,
            },
            SubPath {
                cmds: ring,
                fill: FillRole::StrokeColor,
            },
        ]
    };
    match head {
        ArrowHead::None => Vec::new(),
        ArrowHead::Triangle => vec![polygon(triangle())],
        ArrowHead::Diamond => vec![polygon(diamond())],
        ArrowHead::HollowTriangle => hollow(triangle()),
        ArrowHead::HollowDiamond => hollow(diamond()),
        ArrowHead::Circle => {
            let r = len * 0.25;
            let c = at(r, 0.0);
            let pts = (0..CIRCLE_SEGMENTS)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / CIRCLE_SEGMENTS as f32;
                    (c.0 + r * a.cos(), c.1 + r * a.sin())
                })
                .collect();
            vec![polygon(pts)]
        }
        ArrowHead::Arrow => {
            // Each arm is a bar `width` thick from the tip to a wing.
            let t = width.max(1.0) / 2.0;
            [half, -half]
                .into_iter()
                .map(|side| {
                    let wing = at(len, side);
                    let l = dist(tip, wing);
                    let u = ((wing.0 - tip.0) / l, (wing.1 - tip.1) / l);
                    let m = (-u.1 * t, u.0 * t);
                    polygon(vec![
                        (tip.0 + m.0, tip.1 + m.1),
                        (wing.0 + m.0, wing.1 + m.1),
                        (wing.0 - m.0, wing.1 - m.1),
                        (tip.0 - m.0, tip.1 - m.1),
                    ])
                })
                .collect()
        }
    }
}

/// A closed polygon's commands: a move to the first point, lines to the
/// rest, and a close.
fn polygon_cmds(pts: &[Pt]) -> Vec<PathCmd> {
    let mut cmds: Vec<PathCmd> = pts.iter().map(|&p| PathCmd::LineTo(p)).collect();
    cmds[0] = PathCmd::MoveTo(pts[0]);
    cmds.push(PathCmd::Close);
    cmds
}

/// Twice the signed area of the polygon `pts` (last point not repeated):
/// positive when it winds so that its inside is to the left of travel.
fn signed_area2(pts: &[Pt]) -> f32 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

/// The radius of the largest circle in a polygon `pts` that has one
/// touching every side (twice its area over its perimeter).
fn inradius(pts: &[Pt]) -> f32 {
    let n = pts.len();
    let perimeter: f32 = (0..n).map(|i| dist(pts[i], pts[(i + 1) % n])).sum();
    signed_area2(pts).abs() / perimeter.max(1e-6)
}

/// The convex polygon `pts` (last point not repeated) moved `d` inwards,
/// whichever way it winds.
fn inset_polygon(pts: &[Pt], d: f32) -> Vec<Pt> {
    offset_closed(pts, if signed_area2(pts) > 0.0 { d } else { -d })
}

/// The polyline moved `d` to its left (y down: to the right of travel for
/// negative `d`), corners mitred.
fn offset_polyline(pts: &[Pt], d: f32) -> Vec<Pt> {
    let normal = |a: Pt, b: Pt| {
        let l = dist(a, b).max(1e-6);
        (-(b.1 - a.1) / l, (b.0 - a.0) / l)
    };
    let n = pts.len();
    (0..n)
        .map(|i| {
            let prev = (i > 0).then(|| normal(pts[i - 1], pts[i]));
            let next = (i + 1 < n).then(|| normal(pts[i], pts[i + 1]));
            let m = match (prev, next) {
                (Some(a), Some(b)) => {
                    let s = (a.0 + b.0, a.1 + b.1);
                    let l = s.0.hypot(s.1);
                    if l < 1e-6 {
                        a
                    } else {
                        // Mitre length 1/cos(half the turn), capped.
                        let m = (s.0 / l, s.1 / l);
                        let cos = (m.0 * a.0 + m.1 * a.1).max(0.25);
                        (m.0 / cos, m.1 / cos)
                    }
                }
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => (0.0, 0.0),
            };
            (pts[i].0 + m.0 * d, pts[i].1 + m.1 * d)
        })
        .collect()
}

fn polyline_path(pts: &[Pt]) -> SubPath {
    let mut cmds: Vec<PathCmd> = pts.iter().map(|&p| PathCmd::LineTo(p)).collect();
    if let Some(first) = cmds.first_mut() {
        *first = PathCmd::MoveTo(pts[0]);
    }
    SubPath {
        cmds,
        fill: FillRole::None,
    }
}

/// How far a jump reaches either side of its crossing, for a line
/// stroked `width` wide.
fn jump_radius(props: &LineProps, width: f32) -> f32 {
    let size = if props.jump_size.is_finite() {
        props.jump_size
    } else {
        default_jump_size()
    };
    (size / 2.0).clamp(width.max(1.0), 100.0)
}

/// Lengths along `pts` to each point, from 0 to the whole length.
fn cumulative(pts: &[Pt]) -> Vec<f32> {
    let mut out = Vec::with_capacity(pts.len());
    let mut total = 0.0;
    for (i, &p) in pts.iter().enumerate() {
        if i > 0 {
            total += dist(pts[i - 1], p);
        }
        out.push(total);
    }
    out
}

/// The point `s` along `pts` (with lengths `cum`), clamped to its ends.
fn point_at(pts: &[Pt], cum: &[f32], s: f32) -> Pt {
    let i = cum.partition_point(|&c| c < s).clamp(1, pts.len() - 1);
    let (a, b) = (pts[i - 1], pts[i]);
    let seg = cum[i] - cum[i - 1];
    if seg <= 0.0 {
        return b;
    }
    let t = ((s - cum[i - 1]) / seg).clamp(0.0, 1.0);
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Segments of an arc jump.
const ARC_SEGMENTS: usize = 10;

/// `pts` with a jump of `kind` and reach `r` at each of `at` (lengths
/// along it, ascending), hopping towards `up(dir)` for a jump heading
/// along unit `dir`. Jumps too near an end or overlapping the previous
/// jump are left out. A gap splits the line, so this returns its pieces.
fn with_jumps(
    pts: &[Pt],
    at: &[f32],
    kind: LineJump,
    r: f32,
    up: impl Fn(Pt) -> Pt,
) -> Vec<Vec<Pt>> {
    if kind == LineJump::None || at.is_empty() || pts.len() < 2 {
        return vec![pts.to_vec()];
    }
    let cum = cumulative(pts);
    let total = *cum.last().unwrap_or(&0.0);
    let mut pieces = Vec::new();
    let mut cur = vec![pts[0]];
    let mut next = 1;
    let mut done = 0.0_f32;
    for &s in at {
        let (a, b) = (s - r, s + r);
        if a < done.max(r * 0.5) || b > total - r * 0.5 {
            continue;
        }
        while next < pts.len() && cum[next] < a {
            cur.push(pts[next]);
            next += 1;
        }
        let (pa, pb) = (point_at(pts, &cum, a), point_at(pts, &cum, b));
        let chord = dist(pa, pb);
        if chord <= 1e-4 {
            continue;
        }
        let dir = ((pb.0 - pa.0) / chord, (pb.1 - pa.1) / chord);
        let n = up(dir);
        let h = chord / 2.0;
        let c = ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0);
        cur.push(pa);
        match kind {
            LineJump::None => {}
            LineJump::Gap => {
                pieces.push(std::mem::take(&mut cur));
            }
            LineJump::Arc => {
                for k in 1..ARC_SEGMENTS {
                    let t = std::f32::consts::PI * k as f32 / ARC_SEGMENTS as f32;
                    let (along, side) = (-h * t.cos(), h * t.sin());
                    cur.push((
                        c.0 + dir.0 * along + n.0 * side,
                        c.1 + dir.1 * along + n.1 * side,
                    ));
                }
            }
            LineJump::Sharp => cur.push((c.0 + n.0 * h, c.1 + n.1 * h)),
            LineJump::Line => {
                cur.push((pa.0 + n.0 * h, pa.1 + n.1 * h));
                cur.push((pb.0 + n.0 * h, pb.1 + n.1 * h));
            }
        }
        cur.push(pb);
        while next < pts.len() && cum[next] <= b {
            next += 1;
        }
        done = b;
    }
    cur.extend_from_slice(&pts[next.min(pts.len())..]);
    pieces.push(cur);
    pieces.retain(|p| p.len() >= 2);
    pieces
}

/// Canvas direction `v` in `f`'s local box (the inverse of
/// [`canvas_direction`]).
fn local_direction(f: &Frame, (x, y): Pt) -> Pt {
    let (sin, cos) = (-f.rotation).to_radians().sin_cos();
    let (x, y) = (x * cos - y * sin, x * sin + y * cos);
    (if f.flip_x { -x } else { x }, if f.flip_y { -y } else { y })
}

/// A line's bend in its frame's local box.
fn local_bend(f: &Frame, props: &LineProps) -> Pt {
    props.bend.map_or((0.0, 0.0), |b| local_direction(f, b))
}

/// The middle of a line's route, in canvas coordinates: where its bend
/// handle sits and its label is centred.
pub fn line_midpoint(f: &Frame, props: &LineProps) -> Pt {
    let (bx, by) = local_bend(f, props);
    let (w, h) = (f.w, f.h);
    let local = match props.route {
        LineRoute::Angular | LineRoute::Rounded if props.vertical => (w / 2.0, h / 2.0 + by),
        LineRoute::Angular | LineRoute::Rounded => (w / 2.0 + bx, h / 2.0),
        _ => (w / 2.0 + bx, h / 2.0 + by),
    };
    let g = to_canvas(
        &Geometry {
            paths: vec![SubPath {
                cmds: vec![PathCmd::MoveTo(local)],
                fill: FillRole::None,
            }],
        },
        f,
    );
    match g.paths[0].cmds[0] {
        PathCmd::MoveTo(p) => p,
        _ => local,
    }
}

/// The bend that puts a line's middle at canvas point `p`.
pub fn bend_through(f: &Frame, props: &LineProps, p: Pt) -> (f32, f32) {
    let plain = LineProps {
        bend: None,
        ..props.clone()
    };
    let mid = line_midpoint(f, &plain);
    (p.0 - mid.0, p.1 - mid.1)
}

/// `v`, a direction in `f`'s local box, as a canvas direction.
fn canvas_direction(f: &Frame, (x, y): Pt) -> Pt {
    let x = if f.flip_x { -x } else { x };
    let y = if f.flip_y { -y } else { y };
    let (sin, cos) = f.rotation.to_radians().sin_cos();
    (x * cos - y * sin, x * sin + y * cos)
}

/// A line's outline in its frame's local box (start at `(0, 0)`, end at
/// `(w, h)`) for a stroke `width` wide, `double` or not, with jumps at
/// `jumps` (lengths along its route, see [`line_jumps`]): the route as
/// stroke-only sub-paths (two per piece for a double line), then the
/// heads filled with the stroke colour. Jumps hop upwards on the canvas,
/// or leftwards on an upright line.
pub fn line_geometry(
    frame: &Frame,
    props: &LineProps,
    width: f32,
    double: bool,
    jumps: &[f32],
) -> Geometry {
    line_geometry_min_ring(frame, props, width, double, jumps, 0.0)
}

/// [`line_geometry`] with hollow heads' rings at least `min_ring` thick
/// (canvas units), so they stay as visible as the line when zoomed far
/// out; a ring too thick for its head fills it.
pub fn line_geometry_min_ring(
    frame: &Frame,
    props: &LineProps,
    width: f32,
    double: bool,
    jumps: &[f32],
    min_ring: f32,
) -> Geometry {
    let mut pts = route_points(frame.w, frame.h, props, local_bend(frame, props));
    // A double line is about three strokes wide; its heads match.
    let head_width = if double { 3.0 * width } else { width };
    let len = head_length(head_width);
    let start_dir = {
        let rev: Vec<Pt> = pts.iter().rev().copied().collect();
        end_direction(&rev)
    };
    let end_dir = end_direction(&pts);
    let (start_tip, end_tip) = (pts[0], *pts.last().unwrap_or(&pts[0]));
    let start_inset = head_inset(props.start_head, len, head_width);
    trim_end(&mut pts, head_inset(props.end_head, len, head_width));
    pts.reverse();
    trim_end(&mut pts, start_inset);
    pts.reverse();
    let shifted: Vec<f32> = jumps.iter().map(|s| s - start_inset).collect();
    let up = |dir: Pt| {
        let n = (-dir.1, dir.0);
        let c = canvas_direction(frame, n);
        let upward = if c.1.abs() > 1e-3 {
            c.1 < 0.0
        } else {
            c.0 < 0.0
        };
        if upward {
            n
        } else {
            (-n.0, -n.1)
        }
    };
    let pieces = with_jumps(
        &pts,
        &shifted,
        props.jump,
        jump_radius(props, head_width),
        up,
    );
    let mut paths: Vec<SubPath> = pieces
        .iter()
        .flat_map(|piece| {
            if double {
                vec![
                    polyline_path(&offset_polyline(piece, width)),
                    polyline_path(&offset_polyline(piece, -width)),
                ]
            } else {
                vec![polyline_path(piece)]
            }
        })
        .collect();
    if let Some(dir) = start_dir {
        paths.extend(head_paths(
            props.start_head,
            start_tip,
            dir,
            len,
            head_width,
            min_ring,
        ));
    }
    if let Some(dir) = end_dir {
        paths.extend(head_paths(
            props.end_head,
            end_tip,
            dir,
            len,
            head_width,
            min_ring,
        ));
    }
    Geometry { paths }
}

/// A line's route in canvas coordinates, start first.
fn canvas_route(shape: &Shape, props: &LineProps) -> Vec<Pt> {
    let local = polyline_path(&route_points(
        shape.frame.w,
        shape.frame.h,
        props,
        local_bend(&shape.frame, props),
    ));
    let g = to_canvas(&Geometry { paths: vec![local] }, &shape.frame);
    g.paths[0]
        .cmds
        .iter()
        .filter_map(|c| match *c {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(p),
            _ => None,
        })
        .collect()
}

/// Where each line of `page` that jumps crosses lines drawn beneath it
/// (on lower visible layers, or earlier on its own): lengths along its
/// route, ascending. Crossings at a line's very ends (lines meeting at a
/// shared point) are not jumped.
pub fn line_jumps(page: &Page) -> HashMap<uuid::Uuid, Vec<f32>> {
    struct Route {
        id: uuid::Uuid,
        jumps: bool,
        pts: Vec<Pt>,
        cum: Vec<f32>,
        bounds: (Pt, Pt),
    }
    // Layers are stored top first; paint order is bottom first.
    let routes: Vec<Route> = page
        .layers
        .iter()
        .rev()
        .filter(|l| l.visible)
        .flat_map(|l| &l.objects)
        .filter_map(|s| {
            let props = s.line.as_ref()?;
            let pts = canvas_route(s, props);
            let cum = cumulative(&pts);
            let lo = pts
                .iter()
                .fold((f32::MAX, f32::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
            let hi = pts
                .iter()
                .fold((f32::MIN, f32::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
            Some(Route {
                id: s.id,
                jumps: props.jump != LineJump::None,
                pts,
                cum,
                bounds: (lo, hi),
            })
        })
        .collect();
    let mut out = HashMap::new();
    for (i, top) in routes.iter().enumerate() {
        if !top.jumps {
            continue;
        }
        let mut at = Vec::new();
        for below in &routes[..i] {
            let ((a0, a1), (b0, b1)) = (top.bounds, below.bounds);
            if a1.0 < b0.0 || b1.0 < a0.0 || a1.1 < b0.1 || b1.1 < a0.1 {
                continue;
            }
            let below_len = *below.cum.last().unwrap_or(&0.0);
            for k in 1..top.pts.len() {
                for m in 1..below.pts.len() {
                    let (p, q) = (top.pts[k - 1], top.pts[k]);
                    let (a, b) = (below.pts[m - 1], below.pts[m]);
                    let Some(t) = segment_crossing(p, q, a, b) else {
                        continue;
                    };
                    let Some(u) = segment_crossing(a, b, p, q) else {
                        continue;
                    };
                    let s_below = below.cum[m - 1] + u * dist(a, b);
                    if s_below < END_MARGIN || s_below > below_len - END_MARGIN {
                        continue;
                    }
                    at.push(top.cum[k - 1] + t * dist(p, q));
                }
            }
        }
        at.sort_by(f32::total_cmp);
        at.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        if !at.is_empty() {
            out.insert(top.id, at);
        }
    }
    out
}

/// Crossings this near a lower line's ends are not jumped.
const END_MARGIN: f32 = 1.0;

/// `g` with a double outline `width` apart for a stroke `width` wide:
/// each sub-path keeps its fill but is not stroked, and its contours are
/// stroked twice, `width` outside and inside. Arrowheads stay as they are.
pub fn double_outline(g: &Geometry, width: f32) -> Geometry {
    let mut paths = Vec::new();
    for sub in &g.paths {
        if sub.fill == FillRole::StrokeColor {
            paths.push(sub.clone());
            continue;
        }
        if sub.fill == FillRole::Shape {
            paths.push(SubPath {
                cmds: sub.cmds.clone(),
                fill: FillRole::FillOnly,
            });
        }
        let one = Geometry {
            paths: vec![SubPath {
                cmds: sub.cmds.clone(),
                fill: FillRole::None,
            }],
        };
        for (contour, _) in flatten(&one, ROUTE_TOLERANCE) {
            let closed = contour.len() > 2 && dist(contour[0], contour[contour.len() - 1]) < 1e-4;
            for d in [width, -width] {
                let mut pts = if closed {
                    offset_closed(&contour[..contour.len() - 1], d)
                } else {
                    offset_polyline(&contour, d)
                };
                if closed {
                    pts.push(pts[0]);
                }
                paths.push(polyline_path(&pts));
            }
        }
    }
    Geometry { paths }
}

/// The closed polygon `pts` (last point not repeated) moved `d` to its
/// left, corners mitred.
fn offset_closed(pts: &[Pt], d: f32) -> Vec<Pt> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    // Offsetting the polygon with its neighbours wrapped around gives
    // every corner a mitre, the first and last included.
    let mut ring = Vec::with_capacity(n + 2);
    ring.push(pts[n - 1]);
    ring.extend_from_slice(pts);
    ring.push(pts[0]);
    let out = offset_polyline(&ring, d);
    out[1..=n].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::bounds;

    fn square(x: f32, y: f32, side: f32) -> Target {
        let cmds = vec![
            PathCmd::MoveTo((x, y)),
            PathCmd::LineTo((x + side, y)),
            PathCmd::LineTo((x + side, y + side)),
            PathCmd::LineTo((x, y + side)),
            PathCmd::Close,
        ];
        Target {
            centre: (x + side / 2.0, y + side / 2.0),
            outline: Geometry {
                paths: vec![SubPath {
                    cmds,
                    fill: FillRole::Shape,
                }],
            },
        }
    }

    /// An unflipped frame `w × h` at the origin.
    fn box_frame(w: f32, h: f32) -> Frame {
        Frame {
            w,
            h,
            ..Frame::default()
        }
    }

    fn close(a: Pt, b: Pt) -> bool {
        dist(a, b) < 1e-3
    }

    #[test]
    fn line_frame_round_trips_through_line_endpoints() {
        for (a, b) in [
            ((0.0, 0.0), (10.0, 5.0)),
            ((10.0, 5.0), (0.0, 0.0)),
            ((3.0, 9.0), (7.0, 1.0)),
            ((4.0, 4.0), (4.0, 20.0)),
        ] {
            let (p, q) = line_endpoints(&line_frame(a, b));
            assert!(close(p, a) && close(q, b), "{a:?} {b:?} -> {p:?} {q:?}");
        }
    }

    #[test]
    fn edge_point_is_where_the_ray_leaves_the_outline() {
        let t = square(0.0, 0.0, 100.0);
        assert!(close(edge_point(&t, (500.0, 50.0)), (100.0, 50.0)));
        assert!(close(edge_point(&t, (50.0, -300.0)), (50.0, 0.0)));
        assert!(close(edge_point(&t, (50.0, 50.0)), (50.0, 50.0)));
    }

    #[test]
    fn attached_ends_meet_both_outlines() {
        let (a, b) = (square(0.0, 0.0, 100.0), square(300.0, 0.0, 100.0));
        let mut frame = line_frame((0.0, 0.0), (1.0, 1.0));
        let mut props = LineProps::default();
        route_line(&mut frame, &mut props, Some(&a), Some(&b));
        let (p, q) = line_endpoints(&frame);
        assert!(
            close(p, (100.0, 50.0)) && close(q, (300.0, 50.0)),
            "{p:?} {q:?}"
        );
        assert!(!props.vertical);
        // Backwards: the start is on the right.
        route_line(&mut frame, &mut props, Some(&b), Some(&a));
        let (p, q) = line_endpoints(&frame);
        assert!(close(p, (300.0, 50.0)) && close(q, (100.0, 50.0)));
    }

    #[test]
    fn a_free_end_stays_and_the_attached_end_aims_at_it() {
        let a = square(0.0, 0.0, 100.0);
        let mut frame = line_frame((50.0, 50.0), (50.0, 400.0));
        let mut props = LineProps::default();
        route_line(&mut frame, &mut props, Some(&a), None);
        let (p, q) = line_endpoints(&frame);
        assert!(close(p, (50.0, 100.0)) && close(q, (50.0, 400.0)));
        assert!(props.vertical);
    }

    #[test]
    fn elbow_routes_leave_along_the_main_axis() {
        // Diagonal neighbours farther apart horizontally: leave the sides.
        let (a, b) = (square(0.0, 0.0, 100.0), square(300.0, 150.0, 100.0));
        let mut frame = line_frame((0.0, 0.0), (1.0, 1.0));
        let mut props = LineProps {
            route: LineRoute::Angular,
            ..LineProps::default()
        };
        route_line(&mut frame, &mut props, Some(&a), Some(&b));
        let (p, q) = line_endpoints(&frame);
        assert!(
            close(p, (100.0, 50.0)) && close(q, (300.0, 200.0)),
            "{p:?} {q:?}"
        );
        let g = line_geometry(&frame, &props, 2.0, false, &[]);
        let pts = route_points(frame.w, frame.h, &props, (0.0, 0.0));
        assert_eq!(
            pts,
            vec![(0.0, 0.0), (100.0, 0.0), (100.0, 150.0), (200.0, 150.0)]
        );
        assert_eq!(g.paths.len(), 1);
    }

    #[test]
    fn heads_cut_the_stroke_back_and_are_filled() {
        let props = LineProps {
            start_head: ArrowHead::Circle,
            end_head: ArrowHead::Triangle,
            ..LineProps::default()
        };
        let g = line_geometry(&box_frame(100.0, 0.0), &props, 2.0, false, &[]);
        let len = head_length(2.0);
        let stroke = &g.paths[0];
        assert_eq!(stroke.fill, FillRole::None);
        assert_eq!(stroke.cmds[0], PathCmd::MoveTo((len * 0.5, 0.0)));
        assert_eq!(stroke.cmds[1], PathCmd::LineTo((100.0 - len * 0.8, 0.0)));
        assert_eq!(g.paths.len(), 3);
        assert!(g.paths[1..].iter().all(|p| p.fill == FillRole::StrokeColor));
        // The triangle's tip is the line's end.
        assert_eq!(g.paths[2].cmds[0], PathCmd::MoveTo((100.0, 0.0)));
    }

    #[test]
    fn hollow_heads_trim_like_filled_ones() {
        let frame = box_frame(100.0, 0.0);
        for (filled, hollow) in [
            (ArrowHead::Triangle, ArrowHead::HollowTriangle),
            (ArrowHead::Diamond, ArrowHead::HollowDiamond),
        ] {
            let with = |head| {
                let props = LineProps {
                    start_head: head,
                    end_head: head,
                    ..LineProps::default()
                };
                line_geometry(&frame, &props, 2.0, false, &[])
            };
            let (f, h) = (with(filled), with(hollow));
            // The line is cut back to the same place at both ends.
            assert_eq!(h.paths[0], f.paths[0], "{hollow:?}");
            // Each hollow head: its outline filled with the canvas colour,
            // then a ring in the stroke colour around the same outline.
            assert_eq!(h.paths.len(), 5, "{hollow:?}");
            for (i, end) in [(1, 1), (3, 2)] {
                let (inside, ring) = (&h.paths[i], &h.paths[i + 1]);
                assert_eq!(inside.fill, FillRole::Canvas);
                assert_eq!(inside.cmds, f.paths[end].cmds);
                assert_eq!(ring.fill, FillRole::StrokeColor);
                // The outer contour is the filled head; an inner one follows.
                let n = f.paths[end].cmds.len();
                assert_eq!(&ring.cmds[..n], &f.paths[end].cmds[..]);
                assert_eq!(ring.cmds.len(), 2 * n);
                assert!(matches!(ring.cmds[n], PathCmd::MoveTo(_)));
            }
            // The ring's inside is hollow: the head's middle is not in it,
            // but its edge is.
            let ring = Geometry {
                paths: vec![h.paths[4].clone()],
            };
            let corners: Vec<Pt> = f.paths[2]
                .cmds
                .iter()
                .filter_map(|c| match c {
                    PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(*p),
                    _ => None,
                })
                .collect();
            let k = corners.len() as f32;
            let middle = corners
                .iter()
                .fold((0.0, 0.0), |m, p| (m.0 + p.0 / k, m.1 + p.1 / k));
            assert!(!crate::geometry::hit_test(&ring, middle, 0.0));
            assert!(crate::geometry::hit_test(&ring, (99.5, 0.0), 0.0));
            // A very thick line's head still has a hole in its middle.
            let thick = LineProps {
                end_head: hollow,
                ..LineProps::default()
            };
            let g = line_geometry(&frame, &thick, 30.0, false, &[]);
            let ring = Geometry {
                paths: vec![g.paths.last().unwrap().clone()],
            };
            let corners: Vec<Pt> = g.paths[g.paths.len() - 2]
                .cmds
                .iter()
                .filter_map(|c| match c {
                    PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(*p),
                    _ => None,
                })
                .collect();
            let k = corners.len() as f32;
            let middle = corners
                .iter()
                .fold((0.0, 0.0), |m, p| (m.0 + p.0 / k, m.1 + p.1 / k));
            assert!(!crate::geometry::hit_test(&ring, middle, 0.0), "{hollow:?}");
        }
    }

    #[test]
    fn hollow_head_rings_are_never_thinner_than_the_floor() {
        let frame = box_frame(100.0, 0.0);
        // The contours of a ring: the head's outline, then its hole if any.
        let contours = |ring: &SubPath| -> Vec<Vec<Pt>> {
            let mut out: Vec<Vec<Pt>> = Vec::new();
            for c in &ring.cmds {
                match c {
                    PathCmd::MoveTo(p) => out.push(vec![*p]),
                    PathCmd::LineTo(p) => out.last_mut().unwrap().push(*p),
                    _ => {}
                }
            }
            out
        };
        for hollow in [ArrowHead::HollowTriangle, ArrowHead::HollowDiamond] {
            let props = LineProps {
                end_head: hollow,
                ..LineProps::default()
            };
            // No floor: the same as before.
            assert_eq!(
                line_geometry_min_ring(&frame, &props, 2.0, false, &[], 0.0),
                line_geometry(&frame, &props, 2.0, false, &[])
            );
            for floor in [0.5, 2.5, 4.0, 1000.0] {
                let g = line_geometry_min_ring(&frame, &props, 2.0, false, &[], floor);
                let ring = g.paths.last().unwrap();
                assert_eq!(ring.fill, FillRole::StrokeColor);
                match &contours(ring)[..] {
                    [outer, inner] => {
                        let thickness = inradius(outer) - inradius(inner);
                        assert!(thickness >= floor - 1e-3, "{hollow:?} {floor}: {thickness}");
                    }
                    // Thicker than the head allows: drawn solid.
                    [_] => assert!(floor > 2.0, "{hollow:?} {floor}"),
                    other => panic!("{hollow:?} {floor}: {} contours", other.len()),
                }
            }
            // Far out the head is solid, never thinner than the line.
            let g = line_geometry_min_ring(&frame, &props, 2.0, false, &[], 1000.0);
            assert_eq!(contours(g.paths.last().unwrap()).len(), 1, "{hollow:?}");
        }
    }

    #[test]
    fn hollow_heads_round_trip() {
        assert!(ArrowHead::ALL.contains(&ArrowHead::HollowTriangle));
        assert!(ArrowHead::ALL.contains(&ArrowHead::HollowDiamond));
        assert_eq!(ArrowHead::HollowTriangle.name(), "Hollow triangle");
        assert_eq!(ArrowHead::HollowDiamond.name(), "Hollow diamond");
        let props = LineProps {
            start_head: ArrowHead::HollowDiamond,
            end_head: ArrowHead::HollowTriangle,
            ..LineProps::default()
        };
        let json = serde_json::to_value(&props).unwrap();
        assert_eq!(json["start_head"], "hollow-diamond");
        assert_eq!(json["end_head"], "hollow-triangle");
        let back: LineProps = serde_json::from_value(json).unwrap();
        assert_eq!(back, props);
        // The existing names are unchanged.
        assert_eq!(
            serde_json::to_value(ArrowHead::Triangle).unwrap(),
            "triangle"
        );
    }

    #[test]
    fn open_arrow_has_two_arms_and_double_lines_two_strokes() {
        let props = LineProps {
            end_head: ArrowHead::Arrow,
            ..LineProps::default()
        };
        let g = line_geometry(&box_frame(100.0, 0.0), &props, 2.0, true, &[]);
        let strokes: Vec<_> = g
            .paths
            .iter()
            .filter(|p| p.fill == FillRole::None)
            .collect();
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].cmds[0], PathCmd::MoveTo((0.0, 2.0)));
        assert_eq!(strokes[1].cmds[0], PathCmd::MoveTo((0.0, -2.0)));
        assert_eq!(g.paths.len(), 4);
    }

    #[test]
    fn every_route_starts_and_ends_at_the_corners() {
        for route in LineRoute::ALL {
            for vertical in [false, true] {
                let props = LineProps {
                    route,
                    vertical,
                    ..LineProps::default()
                };
                let pts = route_points(80.0, 40.0, &props, (0.0, 0.0));
                assert!(close(pts[0], (0.0, 0.0)), "{route:?}");
                assert!(close(*pts.last().unwrap(), (80.0, 40.0)), "{route:?}");
                let b = bounds(&line_geometry(
                    &box_frame(80.0, 40.0),
                    &props,
                    1.0,
                    false,
                    &[],
                ))
                .unwrap();
                assert!(b.x >= -1e-3 && b.y >= -1e-3 && b.x + b.w <= 80.001 && b.y + b.h <= 40.001);
            }
        }
    }

    #[test]
    fn zero_length_lines_have_no_heads_and_do_not_panic() {
        let props = LineProps {
            start_head: ArrowHead::Arrow,
            end_head: ArrowHead::Diamond,
            route: LineRoute::Rounded,
            ..LineProps::default()
        };
        let g = line_geometry(&box_frame(0.0, 0.0), &props, 2.0, true, &[]);
        assert!(g.paths.iter().all(|p| p.fill == FillRole::None));
    }

    fn page_with(shapes: Vec<Shape>) -> Page {
        let mut page = Page::new("p");
        page.layers[0].objects = shapes;
        page
    }

    fn line(a: Pt, b: Pt, jump: LineJump) -> Shape {
        let libs = crate::library::builtin_libraries();
        let mut s = Shape::new(
            libs[0].shape("line").unwrap(),
            LINE_LIBRARY,
            line_frame(a, b),
        );
        s.line.as_mut().unwrap().jump = jump;
        s
    }

    #[test]
    fn the_top_line_jumps_every_line_beneath_it() {
        let under1 = line((30.0, -50.0), (30.0, 50.0), LineJump::None);
        let under2 = line((70.0, 50.0), (70.0, -50.0), LineJump::Arc);
        let top = line((0.0, 0.0), (100.0, 0.0), LineJump::Arc);
        let ids = (under1.id, under2.id, top.id);
        let jumps = line_jumps(&page_with(vec![under1, under2, top]));
        let at = &jumps[&ids.2];
        assert_eq!(at.len(), 2);
        assert!(
            (at[0] - 30.0).abs() < 1e-3 && (at[1] - 70.0).abs() < 1e-3,
            "{at:?}"
        );
        // The lower lines jump nothing: what crosses them is above them.
        assert!(!jumps.contains_key(&ids.0) && !jumps.contains_key(&ids.1));
    }

    #[test]
    fn lines_meeting_at_an_end_do_not_jump() {
        let under = line((50.0, 0.0), (50.0, 80.0), LineJump::None);
        let top = line((0.0, 0.0), (100.0, 0.0), LineJump::Gap);
        let id = top.id;
        assert!(!line_jumps(&page_with(vec![under, top])).contains_key(&id));
    }

    #[test]
    fn each_jump_kind_hops_up_or_breaks_the_line() {
        for kind in LineJump::ALL {
            let props = LineProps {
                jump: kind,
                ..LineProps::default()
            };
            let g = line_geometry(&box_frame(100.0, 0.0), &props, 2.0, false, &[30.0, 70.0]);
            let pieces = g.paths.len();
            let lowest_y = g
                .paths
                .iter()
                .flat_map(|p| &p.cmds)
                .filter_map(|c| match *c {
                    PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(p.1),
                    _ => None,
                })
                .fold(0.0_f32, f32::min);
            match kind {
                LineJump::None => assert_eq!((pieces, lowest_y), (1, 0.0)),
                LineJump::Gap => assert_eq!((pieces, lowest_y), (3, 0.0)),
                // Hops go up the canvas, as far as the jump's reach.
                _ => {
                    assert_eq!(pieces, 1, "{kind:?}");
                    assert!((lowest_y + 4.0).abs() < 1e-3, "{kind:?} {lowest_y}");
                }
            }
        }
        // A backwards line still hops upwards.
        let props = LineProps {
            jump: LineJump::Sharp,
            ..LineProps::default()
        };
        let back = line_frame((100.0, 0.0), (0.0, 0.0));
        let g = to_canvas(&line_geometry(&back, &props, 2.0, false, &[50.0]), &back);
        let ys: Vec<f32> = g.paths[0]
            .cmds
            .iter()
            .filter_map(|c| match *c {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(p.1),
                _ => None,
            })
            .collect();
        assert!(ys.iter().all(|&y| y <= 1e-3) && ys.iter().any(|&y| y < -3.0));
    }

    #[test]
    fn jumps_near_the_ends_or_overlapping_are_left_out() {
        let props = LineProps {
            jump: LineJump::Gap,
            ..LineProps::default()
        };
        let g = line_geometry(
            &box_frame(100.0, 0.0),
            &props,
            2.0,
            false,
            &[1.0, 50.0, 52.0, 99.0],
        );
        assert_eq!(g.paths.len(), 2);
    }

    #[test]
    fn double_outlines_keep_the_fill_and_stroke_both_sides() {
        let sq = square(0.0, 0.0, 10.0).outline;
        let g = double_outline(&sq, 1.0);
        assert_eq!(g.paths[0].fill, FillRole::FillOnly);
        assert_eq!(g.paths.len(), 3);
        let outer = bounds(&Geometry {
            paths: vec![g.paths[1].clone()],
        })
        .unwrap();
        let inner = bounds(&Geometry {
            paths: vec![g.paths[2].clone()],
        })
        .unwrap();
        // One ring is a unit outside the square, the other a unit inside.
        let (big, small) = if outer.w > inner.w {
            (outer, inner)
        } else {
            (inner, outer)
        };
        assert!((big.w - 12.0).abs() < 1e-3 && (small.w - 8.0).abs() < 1e-3);
    }

    #[test]
    fn a_bent_line_passes_through_its_dragged_middle() {
        for route in LineRoute::ALL {
            for (a, b) in [((0.0, 0.0), (100.0, 40.0)), ((100.0, 40.0), (0.0, 0.0))] {
                let f = line_frame(a, b);
                let mut props = LineProps {
                    route,
                    ..LineProps::default()
                };
                let target = (70.0, -30.0);
                props.bend = Some(bend_through(&f, &props, target));
                let mid = line_midpoint(&f, &props);
                // Elbows move only their middle segment, across it.
                match route {
                    LineRoute::Angular | LineRoute::Rounded => assert!((mid.0 - 70.0).abs() < 1e-3),
                    _ => assert!(close(mid, target), "{route:?} {mid:?}"),
                }
                // The drawn route goes through that middle.
                let g = to_canvas(&line_geometry(&f, &props, 2.0, false, &[]), &f);
                assert!(
                    crate::geometry::hit_test(&g, mid, 0.5),
                    "{route:?} does not pass {mid:?}"
                );
            }
        }
    }

    #[test]
    fn dash_patterns_scale_with_the_stroke() {
        assert_eq!(LineDash::Solid.pattern(2.0), None);
        assert_eq!(LineDash::Dashed.pattern(2.0), Some([8.0, 5.0]));
        assert_eq!(LineDash::Dotted.pattern(2.0), Some([2.0, 3.0]));
    }

    #[test]
    fn props_serialize_compactly_and_read_defaults() {
        let json = serde_json::to_value(LineProps::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"start_head":"none","end_head":"none","route":"straight","jump":"none","jump_size":8.0})
        );
        let p: LineProps = serde_json::from_str(r#"{"end_head":"triangle"}"#).unwrap();
        assert_eq!(p.end_head, ArrowHead::Triangle);
        assert_eq!(p.route, LineRoute::Straight);
    }
}
