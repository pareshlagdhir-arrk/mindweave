//! The infinite canvas: paints the active page's background pattern and
//! shapes and selection, turns scroll, pinch and drag input into viewport
//! changes, and passes left drags to placement or the Pointer tool's
//! gestures.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mindweave_core::geometry::Rect;
use mindweave_core::{
    dot_stride, grid_lines, is_major_line, pinch_zoom_factor, scroll_zoom_factor, stride_offset,
    BackgroundPattern, CanvasSettings, Page, Viewport, ZOOM_STEP,
};

use super::interaction::{
    marquee_rect, visible_handles, Gesture, HANDLE_GRAB_PX, PREVIEW_OPACITY,
    ROTATE_HANDLE_OFFSET_PX,
};
use super::shape_paint::{paint_marquee, paint_preview, paint_selection, paint_shapes};
use super::tool::Tool;
use super::{hex_to_hsla, EditorWindow};
use crate::actions::{ZoomIn, ZoomOut, ZoomReset};
use crate::libraries::Libraries;

/// Width of a major grid line, in pixels (other lines are 1px).
const MAJOR_LINE_WIDTH: f32 = 2.;
/// Most dots painted in one frame; denser patterns skip lines (see
/// [`dot_stride`]).
const MAX_DOTS: usize = 40_000;

/// The canvas area a canvas of `size` pixels shows through `viewport`, in
/// canvas units.
pub(super) fn visible_rect(viewport: &Viewport, size: (f32, f32)) -> Rect {
    let (x, y) = viewport.screen_to_canvas((0., 0.));
    Rect {
        x,
        y,
        w: size.0 / viewport.zoom,
        h: size.1 / viewport.zoom,
    }
}

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
            editor.update(cx, |editor, cx| editor.paint_content(bounds, window, cx));
            let (dragging, hand, crosshair, pointer) = {
                let editor = editor.read(cx);
                let pointer = (editor.tools.tool == Tool::Pointer)
                    .then(|| {
                        let at = editor.pointer_local.map(|p| editor.to_canvas(p));
                        editor.pointer_cursor(at)
                    })
                    .flatten();
                (
                    editor.pan_drag.is_some(),
                    editor.left_drag_pans(),
                    matches!(editor.tools.tool, Tool::Shape { .. })
                        || matches!(editor.gesture, Gesture::Placing { .. }),
                    pointer,
                )
            };
            if dragging {
                window.set_cursor_style(CursorStyle::ClosedHand, &hitbox);
            } else if hand {
                window.set_cursor_style(CursorStyle::OpenHand, &hitbox);
            } else if crosshair {
                window.set_cursor_style(CursorStyle::Crosshair, &hitbox);
            } else if let Some(cursor) = pointer {
                window.set_cursor_style(cursor, &hitbox);
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
    // drag with Space held or the Hand tool, starts a pan; a left press
    // with a shape tool starts placing a shape, and with the Pointer tool
    // selects, moves, resizes or rotates.
    window.on_mouse_event({
        let editor = editor.clone();
        let hitbox = hitbox.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            editor.update(cx, |editor, cx| {
                // A shape value being typed or picked belongs to the
                // selection the press may be about to change.
                // The colour drag ends first, so the typed value is a step
                // of its own.
                editor.finish_color_edit(cx);
                editor.commit_focused_shape_number(window, cx);
                editor.focus_handle.focus(window, cx);
                let pans = match event.button {
                    MouseButton::Middle => true,
                    MouseButton::Left => editor.left_drag_pans(),
                    _ => false,
                };
                if pans {
                    editor.pan_drag = Some(event.position);
                    cx.notify();
                } else if event.button == MouseButton::Left {
                    let at = editor.to_canvas(local(event.position, bounds));
                    match editor.tools.tool {
                        Tool::Pointer => editor.begin_pointer_press(at, event.modifiers, cx),
                        Tool::Shape { .. } => editor.begin_placing(at, event.modifiers, window, cx),
                        Tool::Hand => {}
                    }
                }
            });
        }
    });

    // Pointer movement: continues a pan or a gesture (even outside the
    // canvas) and reports the pointer's canvas position to the status bar.
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
                if editor.gesture != Gesture::Idle {
                    let at = editor.to_canvas(local(event.position, bounds));
                    if event.pressed_button.is_some() {
                        editor.update_gesture(Some(at), event.modifiers, cx);
                    } else {
                        // The button was released where we could not see it.
                        editor.finish_gesture(Some(at), event.modifiers, cx);
                    }
                }
                editor.refresh_cursor(cx);
            });
        }
    });

    window.on_mouse_event({
        let editor = editor.clone();
        move |event: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            editor.update(cx, |editor, cx| {
                if editor.pan_drag.take().is_some() {
                    cx.notify();
                }
                if event.button == MouseButton::Left && editor.gesture != Gesture::Idle {
                    let at = editor.to_canvas(local(event.position, bounds));
                    editor.finish_gesture(Some(at), event.modifiers, cx);
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
    /// Paints the active page's shapes, the shape being placed, the
    /// selection chrome and the marquee.
    fn paint_content(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let libs = cx.global::<Libraries>().0.clone();
        let page = self.session.page();
        let viewport = page.viewport;
        paint_shapes(
            window,
            cx,
            bounds,
            page,
            &libs,
            &viewport,
            &mut self.geometry_cache,
        );
        if let Some(shape) = self.placement_preview(&libs) {
            paint_preview(
                window,
                cx,
                bounds,
                &shape,
                &libs,
                &viewport,
                PREVIEW_OPACITY,
            );
        }
        let accent = cx.theme().primary;
        if let Some(chrome) = self.selection_chrome() {
            paint_selection(
                window,
                bounds,
                &viewport,
                &chrome,
                ROTATE_HANDLE_OFFSET_PX,
                &visible_handles(&chrome, HANDLE_GRAB_PX / viewport.zoom),
                accent,
            );
        }
        if let Gesture::Marquee { start, current, .. } = self.gesture {
            if self.press.dragged {
                paint_marquee(
                    window,
                    bounds,
                    &viewport,
                    &marquee_rect(start, current),
                    accent,
                );
            }
        }
    }

    /// A point relative to the canvas origin, in canvas coordinates.
    pub(super) fn to_canvas(&self, local: (f32, f32)) -> (f32, f32) {
        self.session.page().viewport.screen_to_canvas(local)
    }

    /// Whether a left drag on the canvas pans: with Space held or the Hand
    /// tool active.
    fn left_drag_pans(&self) -> bool {
        self.space_held || self.tools.tool == Tool::Hand
    }

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
            self.set_viewport(viewport, cx);
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

    /// The canvas area in view, in canvas units; empty at the viewport's
    /// corner before the canvas has been painted.
    pub(super) fn visible_canvas(&self) -> Rect {
        let size = self
            .canvas_bounds
            .map(|b| (f32::from(b.size.width), f32::from(b.size.height)))
            .unwrap_or_default();
        visible_rect(&self.session.page().viewport, size)
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

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::visible_rect;
    use mindweave_core::geometry::Rect;
    use mindweave_core::Viewport;

    #[test]
    fn the_visible_area_is_the_canvas_size_in_canvas_units() {
        let at_rest = Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        };
        assert_eq!(
            visible_rect(&at_rest, (800.0, 600.0)),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0
            }
        );
        let zoomed = Viewport {
            x: -100.0,
            y: 50.0,
            zoom: 2.0,
        };
        assert_eq!(
            visible_rect(&zoomed, (800.0, 600.0)),
            Rect {
                x: -100.0,
                y: 50.0,
                w: 400.0,
                h: 300.0
            }
        );
        // Before the first paint: the viewport's corner, empty.
        assert_eq!(visible_rect(&zoomed, (0.0, 0.0)).w, 0.0);
    }
}
