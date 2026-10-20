//! Viewport transform and adaptive grid maths (pure f32 tuples, no gpui).

use crate::document::Viewport;

pub const ZOOM_MIN: f32 = 0.1;
pub const ZOOM_MAX: f32 = 10.0;
pub const ZOOM_STEP: f32 = 1.25;

/// Minimum on-screen distance between grid lines, in pixels.
const MIN_SPACING_PX: f32 = 8.0;

impl Viewport {
    /// Canvas point under the given screen point.
    pub fn screen_to_canvas(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0 / self.zoom + self.x, p.1 / self.zoom + self.y)
    }

    /// Screen point at which the given canvas point is shown.
    pub fn canvas_to_screen(&self, p: (f32, f32)) -> (f32, f32) {
        ((p.0 - self.x) * self.zoom, (p.1 - self.y) * self.zoom)
    }

    /// Moves the content with the pointer by the given screen delta.
    pub fn pan_by(&mut self, screen_dx: f32, screen_dy: f32) {
        self.x -= screen_dx / self.zoom;
        self.y -= screen_dy / self.zoom;
    }

    /// Multiplies zoom by `factor` (clamped), keeping the canvas point under
    /// `screen_anchor` fixed on screen.
    pub fn zoom_at(&mut self, factor: f32, screen_anchor: (f32, f32)) {
        let anchor = self.screen_to_canvas(screen_anchor);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.x = anchor.0 - screen_anchor.0 / self.zoom;
        self.y = anchor.1 - screen_anchor.1 / self.zoom;
    }

    pub fn zoom_percent(&self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }
}

/// Grid line positions in screen coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct GridLines {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    pub spacing_px: f32,
    pub alpha: f32,
}

/// Computes grid lines for a canvas area of `size` screen pixels. Spacing is
/// doubled until lines are at least 8px apart so the count stays bounded.
/// Returns no lines for degenerate (non-finite or non-positive) input.
pub fn grid_lines(vp: &Viewport, size: (f32, f32), grid_size: f32) -> GridLines {
    let valid = |v: f32| v.is_finite() && v > 0.0;
    let empty = GridLines {
        xs: Vec::new(),
        ys: Vec::new(),
        spacing_px: 0.0,
        alpha: 0.0,
    };
    if !valid(grid_size)
        || !valid(vp.zoom)
        || !valid(size.0)
        || !valid(size.1)
        || !vp.x.is_finite()
        || !vp.y.is_finite()
    {
        return empty;
    }

    let mut canvas_spacing = grid_size;
    while canvas_spacing * vp.zoom < MIN_SPACING_PX {
        canvas_spacing *= 2.0;
    }
    let spacing_px = canvas_spacing * vp.zoom;
    if !spacing_px.is_finite() {
        return empty;
    }
    let alpha = ((spacing_px - MIN_SPACING_PX) / 8.0).clamp(0.3, 1.0);

    let lines = |origin: f32, extent: f32| {
        let mut out = Vec::new();
        let mut i = (origin / canvas_spacing).ceil();
        loop {
            let screen = (i * canvas_spacing - origin) * vp.zoom;
            if screen > extent {
                break;
            }
            out.push(screen);
            i += 1.0;
        }
        out
    };

    GridLines {
        xs: lines(vp.x, size.0),
        ys: lines(vp.y, size.1),
        spacing_px,
        alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(x: f32, y: f32, zoom: f32) -> Viewport {
        Viewport { x, y, zoom }
    }

    #[test]
    fn round_trip_screen_canvas() {
        let v = vp(12.5, -40.0, 2.5);
        let p = (123.0, 77.0);
        let back = v.canvas_to_screen(v.screen_to_canvas(p));
        assert!((back.0 - p.0).abs() < 1e-3 && (back.1 - p.1).abs() < 1e-3);
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut v = vp(30.0, 10.0, 1.0);
        let before = v.screen_to_canvas((300.0, 200.0));
        v.zoom_at(2.0, (300.0, 200.0));
        let after = v.screen_to_canvas((300.0, 200.0));
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        assert_eq!(v.zoom, 2.0);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = vp(0.0, 0.0, 1.0);
        v.zoom_at(1000.0, (0.0, 0.0));
        assert_eq!(v.zoom, 10.0);
        v.zoom_at(0.0001, (0.0, 0.0));
        assert_eq!(v.zoom, 0.1);
    }

    #[test]
    fn pan_moves_content() {
        let mut v = vp(0.0, 0.0, 2.0);
        let before = v.canvas_to_screen((50.0, 50.0));
        v.pan_by(10.0, 0.0);
        let after = v.canvas_to_screen((50.0, 50.0));
        assert!((after.0 - before.0 - 10.0).abs() < 1e-3);
        assert!((after.1 - before.1).abs() < 1e-3);
    }

    #[test]
    fn grid_lines_aligned_to_canvas_multiples() {
        let g = grid_lines(&vp(5.0, 0.0, 1.0), (100.0, 100.0), 20.0);
        assert_eq!(g.xs, vec![15.0, 35.0, 55.0, 75.0, 95.0]);
        assert_eq!(g.spacing_px, 20.0);
    }

    #[test]
    fn grid_lines_bounded_at_min_zoom() {
        let g = grid_lines(&vp(0.0, 0.0, 0.1), (4000.0, 3000.0), 4.0);
        assert!(g.xs.len() <= 4000 / 8 + 2);
        assert!(g.spacing_px >= 8.0);
    }

    #[test]
    fn grid_lines_empty_for_degenerate_input() {
        let size = (100.0, 100.0);
        for (z, gs) in [
            (0.0, 20.0),
            (-1.0, 20.0),
            (f32::NAN, 20.0),
            (f32::INFINITY, 20.0),
            (1.0, 0.0),
            (1.0, -5.0),
            (1.0, f32::NAN),
            (1.0, f32::INFINITY),
        ] {
            let g = grid_lines(&vp(0.0, 0.0, z), size, gs);
            assert!(g.xs.is_empty() && g.ys.is_empty(), "zoom {z} grid {gs}");
        }
        let g = grid_lines(&vp(0.0, 0.0, 1.0), (f32::NAN, 0.0), 20.0);
        assert!(g.xs.is_empty() && g.ys.is_empty());
        let g = grid_lines(&vp(0.0, 0.0, 1.0), (f32::INFINITY, 100.0), 20.0);
        assert!(g.xs.is_empty() && g.ys.is_empty());
    }

    #[test]
    fn zoom_percent_rounds() {
        assert_eq!(vp(0.0, 0.0, 1.255).zoom_percent(), 126);
    }
}
