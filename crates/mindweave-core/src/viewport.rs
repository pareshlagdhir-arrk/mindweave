//! Viewport transform and adaptive grid maths (pure f32 tuples, no gpui).

use crate::document::Viewport;

pub const ZOOM_MIN: f32 = 0.1;
pub const ZOOM_MAX: f32 = 10.0;
pub const ZOOM_STEP: f32 = 1.25;

/// Minimum on-screen distance between grid lines, in pixels.
const MIN_SPACING_PX: f32 = 8.0;

/// Hard cap on lines per axis, so huge extents cannot allocate unboundedly.
const MAX_LINES_PER_AXIS: f64 = 10_000.0;

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
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        self.zoom_to(self.zoom * factor, screen_anchor);
    }

    /// Sets zoom to `zoom` (clamped), keeping the canvas point under
    /// `screen_anchor` fixed on screen. Invalid input is ignored.
    pub fn zoom_to(&mut self, zoom: f32, screen_anchor: (f32, f32)) {
        if !zoom.is_finite()
            || zoom <= 0.0
            || !screen_anchor.0.is_finite()
            || !screen_anchor.1.is_finite()
        {
            return;
        }
        let anchor = self.screen_to_canvas(screen_anchor);
        self.zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
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
    /// Canvas grid index (multiple of the current spacing) of `xs[0]` and
    /// `ys[0]`, so a thinned pattern can stay anchored to the canvas.
    pub first_ix: (i64, i64),
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
        first_ix: (0, 0),
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
        let spacing = f64::from(spacing_px);
        let first = (f64::from(origin) / f64::from(canvas_spacing)).ceil();
        let start = (first * f64::from(canvas_spacing) - f64::from(origin)) * f64::from(vp.zoom);
        if start > f64::from(extent) {
            return (Vec::new(), first as i64);
        }
        let count = (((f64::from(extent) - start) / spacing).floor() + 1.0).min(MAX_LINES_PER_AXIS);
        let positions = (0..count as usize)
            .map(|k| (start + k as f64 * spacing) as f32)
            .collect();
        (positions, first as i64)
    };

    let (xs, first_x) = lines(vp.x, size.0);
    let (ys, first_y) = lines(vp.y, size.1);
    GridLines {
        xs,
        ys,
        spacing_px,
        alpha,
        first_ix: (first_x, first_y),
    }
}

/// Smallest and largest zoom factor a single scroll or pinch event applies.
const EVENT_ZOOM_MIN: f32 = 0.5;
const EVENT_ZOOM_MAX: f32 = 2.0;

fn event_zoom_factor(fraction: f32) -> f32 {
    if fraction.is_nan() {
        return 1.0;
    }
    (1.0 + fraction).clamp(EVENT_ZOOM_MIN, EVENT_ZOOM_MAX)
}

/// Zoom factor for a Cmd+scroll of `dy` screen pixels: 1% per pixel,
/// clamped to [0.5, 2.0]. NaN gives 1.0 (no change).
pub fn scroll_zoom_factor(dy: f32) -> f32 {
    event_zoom_factor(dy * 0.01)
}

/// Zoom factor for a trackpad pinch whose magnification is `delta`
/// (0.1 = 10% larger), clamped to [0.5, 2.0]. NaN gives 1.0.
pub fn pinch_zoom_factor(delta: f32) -> f32 {
    event_zoom_factor(delta)
}

/// How many grid lines to step per dot so that a dot pattern of `nx` x `ny`
/// intersections draws at most `max_dots` dots: 1 when it already fits,
/// otherwise the smallest power of two that fits (or that leaves one dot).
pub fn dot_stride(nx: usize, ny: usize, max_dots: usize) -> usize {
    let longest = nx.max(ny).max(1);
    let mut stride = 1usize;
    while nx.div_ceil(stride).saturating_mul(ny.div_ceil(stride)) > max_dots && stride < longest {
        stride *= 2;
    }
    stride
}

/// How many lines to skip before stepping by `stride`, so that the kept
/// lines are the canvas lines whose grid index is a multiple of `stride`
/// (given the index `first_ix` of the first line). Keeps a thinned pattern
/// fixed to the canvas while panning. A zero stride is treated as 1.
pub fn stride_offset(first_ix: i64, stride: usize) -> usize {
    let stride = stride.max(1) as i64;
    first_ix.wrapping_neg().rem_euclid(stride) as usize
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
    fn grid_lines_terminates_at_huge_offset() {
        let g = grid_lines(&vp(1e9, 1e9, 1.0), (1000.0, 800.0), 20.0);
        assert!(g.xs.len() <= 1000 / 8 + 2);
        assert!(g.ys.len() <= 800 / 8 + 2);
        assert!(!g.xs.is_empty());
    }

    #[test]
    fn grid_lines_capped_for_huge_size() {
        let g = grid_lines(&vp(0.0, 0.0, 1.0), (1e9, 1e9), 20.0);
        assert!(g.xs.len() <= 10_000 && g.ys.len() <= 10_000);
        assert_eq!(g.xs.len(), 10_000);
    }

    #[test]
    fn zoom_at_ignores_invalid_input() {
        let mut v = vp(3.0, 4.0, 1.5);
        for (f, a) in [
            (f32::NAN, (0.0, 0.0)),
            (0.0, (0.0, 0.0)),
            (-2.0, (0.0, 0.0)),
            (f32::INFINITY, (0.0, 0.0)),
            (2.0, (f32::NAN, 0.0)),
            (2.0, (0.0, f32::INFINITY)),
        ] {
            v.zoom_at(f, a);
            assert_eq!(v, vp(3.0, 4.0, 1.5));
        }
    }

    #[test]
    fn zoom_to_is_exact_and_keeps_anchor_fixed() {
        let mut v = vp(30.0, 10.0, 1.25 * 1.25 * 1.25);
        let before = v.screen_to_canvas((300.0, 200.0));
        v.zoom_to(1.0, (300.0, 200.0));
        assert_eq!(v.zoom, 1.0);
        let after = v.screen_to_canvas((300.0, 200.0));
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
    }

    #[test]
    fn zoom_to_clamps_and_ignores_invalid_input() {
        let mut v = vp(3.0, 4.0, 1.5);
        for (z, a) in [
            (f32::NAN, (0.0, 0.0)),
            (0.0, (0.0, 0.0)),
            (-1.0, (0.0, 0.0)),
            (2.0, (f32::NAN, 0.0)),
        ] {
            v.zoom_to(z, a);
            assert_eq!(v, vp(3.0, 4.0, 1.5));
        }
        v.zoom_to(50.0, (0.0, 0.0));
        assert_eq!(v.zoom, ZOOM_MAX);
    }

    #[test]
    fn zoom_percent_rounds() {
        assert_eq!(vp(0.0, 0.0, 1.255).zoom_percent(), 126);
    }

    #[test]
    fn scroll_zoom_factor_is_one_percent_per_pixel() {
        assert!((scroll_zoom_factor(0.0) - 1.0).abs() < 1e-6);
        assert!((scroll_zoom_factor(10.0) - 1.1).abs() < 1e-6);
        assert!((scroll_zoom_factor(-10.0) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn scroll_zoom_factor_is_clamped() {
        assert_eq!(scroll_zoom_factor(500.0), 2.0);
        assert_eq!(scroll_zoom_factor(-500.0), 0.5);
        assert_eq!(scroll_zoom_factor(f32::INFINITY), 2.0);
        assert_eq!(scroll_zoom_factor(f32::NEG_INFINITY), 0.5);
    }

    #[test]
    fn scroll_zoom_factor_ignores_nan() {
        assert_eq!(scroll_zoom_factor(f32::NAN), 1.0);
    }

    #[test]
    fn pinch_zoom_factor_uses_magnification_directly() {
        assert!((pinch_zoom_factor(0.1) - 1.1).abs() < 1e-6);
        assert!((pinch_zoom_factor(-0.1) - 0.9).abs() < 1e-6);
        assert_eq!(pinch_zoom_factor(5.0), 2.0);
        assert_eq!(pinch_zoom_factor(-5.0), 0.5);
        assert_eq!(pinch_zoom_factor(f32::NAN), 1.0);
    }

    #[test]
    fn dot_stride_is_one_when_under_the_cap() {
        assert_eq!(dot_stride(100, 100, 40_000), 1);
        assert_eq!(dot_stride(200, 200, 40_000), 1);
        assert_eq!(dot_stride(0, 0, 40_000), 1);
    }

    #[test]
    fn dot_stride_doubles_until_dots_fit() {
        // 500 x 375 = 187_500 dots; every 2nd: 250 x 188 = 47_000; every 4th: 125 x 94.
        assert_eq!(dot_stride(500, 375, 40_000), 4);
        let s = dot_stride(10_000, 10_000, 40_000);
        assert!(10_000usize.div_ceil(s).pow(2) <= 40_000);
        assert!(10_000usize.div_ceil(s / 2).pow(2) > 40_000);
    }

    /// Canvas grid indices of the lines kept when every `stride`-th line is
    /// drawn, anchored with `stride_offset`.
    fn kept_indices(first_ix: i64, count: usize, stride: usize) -> Vec<i64> {
        (stride_offset(first_ix, stride)..count)
            .step_by(stride)
            .map(|k| first_ix + k as i64)
            .collect()
    }

    #[test]
    fn grid_lines_report_the_first_line_index() {
        let g = grid_lines(&vp(5.0, -45.0, 1.0), (100.0, 100.0), 20.0);
        // First visible lines: x = 20 (index 1), y = -40 (index -2).
        assert_eq!(g.first_ix, (1, -2));
    }

    #[test]
    fn stride_offset_selects_global_multiples() {
        assert_eq!(stride_offset(0, 4), 0);
        assert_eq!(stride_offset(1, 4), 3);
        assert_eq!(stride_offset(4, 4), 0);
        assert_eq!(stride_offset(-1, 4), 1);
        assert_eq!(stride_offset(-5, 4), 1);
        assert_eq!(stride_offset(7, 1), 0);
        assert_eq!(stride_offset(7, 0), 0);
    }

    #[test]
    fn thinned_lines_stay_on_the_same_canvas_lines_while_panning() {
        let stride = 4;
        let before = grid_lines(&vp(0.0, 0.0, 1.0), (400.0, 400.0), 20.0);
        // Pan by exactly one spacing: the first visible line changes.
        let after = grid_lines(&vp(20.0, 0.0, 1.0), (400.0, 400.0), 20.0);
        assert_ne!(before.first_ix.0, after.first_ix.0);
        let a = kept_indices(before.first_ix.0, before.xs.len(), stride);
        let b = kept_indices(after.first_ix.0, after.xs.len(), stride);
        assert!(a.iter().all(|i| i.rem_euclid(stride as i64) == 0));
        assert!(b.iter().all(|i| i.rem_euclid(stride as i64) == 0));
        // Every canvas line kept in both frames' overlap is kept in both.
        let overlap = |v: &[i64]| -> Vec<i64> {
            v.iter().copied().filter(|i| (1..=20).contains(i)).collect()
        };
        assert_eq!(overlap(&a), overlap(&b));
        assert!(!overlap(&a).is_empty());
    }

    #[test]
    fn dot_stride_survives_a_zero_cap() {
        assert!(dot_stride(10, 10, 0) >= 1);
    }
}
