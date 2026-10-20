//! The infinite canvas: paints the active page's background pattern and
//! turns scroll, pinch and drag input into viewport changes.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mindweave_core::{
    dot_stride, grid_lines, pinch_zoom_factor, scroll_zoom_factor, BackgroundPattern, Page,
    Viewport, ZOOM_STEP,
};

use super::EditorWindow;
use crate::actions::{ZoomIn, ZoomOut, ZoomReset};

/// Side of a dot in the Dots pattern, in pixels.
const DOT_SIZE: f32 = 1.5;
/// Most dots painted in one frame; denser patterns skip lines (see
/// [`dot_stride`]).
const MAX_DOTS: usize = 40_000;

/// The canvas element for `page`, filling its bounds.
pub fn canvas(editor: Entity<EditorWindow>, page: &Page) -> impl IntoElement {
    let pattern = page.canvas.pattern;
    let grid_size = page.canvas.grid_size;
    let viewport = page.viewport;
    let prepaint_editor = editor.clone();
    gpui_kit::canvas(
        move |bounds, window, cx| {
            prepaint_editor.update(cx, |editor, _| editor.canvas_bounds = Some(bounds));
            window.insert_hitbox(bounds, HitboxBehavior::Normal)
        },
        move |bounds, hitbox, window, cx| {
            paint_background(bounds, pattern, grid_size, &viewport, window, cx);
            let (dragging, space_held) = {
                let editor = editor.read(cx);
                (editor.pan_drag.is_some(), editor.space_held)
            };
            if dragging {
                window.set_cursor_style(CursorStyle::ClosedHand, &hitbox);
            } else if space_held {
                window.set_cursor_style(CursorStyle::OpenHand, &hitbox);
            }
            register_input(editor, bounds, hitbox, window);
        },
    )
    .size_full()
}

fn paint_background(
    bounds: Bounds<Pixels>,
    pattern: BackgroundPattern,
    grid_size: f32,
    viewport: &Viewport,
    window: &mut Window,
    cx: &mut App,
) {
    let theme = cx.theme();
    window.paint_quad(fill(bounds, theme.background));
    if pattern == BackgroundPattern::None {
        return;
    }
    let extent = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let grid = grid_lines(viewport, extent, grid_size);
    let color = theme.border.opacity(grid.alpha);
    let origin = bounds.origin;
    window.with_content_mask(Some(ContentMask { bounds }), |window| match pattern {
        BackgroundPattern::Lines => {
            for &x in &grid.xs {
                let line = Bounds::new(
                    point(origin.x + px(x), origin.y),
                    size(px(1.), bounds.size.height),
                );
                window.paint_quad(fill(line, color));
            }
            for &y in &grid.ys {
                let line = Bounds::new(
                    point(origin.x, origin.y + px(y)),
                    size(bounds.size.width, px(1.)),
                );
                window.paint_quad(fill(line, color));
            }
        }
        BackgroundPattern::Dots => {
            let stride = dot_stride(grid.xs.len(), grid.ys.len(), MAX_DOTS);
            let half = DOT_SIZE / 2.;
            for &y in grid.ys.iter().step_by(stride) {
                for &x in grid.xs.iter().step_by(stride) {
                    let dot = Bounds::new(
                        point(origin.x + px(x - half), origin.y + px(y - half)),
                        size(px(DOT_SIZE), px(DOT_SIZE)),
                    );
                    window.paint_quad(fill(dot, color));
                }
            }
        }
        BackgroundPattern::None => {}
    });
}

/// `position` relative to the canvas origin, as a screen-pixel tuple.
fn local(position: Point<Pixels>, bounds: Bounds<Pixels>) -> (f32, f32) {
    let p = position - bounds.origin;
    (f32::from(p.x), f32::from(p.y))
}

/// Registers this frame's pointer listeners for the canvas.
fn register_input(
    editor: Entity<EditorWindow>,
    bounds: Bounds<Pixels>,
    hitbox: Hitbox,
    window: &mut Window,
) {
    // Two-finger scroll pans; with Cmd it zooms around the pointer.
    window.on_mouse_event({
        let editor = editor.clone();
        let hitbox = hitbox.clone();
        move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            let delta = event.delta.pixel_delta(window.line_height());
            let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
            let anchor = local(event.position, bounds);
            editor.update(cx, |editor, cx| {
                if event.modifiers.platform {
                    editor.change_viewport(|vp| vp.zoom_at(scroll_zoom_factor(dy), anchor), cx);
                } else {
                    // Scroll deltas follow the fingers (gpui scrolls content
                    // by +delta), and `pan_by` moves content with them.
                    editor.change_viewport(|vp| vp.pan_by(dx, dy), cx);
                }
            });
            cx.stop_propagation();
        }
    });

    // Trackpad pinch zooms around the pinch centre.
    window.on_mouse_event({
        let editor = editor.clone();
        let hitbox = hitbox.clone();
        move |event: &PinchEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            let anchor = local(event.position, bounds);
            let factor = pinch_zoom_factor(event.delta);
            editor.update(cx, |editor, cx| {
                editor.change_viewport(|vp| vp.zoom_at(factor, anchor), cx)
            });
            cx.stop_propagation();
        }
    });

    // A click focuses the editor (so Space can pan); middle drag, or left
    // drag with Space held, starts a pan.
    window.on_mouse_event({
        let editor = editor.clone();
        let hitbox = hitbox.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            editor.update(cx, |editor, cx| {
                editor.focus_handle.focus(window, cx);
                let pans = match event.button {
                    MouseButton::Middle => true,
                    MouseButton::Left => editor.space_held,
                    _ => false,
                };
                if pans {
                    editor.pan_drag = Some(event.position);
                    cx.notify();
                }
            });
        }
    });

    // Pointer movement: continues a pan (even outside the canvas) and
    // reports the pointer's canvas position to the status bar.
    window.on_mouse_event({
        let editor = editor.clone();
        let hitbox = hitbox.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = hitbox.is_hovered(window);
            editor.update(cx, |editor, cx| {
                if let Some(last) = editor.pan_drag {
                    if event.pressed_button.is_some() {
                        let d = event.position - last;
                        editor.pan_drag = Some(event.position);
                        editor.change_viewport(|vp| vp.pan_by(f32::from(d.x), f32::from(d.y)), cx);
                    } else {
                        // The button was released where we could not see it.
                        editor.pan_drag = None;
                        cx.notify();
                    }
                }
                let cursor = hovered.then(|| {
                    editor
                        .session
                        .page()
                        .viewport
                        .screen_to_canvas(local(event.position, bounds))
                });
                editor.set_cursor(cursor, cx);
            });
        }
    });

    window.on_mouse_event({
        let editor = editor.clone();
        move |_: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            editor.update(cx, |editor, cx| {
                if editor.pan_drag.take().is_some() {
                    cx.notify();
                }
            });
        }
    });

    // Leaving the window hides the pointer position.
    window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            editor.update(cx, |editor, cx| editor.set_cursor(None, cx));
        }
    });
}

impl EditorWindow {
    /// Applies `f` to a copy of the active page's viewport and stores the
    /// result when it changed. Viewport changes never mark the document
    /// dirty.
    pub(super) fn change_viewport(
        &mut self,
        f: impl FnOnce(&mut Viewport),
        cx: &mut Context<Self>,
    ) {
        let mut viewport = self.session.page().viewport;
        f(&mut viewport);
        if viewport != self.session.page().viewport {
            self.update_session(|s| s.set_viewport(viewport), cx);
        }
    }

    /// The centre of the canvas, relative to its origin; the origin itself
    /// before the canvas has been painted.
    fn canvas_centre(&self) -> (f32, f32) {
        self.canvas_bounds
            .map(|b| (f32::from(b.size.width) / 2., f32::from(b.size.height) / 2.))
            .unwrap_or_default()
    }

    /// Zooms by `factor` around the canvas centre.
    fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        let anchor = self.canvas_centre();
        self.change_viewport(|vp| vp.zoom_at(factor, anchor), cx);
    }

    pub(super) fn on_zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(ZOOM_STEP, cx);
    }

    pub(super) fn on_zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(1. / ZOOM_STEP, cx);
    }

    /// Back to exactly 100%, keeping the canvas centre in place.
    pub(super) fn on_zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.canvas_centre();
        self.change_viewport(|vp| vp.zoom_to(1., anchor), cx);
    }
}
