//! SVG outlines for library shapes: geometry only, normalised to 0..1.
//!
//! Library SVG files are untrusted. Only these elements contribute
//! geometry: `path`, `rect`, `circle`, `ellipse`, `line`, `polyline` and
//! `polygon`, found directly under the root `svg` or inside `g` (or nested
//! `svg`) elements. Everything else — scripts, styles, images, text, `use`,
//! `defs` and their contents — is ignored, so nothing is executed or
//! fetched. Paint attributes are ignored too: `line`/`polyline` are stroke
//! only, every other element is filled with the shape's fill.
//!
//! Limitations: of `transform`, only `translate` and `scale` are applied
//! (`rotate`, `skewX`, `skewY` and `matrix` are skipped); a nested `svg`'s
//! own viewport is ignored; `width`/`height` in units other than px are
//! treated as missing. Documents with a DTD, or with elements nested more
//! than [`MAX_NESTING`] deep, are rejected. A `path` whose data holds a
//! non-finite number or one larger than 1e6 in size (or an arc spanning
//! more than 2e6) contributes no geometry. Arc conversion cost grows with
//! the radius, so such paths are skipped.

use std::str::FromStr;

use roxmltree::Node;
use svgtypes::{
    Length, LengthUnit, PathParser, PathSegment, PointsParser, SimplePathSegment,
    SimplifyingPathParser, TransformListParser, TransformListToken, ViewBox, ViewBoxError,
};
use thiserror::Error;

use crate::geometry::{
    bounds, ellipse_cmds, map_points, rounded_rect_cmds, FillRole, Geometry, PathCmd, Pt, SubPath,
};

/// Largest SVG source accepted, in bytes.
pub const MAX_SVG_BYTES: usize = 1024 * 1024;

/// Deepest element nesting accepted. The XML parser recurses once per
/// level, using about 16 KB of stack per level in debug builds, so this
/// keeps parsing within roughly 1 MB, safe on a 2 MB thread stack.
pub const MAX_NESTING: usize = 64;

/// Largest path number accepted. Bigger values (notably arc radii) make
/// arc-to-curve conversion produce unbounded numbers of segments.
const MAX_PATH_VALUE: f64 = 1e6;

/// Why an SVG outline could not be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SvgError {
    #[error("The SVG could not be read: {0}")]
    Parse(String),
    #[error("The SVG contains no shapes to use as an outline.")]
    NoGeometry,
    #[error("The SVG has no size: its viewBox, width or height is zero, or its outline is flat.")]
    ZeroSize,
    #[error("The SVG is larger than 1 MB.")]
    TooLarge,
}

/// Reads the outline of an SVG document, normalised so its viewBox (or,
/// without one, its `width`/`height`, or failing that the outline's own
/// bounds) maps to the unit box `0..1 × 0..1`.
pub fn parse_outline(src: &str) -> Result<Geometry, SvgError> {
    if src.len() > MAX_SVG_BYTES {
        return Err(SvgError::TooLarge);
    }
    // roxmltree recurses once per nesting level with no limit of its own.
    if nesting_exceeds(src, MAX_NESTING) {
        return Err(SvgError::Parse("elements are nested too deeply".into()));
    }
    let doc = roxmltree::Document::parse(src).map_err(|e| SvgError::Parse(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(SvgError::Parse("the root element is not <svg>".into()));
    }

    let mut paths = Vec::new();
    collect(root, &mut paths);
    // Huge numbers can overflow; drop such sub-paths before measuring.
    paths.retain(|sp| sp.cmds.iter().all(cmd_is_finite));
    let raw = Geometry { paths };

    let (x, y, w, h) = match viewport(root)? {
        Some(v) => v,
        None => {
            let b = bounds(&raw).ok_or(SvgError::NoGeometry)?;
            (b.x, b.y, b.w, b.h)
        }
    };
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err(SvgError::ZeroSize);
    }
    let mut g = map_points(&raw, |(px, py)| ((px - x) / w, (py - y) / h));
    // A tiny viewport can overflow normalised values too.
    g.paths.retain(|sp| sp.cmds.iter().all(cmd_is_finite));
    if g.paths.is_empty() {
        return Err(SvgError::NoGeometry);
    }
    Ok(g)
}

/// Whether element nesting in `src` goes deeper than `max`, by a quick
/// byte scan that runs before the recursive XML parser. Comments, CDATA,
/// processing instructions and quoted attribute values are skipped; any
/// miscount can only over-count, which merely rejects the file.
fn nesting_exceeds(src: &str, max: usize) -> bool {
    let b = src.as_bytes();
    let skip_past = |from: usize, end: &[u8]| {
        b[from..]
            .windows(end.len())
            .position(|w| w == end)
            .map_or(b.len(), |j| from + j + end.len())
    };
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        i = if rest.starts_with(b"<!--") {
            skip_past(i + 4, b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            skip_past(i + 9, b"]]>")
        } else if rest.starts_with(b"<?") {
            skip_past(i + 2, b"?>")
        } else if rest.starts_with(b"<!") {
            skip_past(i + 2, b">")
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            skip_past(i + 2, b">")
        } else {
            // A start tag: find its `>` outside quoted attribute values.
            let mut j = i + 1;
            let mut quote = None;
            while j < b.len() {
                match (quote, b[j]) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, c @ (b'"' | b'\'')) => quote = Some(c),
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            if j >= b.len() || b[j - 1] != b'/' {
                depth += 1;
                if depth > max {
                    return true;
                }
            }
            j + 1
        };
    }
    false
}

/// Whether path data is safe to simplify: every number is finite and at
/// most [`MAX_PATH_VALUE`] in size, and no arc spans more than twice that
/// (radii grow to fit the chord). Only the part before any syntax error
/// is checked, as only that part is used.
fn path_is_reasonable(d: &str) -> bool {
    let ok = |v: f64| v.is_finite() && v.abs() <= MAX_PATH_VALUE;
    let (mut cur, mut start) = ((0.0f64, 0.0f64), (0.0f64, 0.0f64));
    for seg in PathParser::from(d).map_while(Result::ok) {
        let to = |abs: bool, x: f64, y: f64| if abs { (x, y) } else { (cur.0 + x, cur.1 + y) };
        let (values, end): (&[f64], (f64, f64)) = match seg {
            PathSegment::MoveTo { abs, x, y } => {
                let end = to(abs, x, y);
                start = end;
                (&[x, y], end)
            }
            PathSegment::LineTo { abs, x, y } | PathSegment::SmoothQuadratic { abs, x, y } => {
                (&[x, y], to(abs, x, y))
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                (&[x], (if abs { x } else { cur.0 + x }, cur.1))
            }
            PathSegment::VerticalLineTo { abs, y } => {
                (&[y], (cur.0, if abs { y } else { cur.1 + y }))
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => (&[x1, y1, x2, y2, x, y], to(abs, x, y)),
            PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => (&[x2, y2, x, y], to(abs, x, y)),
            PathSegment::Quadratic { abs, x1, y1, x, y } => (&[x1, y1, x, y], to(abs, x, y)),
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                x,
                y,
                ..
            } => {
                let end = to(abs, x, y);
                let chord = ((end.0 - cur.0).powi(2) + (end.1 - cur.1).powi(2)).sqrt();
                if !chord.is_finite() || chord > 2.0 * MAX_PATH_VALUE {
                    return false;
                }
                (&[rx, ry, x_axis_rotation, x, y], end)
            }
            PathSegment::ClosePath { .. } => (&[], start),
        };
        if !values.iter().all(|&v| ok(v)) {
            return false;
        }
        cur = end;
    }
    true
}

fn cmd_is_finite(cmd: &PathCmd) -> bool {
    let ok = |p: &Pt| p.0.is_finite() && p.1.is_finite();
    match cmd {
        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => ok(p),
        PathCmd::QuadTo { ctrl, to } => ok(ctrl) && ok(to),
        PathCmd::CubicTo { c1, c2, to } => ok(c1) && ok(c2) && ok(to),
        PathCmd::Close => true,
    }
}

/// `(x, y, w, h)` of the root viewport from `viewBox`, else from
/// `width`/`height`; `None` when neither is given.
fn viewport(root: Node) -> Result<Option<(f32, f32, f32, f32)>, SvgError> {
    if let Some(text) = root.attribute("viewBox") {
        return match ViewBox::from_str(text) {
            Ok(v) => Ok(Some((v.x as f32, v.y as f32, v.w as f32, v.h as f32))),
            Err(ViewBoxError::InvalidSize) => Err(SvgError::ZeroSize),
            Err(e) => Err(SvgError::Parse(format!("invalid viewBox: {e}"))),
        };
    }
    match (length(root, "width"), length(root, "height")) {
        (Some(w), Some(h)) => Ok(Some((0.0, 0.0, w, h))),
        _ => Ok(None),
    }
}

/// A plain or px length attribute as a finite number.
fn length(node: Node, name: &str) -> Option<f32> {
    let l = Length::from_str(node.attribute(name)?).ok()?;
    let v = l.number as f32;
    (matches!(l.unit, LengthUnit::None | LengthUnit::Px) && v.is_finite()).then_some(v)
}

/// A length attribute, or 0 when missing or unusable.
fn num(node: Node, name: &str) -> f32 {
    length(node, name).unwrap_or(0.0)
}

/// Scale-then-translate transform: `p ↦ (sx·x + tx, sy·y + ty)`.
#[derive(Clone, Copy)]
struct Affine {
    sx: f32,
    sy: f32,
    tx: f32,
    ty: f32,
}

impl Affine {
    const IDENTITY: Affine = Affine {
        sx: 1.0,
        sy: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// `self ∘ inner`: apply `inner` first.
    fn then_inner(self, inner: Affine) -> Affine {
        Affine {
            sx: self.sx * inner.sx,
            sy: self.sy * inner.sy,
            tx: self.sx * inner.tx + self.tx,
            ty: self.sy * inner.ty + self.ty,
        }
    }

    fn apply(self, (x, y): Pt) -> Pt {
        (self.sx * x + self.tx, self.sy * y + self.ty)
    }

    /// `self` followed (inside) by the element's `transform` attribute,
    /// keeping only its `translate` and `scale` parts.
    fn with_attribute(self, node: Node) -> Affine {
        let Some(text) = node.attribute("transform") else {
            return self;
        };
        let mut out = self;
        for token in TransformListParser::from(text) {
            let inner = match token {
                Ok(TransformListToken::Translate { tx, ty }) => Affine {
                    tx: tx as f32,
                    ty: ty as f32,
                    ..Affine::IDENTITY
                },
                Ok(TransformListToken::Scale { sx, sy }) => Affine {
                    sx: sx as f32,
                    sy: sy as f32,
                    ..Affine::IDENTITY
                },
                Ok(_) => continue,
                Err(_) => break,
            };
            out = out.then_inner(inner);
        }
        out
    }
}

/// Appends the geometry under `root` (in document order) to `out`. Walks
/// with an explicit stack rather than recursion; groups nested deeper
/// than [`MAX_NESTING`] (root = 1) are skipped.
fn collect(root: Node, out: &mut Vec<SubPath>) {
    // (element, its parent's transform, its nesting level)
    let mut stack: Vec<(Node, Affine, usize)> = Vec::new();
    push_children(&mut stack, root, Affine::IDENTITY.with_attribute(root), 1);
    while let Some((node, parent, depth)) = stack.pop() {
        let t = parent.with_attribute(node);
        if matches!(node.tag_name().name(), "svg" | "g") {
            if depth <= MAX_NESTING {
                push_children(&mut stack, node, t, depth);
            }
            continue;
        }
        if let Some((cmds, fill)) = element_cmds(node) {
            if cmds.iter().any(|c| !matches!(c, PathCmd::MoveTo(_))) {
                let local = Geometry {
                    paths: vec![SubPath { cmds, fill }],
                };
                out.extend(map_points(&local, |p| t.apply(p)).paths);
            }
        }
    }
}

/// Pushes `node`'s element children so they pop in document order.
fn push_children<'a, 'i>(
    stack: &mut Vec<(Node<'a, 'i>, Affine, usize)>,
    node: Node<'a, 'i>,
    t: Affine,
    depth: usize,
) {
    let children = node.children().filter(Node::is_element).rev();
    stack.extend(children.map(|c| (c, t, depth + 1)));
}

/// The local outline of one geometry element, if it is one and is usable.
fn element_cmds(node: Node) -> Option<(Vec<PathCmd>, FillRole)> {
    match node.tag_name().name() {
        // Unreasonable path data is skipped whole (see `path_is_reasonable`).
        "path" => node
            .attribute("d")
            .filter(|d| path_is_reasonable(d))
            .map(path_cmds)
            .map(|c| (c, FillRole::Shape)),
        "rect" => {
            let (w, h) = (num(node, "width"), num(node, "height"));
            let (rx, ry) = match (length(node, "rx"), length(node, "ry")) {
                (Some(rx), Some(ry)) => (rx, ry),
                (Some(r), None) | (None, Some(r)) => (r, r),
                (None, None) => (0.0, 0.0),
            };
            (w > 0.0 && h > 0.0).then(|| {
                let (x, y) = (num(node, "x"), num(node, "y"));
                (rounded_rect_cmds(x, y, w, h, rx, ry), FillRole::Shape)
            })
        }
        "circle" => {
            let r = num(node, "r");
            let c = (num(node, "cx"), num(node, "cy"));
            (r > 0.0).then(|| (ellipse_cmds(c, r, r), FillRole::Shape))
        }
        "ellipse" => {
            let (rx, ry) = match (length(node, "rx"), length(node, "ry")) {
                (Some(rx), Some(ry)) => (rx, ry),
                (Some(r), None) | (None, Some(r)) => (r, r),
                (None, None) => (0.0, 0.0),
            };
            let c = (num(node, "cx"), num(node, "cy"));
            (rx > 0.0 && ry > 0.0).then(|| (ellipse_cmds(c, rx, ry), FillRole::Shape))
        }
        "line" => Some((
            vec![
                PathCmd::MoveTo((num(node, "x1"), num(node, "y1"))),
                PathCmd::LineTo((num(node, "x2"), num(node, "y2"))),
            ],
            FillRole::None,
        )),
        "polyline" => node
            .attribute("points")
            .map(|p| (points_cmds(p, false), FillRole::None)),
        "polygon" => node
            .attribute("points")
            .map(|p| (points_cmds(p, true), FillRole::Shape)),
        _ => None,
    }
}

/// Path data as commands; parsing stops at the first error, keeping what
/// came before it (as SVG renderers do).
fn path_cmds(d: &str) -> Vec<PathCmd> {
    let p = |x: f64, y: f64| (x as f32, y as f32);
    SimplifyingPathParser::from(d)
        .map_while(Result::ok)
        .map(|seg| match seg {
            SimplePathSegment::MoveTo { x, y } => PathCmd::MoveTo(p(x, y)),
            SimplePathSegment::LineTo { x, y } => PathCmd::LineTo(p(x, y)),
            SimplePathSegment::Quadratic { x1, y1, x, y } => PathCmd::QuadTo {
                ctrl: p(x1, y1),
                to: p(x, y),
            },
            SimplePathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => PathCmd::CubicTo {
                c1: p(x1, y1),
                c2: p(x2, y2),
                to: p(x, y),
            },
            SimplePathSegment::ClosePath => PathCmd::Close,
        })
        .collect()
}

/// A `points` list as a polyline, closed when `close` is set.
fn points_cmds(points: &str, close: bool) -> Vec<PathCmd> {
    let mut cmds: Vec<PathCmd> = PointsParser::from(points)
        .enumerate()
        .map(|(i, (x, y))| {
            let p = (x as f32, y as f32);
            if i == 0 {
                PathCmd::MoveTo(p)
            } else {
                PathCmd::LineTo(p)
            }
        })
        .collect();
    if close && cmds.len() > 1 {
        cmds.push(PathCmd::Close);
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{bounds, FillRole, PathCmd};

    #[test]
    fn parses_path_and_normalises() {
        let g = parse_outline(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100">
                 <path d="M0 0 L200 100"/>
               </svg>"#,
        )
        .unwrap();
        assert_eq!(g.paths.len(), 1);
        assert_eq!(g.paths[0].fill, FillRole::Shape);
        assert_eq!(
            g.paths[0].cmds,
            vec![PathCmd::MoveTo((0.0, 0.0)), PathCmd::LineTo((1.0, 1.0))]
        );
    }

    #[test]
    fn rect_and_circle_become_paths() {
        let g = parse_outline(
            r#"<svg width="100" height="100">
                 <rect x="0" y="0" width="50" height="50"/>
                 <g transform="translate(50 50)"><circle cx="25" cy="25" r="25"/></g>
                 <polyline points="0,100 100,100"/>
               </svg>"#,
        )
        .unwrap();
        assert_eq!(g.paths.len(), 3);
        assert_eq!(g.paths[0].fill, FillRole::Shape);
        assert_eq!(g.paths[1].fill, FillRole::Shape);
        assert_eq!(g.paths[2].fill, FillRole::None);
        let b = bounds(&g).unwrap();
        assert!((b.w - 1.0).abs() < 1e-4 && (b.h - 1.0).abs() < 1e-4);
        let circle = bounds(&crate::geometry::Geometry {
            paths: vec![g.paths[1].clone()],
        })
        .unwrap();
        assert!((circle.x - 0.5).abs() < 1e-4 && (circle.y - 0.5).abs() < 1e-4);
    }

    #[test]
    fn svg_ignores_script_and_style() {
        let g = parse_outline(
            r#"<svg viewBox="0 0 10 10">
                 <style>rect { fill: red }</style>
                 <script>alert(1)</script>
                 <defs><rect width="5" height="5"/></defs>
                 <image href="https://example.com/x.png" width="10" height="10"/>
                 <text x="1" y="1">hello</text>
                 <rect width="10" height="10"/>
               </svg>"#,
        )
        .unwrap();
        assert_eq!(g.paths.len(), 1);
    }

    #[test]
    fn zero_size_viewbox_is_error() {
        let src = r#"<svg viewBox="0 0 0 100"><rect width="1" height="1"/></svg>"#;
        assert_eq!(parse_outline(src), Err(SvgError::ZeroSize));
        let src = r#"<svg width="0" height="10"><rect width="1" height="1"/></svg>"#;
        assert_eq!(parse_outline(src), Err(SvgError::ZeroSize));
    }

    #[test]
    fn no_geometry_is_error() {
        let src = r#"<svg viewBox="0 0 10 10"><text>hi</text></svg>"#;
        assert_eq!(parse_outline(src), Err(SvgError::NoGeometry));
        // Overflowing coordinates are dropped rather than kept as NaN/inf.
        let src = r#"<svg viewBox="0 0 10 10"><path d="M0 0 L1e300 1"/></svg>"#;
        assert_eq!(parse_outline(src), Err(SvgError::NoGeometry));
        assert!(matches!(parse_outline("not xml"), Err(SvgError::Parse(_))));
        assert!(matches!(
            parse_outline("<html><rect width='1' height='1'/></html>"),
            Err(SvgError::Parse(_))
        ));
    }

    #[test]
    fn rejects_oversized_input() {
        let padding = " ".repeat(MAX_SVG_BYTES);
        let src = format!(r#"<svg viewBox="0 0 1 1"><rect width="1" height="1"/>{padding}</svg>"#);
        assert_eq!(parse_outline(&src), Err(SvgError::TooLarge));
    }

    /// Runs `f` on a thread with a 512 KB stack.
    fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        on_stack(512 * 1024, f)
    }

    fn on_stack<T: Send + 'static>(bytes: usize, f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(bytes)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    fn nested(depth: usize) -> String {
        format!(
            r#"<svg viewBox="0 0 1 1">{}<rect width="1" height="1"/>{}</svg>"#,
            "<g>".repeat(depth),
            "</g>".repeat(depth)
        )
    }

    #[test]
    fn deep_nesting_is_rejected_without_deep_recursion() {
        let result = on_small_stack(|| parse_outline(&nested(10_000)));
        assert!(matches!(result, Err(SvgError::Parse(_))), "{result:?}");
        // Quotes, comments and self-closing tags do not fool the depth scan.
        let tricky = format!(
            r#"<svg viewBox="0 0 1 1">{}<rect width="1" height="1"/></svg>"#,
            r#"<g a="/>"><!-- </g> --><g/>"#.repeat(10_000)
        );
        let result = on_small_stack(move || parse_outline(&tricky));
        assert!(matches!(result, Err(SvgError::Parse(_))), "{result:?}");
    }

    #[test]
    fn deepest_accepted_nesting_fits_a_2mb_stack() {
        // The root svg plus MAX_NESTING - 1 groups (the rect is self-closing).
        let g = on_stack(2 * 1024 * 1024, || parse_outline(&nested(MAX_NESTING - 1))).unwrap();
        assert_eq!(g.paths.len(), 1);
        let result = parse_outline(&nested(MAX_NESTING));
        assert!(matches!(result, Err(SvgError::Parse(_))), "{result:?}");
    }

    #[test]
    fn moderate_nesting_is_accepted() {
        let g = on_small_stack(|| parse_outline(&nested(16))).unwrap();
        assert_eq!(g.paths.len(), 1);
    }

    #[test]
    fn huge_arc_is_skipped_quickly() {
        let start = std::time::Instant::now();
        let src = r#"<svg viewBox="0 0 10 10">
                       <path d="M0 0 A1e36 1e36 0 1 1 1e35 0"/>
                       <path d="M1e6 0 l1e6 0 l1e6 0 A1 1 0 0 1 0 0"/>
                       <rect width="1" height="1"/>
                     </svg>"#;
        let g = parse_outline(src).unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        // The second path's numbers are in range, but relative moves carry
        // the arc's start 3e6 from its end, beyond the chord limit.
        assert_eq!(g.paths.len(), 1, "only the rect survives");
    }

    #[test]
    fn arc_becomes_cubics() {
        let g = parse_outline(r#"<svg viewBox="0 0 2 2"><path d="M0 1 A1 1 0 0 1 2 1Z"/></svg>"#)
            .unwrap();
        let cmds = &g.paths[0].cmds;
        assert!(cmds.iter().any(|c| matches!(c, PathCmd::CubicTo { .. })));
        assert!(cmds
            .iter()
            .all(|c| !matches!(c, PathCmd::LineTo(_) | PathCmd::QuadTo { .. })));
        let b = bounds(&g).unwrap();
        assert!((b.x - 0.0).abs() < 1e-3 && (b.w - 1.0).abs() < 1e-3);
    }

    #[test]
    fn no_size_falls_back_to_outline_bounds() {
        let g =
            parse_outline(r#"<svg><rect x="10" y="20" width="40" height="10"/></svg>"#).unwrap();
        let b = bounds(&g).unwrap();
        assert!(b.x.abs() < 1e-5 && b.y.abs() < 1e-5);
        assert!((b.w - 1.0).abs() < 1e-5 && (b.h - 1.0).abs() < 1e-5);
        // A lone horizontal line has no height to normalise by.
        let src = r#"<svg><line x1="0" y1="5" x2="10" y2="5"/></svg>"#;
        assert_eq!(parse_outline(src), Err(SvgError::ZeroSize));
        // Non-finite sub-paths do not poison the fallback bounds.
        let src = r#"<svg><path d="M0 0 L1e300 1e300"/><rect width="4" height="2"/></svg>"#;
        let g = parse_outline(src).unwrap();
        assert_eq!(g.paths.len(), 1);
        let b = bounds(&g).unwrap();
        assert!((b.w - 1.0).abs() < 1e-5 && (b.h - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rect_radii_clamp_per_axis() {
        let g = parse_outline(
            r#"<svg viewBox="0 0 100 20"><rect width="100" height="20" rx="30"/></svg>"#,
        )
        .unwrap();
        let cmds = &g.paths[0].cmds;
        // rx stays 30 (≤ w/2); ry is clamped to h/2 = 10.
        assert_eq!(cmds[0], PathCmd::MoveTo((0.3, 0.0)));
        assert!(matches!(cmds[2], PathCmd::CubicTo { to, .. } if to == (1.0, 0.5)));
    }
}
