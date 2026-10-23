//! Path shapes: free outlines that belong to no library, such as the
//! elements of an imported SVG. They are shapes of
//! [`crate::shape::TEXT_LIBRARY`] and shape [`PATH_SHAPE`]; their outline is
//! SVG path data in the shape's unit box, so it scales with the frame.

use serde::{Deserialize, Serialize};

use crate::geometry::{map_points, FillRole, Geometry, PathCmd, Pt, Rect, SubPath};
use crate::svg::{path_cmds, path_is_reasonable};

/// Shape id of a path shape in [`crate::shape::TEXT_LIBRARY`].
pub const PATH_SHAPE: &str = "path";

/// Decimal places kept of unit-box coordinates in path data.
const PATH_PRECISION: f32 = 1e5;

/// The outline of a path shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PathProps {
    /// SVG path data in the shape's unit box: `0..1` on both axes spans
    /// the frame.
    pub d: String,
    /// Filled by the even-odd rule rather than non-zero.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub even_odd: bool,
}

impl PathProps {
    /// The outline in the local box `0..w × 0..h`; empty when the path
    /// data holds nothing usable (see [`crate::svg`] for what is refused).
    pub fn local_geometry(&self, w: f32, h: f32) -> Geometry {
        if !path_is_reasonable(&self.d) {
            return Geometry::default();
        }
        let cmds = path_cmds(&self.d);
        if cmds.is_empty() {
            return Geometry::default();
        }
        let fill = if self.even_odd {
            FillRole::Shape
        } else {
            FillRole::NonZero
        };
        let unit = Geometry {
            paths: vec![SubPath { cmds, fill }],
        };
        map_points(&unit, |(x, y)| (x * w, y * h))
    }

    /// The path data of `cmds` (canvas coordinates) relative to `bounds`,
    /// which becomes the unit box. A side of no extent maps to 0.
    pub fn from_cmds(cmds: &[PathCmd], bounds: &Rect, even_odd: bool) -> PathProps {
        let unit = |(x, y): Pt| {
            let along = |v: f32, start: f32, len: f32| {
                if len > f32::EPSILON {
                    (v - start) / len
                } else {
                    0.0
                }
            };
            (along(x, bounds.x, bounds.w), along(y, bounds.y, bounds.h))
        };
        let mut d = String::new();
        let mut push = |letter: char, points: &[Pt]| {
            if !d.is_empty() {
                d.push(' ');
            }
            d.push(letter);
            for &p in points {
                let (x, y) = unit(p);
                d.push_str(&format!(" {} {}", number(x), number(y)));
            }
        };
        for cmd in cmds {
            match *cmd {
                PathCmd::MoveTo(p) => push('M', &[p]),
                PathCmd::LineTo(p) => push('L', &[p]),
                PathCmd::QuadTo { ctrl, to } => push('Q', &[ctrl, to]),
                PathCmd::CubicTo { c1, c2, to } => push('C', &[c1, c2, to]),
                PathCmd::Close => push('Z', &[]),
            }
        }
        PathProps { d, even_odd }
    }
}

/// `v` rounded to [`PATH_PRECISION`], written as briefly as it reads back.
fn number(v: f32) -> String {
    let r = (v * PATH_PRECISION).round() / PATH_PRECISION;
    // No "-0".
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{r}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::bounds;

    fn square() -> Vec<PathCmd> {
        vec![
            PathCmd::MoveTo((10.0, 20.0)),
            PathCmd::LineTo((30.0, 20.0)),
            PathCmd::CubicTo {
                c1: (30.0, 30.0),
                c2: (30.0, 40.0),
                to: (30.0, 60.0),
            },
            PathCmd::LineTo((10.0, 60.0)),
            PathCmd::Close,
        ]
    }

    #[test]
    fn path_data_fills_the_unit_box_and_scales_with_the_frame() {
        let r = Rect {
            x: 10.0,
            y: 20.0,
            w: 20.0,
            h: 40.0,
        };
        let p = PathProps::from_cmds(&square(), &r, false);
        assert_eq!(p.d, "M 0 0 L 1 0 C 1 0.25 1 0.5 1 1 L 0 1 Z");
        let g = p.local_geometry(200.0, 50.0);
        assert_eq!(g.paths[0].fill, FillRole::NonZero);
        assert_eq!(
            bounds(&g),
            Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 50.0
            })
        );
        let even = PathProps::from_cmds(&square(), &r, true);
        assert_eq!(even.local_geometry(1.0, 1.0).paths[0].fill, FillRole::Shape);
    }

    #[test]
    fn a_flat_side_maps_to_zero_and_bad_data_draws_nothing() {
        let line = [PathCmd::MoveTo((5.0, 7.0)), PathCmd::LineTo((15.0, 7.0))];
        let r = Rect {
            x: 5.0,
            y: 7.0,
            w: 10.0,
            h: 0.0,
        };
        assert_eq!(PathProps::from_cmds(&line, &r, false).d, "M 0 0 L 1 0");
        let bad = PathProps {
            d: "M 0 0 L 1e30 0".into(),
            even_odd: false,
        };
        assert!(bad.local_geometry(10.0, 10.0).paths.is_empty());
        assert!(PathProps::default()
            .local_geometry(10.0, 10.0)
            .paths
            .is_empty());
    }
}
