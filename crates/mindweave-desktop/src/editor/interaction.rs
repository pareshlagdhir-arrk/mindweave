//! Pointer gestures on the canvas. Placing: with a shape tool, a left drag
//! draws the new shape's box; a click places it at its default size.
//! Pointer tool: a click selects (Shift toggles), a drag on empty canvas
//! draws a marquee (Shift adds), a drag on a selected shape moves the
//! selection, and the selection's handles resize and rotate it.

use std::collections::{HashMap, HashSet};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::{hit_test, FillRole, Frame, Primitive, Pt, Rect};
use mindweave_core::{
    drag_create, handle_positions, marquee_selects, resize_with_text_boxes, rotate,
    rotate_handle_position, selection_bounds, translate, DocumentSession, Handle, LibraryRegistry,
    Shape, ShapeDef, ShapeSource, CLICK_DRAG_THRESHOLD,
};
use uuid::Uuid;

use super::shape_paint::{shape_geometry, CanvasGeometry};
use super::tool::Tool;
use super::{EditorWindow, GestureOwner};
use crate::libraries::Libraries;

/// Opacity of the shape previewed while placing, times its own.
pub const PREVIEW_OPACITY: f32 = 0.6;
/// How far from a shape's outline the pointer still hits it, in pixels.
pub const HIT_TOLERANCE_PX: f32 = 4.;
/// How far from a handle's centre the pointer still grabs it, in pixels.
pub const HANDLE_GRAB_PX: f32 = 6.;
/// Distance of the rotate handle above the selection's top edge, in
/// pixels.
pub const ROTATE_HANDLE_OFFSET_PX: f32 = 12.;
/// Arrow-key nudge distances in canvas units: plain and with Shift.
pub const NUDGE_STEP: f32 = 1.;
pub const NUDGE_FAR_STEP: f32 = 10.;

/// Shapes and their frames when a transform gesture began.
pub type Originals = Vec<(Uuid, Frame)>;

/// The pointer gesture in progress on the canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Gesture {
    #[default]
    Idle,
    /// Drawing a new shape `shape` of `library` from `start` to `current`
    /// (canvas coordinates).
    Placing {
        start: Pt,
        current: Pt,
        library: String,
        shape: String,
        /// Shift: square box or 45° line.
        constrain: bool,
        /// Alt: `start` is the centre.
        from_center: bool,
        /// The pointer has travelled [`CLICK_DRAG_THRESHOLD`] pixels at
        /// some point: the shape gets the dragged box, not its default
        /// size, even if the pointer comes back.
        dragged: bool,
    },
    /// Selecting the shapes inside the box from `start` to `current`;
    /// `additive` (Shift) adds them to the selection.
    Marquee {
        start: Pt,
        current: Pt,
        additive: bool,
    },
    /// Moving the selection, pressed at `start`.
    Moving { start: Pt, originals: Originals },
    /// Dragging resize `handle` of the selection, pressed at `start`.
    Resizing {
        handle: Handle,
        start: Pt,
        originals: Originals,
        /// For each of `originals`: `Some(auto_width)` when it is a text
        /// box, which only changes width and then stops growing with its
        /// text; `None` for other shapes.
        auto_widths: Vec<Option<bool>>,
    },
    /// Turning the selection about `center`, pressed at `start`.
    Rotating {
        center: Pt,
        start: Pt,
        originals: Originals,
    },
}

impl Gesture {
    /// The shapes' frames before a move, resize or rotate.
    pub fn originals(&self) -> Option<&Originals> {
        match self {
            Gesture::Moving { originals, .. }
            | Gesture::Resizing { originals, .. }
            | Gesture::Rotating { originals, .. } => Some(originals),
            _ => None,
        }
    }
}

/// The left press behind a Pointer-tool gesture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Press {
    /// The pointer has travelled [`CLICK_DRAG_THRESHOLD`] pixels: the press
    /// is a drag, not a click.
    pub dragged: bool,
    /// The selection to apply if the press ends as a click.
    pub on_click: Option<Vec<Uuid>>,
    /// The last pointer position seen, in canvas coordinates.
    pub last: Pt,
}

/// A selection handle under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleHit {
    Resize(Handle),
    Rotate,
}

/// What a left press with the Pointer tool starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressKind {
    Rotate,
    Resize(Handle),
    /// Select the pressed shape (see [`press_selection`]) and move the
    /// selection.
    Move(Uuid),
    Marquee {
        additive: bool,
    },
}

/// Decides what a press starts: a handle wins over shapes, a shape over
/// empty canvas.
pub fn press_kind(handle: Option<HandleHit>, shape: Option<Uuid>, shift: bool) -> PressKind {
    match (handle, shape) {
        (Some(HandleHit::Rotate), _) => PressKind::Rotate,
        (Some(HandleHit::Resize(h)), _) => PressKind::Resize(h),
        (None, Some(id)) => PressKind::Move(id),
        (None, None) => PressKind::Marquee { additive: shift },
    }
}

/// What a press at `p` starts, given the selection chrome (if anything is
/// selected), the grab radius and rotate-handle offset in canvas units,
/// whether the chrome has only side handles (`width_only`, see
/// [`visible_handles`]), and the shape hit there. Handles win, except that
/// inside a selection smaller than two grab radii on both axes the shape
/// body wins over its resize handles, so tiny shapes can still be moved.
pub fn press_target(
    chrome: Option<&Frame>,
    p: Pt,
    grab: f32,
    rotate_offset: f32,
    width_only: bool,
    shape: Option<Uuid>,
    shift: bool,
) -> PressKind {
    let handle = chrome.and_then(|f| handle_at(f, p, grab, rotate_offset, width_only));
    let tiny = chrome.is_some_and(|f| f.w < 2. * grab && f.h < 2. * grab);
    let handle = match handle {
        Some(HandleHit::Resize(_)) if tiny && shape.is_some() => None,
        h => h,
    };
    press_kind(handle, shape, shift)
}

/// The selection after pressing shape `hit`, and the selection to apply
/// instead if the press ends as a click (no drag). An unselected shape is
/// selected at once (Shift: added), so a drag moves it; on a selected
/// shape the selection stays for the drag, and a click selects only it
/// (Shift: deselects it).
pub fn press_selection(
    selection: &[Uuid],
    hit: Uuid,
    shift: bool,
) -> (Vec<Uuid>, Option<Vec<Uuid>>) {
    let selected = selection.contains(&hit);
    match (selected, shift) {
        (false, false) => (vec![hit], None),
        (false, true) => {
            let mut sel = selection.to_vec();
            sel.push(hit);
            (sel, None)
        }
        (true, false) => (selection.to_vec(), Some(vec![hit])),
        (true, true) => {
            let rest = selection.iter().copied().filter(|&id| id != hit).collect();
            (selection.to_vec(), Some(rest))
        }
    }
}

/// Whether the pointer has moved from `start` to `p` (canvas coordinates)
/// far enough at `zoom` for a press to be a drag.
pub fn is_drag(start: Pt, p: Pt, zoom: f32) -> bool {
    let d = (p.0 - start.0).hypot(p.1 - start.1);
    d * zoom >= CLICK_DRAG_THRESHOLD
}

/// The box between two corners.
pub fn marquee_rect(a: Pt, b: Pt) -> Rect {
    Rect {
        x: a.0.min(b.0),
        y: a.1.min(b.1),
        w: (a.0 - b.0).abs(),
        h: (a.1 - b.1).abs(),
    }
}

/// The selection after a marquee `rect` over `shapes` (the active layer):
/// the unlocked shapes fully inside it, added to `current` when `additive`.
/// A hidden layer (`selectable` false) gives nothing.
pub fn marquee_selection(
    shapes: &[Shape],
    selectable: bool,
    rect: &Rect,
    additive: bool,
    current: &[Uuid],
) -> Vec<Uuid> {
    let mut sel = if additive {
        current.to_vec()
    } else {
        Vec::new()
    };
    if !selectable {
        return sel;
    }
    let already: HashSet<Uuid> = sel.iter().copied().collect();
    sel.extend(
        shapes
            .iter()
            .filter(|s| !s.locked && !already.contains(&s.id) && marquee_selects(&s.frame, rect))
            .map(|s| s.id),
    );
    sel
}

/// The frame the selection chrome is drawn on: a single shape's own
/// (rotated) frame, or the axis-aligned bounds of several.
pub fn chrome_frame(frames: &[Frame]) -> Option<Frame> {
    match frames {
        [] => None,
        [f] => Some(*f),
        _ => selection_bounds(frames).map(|r| Frame::from_rect(&r)),
    }
}

/// The centre of a frame's box.
fn frame_centre(f: &Frame) -> Pt {
    (f.x + f.w / 2., f.y + f.h / 2.)
}

/// The resize handles shown on the chrome `frame`: an axis shorter than
/// two grab radii (`grab` canvas units) has no edge-midpoint handles on it,
/// so the middle of a flat line or a thin box is not a handle. With
/// `width_only` (one text box, which only changes width) only the side
/// handles are shown.
pub fn visible_handles(frame: &Frame, grab: f32, width_only: bool) -> Vec<(Handle, Pt)> {
    let (thin_w, thin_h) = (frame.w < 2. * grab, frame.h < 2. * grab);
    handle_positions(frame)
        .into_iter()
        .filter(|(h, _)| match h {
            Handle::N | Handle::S => !thin_h && !width_only,
            Handle::E | Handle::W => !thin_w,
            _ => !width_only,
        })
        .collect()
}

/// The handle of the chrome `frame` nearest to `p` within `grab` canvas
/// units (of [`visible_handles`] with `width_only`); the rotate handle
/// sits `rotate_offset` above the top edge.
pub fn handle_at(
    frame: &Frame,
    p: Pt,
    grab: f32,
    rotate_offset: f32,
    width_only: bool,
) -> Option<HandleHit> {
    let dist = |q: Pt| (q.0 - p.0).hypot(q.1 - p.1);
    visible_handles(frame, grab, width_only)
        .into_iter()
        .map(|(h, q)| (HandleHit::Resize(h), dist(q)))
        .chain([(
            HandleHit::Rotate,
            dist(rotate_handle_position(frame, rotate_offset)),
        )])
        .filter(|&(_, d)| d <= grab)
        // The first of equally near handles wins.
        .fold(None, |best: Option<(HandleHit, f32)>, (h, d)| match best {
            Some((_, bd)) if bd <= d => best,
            _ => Some((h, d)),
        })
        .map(|(h, _)| h)
}

/// The resize cursor for `handle` on a frame rotated by `rotation`
/// degrees: the handle's direction from the centre, turned by the
/// rotation, picks the nearest of the four resize cursors.
pub fn resize_cursor(handle: Handle, rotation: f32) -> CursorStyle {
    let base = match handle {
        Handle::E | Handle::W => 0.,
        Handle::SE | Handle::NW => 45.,
        Handle::S | Handle::N => 90.,
        Handle::SW | Handle::NE => 135.,
    };
    // y grows downward, so 45° points to the lower right.
    let octant = (((base + rotation) / 45.).round() as i64).rem_euclid(4);
    match octant {
        0 => CursorStyle::ResizeLeftRight,
        1 => CursorStyle::ResizeUpLeftDownRight,
        2 => CursorStyle::ResizeUpDown,
        _ => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// The frames a move, resize or rotate gesture gives with the pointer at
/// `p`: computed from the original frames each time. Shift keeps the
/// aspect ratio (resize) or snaps to 15° (rotate); Alt resizes from the
/// centre. Text boxes only change width when resized. `None` for other
/// gestures.
pub fn transformed(gesture: &Gesture, p: Pt, shift: bool, alt: bool) -> Option<Originals> {
    let originals = gesture.originals()?;
    let start_frames: Vec<Frame> = originals.iter().map(|(_, f)| *f).collect();
    let frames = match gesture {
        Gesture::Moving { start, .. } => translate(&start_frames, p.0 - start.0, p.1 - start.1),
        Gesture::Resizing {
            handle,
            start,
            auto_widths,
            ..
        } => {
            let text_boxes: Vec<bool> = auto_widths.iter().map(Option::is_some).collect();
            resize_with_text_boxes(&start_frames, &text_boxes, *handle, *start, p, shift, alt)
        }
        Gesture::Rotating { center, start, .. } => rotate(&start_frames, *center, *start, p, shift),
        _ => return None,
    };
    Some(originals.iter().map(|(id, _)| *id).zip(frames).collect())
}

/// The text boxes a resize gesture changes, which stop growing with their
/// text once dragged; none for other gestures.
pub fn text_box_widths_off(gesture: &Gesture) -> Vec<Uuid> {
    match gesture {
        Gesture::Resizing {
            originals,
            auto_widths,
            ..
        } => originals
            .iter()
            .zip(auto_widths)
            .filter(|(_, w)| w.is_some())
            .map(|((id, _), _)| *id)
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether the pointer at `p` hits `outline` within `tolerance` canvas
/// units. A placeholder counts as its filled box.
pub fn outline_hit(outline: &CanvasGeometry, p: Pt, tolerance: f32) -> bool {
    let a = &outline.aabb;
    let near_box = p.0 >= a.x - tolerance
        && p.0 <= a.x + a.w + tolerance
        && p.1 >= a.y - tolerance
        && p.1 <= a.y + a.h + tolerance;
    if !near_box {
        return false;
    }
    if outline.placeholder {
        let mut filled = outline.geometry.clone();
        for sub in &mut filled.paths {
            sub.fill = FillRole::Shape;
        }
        hit_test(&filled, p, tolerance)
    } else {
        hit_test(&outline.geometry, p, tolerance)
    }
}

/// The topmost unlocked shape of the active layer for which `hits(shape,
/// tolerance)` holds; the tolerance grows by half the shape's stroke. A
/// hidden active layer gives `None`.
pub(super) fn hit_shape_with(
    session: &DocumentSession,
    tolerance: f32,
    mut hits: impl FnMut(&Shape, f32) -> bool,
) -> Option<Uuid> {
    if !session.active_layer_selectable() {
        return None;
    }
    session
        .shapes_on_active_layer()
        .iter()
        .rev()
        .filter(|s| !s.locked)
        .find(|s| {
            let half_stroke = if s.style.stroke.is_some() {
                s.style.stroke_width.max(0.) / 2.
            } else {
                0.
            };
            hits(s, tolerance + half_stroke)
        })
        .map(|s| s.id)
}

/// Whether `def` is drawn end to end (lines and arrows) rather than as a
/// box.
pub fn is_line_like(def: &ShapeDef) -> bool {
    matches!(
        def.source,
        ShapeSource::Primitive(Primitive::Line | Primitive::Arrow)
    )
}

/// The frame of `def` drawn from `start` to `end`, or placed with a click
/// at `start` when `end` is `None`.
pub fn placement_frame(
    def: &ShapeDef,
    start: Pt,
    end: Option<Pt>,
    constrain: bool,
    from_center: bool,
) -> Frame {
    drag_create(
        start,
        end,
        def.default_size,
        is_line_like(def),
        constrain,
        from_center,
    )
}

impl EditorWindow {
    /// Left mouse-down at canvas point `at`: starts placing when a shape
    /// tool is active. A hidden active layer refuses with a notification.
    pub(super) fn begin_placing(
        &mut self,
        at: Pt,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A gesture whose mouse-up was missed ends as it would have.
        self.finish_gesture(None, modifiers, cx);
        let Tool::Shape { library, shape } = &self.tools.tool else {
            return;
        };
        if !self.session.active_layer_selectable() {
            window.push_notification(Notification::warning("Show the layer to draw on it."), cx);
            return;
        }
        self.gesture = Gesture::Placing {
            start: at,
            current: at,
            library: library.clone(),
            shape: shape.clone(),
            constrain: modifiers.shift,
            from_center: modifiers.alt,
            dragged: false,
        };
        cx.notify();
    }

    /// Pointer moved to canvas point `at`, or a modifier changed (`at` is
    /// `None`), while placing.
    pub(super) fn update_placing(
        &mut self,
        at: Option<Pt>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let zoom = self.session.page().viewport.zoom;
        if move_placement(&mut self.gesture, at, modifiers.shift, modifiers.alt, zoom) {
            cx.notify();
        }
    }

    /// Mouse released at canvas point `at` (or where the pointer was last
    /// seen): adds the shape as one undo step, selects it and lets the
    /// tool return to Pointer unless locked.
    pub(super) fn finish_placing(
        &mut self,
        at: Option<Pt>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.update_placing(at, modifiers, cx);
        let gesture = std::mem::take(&mut self.gesture);
        let Some(shape) = placement_shape(&gesture, &cx.global::<Libraries>().0) else {
            return;
        };
        let id = shape.id;
        self.update_session(|s| s.add_shapes(vec![shape]), cx);
        self.selection = vec![id];
        self.tools.after_place();
        cx.notify();
    }

    /// Aborts the gesture in progress, if any, and returns whether there
    /// was one. A move, resize or rotate puts the shapes back, leaving no
    /// undo step; a placement or marquee is dropped.
    pub(super) fn cancel_gesture(&mut self, cx: &mut Context<Self>) -> bool {
        let gesture = std::mem::take(&mut self.gesture);
        if gesture == Gesture::Idle {
            return false;
        }
        if let Some(originals) = gesture.originals() {
            let originals = originals.clone();
            self.apply_frames(originals, cx);
            if let Gesture::Resizing {
                originals,
                auto_widths,
                ..
            } = &gesture
            {
                let widths: Vec<(Uuid, bool)> = originals
                    .iter()
                    .zip(auto_widths)
                    .filter_map(|((id, _), w)| Some((*id, (*w)?)))
                    .collect();
                self.set_auto_widths(&widths, cx);
            }
            self.session_gesture
                .end(&mut self.session, GestureOwner::Pointer);
            self.update_session(|_| {}, cx);
        }
        self.press = Press::default();
        cx.notify();
        true
    }

    /// Pointer moved to canvas point `at`, or a modifier changed (`at` is
    /// `None`), during any gesture.
    pub(super) fn update_gesture(
        &mut self,
        at: Option<Pt>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        match self.gesture {
            Gesture::Idle => {}
            Gesture::Placing { .. } => self.update_placing(at, modifiers, cx),
            _ => self.update_pointer_drag(at, modifiers, cx),
        }
    }

    /// Mouse released at canvas point `at` (or where the pointer was last
    /// seen): ends any gesture.
    pub(super) fn finish_gesture(
        &mut self,
        at: Option<Pt>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        match self.gesture {
            Gesture::Idle => {}
            Gesture::Placing { .. } => self.finish_placing(at, modifiers, cx),
            _ => self.finish_pointer_drag(at, modifiers, cx),
        }
    }

    /// Left mouse-down at canvas point `at` with the Pointer tool: grabs a
    /// selection handle, or selects and starts moving a shape, or starts a
    /// marquee.
    pub(super) fn begin_pointer_press(
        &mut self,
        at: Pt,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        // A gesture whose mouse-up was missed ends as it would have.
        self.finish_gesture(None, modifiers, cx);
        let zoom = self.session.page().viewport.zoom;
        let chrome = chrome_frame(
            &self
                .selected_frames()
                .into_iter()
                .map(|(_, f)| f)
                .collect::<Vec<_>>(),
        );
        let shape = self.shape_at(at, cx);
        let kind = press_target(
            chrome.as_ref(),
            at,
            HANDLE_GRAB_PX / zoom,
            ROTATE_HANDLE_OFFSET_PX / zoom,
            self.chrome_width_only(),
            shape,
            modifiers.shift,
        );
        self.press = Press {
            dragged: false,
            on_click: None,
            last: at,
        };
        self.gesture = match kind {
            PressKind::Marquee { additive } => Gesture::Marquee {
                start: at,
                current: at,
                additive,
            },
            PressKind::Move(id) => {
                let (selection, on_click) = press_selection(&self.selection, id, modifiers.shift);
                self.selection = selection;
                self.press.on_click = on_click;
                Gesture::Moving {
                    start: at,
                    originals: self.selected_frames(),
                }
            }
            PressKind::Resize(handle) => {
                let originals = self.selected_frames();
                let auto_widths = originals
                    .iter()
                    .map(|(id, _)| {
                        let shape = self.session.find_shape(*id)?;
                        shape.is_text_box().then(|| shape.auto_width())
                    })
                    .collect();
                Gesture::Resizing {
                    handle,
                    start: at,
                    originals,
                    auto_widths,
                }
            }
            PressKind::Rotate => Gesture::Rotating {
                center: chrome.map(|f| frame_centre(&f)).unwrap_or(at),
                start: at,
                originals: self.selected_frames(),
            },
        };
        match self.gesture.originals() {
            Some(o) if o.is_empty() => self.gesture = Gesture::Idle,
            Some(_) => {
                self.session_gesture
                    .begin(&mut self.session, GestureOwner::Pointer);
            }
            None => {}
        }
        cx.notify();
    }

    /// Continues a marquee, move, resize or rotate. Nothing moves until the
    /// pointer has travelled far enough for a drag.
    fn update_pointer_drag(
        &mut self,
        at: Option<Pt>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if let Some(at) = at {
            self.press.last = at;
        }
        let p = self.press.last;
        if !self.press.dragged {
            let start = match &self.gesture {
                Gesture::Marquee { start, .. }
                | Gesture::Moving { start, .. }
                | Gesture::Resizing { start, .. }
                | Gesture::Rotating { start, .. } => *start,
                _ => return,
            };
            if !is_drag(start, p, self.session.page().viewport.zoom) {
                return;
            }
            self.press.dragged = true;
            // Resized text boxes stop growing with their text, once.
            let widths: Vec<(Uuid, bool)> = text_box_widths_off(&self.gesture)
                .into_iter()
                .map(|id| (id, false))
                .collect();
            self.set_auto_widths(&widths, cx);
        }
        if let Gesture::Marquee { current, .. } = &mut self.gesture {
            if *current != p {
                *current = p;
                cx.notify();
            }
        } else if let Some(frames) = transformed(&self.gesture, p, modifiers.shift, modifiers.alt) {
            self.apply_frames(frames, cx);
        }
    }

    /// Ends a marquee (selecting what it encloses), or a move, resize or
    /// rotate (one undo step). A press that never became a drag is a click.
    fn finish_pointer_drag(
        &mut self,
        at: Option<Pt>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.update_pointer_drag(at, modifiers, cx);
        let press = std::mem::take(&mut self.press);
        match std::mem::take(&mut self.gesture) {
            Gesture::Marquee {
                start,
                current,
                additive,
            } => {
                self.selection = if press.dragged {
                    marquee_selection(
                        self.session.shapes_on_active_layer(),
                        self.session.active_layer_selectable(),
                        &marquee_rect(start, current),
                        additive,
                        &self.selection,
                    )
                } else if additive {
                    std::mem::take(&mut self.selection)
                } else {
                    Vec::new()
                };
            }
            Gesture::Moving { .. } | Gesture::Resizing { .. } | Gesture::Rotating { .. } => {
                self.session_gesture
                    .end(&mut self.session, GestureOwner::Pointer);
                self.update_session(|_| {}, cx);
                if !press.dragged {
                    if let Some(selection) = press.on_click {
                        self.selection = selection;
                    }
                }
            }
            other => self.gesture = other,
        }
        cx.notify();
    }

    /// Moves the selection by `(dx, dy)` canvas units as one undo step.
    pub(super) fn nudge(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        if self.gesture != Gesture::Idle || self.selection.is_empty() {
            return;
        }
        let moved = Gesture::Moving {
            start: (0., 0.),
            originals: self.selected_frames(),
        };
        if let Some(frames) = transformed(&moved, (dx, dy), false, false) {
            self.apply_frames(frames, cx);
        }
    }

    /// Sets the frames of the shapes in `frames` (on the active layer).
    fn apply_frames(&mut self, frames: Originals, cx: &mut Context<Self>) {
        let ids: Vec<Uuid> = frames.iter().map(|(id, _)| *id).collect();
        let frames: HashMap<Uuid, Frame> = frames.into_iter().collect();
        self.update_session(
            |s| {
                s.update_shapes(&ids, |shape| {
                    if let Some(f) = frames.get(&shape.id) {
                        shape.frame = *f;
                    }
                })
            },
            cx,
        );
    }

    /// Sets whether the text boxes in `widths` grow with their text.
    fn set_auto_widths(&mut self, widths: &[(Uuid, bool)], cx: &mut Context<Self>) {
        if widths.is_empty() {
            return;
        }
        let ids: Vec<Uuid> = widths.iter().map(|(id, _)| *id).collect();
        self.update_session(
            |s| {
                s.update_shapes(&ids, |shape| {
                    let auto = widths.iter().find(|(id, _)| *id == shape.id);
                    if let (Some(props), Some((_, auto))) = (shape.text_box.as_mut(), auto) {
                        props.auto_width = *auto;
                    }
                })
            },
            cx,
        );
    }

    /// Whether the selection chrome shows only side handles: when the
    /// selection is one text box, which only changes width.
    pub(super) fn chrome_width_only(&self) -> bool {
        match self.selection.as_slice() {
            [id] => self
                .session
                .find_shape(*id)
                .is_some_and(|s| s.is_text_box()),
            _ => false,
        }
    }

    /// The selected shapes and their frames, back to front.
    pub(super) fn selected_frames(&self) -> Originals {
        if self.selection.is_empty() {
            return Vec::new();
        }
        let ids: HashSet<Uuid> = self.selection.iter().copied().collect();
        self.session
            .shapes_on_active_layer()
            .iter()
            .filter(|s| ids.contains(&s.id))
            .map(|s| (s.id, s.frame))
            .collect()
    }

    /// The selection chrome's frame, if anything is selected.
    pub(super) fn selection_chrome(&self) -> Option<Frame> {
        let frames: Vec<Frame> = self.selected_frames().into_iter().map(|(_, f)| f).collect();
        chrome_frame(&frames)
    }

    /// The topmost selectable shape at canvas point `at`, hit-tested
    /// against the outlines cached by the last paint.
    pub(super) fn shape_at(&mut self, at: Pt, cx: &mut Context<Self>) -> Option<Uuid> {
        let libs = cx.global::<Libraries>().0.clone();
        let tolerance = HIT_TOLERANCE_PX / self.session.page().viewport.zoom;
        let cache = &mut self.geometry_cache;
        hit_shape_with(&self.session, tolerance, |shape, tol| {
            outline_hit(cache.outline(shape, &libs), at, tol)
        })
    }

    /// The cursor for the pointer at canvas point `at` with the Pointer
    /// tool: a resize or rotate cursor over a handle, or during a resize
    /// or rotate.
    pub(super) fn pointer_cursor(&self, at: Option<Pt>) -> Option<CursorStyle> {
        let chrome = self.selection_chrome()?;
        let hit = match &self.gesture {
            Gesture::Resizing { handle, .. } => Some(HandleHit::Resize(*handle)),
            Gesture::Rotating { .. } => Some(HandleHit::Rotate),
            Gesture::Idle => {
                let zoom = self.session.page().viewport.zoom;
                handle_at(
                    &chrome,
                    at?,
                    HANDLE_GRAB_PX / zoom,
                    ROTATE_HANDLE_OFFSET_PX / zoom,
                    self.chrome_width_only(),
                )
            }
            _ => None,
        }?;
        Some(match hit {
            HandleHit::Resize(h) => resize_cursor(h, chrome.rotation),
            HandleHit::Rotate => CursorStyle::Crosshair,
        })
    }

    /// The shape being placed, for the preview.
    pub(super) fn placement_preview(&self, libs: &LibraryRegistry) -> Option<Shape> {
        preview_shape(&self.gesture, libs)
    }
}

/// The preview of the shape `gesture` places: nothing until the pointer
/// has dragged, so a press doesn't flash the default size under the cursor.
fn preview_shape(gesture: &Gesture, libs: &LibraryRegistry) -> Option<Shape> {
    match gesture {
        Gesture::Placing { dragged: true, .. } => placement_shape(gesture, libs),
        _ => None,
    }
}

/// The topmost unlocked shape of the active layer at canvas point `p`
/// (within `tolerance` canvas units of its outline); `None` when the active
/// layer is hidden. Builds each outline afresh; the editor hit-tests
/// through the paint cache instead (see [`hit_shape_with`]).
#[cfg_attr(not(test), allow(dead_code))]
pub fn hit_shape(
    session: &DocumentSession,
    libs: &LibraryRegistry,
    p: Pt,
    tolerance: f32,
) -> Option<Uuid> {
    hit_shape_with(session, tolerance, |shape, tol| {
        outline_hit(&shape_geometry(shape, libs), p, tol)
    })
}

/// Moves a placement's pointer to `at` (if given) and sets its modifiers.
/// Once the pointer has travelled far enough at `zoom` for a drag, the
/// placement stays a drag. Returns whether anything changed.
fn move_placement(
    gesture: &mut Gesture,
    at: Option<Pt>,
    shift: bool,
    alt: bool,
    zoom: f32,
) -> bool {
    let Gesture::Placing {
        start,
        current,
        constrain,
        from_center,
        dragged,
        ..
    } = gesture
    else {
        return false;
    };
    let before = (*current, *constrain, *from_center, *dragged);
    if let Some(at) = at {
        *current = at;
        *dragged |= is_drag(*start, at, zoom);
    }
    *constrain = shift;
    *from_center = alt;
    before != (*current, *constrain, *from_center, *dragged)
}

/// The shape `gesture` places, if it is placing an installed shape.
fn placement_shape(gesture: &Gesture, libs: &LibraryRegistry) -> Option<Shape> {
    let Gesture::Placing {
        start,
        current,
        library,
        shape,
        constrain,
        from_center,
        dragged,
    } = gesture
    else {
        return None;
    };
    let def = libs.shape(library, shape)?;
    let end = dragged.then_some(*current);
    let frame = placement_frame(def, *start, end, *constrain, *from_center);
    Some(Shape::new(def, library, frame))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        chrome_frame, handle_at, hit_shape, is_drag, is_line_like, marquee_rect, marquee_selection,
        move_placement, placement_frame, placement_shape, press_kind, press_selection,
        press_target, preview_shape, resize_cursor, text_box_widths_off, transformed,
        visible_handles, Gesture, HandleHit, PressKind,
    };
    use gpui_kit::CursorStyle;
    use mindweave_core::geometry::Frame;
    use mindweave_core::geometry::Rect;
    use mindweave_core::Handle;
    use mindweave_core::{builtin_libraries, DocumentSession, LibraryRegistry, Shape};
    use uuid::Uuid;

    fn registry() -> LibraryRegistry {
        LibraryRegistry::new(builtin_libraries(), None).0
    }

    #[test]
    fn lines_and_arrows_are_line_like() {
        let libs = registry();
        let def = |id| libs.shape("basic-shapes", id).unwrap();
        assert!(is_line_like(def("line")));
        assert!(is_line_like(def("arrow")));
        assert!(!is_line_like(def("rectangle")));
        assert!(!is_line_like(def("ellipse")));
    }

    #[test]
    fn a_backwards_line_drag_flips_and_a_click_uses_the_default_size() {
        let libs = registry();
        let line = libs.shape("basic-shapes", "line").unwrap();
        let f = placement_frame(line, (100., 100.), Some((40., 70.)), false, false);
        assert_eq!((f.x, f.y, f.w, f.h), (40., 70., 60., 30.));
        assert!(f.flip_x && f.flip_y);

        let rect = libs.shape("basic-shapes", "rectangle").unwrap();
        let f = placement_frame(rect, (100., 100.), None, false, false);
        let (w, h) = rect.default_size;
        assert_eq!((f.x, f.y, f.w, f.h), (100. - w / 2., 100. - h / 2., w, h));
    }

    #[test]
    fn shift_squares_and_alt_draws_from_the_centre() {
        let libs = registry();
        let rect = libs.shape("basic-shapes", "rectangle").unwrap();
        let f = placement_frame(rect, (0., 0.), Some((40., 10.)), true, false);
        assert_eq!((f.w, f.h), (40., 40.));
        let f = placement_frame(rect, (50., 50.), Some((60., 70.)), false, true);
        assert_eq!((f.x, f.y, f.w, f.h), (40., 30., 20., 40.));
    }

    #[test]
    fn placing_builds_the_tool_shape_and_skips_missing_ones() {
        let libs = registry();
        let placing = |shape: &str| Gesture::Placing {
            start: (0., 0.),
            current: (30., 20.),
            library: "basic-shapes".into(),
            shape: shape.into(),
            constrain: false,
            from_center: false,
            dragged: true,
        };
        let shape = placement_shape(&placing("ellipse"), &libs).unwrap();
        assert_eq!(
            (shape.library.as_str(), shape.shape.as_str()),
            ("basic-shapes", "ellipse")
        );
        assert_eq!((shape.frame.w, shape.frame.h), (30., 20.));
        assert!(placement_shape(&placing("no-such-shape"), &libs).is_none());
        assert!(placement_shape(&Gesture::Idle, &libs).is_none());
    }

    #[test]
    fn no_preview_until_the_pointer_drags() {
        let libs = registry();
        let mut gesture = start_placing((100., 100.));
        // A press alone shows nothing: no default-size flash under the cursor.
        assert!(preview_shape(&gesture, &libs).is_none());
        // Once dragged, the preview grows from the press point.
        move_placement(&mut gesture, Some((130., 120.)), false, false, 1.0);
        let shape = preview_shape(&gesture, &libs).unwrap();
        assert_eq!((shape.frame.w, shape.frame.h), (30., 20.));
        // A click (no drag) still places the default size on release.
        let click = start_placing((100., 100.));
        assert!(placement_shape(&click, &libs).is_some());
    }

    fn start_placing(at: (f32, f32)) -> Gesture {
        Gesture::Placing {
            start: at,
            current: at,
            library: "basic-shapes".into(),
            shape: "rectangle".into(),
            constrain: false,
            from_center: false,
            dragged: false,
        }
    }

    fn placed_size(gesture: &Gesture) -> (f32, f32) {
        let frame = placement_shape(gesture, &registry()).unwrap().frame;
        (frame.w, frame.h)
    }

    #[test]
    fn the_click_or_drag_decision_uses_screen_pixels() {
        let default = registry()
            .shape("basic-shapes", "rectangle")
            .unwrap()
            .default_size;
        // At 1000% a 25 px drag (2.5 canvas units) draws the box.
        let mut g = start_placing((0., 0.));
        assert!(move_placement(&mut g, Some((2.5, 2.)), false, false, 10.));
        assert_eq!(placed_size(&g), (2.5, 2.));
        // At 10% a 1 px wobble (10 canvas units) is still a click.
        let mut g = start_placing((0., 0.));
        move_placement(&mut g, Some((10., 0.)), false, false, 0.1);
        assert_eq!(placed_size(&g), default);
        // ...but a 3 px drag (30 canvas units) is not.
        move_placement(&mut g, Some((30., 0.)), false, false, 0.1);
        assert_ne!(placed_size(&g), default);
    }

    #[test]
    fn a_placement_stays_a_drag_once_dragged() {
        let default = registry()
            .shape("basic-shapes", "rectangle")
            .unwrap()
            .default_size;
        let mut g = start_placing((100., 100.));
        move_placement(&mut g, Some((150., 140.)), false, false, 1.);
        assert_eq!(placed_size(&g), (50., 40.));
        // Back next to the start: a tiny box, not the default size.
        move_placement(&mut g, Some((101., 100.)), false, false, 1.);
        assert_eq!(placed_size(&g), (1., 1.));
        assert_ne!(placed_size(&g), default);
        // Modifier changes alone keep the decision.
        assert!(move_placement(&mut g, None, true, false, 1.));
        assert!(matches!(
            g,
            Gesture::Placing {
                dragged: true,
                constrain: true,
                ..
            }
        ));
        assert!(!move_placement(&mut g, None, true, false, 1.));
    }

    fn session() -> (tempfile::TempDir, DocumentSession) {
        let dir = tempfile::tempdir().unwrap();
        let session = DocumentSession::create(dir.path().join("hit.mindweave")).unwrap();
        (dir, session)
    }

    /// Adds a `shape` of `library` with box `(x, y, w, h)` to the active
    /// layer and returns its id.
    fn add(
        s: &mut DocumentSession,
        libs: &LibraryRegistry,
        library: &str,
        shape: &str,
        (x, y, w, h): (f32, f32, f32, f32),
    ) -> Uuid {
        let def = libs.shape("basic-shapes", shape).unwrap();
        let mut new = Shape::new(
            def,
            library,
            Frame {
                x,
                y,
                w,
                h,
                ..Frame::default()
            },
        );
        new.shape = shape.into();
        let id = new.id;
        s.add_shapes(vec![new]);
        id
    }

    #[test]
    fn the_topmost_shape_under_the_pointer_is_hit() {
        let libs = registry();
        let (_dir, mut s) = session();
        let below = add(
            &mut s,
            &libs,
            "basic-shapes",
            "rectangle",
            (0., 0., 100., 100.),
        );
        let above = add(
            &mut s,
            &libs,
            "basic-shapes",
            "ellipse",
            (50., 50., 100., 100.),
        );
        assert_eq!(hit_shape(&s, &libs, (20., 20.), 1.), Some(below));
        assert_eq!(hit_shape(&s, &libs, (90., 90.), 1.), Some(above));
        // The ellipse's box corner is outside the ellipse.
        assert_eq!(hit_shape(&s, &libs, (55., 55.), 1.), Some(below));
        assert_eq!(hit_shape(&s, &libs, (200., 200.), 1.), None);
    }

    #[test]
    fn lines_are_hit_within_the_tolerance_of_their_stroke() {
        let libs = registry();
        let (_dir, mut s) = session();
        let line = add(&mut s, &libs, "basic-shapes", "line", (0., 50., 100., 0.));
        assert_eq!(hit_shape(&s, &libs, (50., 52.), 3.), Some(line));
        assert_eq!(hit_shape(&s, &libs, (50., 60.), 3.), None);
    }

    #[test]
    fn locked_shapes_are_skipped() {
        let libs = registry();
        let (_dir, mut s) = session();
        let below = add(
            &mut s,
            &libs,
            "basic-shapes",
            "rectangle",
            (0., 0., 100., 100.),
        );
        let above = add(
            &mut s,
            &libs,
            "basic-shapes",
            "rectangle",
            (0., 0., 100., 100.),
        );
        s.update_shapes(&[above], |shape| shape.locked = true);
        assert_eq!(hit_shape(&s, &libs, (50., 50.), 1.), Some(below));
        s.update_shapes(&[below], |shape| shape.locked = true);
        assert_eq!(hit_shape(&s, &libs, (50., 50.), 1.), None);
    }

    #[test]
    fn only_the_active_visible_layer_is_hit() {
        let libs = registry();
        let (_dir, mut s) = session();
        let bottom = add(
            &mut s,
            &libs,
            "basic-shapes",
            "rectangle",
            (0., 0., 100., 100.),
        );
        // A new layer goes on top and becomes active.
        let top_layer = s.add_layer();
        assert_eq!(hit_shape(&s, &libs, (50., 50.), 1.), None);
        let top = add(
            &mut s,
            &libs,
            "basic-shapes",
            "rectangle",
            (200., 0., 100., 100.),
        );
        assert_eq!(hit_shape(&s, &libs, (250., 50.), 1.), Some(top));

        let bottom_layer = s.page().layers.len() - 1;
        s.set_active_layer(bottom_layer);
        assert_eq!(hit_shape(&s, &libs, (250., 50.), 1.), None);
        assert_eq!(hit_shape(&s, &libs, (50., 50.), 1.), Some(bottom));

        s.toggle_layer_visibility(bottom_layer);
        assert_eq!(hit_shape(&s, &libs, (50., 50.), 1.), None);
        s.set_active_layer(top_layer);
        assert_eq!(hit_shape(&s, &libs, (250., 50.), 1.), Some(top));
    }

    #[test]
    fn placeholders_are_hit_as_their_filled_box() {
        let libs = registry();
        let (_dir, mut s) = session();
        // A line would only be hit near its stroke; as a placeholder its
        // whole box counts.
        let missing = add(
            &mut s,
            &libs,
            "no-such-library",
            "line",
            (0., 0., 100., 100.),
        );
        assert_eq!(hit_shape(&s, &libs, (80., 20.), 1.), Some(missing));
        assert_eq!(hit_shape(&s, &libs, (120., 20.), 1.), None);
    }

    fn ids<const N: usize>() -> [Uuid; N] {
        [(); N].map(|_| Uuid::new_v4())
    }

    fn boxed(x: f32, y: f32, w: f32, h: f32) -> Frame {
        Frame {
            x,
            y,
            w,
            h,
            ..Frame::default()
        }
    }

    #[test]
    fn handles_win_over_shapes_and_shapes_over_empty_canvas() {
        let [id] = ids();
        let rotate = Some(HandleHit::Rotate);
        let corner = Some(HandleHit::Resize(Handle::SE));
        assert_eq!(press_kind(rotate, Some(id), false), PressKind::Rotate);
        assert_eq!(
            press_kind(corner, Some(id), true),
            PressKind::Resize(Handle::SE)
        );
        assert_eq!(press_kind(None, Some(id), true), PressKind::Move(id));
        assert_eq!(
            press_kind(None, None, false),
            PressKind::Marquee { additive: false }
        );
        assert_eq!(
            press_kind(None, None, true),
            PressKind::Marquee { additive: true }
        );
    }

    #[test]
    fn pressing_a_shape_selects_it_and_a_click_settles_the_selection() {
        let [a, b, c] = ids();
        // Unselected: selected at once (Shift adds), nothing more on click.
        assert_eq!(press_selection(&[a], b, false), (vec![b], None));
        assert_eq!(press_selection(&[a], b, true), (vec![a, b], None));
        // Selected: the selection stays for a drag; a click selects only it
        // or, with Shift, deselects it.
        assert_eq!(
            press_selection(&[a, b, c], b, false),
            (vec![a, b, c], Some(vec![b]))
        );
        assert_eq!(
            press_selection(&[a, b, c], b, true),
            (vec![a, b, c], Some(vec![a, c]))
        );
    }

    #[test]
    fn a_press_becomes_a_drag_after_the_threshold_in_screen_pixels() {
        assert!(!is_drag((0., 0.), (2., 0.), 1.));
        assert!(is_drag((0., 0.), (3., 0.), 1.));
        // At 200% one canvas unit is two pixels.
        assert!(is_drag((0., 0.), (0., 1.5), 2.));
        assert!(!is_drag((0., 0.), (0., 1.4), 2.));
    }

    #[test]
    fn a_marquee_selects_unlocked_shapes_fully_inside() {
        let libs = registry();
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        let shape = |f| Shape::new(def, "basic-shapes", f);
        let inside = shape(boxed(10., 10., 20., 20.));
        let partly = shape(boxed(90., 10., 20., 20.));
        let mut locked = shape(boxed(40., 40., 10., 10.));
        locked.locked = true;
        let shapes = vec![inside.clone(), partly, locked];
        let rect = marquee_rect((100., 100.), (0., 0.));
        assert_eq!(
            rect,
            Rect {
                x: 0.,
                y: 0.,
                w: 100.,
                h: 100.
            }
        );
        assert_eq!(
            marquee_selection(&shapes, true, &rect, false, &[]),
            vec![inside.id]
        );
        let [other] = ids();
        // Shift adds, without repeating shapes already selected.
        assert_eq!(
            marquee_selection(&shapes, true, &rect, true, &[other, inside.id]),
            vec![other, inside.id]
        );
        assert_eq!(
            marquee_selection(&shapes, true, &rect, false, &[other]),
            vec![inside.id]
        );
        // A hidden layer selects nothing.
        assert!(marquee_selection(&shapes, false, &rect, false, &[]).is_empty());
    }

    #[test]
    fn chrome_is_the_shape_frame_or_the_group_bounds() {
        let mut one = boxed(0., 0., 10., 20.);
        one.rotation = 30.;
        assert_eq!(chrome_frame(&[]), None);
        assert_eq!(chrome_frame(&[one]), Some(one));
        let group = chrome_frame(&[boxed(0., 0., 10., 10.), boxed(20., 5., 10., 10.)]).unwrap();
        assert_eq!(group, boxed(0., 0., 30., 15.));
    }

    #[test]
    fn handles_are_grabbed_within_their_radius() {
        let f = boxed(0., 0., 100., 50.);
        assert_eq!(
            handle_at(&f, (101., 49.), 6., 12., false),
            Some(HandleHit::Resize(Handle::SE))
        );
        assert_eq!(
            handle_at(&f, (50., 2.), 6., 12., false),
            Some(HandleHit::Resize(Handle::N))
        );
        assert_eq!(
            handle_at(&f, (50., -11.), 6., 12., false),
            Some(HandleHit::Rotate)
        );
        assert_eq!(handle_at(&f, (50., 25.), 6., 12., false), None);
        assert_eq!(handle_at(&f, (108., 50.), 6., 12., false), None);
    }

    #[test]
    fn resize_cursors_follow_the_handle_turned_by_the_rotation() {
        assert_eq!(resize_cursor(Handle::E, 0.), CursorStyle::ResizeLeftRight);
        assert_eq!(resize_cursor(Handle::N, 0.), CursorStyle::ResizeUpDown);
        assert_eq!(
            resize_cursor(Handle::SE, 0.),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            resize_cursor(Handle::NE, 0.),
            CursorStyle::ResizeUpRightDownLeft
        );
        assert_eq!(resize_cursor(Handle::E, 90.), CursorStyle::ResizeUpDown);
        assert_eq!(
            resize_cursor(Handle::E, 40.),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            resize_cursor(Handle::W, -45.),
            resize_cursor(Handle::NE, 0.)
        );
    }

    #[test]
    fn transforms_start_from_the_original_frames() {
        let [a, b] = ids();
        let originals = vec![(a, boxed(0., 0., 10., 10.)), (b, boxed(20., 0., 10., 10.))];
        let moving = Gesture::Moving {
            start: (5., 5.),
            originals: originals.clone(),
        };
        let moved = transformed(&moving, (8., 1.), false, false).unwrap();
        assert_eq!(
            moved,
            vec![
                (a, boxed(3., -4., 10., 10.)),
                (b, boxed(23., -4., 10., 10.))
            ]
        );
        // The same pointer gives the same frames however often it is asked.
        assert_eq!(transformed(&moving, (8., 1.), false, false).unwrap(), moved);

        let resizing = Gesture::Resizing {
            handle: Handle::E,
            start: (30., 5.),
            originals: originals.clone(),
            auto_widths: vec![None, None],
        };
        let resized = transformed(&resizing, (60., 5.), false, false).unwrap();
        assert_eq!(resized[1], (b, boxed(40., 0., 20., 10.)));

        let rotating = Gesture::Rotating {
            center: (15., 5.),
            start: (15., -10.),
            originals,
        };
        // A little past 90°, snapped to it with Shift.
        let turned = transformed(&rotating, (31., 6.), true, false).unwrap();
        assert!(turned.iter().all(|(_, f)| f.rotation == 90.));
        assert!(transformed(&Gesture::Idle, (0., 0.), false, false).is_none());
    }

    #[test]
    fn edge_handles_of_a_flat_or_thin_box_are_dropped() {
        let flat = boxed(0., 50., 100., 0.);
        let kept: Vec<Handle> = visible_handles(&flat, 6., false)
            .iter()
            .map(|(h, _)| *h)
            .collect();
        assert!(!kept.contains(&Handle::N) && !kept.contains(&Handle::S));
        assert!(kept.contains(&Handle::E) && kept.contains(&Handle::NE));
        let thin = boxed(0., 0., 4., 100.);
        let kept: Vec<Handle> = visible_handles(&thin, 6., false)
            .iter()
            .map(|(h, _)| *h)
            .collect();
        assert!(!kept.contains(&Handle::E) && !kept.contains(&Handle::W));
        assert!(kept.contains(&Handle::N));
        assert_eq!(
            visible_handles(&boxed(0., 0., 100., 50.), 6., false).len(),
            8
        );
        // Nothing to grab in the middle of a flat line.
        assert_eq!(handle_at(&flat, (50., 50.), 6., 12., false), None);
    }

    #[test]
    fn pressing_the_middle_of_a_selected_flat_line_moves_it() {
        let [line] = ids();
        let flat = boxed(0., 50., 100., 0.);
        assert_eq!(
            press_target(Some(&flat), (50., 50.), 6., 12., false, Some(line), false),
            PressKind::Move(line)
        );
        // Its ends still resize.
        assert_eq!(
            press_target(Some(&flat), (100., 50.), 6., 12., false, Some(line), false),
            PressKind::Resize(Handle::NE)
        );
    }

    #[test]
    fn a_text_box_resize_only_changes_its_width() {
        let [a] = ids();
        let resizing = |handle| Gesture::Resizing {
            handle,
            start: (100., 20.),
            originals: vec![(a, boxed(0., 0., 100., 20.))],
            auto_widths: vec![Some(true)],
        };
        // A corner drag down and right: wider, the height kept.
        let out = transformed(&resizing(Handle::SE), (150., 90.), true, false).unwrap();
        assert_eq!(out, vec![(a, boxed(0., 0., 150., 20.))]);
        // Top and bottom handles change nothing.
        let out = transformed(&resizing(Handle::S), (150., 90.), false, false).unwrap();
        assert_eq!(out, vec![(a, boxed(0., 0., 100., 20.))]);
        assert_eq!(text_box_widths_off(&resizing(Handle::E)), vec![a]);
        let shape = Gesture::Resizing {
            handle: Handle::E,
            start: (0., 0.),
            originals: vec![(a, boxed(0., 0., 100., 20.))],
            auto_widths: vec![None],
        };
        assert!(text_box_widths_off(&shape).is_empty());
        assert!(text_box_widths_off(&Gesture::Idle).is_empty());
    }

    #[test]
    fn one_selected_text_box_shows_only_side_handles() {
        let f = boxed(0., 0., 100., 30.);
        let kept: Vec<Handle> = visible_handles(&f, 6., true)
            .iter()
            .map(|(h, _)| *h)
            .collect();
        assert_eq!(kept.len(), 2);
        assert!(kept.contains(&Handle::E) && kept.contains(&Handle::W));
        // Corners are not grabbed; the sides are.
        assert_eq!(handle_at(&f, (100., 30.), 6., 12., true), None);
        assert_eq!(
            handle_at(&f, (100., 15.), 6., 12., true),
            Some(HandleHit::Resize(Handle::E))
        );
        assert_eq!(
            handle_at(&f, (50., -12.), 6., 12., true),
            Some(HandleHit::Rotate)
        );
        // Other shapes keep all eight.
        assert_eq!(visible_handles(&f, 6., false).len(), 8);
    }

    #[test]
    fn pressing_inside_a_tiny_selected_shape_moves_it() {
        let [tiny] = ids();
        let f = boxed(0., 0., 8., 8.);
        assert_eq!(
            press_target(Some(&f), (4., 4.), 6., 12., false, Some(tiny), false),
            PressKind::Move(tiny)
        );
        // Outside the body a corner still resizes.
        assert_eq!(
            press_target(Some(&f), (10., 10.), 6., 12., false, None, false),
            PressKind::Resize(Handle::SE)
        );
        // A larger shape keeps handles first.
        let big = boxed(0., 0., 100., 100.);
        assert_eq!(
            press_target(Some(&big), (2., 2.), 6., 12., false, Some(tiny), false),
            PressKind::Resize(Handle::NW)
        );
        assert_eq!(
            press_target(None, (2., 2.), 6., 12., false, None, true),
            PressKind::Marquee { additive: true }
        );
    }
}
