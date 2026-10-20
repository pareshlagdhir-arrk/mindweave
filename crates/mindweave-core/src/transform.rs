//! Shape transforms for canvas gestures: drag-to-create, move, resize,
//! rotate, marquee selection and handle placement (pure f32, no gpui).
//!
//! Coordinates follow [`crate::geometry`]: y grows downward and rotations
//! are degrees clockwise on screen about the centre of a frame's box.

use crate::geometry::{Frame, Pt, Rect};

/// Smallest width or height a resize or a drawn box can produce.
pub const MIN_SIZE: f32 = 1.0;
/// Pointer travel below which a press counts as a click, not a drag, in
/// screen pixels.
pub const CLICK_DRAG_THRESHOLD: f32 = 3.0;
/// Offset of duplicated or pasted shapes on each axis.
pub const DUPLICATE_OFFSET: f32 = 10.0;

/// A resize handle on a box: edge midpoints and corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handle {
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

impl Handle {
    /// All handles, clockwise from the top edge.
    pub const ALL: [Handle; 8] = [
        Handle::N,
        Handle::NE,
        Handle::E,
        Handle::SE,
        Handle::S,
        Handle::SW,
        Handle::W,
        Handle::NW,
    ];

    /// Which sides the handle drags: `-1` left/top, `1` right/bottom,
    /// `0` neither, per axis of the box.
    fn sides(self) -> (f32, f32) {
        match self {
            Handle::N => (0.0, -1.0),
            Handle::NE => (1.0, -1.0),
            Handle::E => (1.0, 0.0),
            Handle::SE => (1.0, 1.0),
            Handle::S => (0.0, 1.0),
            Handle::SW => (-1.0, 1.0),
            Handle::W => (-1.0, 0.0),
            Handle::NW => (-1.0, -1.0),
        }
    }
}

/// The box's local x and y axes as canvas unit vectors.
fn axes(f: &Frame) -> (Pt, Pt) {
    let (sin, cos) = f.rotation.to_radians().sin_cos();
    ((cos, sin), (-sin, cos))
}

fn frame_center(f: &Frame) -> Pt {
    (f.x + f.w / 2.0, f.y + f.h / 2.0)
}

/// `center + a·u + b·v`.
fn along(center: Pt, u: Pt, a: f32, v: Pt, b: f32) -> Pt {
    (center.0 + a * u.0 + b * v.0, center.1 + a * u.1 + b * v.1)
}

/// `f` moved so its box is `w × h` centred on `center`.
fn recentred(f: &Frame, center: Pt, w: f32, h: f32) -> Frame {
    Frame {
        x: center.0 - w / 2.0,
        y: center.1 - h / 2.0,
        w,
        h,
        ..*f
    }
}

/// Degrees wrapped into `[0, 360)`.
fn wrap_degrees(deg: f32) -> f32 {
    let r = deg.rem_euclid(360.0);
    if r >= 360.0 {
        0.0
    } else {
        r + 0.0 // normalise -0.0
    }
}

/// Axis-aligned bounds of the frame's (possibly rotated) box.
pub fn frame_aabb(f: &Frame) -> Rect {
    let (sin, cos) = f.rotation.to_radians().sin_cos();
    let (hw, hh) = (f.w / 2.0, f.h / 2.0);
    let ex = (hw * cos).abs() + (hh * sin).abs();
    let ey = (hw * sin).abs() + (hh * cos).abs();
    let (cx, cy) = frame_center(f);
    Rect {
        x: cx - ex,
        y: cy - ey,
        w: 2.0 * ex,
        h: 2.0 * ey,
    }
}

/// Union of the frames' axis-aligned bounds; `None` when `frames` is empty.
pub fn selection_bounds(frames: &[Frame]) -> Option<Rect> {
    let (first, rest) = frames.split_first()?;
    Some(
        rest.iter()
            .fold(frame_aabb(first), |acc, f| acc.union(&frame_aabb(f))),
    )
}

/// The frame of a shape drawn by dragging from `start` to `end`.
///
/// `end` is `None` for a click, which gives `default_size` centred on
/// `start`. The caller decides what is a click, since the threshold is in
/// screen pixels (see [`CLICK_DRAG_THRESHOLD`]). Box shapes get a normalised rectangle at least
/// [`MIN_SIZE`] on each side (`constrain`: a square of the larger side;
/// `from_center`: `start` is the centre). Line-like shapes get the box
/// between the end points with `flip_x`/`flip_y` recording a backwards
/// drag (`constrain` snaps the direction to 45° steps, keeping the length;
/// `from_center`: `start` is the midpoint).
pub fn drag_create(
    start: Pt,
    end: Option<Pt>,
    default_size: (f32, f32),
    line_like: bool,
    constrain: bool,
    from_center: bool,
) -> Frame {
    let Some(end) = end else {
        let (w, h) = default_size;
        return recentred(&Frame::default(), start, w, h);
    };
    let (mut dx, mut dy) = (end.0 - start.0, end.1 - start.1);
    let len = dx.hypot(dy);
    if constrain {
        if line_like {
            const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
            const DIRS: [Pt; 8] = [
                (1.0, 0.0),
                (D, D),
                (0.0, 1.0),
                (-D, D),
                (-1.0, 0.0),
                (-D, -D),
                (0.0, -1.0),
                (D, -D),
            ];
            let step = dy.atan2(dx) / std::f32::consts::FRAC_PI_4;
            let dir = DIRS[(step.round() as i32).rem_euclid(8) as usize];
            (dx, dy) = (dir.0 * len, dir.1 * len);
        } else {
            let side = dx.abs().max(dy.abs());
            (dx, dy) = (side.copysign(dx), side.copysign(dy));
        }
    }
    let (w, h) = if line_like {
        (dx.abs(), dy.abs())
    } else {
        (dx.abs().max(MIN_SIZE), dy.abs().max(MIN_SIZE))
    };
    let (w, h, center) = if from_center {
        (2.0 * w, 2.0 * h, start)
    } else {
        (w, h, (start.0 + dx / 2.0, start.1 + dy / 2.0))
    };
    let f = Frame {
        flip_x: line_like && dx < 0.0,
        flip_y: line_like && dy < 0.0,
        ..Frame::default()
    };
    recentred(&f, center, w, h)
}

/// The frames moved by `(dx, dy)`.
pub fn translate(frames: &[Frame], dx: f32, dy: f32) -> Vec<Frame> {
    frames
        .iter()
        .map(|f| Frame {
            x: f.x + dx,
            y: f.y + dy,
            ..*f
        })
        .collect()
}

/// New signed width and height of a `w × h` box whose `handle` moved by
/// `(du, dv)` along the box's own axes. A negative size means the handle
/// crossed the opposite edge (or the centre, with `from_center`).
fn resized_size(
    w: f32,
    h: f32,
    handle: Handle,
    (du, dv): Pt,
    keep_aspect: bool,
    from_center: bool,
) -> (f32, f32) {
    let (hx, hy) = handle.sides();
    let gain = if from_center { 2.0 } else { 1.0 };
    let mut sw = w + gain * hx * du;
    let mut sh = h + gain * hy * dv;
    if keep_aspect && w > 0.0 && h > 0.0 {
        let (kx, ky) = (sw / w, sh / h);
        if hx == 0.0 {
            sw = ky.abs() * w;
        } else if hy == 0.0 {
            sh = kx.abs() * h;
        } else {
            let k = kx.abs().max(ky.abs());
            sw = k.copysign(kx) * w;
            sh = k.copysign(ky) * h;
        }
    }
    (sw, sh)
}

/// Frames after dragging `handle` from `start_pointer` to `pointer`.
///
/// One frame resizes in its own rotated axes with the opposite handle
/// fixed in canvas space (`from_center`: the centre stays fixed). Several
/// frames scale their positions and sizes relative to [`selection_bounds`],
/// keeping each rotation (Figma-style; no skew). Dragging past the
/// opposite edge toggles `flip_x`/`flip_y`; a dragged side never drops
/// below [`MIN_SIZE`] (an axis the handle does not move keeps its size,
/// and a zero-extent axis, as on a straight line, is never thickened by
/// `keep_aspect` or by a group resize).
pub fn resize(
    start: &[Frame],
    handle: Handle,
    start_pointer: Pt,
    pointer: Pt,
    keep_aspect: bool,
    from_center: bool,
) -> Vec<Frame> {
    let d = (pointer.0 - start_pointer.0, pointer.1 - start_pointer.1);
    let (hx, hy) = handle.sides();
    // Which axes a `w × h` box's gesture changes. Keeping the aspect ratio
    // only couples the axes when both have extent; a zero-extent axis
    // (a straight line) stays zero.
    let affected = |w: f32, h: f32| {
        let aspect = keep_aspect && w > 0.0 && h > 0.0;
        (hx != 0.0 || aspect, hy != 0.0 || aspect)
    };
    let side = |old: f32, signed: f32, affected: bool| {
        if affected {
            signed.abs().max(MIN_SIZE)
        } else {
            old
        }
    };
    if let [f] = start {
        let (affects_x, affects_y) = affected(f.w, f.h);
        let (u, v) = axes(f);
        let local = (d.0 * u.0 + d.1 * u.1, d.0 * v.0 + d.1 * v.1);
        let (sw, sh) = resized_size(f.w, f.h, handle, local, keep_aspect, from_center);
        let (w, h) = (side(f.w, sw, affects_x), side(f.h, sh, affects_y));
        let c = frame_center(f);
        let center = if from_center {
            c
        } else {
            let anchor = along(c, u, -hx * f.w / 2.0, v, -hy * f.h / 2.0);
            along(
                anchor,
                u,
                hx * w.copysign(sw) / 2.0,
                v,
                hy * h.copysign(sh) / 2.0,
            )
        };
        let mut out = recentred(f, center, w, h);
        out.flip_x ^= sw < 0.0;
        out.flip_y ^= sh < 0.0;
        return vec![out];
    }
    let Some(b) = selection_bounds(start) else {
        return Vec::new();
    };
    let (affects_x, affects_y) = affected(b.w, b.h);
    let (sw, sh) = resized_size(b.w, b.h, handle, d, keep_aspect, from_center);
    let sx = if b.w > 0.0 && affects_x {
        sw / b.w
    } else {
        1.0
    };
    let sy = if b.h > 0.0 && affects_y {
        sh / b.h
    } else {
        1.0
    };
    let anchor = if from_center {
        b.center()
    } else {
        (b.x + (1.0 - hx) / 2.0 * b.w, b.y + (1.0 - hy) / 2.0 * b.h)
    };
    let mirrored = (sx < 0.0) != (sy < 0.0);
    start
        .iter()
        .map(|f| {
            let c = frame_center(f);
            let center = (
                anchor.0 + (c.0 - anchor.0) * sx,
                anchor.1 + (c.1 - anchor.1) * sy,
            );
            // A shape with no extent on an axis (a straight line) keeps it.
            let w = side(f.w, f.w * sx.abs(), affects_x && f.w > 0.0);
            let h = side(f.h, f.h * sy.abs(), affects_y && f.h > 0.0);
            let mut out = recentred(f, center, w, h);
            out.flip_x ^= sx < 0.0;
            out.flip_y ^= sy < 0.0;
            if mirrored {
                out.rotation = wrap_degrees(-f.rotation);
            }
            out
        })
        .collect()
}

/// Frames after turning the pointer about `center` from `start_pointer`
/// to `pointer`: positions rotate about `center` and the angle is added to
/// each rotation (wrapped into `[0, 360)`). `snap` rounds the angle to the
/// nearest 15°.
pub fn rotate(
    start: &[Frame],
    center: Pt,
    start_pointer: Pt,
    pointer: Pt,
    snap: bool,
) -> Vec<Frame> {
    let angle_of = |p: Pt| (p.1 - center.1).atan2(p.0 - center.0).to_degrees();
    let mut delta = angle_of(pointer) - angle_of(start_pointer);
    if snap {
        delta = (delta / 15.0).round() * 15.0;
    }
    let (sin, cos) = delta.to_radians().sin_cos();
    start
        .iter()
        .map(|f| {
            let (cx, cy) = frame_center(f);
            let (dx, dy) = (cx - center.0, cy - center.1);
            let c = (
                center.0 + dx * cos - dy * sin,
                center.1 + dx * sin + dy * cos,
            );
            Frame {
                rotation: wrap_degrees(f.rotation + delta),
                ..recentred(f, c, f.w, f.h)
            }
        })
        .collect()
}

/// Whether a marquee selects the frame: its axis-aligned bounds lie fully
/// inside `marquee`.
pub fn marquee_selects(f: &Frame, marquee: &Rect) -> bool {
    marquee.contains_rect(&frame_aabb(f))
}

/// Canvas positions of the eight resize handles on the frame's (possibly
/// rotated) box, in [`Handle::ALL`] order. For a multi-selection pass
/// `Frame::from_rect(&group_bounds)`.
pub fn handle_positions(f: &Frame) -> [(Handle, Pt); 8] {
    let (u, v) = axes(f);
    let c = frame_center(f);
    Handle::ALL.map(|h| {
        let (hx, hy) = h.sides();
        (h, along(c, u, hx * f.w / 2.0, v, hy * f.h / 2.0))
    })
}

/// Canvas position of the rotate handle: `offset` units beyond the top
/// edge midpoint, along the box's rotated "up" direction.
pub fn rotate_handle_position(f: &Frame, offset: f32) -> Pt {
    let (u, v) = axes(f);
    along(frame_center(f), u, 0.0, v, -(f.h / 2.0 + offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-3;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_pt(a: Pt, b: Pt) {
        assert!(close(a.0, b.0) && close(a.1, b.1), "{a:?} != {b:?}");
    }

    fn assert_frame(f: &Frame, x: f32, y: f32, w: f32, h: f32) {
        assert!(
            close(f.x, x) && close(f.y, y) && close(f.w, w) && close(f.h, h),
            "{f:?} != ({x}, {y}, {w}, {h})"
        );
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

    fn corner(f: &Frame, h: Handle) -> Pt {
        handle_positions(f)
            .iter()
            .find(|(k, _)| *k == h)
            .map(|(_, p)| *p)
            .unwrap()
    }

    #[test]
    fn drag_create_click_uses_default_size() {
        let f = drag_create((50.0, 50.0), None, (100.0, 60.0), false, false, false);
        assert_frame(&f, 0.0, 20.0, 100.0, 60.0);
        assert!(!f.flip_x && !f.flip_y && f.rotation == 0.0);
        // A line placed with a click also takes its default size.
        let f = drag_create((50.0, 50.0), None, (100.0, 0.0), true, true, true);
        assert_frame(&f, 0.0, 50.0, 100.0, 0.0);
    }

    #[test]
    fn drag_create_short_drag_is_still_a_drag() {
        // The caller decides what is a click (the threshold is in screen
        // pixels); a drag back near the start keeps a drawn size.
        let f = drag_create(
            (50.0, 50.0),
            Some((51.0, 50.5)),
            (100.0, 60.0),
            false,
            false,
            false,
        );
        assert_frame(&f, 50.0, 49.75, MIN_SIZE, MIN_SIZE);
        let f = drag_create(
            (50.0, 50.0),
            Some((50.0, 50.0)),
            (100.0, 0.0),
            true,
            false,
            false,
        );
        assert_frame(&f, 50.0, 50.0, 0.0, 0.0);
    }

    #[test]
    fn drag_create_normalises_backwards_box() {
        let f = drag_create(
            (100.0, 80.0),
            Some((40.0, 20.0)),
            (100.0, 60.0),
            false,
            false,
            false,
        );
        assert_frame(&f, 40.0, 20.0, 60.0, 60.0);
        assert!(!f.flip_x && !f.flip_y);
    }

    #[test]
    fn drag_create_constrain_makes_square() {
        let f = drag_create(
            (10.0, 10.0),
            Some((50.0, 30.0)),
            (100.0, 60.0),
            false,
            true,
            false,
        );
        assert_frame(&f, 10.0, 10.0, 40.0, 40.0);
        let f = drag_create(
            (10.0, 10.0),
            Some((0.0, -20.0)),
            (100.0, 60.0),
            false,
            true,
            false,
        );
        assert_frame(&f, -20.0, -20.0, 30.0, 30.0);
    }

    #[test]
    fn drag_create_from_center() {
        let f = drag_create(
            (100.0, 100.0),
            Some((130.0, 80.0)),
            (100.0, 60.0),
            false,
            false,
            true,
        );
        assert_frame(&f, 70.0, 80.0, 60.0, 40.0);
        let f = drag_create(
            (100.0, 100.0),
            Some((130.0, 80.0)),
            (100.0, 60.0),
            false,
            true,
            true,
        );
        assert_frame(&f, 70.0, 70.0, 60.0, 60.0);
    }

    #[test]
    fn line_backwards_sets_flips() {
        let f = drag_create(
            (100.0, 100.0),
            Some((40.0, 70.0)),
            (100.0, 0.0),
            true,
            false,
            false,
        );
        assert_frame(&f, 40.0, 70.0, 60.0, 30.0);
        assert!(f.flip_x && f.flip_y);
        let f = drag_create(
            (10.0, 100.0),
            Some((40.0, 70.0)),
            (100.0, 0.0),
            true,
            false,
            false,
        );
        assert!(!f.flip_x && f.flip_y);
        // A horizontal line keeps zero height.
        let f = drag_create(
            (10.0, 10.0),
            Some((60.0, 10.0)),
            (100.0, 0.0),
            true,
            false,
            false,
        );
        assert_frame(&f, 10.0, 10.0, 50.0, 0.0);
    }

    #[test]
    fn line_constrain_snaps_45() {
        // Nearly horizontal → horizontal, same length.
        let f = drag_create(
            (0.0, 0.0),
            Some((100.0, 10.0)),
            (100.0, 0.0),
            true,
            true,
            false,
        );
        let len = (100.0f32 * 100.0 + 10.0 * 10.0).sqrt();
        assert_frame(&f, 0.0, 0.0, len, 0.0);
        // Up-left near the diagonal → exact 45°, flipped both ways.
        let f = drag_create(
            (0.0, 0.0),
            Some((-50.0, -45.0)),
            (100.0, 0.0),
            true,
            true,
            false,
        );
        assert!(close(f.w, f.h) && f.w > 0.0);
        assert!(f.flip_x && f.flip_y);
        assert!(close(f.x + f.w, 0.0) && close(f.y + f.h, 0.0));
        // Nearly vertical downward.
        let f = drag_create(
            (0.0, 0.0),
            Some((5.0, 80.0)),
            (100.0, 0.0),
            true,
            true,
            false,
        );
        assert!(close(f.w, 0.0) && !f.flip_x && !f.flip_y);
    }

    #[test]
    fn resize_se_grows() {
        let r = resize(
            &[frame(10.0, 10.0, 100.0, 50.0)],
            Handle::SE,
            (110.0, 60.0),
            (130.0, 90.0),
            false,
            false,
        );
        assert_frame(&r[0], 10.0, 10.0, 120.0, 80.0);
        // An edge handle changes one axis only.
        let r = resize(
            &[frame(10.0, 10.0, 100.0, 50.0)],
            Handle::W,
            (10.0, 35.0),
            (0.0, 99.0),
            false,
            false,
        );
        assert_frame(&r[0], 0.0, 10.0, 110.0, 50.0);
    }

    #[test]
    fn resize_past_edge_flips() {
        // Drag E handle 30 past the left edge.
        let r = resize(
            &[frame(10.0, 10.0, 100.0, 50.0)],
            Handle::E,
            (110.0, 35.0),
            (-20.0, 35.0),
            false,
            false,
        );
        assert_frame(&r[0], -20.0, 10.0, 30.0, 50.0);
        assert!(r[0].flip_x && !r[0].flip_y);
        // Crossing again flips back.
        let start = Frame {
            flip_y: true,
            ..frame(0.0, 0.0, 10.0, 10.0)
        };
        let r = resize(&[start], Handle::N, (5.0, 0.0), (5.0, 15.0), false, false);
        assert_frame(&r[0], 0.0, 10.0, 10.0, 5.0);
        assert!(!r[0].flip_y);
    }

    #[test]
    fn resize_never_below_min_size() {
        let r = resize(
            &[frame(10.0, 10.0, 100.0, 50.0)],
            Handle::SE,
            (110.0, 60.0),
            (10.0, 10.0),
            false,
            false,
        );
        assert!(r[0].w >= MIN_SIZE && r[0].h >= MIN_SIZE);
        assert_frame(&r[0], 10.0, 10.0, MIN_SIZE, MIN_SIZE);
        let r = resize(
            &[frame(10.0, 10.0, 100.0, 50.0)],
            Handle::NW,
            (10.0, 10.0),
            (110.0, 60.0),
            true,
            true,
        );
        assert!(r[0].w >= MIN_SIZE && r[0].h >= MIN_SIZE);
    }

    #[test]
    fn resize_keep_aspect() {
        // 2:1 box, SE dragged mostly in x → height follows.
        let r = resize(
            &[frame(0.0, 0.0, 100.0, 50.0)],
            Handle::SE,
            (100.0, 50.0),
            (200.0, 60.0),
            true,
            false,
        );
        assert_frame(&r[0], 0.0, 0.0, 200.0, 100.0);
        // Edge handle: the other axis scales about the centre line.
        let r = resize(
            &[frame(0.0, 0.0, 100.0, 50.0)],
            Handle::S,
            (50.0, 50.0),
            (50.0, 100.0),
            true,
            false,
        );
        assert_frame(&r[0], -50.0, 0.0, 200.0, 100.0);
        // From centre with aspect.
        let r = resize(
            &[frame(0.0, 0.0, 100.0, 50.0)],
            Handle::E,
            (100.0, 25.0),
            (150.0, 25.0),
            true,
            true,
        );
        assert_frame(&r[0], -50.0, -25.0, 200.0, 100.0);
    }

    #[test]
    fn resize_rotated_shape_keeps_opposite_corner_fixed() {
        let f = Frame {
            rotation: 30.0,
            ..frame(100.0, 100.0, 80.0, 40.0)
        };
        let nw = corner(&f, Handle::NW);
        let se = corner(&f, Handle::SE);
        let pointer = (se.0 + 25.0, se.1 + 5.0);
        let r = resize(&[f], Handle::SE, se, pointer, false, false);
        assert!(close(r[0].rotation, 30.0));
        assert_pt(corner(&r[0], Handle::NW), nw);
        // The pointer projected onto the local axes gives the new size.
        let (s, c) = 30f32.to_radians().sin_cos();
        assert!(close(r[0].w, 80.0 + 25.0 * c + 5.0 * s));
        assert!(close(r[0].h, 40.0 - 25.0 * s + 5.0 * c));
        // From centre keeps the centre.
        let r = resize(&[f], Handle::SE, se, pointer, false, true);
        assert_pt(
            (r[0].x + r[0].w / 2.0, r[0].y + r[0].h / 2.0),
            (f.x + f.w / 2.0, f.y + f.h / 2.0),
        );
    }

    #[test]
    fn group_resize_scales_positions_keeps_rotation() {
        let a = frame(0.0, 0.0, 50.0, 50.0);
        let b = Frame {
            rotation: 45.0,
            ..frame(150.0, 50.0, 50.0, 50.0)
        };
        let bounds = selection_bounds(&[a, b]).unwrap();
        let se = (bounds.x + bounds.w, bounds.y + bounds.h);
        let pointer = (bounds.x + bounds.w * 2.0, se.1);
        let r = resize(&[a, b], Handle::SE, se, pointer, false, false);
        let sx = (bounds.w * 2.0) / bounds.w;
        // a's centre (25, 25) → x scaled about the group's left edge.
        assert_frame(
            &r[0],
            bounds.x + (25.0 - bounds.x) * sx - 50.0,
            0.0,
            100.0,
            50.0,
        );
        assert!(close(r[1].rotation, 45.0));
        assert!(close(r[1].w, 100.0) && close(r[1].h, 50.0));
        let cb = (r[1].x + r[1].w / 2.0, r[1].y + r[1].h / 2.0);
        assert_pt(cb, (bounds.x + (175.0 - bounds.x) * sx, 75.0));
        // Mirroring the group horizontally flips shapes and mirrors rotation.
        let left = (bounds.x - bounds.w, se.1);
        let r = resize(&[a, b], Handle::SE, se, left, false, false);
        assert!(r[0].flip_x && r[1].flip_x);
        assert!(close(r[1].rotation, 315.0));
    }

    #[test]
    fn rotate_snaps_to_15() {
        let f = frame(0.0, 0.0, 100.0, 100.0);
        let c = (50.0, 50.0);
        // 20° drag snaps to 15°.
        let a = 20f32.to_radians();
        let p = (c.0 + 100.0 * a.cos(), c.1 + 100.0 * a.sin());
        let r = rotate(&[f], c, (150.0, 50.0), p, true);
        assert!(close(r[0].rotation, 15.0), "{}", r[0].rotation);
        let r = rotate(&[f], c, (150.0, 50.0), p, false);
        assert!(close(r[0].rotation, 20.0));
        // A small counter-clockwise drag snaps back to 0°.
        let r = rotate(&[f], c, (150.0, 50.0), (150.0, 40.0), true);
        assert!(close(r[0].rotation, 0.0));
        // A quarter turn counter-clockwise wraps -90° to 270°.
        let r = rotate(&[f], c, (150.0, 50.0), (50.0, -50.0), false);
        assert!(close(r[0].rotation, 270.0));
        // A single shape rotating about its own centre does not move.
        assert_frame(&r[0], 0.0, 0.0, 100.0, 100.0);
    }

    #[test]
    fn rotate_group_moves_positions() {
        let a = frame(0.0, 0.0, 20.0, 20.0);
        let b = Frame {
            rotation: 350.0,
            ..frame(80.0, 0.0, 20.0, 20.0)
        };
        let c = (50.0, 10.0);
        // Quarter turn clockwise: right of centre → below it.
        let r = rotate(&[a, b], c, (100.0, 10.0), (50.0, 60.0), false);
        assert_frame(&r[0], 40.0, -40.0, 20.0, 20.0);
        assert_frame(&r[1], 40.0, 40.0, 20.0, 20.0);
        assert!(close(r[0].rotation, 90.0));
        assert!(close(r[1].rotation, 80.0));
    }

    #[test]
    fn marquee_requires_full_containment() {
        let marquee = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        assert!(marquee_selects(&frame(10.0, 10.0, 50.0, 50.0), &marquee));
        assert!(!marquee_selects(&frame(80.0, 10.0, 50.0, 50.0), &marquee));
        // Fits unrotated, but its rotated corners stick out.
        let tilted = Frame {
            rotation: 45.0,
            ..frame(0.0, 0.0, 100.0, 100.0)
        };
        assert!(!marquee_selects(&tilted, &marquee));
    }

    #[test]
    fn aabb_of_rotated_square_is_larger() {
        let f = Frame {
            rotation: 45.0,
            ..frame(0.0, 0.0, 100.0, 100.0)
        };
        let r = frame_aabb(&f);
        let d = 100.0 * std::f32::consts::SQRT_2;
        assert!(close(r.w, d) && close(r.h, d));
        assert_pt(r.center(), (50.0, 50.0));
        assert_eq!(
            frame_aabb(&frame(1.0, 2.0, 3.0, 4.0)),
            Rect {
                x: 1.0,
                y: 2.0,
                w: 3.0,
                h: 4.0
            }
        );
        assert!(selection_bounds(&[]).is_none());
    }

    #[test]
    fn handles_follow_rotation() {
        let f = Frame {
            rotation: 90.0,
            ..frame(0.0, 0.0, 100.0, 50.0)
        };
        // Rotated 90° clockwise about (50, 25): the box's local top-left
        // corner (NW handle) lands at the top-right of its canvas bounds.
        assert_pt(corner(&f, Handle::NW), (75.0, -25.0));
        assert_pt(corner(&f, Handle::E), (50.0, 75.0));
        // "Up" for this box points to +x on screen.
        assert_pt(rotate_handle_position(&f, 20.0), (95.0, 25.0));
        let g = frame(0.0, 0.0, 100.0, 50.0);
        assert_pt(rotate_handle_position(&g, 20.0), (50.0, -20.0));
        assert_eq!(handle_positions(&g)[0].0, Handle::N);
    }

    fn horizontal_line() -> Frame {
        frame(0.0, 50.0, 100.0, 0.0)
    }

    #[test]
    fn group_resize_keeps_zero_height_line_flat() {
        let line = horizontal_line();
        let bx = frame(0.0, 0.0, 40.0, 40.0);
        let b = selection_bounds(&[bx, line]).unwrap();
        let se = (b.x + b.w, b.y + b.h);
        for keep_aspect in [false, true] {
            let r = resize(
                &[bx, line],
                Handle::SE,
                se,
                (se.0 + 50.0, se.1 + 30.0),
                keep_aspect,
                false,
            );
            assert_eq!(r[1].h, 0.0, "keep_aspect {keep_aspect}: {:?}", r[1]);
            assert!(r[1].w > 100.0);
        }
    }

    #[test]
    fn keep_aspect_edge_drag_keeps_zero_height_line_flat() {
        let line = horizontal_line();
        for from_center in [false, true] {
            let r = resize(
                &[line],
                Handle::E,
                (100.0, 50.0),
                (150.0, 50.0),
                true,
                from_center,
            );
            assert_eq!(r[0].h, 0.0, "from_center {from_center}: {:?}", r[0]);
            assert!(close(r[0].y, 50.0));
        }
    }

    #[test]
    fn every_handle_keeps_its_opposite_fixed_on_rotated_flipped_frame() {
        let f = Frame {
            rotation: 37.0,
            flip_x: true,
            flip_y: true,
            ..frame(20.0, 30.0, 80.0, 50.0)
        };
        let opposite =
            |h: Handle| Handle::ALL[(Handle::ALL.iter().position(|&k| k == h).unwrap() + 4) % 8];
        for h in Handle::ALL {
            let from = corner(&f, h);
            let r = resize(&[f], h, from, (from.0 + 13.0, from.1 - 7.0), false, false);
            assert!(close(r[0].rotation, 37.0));
            assert_pt(corner(&r[0], opposite(h)), corner(&f, opposite(h)));
        }
    }

    #[test]
    fn crossing_edge_on_rotated_shape_toggles_local_flip() {
        let f = Frame {
            rotation: 90.0,
            ..frame(0.0, 0.0, 100.0, 50.0)
        };
        // Local +x points down the screen; drag the E handle up past W.
        let e = corner(&f, Handle::E);
        let r = resize(&[f], Handle::E, e, (e.0, e.1 - 130.0), false, false);
        assert!(r[0].flip_x && !r[0].flip_y);
        assert!(close(r[0].w, 30.0) && close(r[0].h, 50.0));
        assert!(close(r[0].rotation, 90.0));
        assert_pt(corner(&r[0], Handle::E), corner(&f, Handle::W));
    }

    #[test]
    fn handles_ignore_flips() {
        let f = Frame {
            rotation: 20.0,
            ..frame(5.0, 6.0, 70.0, 30.0)
        };
        let flipped = Frame {
            flip_x: true,
            flip_y: true,
            ..f
        };
        assert_eq!(handle_positions(&f), handle_positions(&flipped));
        assert_eq!(
            rotate_handle_position(&f, 12.0),
            rotate_handle_position(&flipped, 12.0)
        );
    }

    #[test]
    fn translate_moves_all() {
        let r = translate(
            &[frame(0.0, 0.0, 1.0, 1.0), frame(5.0, 5.0, 1.0, 1.0)],
            10.0,
            -2.0,
        );
        assert_frame(&r[0], 10.0, -2.0, 1.0, 1.0);
        assert_frame(&r[1], 15.0, 3.0, 1.0, 1.0);
    }
}
