//! Pointer gestures on the canvas. Placing: with a shape tool, a left drag
//! draws the new shape's box; a click places it at its default size.

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::{Frame, Primitive, Pt};
use mindweave_core::{drag_create, LibraryRegistry, Shape, ShapeDef, ShapeSource};

use super::tool::Tool;
use super::EditorWindow;
use crate::libraries::Libraries;

/// Opacity of the shape previewed while placing, times its own.
pub const PREVIEW_OPACITY: f32 = 0.6;

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
    },
}

/// Whether `def` is drawn end to end (lines and arrows) rather than as a
/// box.
pub fn is_line_like(def: &ShapeDef) -> bool {
    matches!(
        def.source,
        ShapeSource::Primitive(Primitive::Line | Primitive::Arrow)
    )
}

/// The frame of `def` drawn from `start` to `end`.
pub fn placement_frame(
    def: &ShapeDef,
    start: Pt,
    end: Pt,
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
        if let Gesture::Placing {
            current,
            constrain,
            from_center,
            ..
        } = &mut self.gesture
        {
            let before = (*current, *constrain, *from_center);
            if let Some(at) = at {
                *current = at;
            }
            *constrain = modifiers.shift;
            *from_center = modifiers.alt;
            if before != (*current, *constrain, *from_center) {
                cx.notify();
            }
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

    /// Escape while placing: drops the shape being drawn.
    pub(super) fn cancel_gesture(&mut self, cx: &mut Context<Self>) {
        if self.gesture != Gesture::Idle {
            self.gesture = Gesture::Idle;
            cx.notify();
        }
    }

    /// The shape being placed, for the preview.
    pub(super) fn placement_preview(&self, libs: &LibraryRegistry) -> Option<Shape> {
        placement_shape(&self.gesture, libs)
    }
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
    } = gesture
    else {
        return None;
    };
    let def = libs.shape(library, shape)?;
    let frame = placement_frame(def, *start, *current, *constrain, *from_center);
    Some(Shape::new(def, library, frame))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{is_line_like, placement_frame, placement_shape, Gesture};
    use mindweave_core::{builtin_libraries, LibraryRegistry};

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
        let f = placement_frame(line, (100., 100.), (40., 70.), false, false);
        assert_eq!((f.x, f.y, f.w, f.h), (40., 70., 60., 30.));
        assert!(f.flip_x && f.flip_y);

        let rect = libs.shape("basic-shapes", "rectangle").unwrap();
        let f = placement_frame(rect, (100., 100.), (101., 100.), false, false);
        let (w, h) = rect.default_size;
        assert_eq!((f.x, f.y, f.w, f.h), (100. - w / 2., 100. - h / 2., w, h));
    }

    #[test]
    fn shift_squares_and_alt_draws_from_the_centre() {
        let libs = registry();
        let rect = libs.shape("basic-shapes", "rectangle").unwrap();
        let f = placement_frame(rect, (0., 0.), (40., 10.), true, false);
        assert_eq!((f.w, f.h), (40., 40.));
        let f = placement_frame(rect, (50., 50.), (60., 70.), false, true);
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
}
