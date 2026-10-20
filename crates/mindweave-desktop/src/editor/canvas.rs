//! The infinite canvas: paints the active page's background pattern and
//! turns scroll, pinch and drag input into viewport changes.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mindweave_core::{
    dot_stride, grid_lines, is_major_line, pinch_zoom_factor, scroll_zoom_factor, stride_offset,
    BackgroundPattern, CanvasSettings, Page, Viewport, ZOOM_STEP,
};

use super::{hex_to_hsla, EditorWindow};
use crate::actions::{ZoomIn, ZoomOut, ZoomReset};

/// Width of a major grid line, in pixels (other lines are 1px).
const MAJOR_LINE_WIDTH: f32 = 2.;
/// Most dots painted in one frame; denser patterns skip lines (see
/// [`dot_stride`]).
const MAX_DOTS: usize = 40_000;

/// The canvas element for `page`, filling its bounds.
pub fn canvas(editor: Entity<EditorWindow>, page: &Page) -> impl IntoElement {
    let settings = page.canvas.clone();
    let viewport = page.viewport;
    let prepaint_editor = editor.clone();
    gpui_kit::canvas(
        move |bounds, window, cx| {
            prepaint_editor.update(cx, |editor, _| editor.canvas_bounds = Some(bounds));
            window.insert_hitbox(bounds, HitboxBehavior::Normal)
        },
        move |bounds, hitbox, window, cx| {
            paint_background(bounds, &settings, &viewport, window, cx);
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
    settings: &CanvasSettings,
    viewport: &Viewport,
    window: &mut Window,
    cx: &mut App,
) {
    let theme = cx.theme();
    let color = |hex: &Option<String>, theme_color: Hsla| {
        hex.as_deref().and_then(hex_to_hsla).unwrap_or(theme_color)
    };
    window.paint_quad(fill(bounds, color(&settings.background, theme.background)));
    let pattern = settings.pattern;
    if pattern == BackgroundPattern::None {
        return;
    }
    let extent = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let grid = grid_lines(viewport, extent, settings.grid_size);
    let origin = bounds.origin;
    window.with_content_mask(Some(ContentMask { bounds }), |window| match pattern {
        BackgroundPattern::Lines => {
            let base = color(&settings.line_color, theme.border);
            let minor = base.opacity(grid.alpha);
            // Major lines: full opacity and wider, centred on the grid line.
            let line = |ix: i64| {
                if is_major_line(ix, grid.cells_per_line, settings.major_every) {
                    (base, MAJOR_LINE_WIDTH)
                } else {
                    (minor, 1.)
                }
            };
            for (k, &x) in grid.xs.iter().enumerate() {
                let (color, width) = line(grid.first_ix.0.wrapping_add(k as i64));
                let quad = Bounds::new(
                    point(origin.x + px(x - (width - 1.) / 2.), origin.y),
                    size(px(width), bounds.size.height),
                );
                window.paint_quad(fill(quad, color));
            }
            for (k, &y) in grid.ys.iter().enumerate() {
                let (color, width) = line(grid.first_ix.1.wrapping_add(k as i64));
                let quad = Bounds::new(
                    point(origin.x, origin.y + px(y - (width - 1.) / 2.)),
                    size(bounds.size.width, px(width)),
                );
                window.paint_quad(fill(quad, color));
            }
        }
        BackgroundPattern::Dots => {
            let color = color(&settings.dot_color, theme.border).opacity(grid.alpha);
            let dot_size = settings.dot_size;
            let stride = dot_stride(grid.xs.len(), grid.ys.len(), MAX_DOTS);
            let half = dot_size / 2.;
            // Anchor the thinning to canvas grid indices so the kept dots
            // stay on the same canvas points while panning.
            let skip_x = stride_offset(grid.first_ix.0, stride);
            let skip_y = stride_offset(grid.first_ix.1, stride);
            for &y in grid.ys.iter().skip(skip_y).step_by(stride) {
                for &x in grid.xs.iter().skip(skip_x).step_by(stride) {
                    let dot = Bounds::new(
                        point(origin.x + px(x - half), origin.y + px(y - half)),
                        size(px(dot_size), px(dot_size)),
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
                editor.pointer_local = Some(anchor);
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
                editor.pointer_local = Some(anchor);
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
            let pointer = hitbox
                .is_hovered(window)
                .then(|| local(event.position, bounds));
            editor.update(cx, |editor, cx| {
                editor.pointer_local = pointer;
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
                editor.refresh_cursor(cx);
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
            editor.update(cx, |editor, cx| {
                editor.pointer_local = None;
                editor.refresh_cursor(cx);
            });
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
            self.refresh_cursor(cx);
        }
    }

    /// Recomputes the status bar's pointer position from the last pointer
    /// position over the canvas and the active page's viewport, so it stays
    /// right when the viewport moves under a still pointer.
    pub(super) fn refresh_cursor(&mut self, cx: &mut Context<Self>) {
        let viewport = self.session.page().viewport;
        let cursor = self.pointer_local.map(|p| viewport.screen_to_canvas(p));
        self.set_cursor(cursor, cx);
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
