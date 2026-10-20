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
//! treated as missing. Documents with a DTD are rejected.

use std::str::FromStr;

use roxmltree::Node;
use svgtypes::{
    Length, LengthUnit, PointsParser, SimplePathSegment, SimplifyingPathParser,
    TransformListParser, TransformListToken, ViewBox, ViewBoxError,
};
use thiserror::Error;

use crate::geometry::{
    bounds, ellipse_cmds, map_points, rounded_rect_cmds, FillRole, Geometry, PathCmd, Pt, SubPath,
};

/// Largest SVG source accepted, in bytes.
pub const MAX_SVG_BYTES: usize = 1024 * 1024;

/// Why an SVG outline could not be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SvgError {
    #[error("The SVG could not be read: {0}")]
    Parse(String),
    #[error("The SVG contains no shapes to use as an outline.")]
    NoGeometry,
    #[error("The SVG has no size: its viewBox, width or height is zero.")]
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
    let doc = roxmltree::Document::parse(src).map_err(|e| SvgError::Parse(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(SvgError::Parse("the root element is not <svg>".into()));
    }

    let mut paths = Vec::new();
    collect(root, Affine::IDENTITY, &mut paths);
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
    // Huge or degenerate numbers can overflow; drop such sub-paths.
    g.paths.retain(|sp| sp.cmds.iter().all(cmd_is_finite));
    if g.paths.is_empty() {
        return Err(SvgError::NoGeometry);
    }
    Ok(g)
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

/// Appends the geometry of `node`'s children (in document order) to `out`.
fn collect(node: Node, transform: Affine, out: &mut Vec<SubPath>) {
    for child in node.children().filter(Node::is_element) {
        let t = transform.with_attribute(child);
        let shape = match child.tag_name().name() {
            "svg" | "g" => {
                collect(child, t, out);
                continue;
            }
            "path" => child
                .attribute("d")
                .map(path_cmds)
                .map(|c| (c, FillRole::Shape)),
            "rect" => {
                let (w, h) = (num(child, "width"), num(child, "height"));
                let (rx, ry) = match (length(child, "rx"), length(child, "ry")) {
                    (Some(rx), Some(ry)) => (rx, ry),
                    (Some(r), None) | (None, Some(r)) => (r, r),
                    (None, None) => (0.0, 0.0),
                };
                (w > 0.0 && h > 0.0).then(|| {
                    let (x, y) = (num(child, "x"), num(child, "y"));
                    (rounded_rect_cmds(x, y, w, h, rx, ry), FillRole::Shape)
                })
            }
            "circle" => {
                let r = num(child, "r");
                let c = (num(child, "cx"), num(child, "cy"));
                (r > 0.0).then(|| (ellipse_cmds(c, r, r), FillRole::Shape))
            }
            "ellipse" => {
                let (rx, ry) = match (length(child, "rx"), length(child, "ry")) {
                    (Some(rx), Some(ry)) => (rx, ry),
                    (Some(r), None) | (None, Some(r)) => (r, r),
                    (None, None) => (0.0, 0.0),
                };
                let c = (num(child, "cx"), num(child, "cy"));
                (rx > 0.0 && ry > 0.0).then(|| (ellipse_cmds(c, rx, ry), FillRole::Shape))
            }
            "line" => Some((
                vec![
                    PathCmd::MoveTo((num(child, "x1"), num(child, "y1"))),
                    PathCmd::LineTo((num(child, "x2"), num(child, "y2"))),
                ],
                FillRole::None,
            )),
            "polyline" => child
                .attribute("points")
                .map(|p| (points_cmds(p, false), FillRole::None)),
            "polygon" => child
                .attribute("points")
                .map(|p| (points_cmds(p, true), FillRole::Shape)),
            _ => None,
        };
        if let Some((cmds, fill)) = shape {
            if cmds.iter().any(|c| !matches!(c, PathCmd::MoveTo(_))) {
                let local = Geometry {
                    paths: vec![SubPath { cmds, fill }],
                };
                out.extend(map_points(&local, |p| t.apply(p)).paths);
            }
        }
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
}
