//! The infinite canvas: paints the active page's background pattern and
//! shapes and selection, turns scroll, pinch and drag input into viewport
//! changes, and passes left drags to placement or the Pointer tool's
//! gestures.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mindweave_core::geometry::{Pt, Rect};
use mindweave_core::{
    dot_stride, grid_lines, is_major_line, pinch_zoom_factor, scroll_zoom_factor, stride_offset,
    BackgroundPattern, CanvasSettings, Page, Viewport, TOPIC_TEXT_MAX, ZOOM_STEP,
};
use uuid::Uuid;

use super::dot_tile::DotTile;
use super::interaction::{
    marquee_rect, visible_handles, Gesture, HANDLE_GRAB_PX, PREVIEW_OPACITY,
    ROTATE_HANDLE_OFFSET_PX,
};
use super::shape_gallery;
use super::shape_paint::{
    paint_artboard_names, paint_highlight, paint_line_ends, paint_marquee, paint_preview,
    paint_selection, paint_shapes, AutoColors,
};
use super::text_paint::{
    auto_color, canvas_background, equation_color, link_color, text_background, text_box_wrap,
    TextPaint, TEXT_BOX_PADDING,
};
use super::tool::Tool;
use super::{hex_to_hsla, EditorWindow, RenameTarget};
use crate::actions::{ZoomIn, ZoomOut, ZoomReset};
use crate::equation::equation_font_size;
use crate::libraries::Libraries;

/// Width of a major grid line, in pixels (other lines are 1px).
const MAJOR_LINE_WIDTH: f32 = 2.;
/// Most dots painted in one frame; denser patterns skip lines (see
/// [`dot_stride`]).
const MAX_DOTS: usize = 40_000;

/// Mixed into a topic's id to name the layout its size is measured from.
const TOPIC_MEASURE_ID: u128 = 0x6d69_6e64_7765_6176_6574_6f70_6963_7321;

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
            editor.update(cx, |editor, cx| {
                paint_background(
                    bounds,
                    &settings,
                    &viewport,
                    &mut editor.dot_tile,
                    window,
                    cx,
                );
                editor.paint_content(bounds, window, cx)
            });
            let (dragging, hand, ibeam, crosshair, text_tool, pointer) = {
                let editor = editor.read(cx);
                let ibeam = editor
                    .pointer_local
                    .is_some_and(|p| editor.over_edited_text(editor.to_canvas(p)));
                let pointer = (editor.tools.tool == Tool::Pointer)
                    .then(|| {
                        let at = editor.pointer_local.map(|p| editor.to_canvas(p));
                        editor.pointer_cursor(at)
                    })
                    .flatten();
                (
                    editor.pan_drag.is_some(),
                    editor.left_drag_pans(),
                    ibeam,
                    matches!(
                        editor.tools.tool,
                        Tool::Shape { .. } | Tool::Relationship | Tool::Artboard
                    ) || matches!(editor.gesture, Gesture::Placing { .. }),
                    editor.tools.tool == Tool::Text,
                    pointer,
                )
            };
            if dragging {
                window.set_cursor_style(CursorStyle::ClosedHand, &hitbox);
            } else if hand {
                window.set_cursor_style(CursorStyle::OpenHand, &hitbox);
            } else if ibeam || text_tool {
                window.set_cursor_style(CursorStyle::IBeam, &hitbox);
            } else if crosshair {
                window.set_cursor_style(CursorStyle::Crosshair, &hitbox);
            } else if let Some(cursor) = pointer {
                window.set_cursor_style(cursor, &hitbox);
            }
            // Typing and input methods reach the text being edited while
            // the editor has focus.
            let input_focus = {
                let editor = editor.read(cx);
                editor
                    .text_edit
                    .is_some()
                    .then(|| editor.focus_handle.clone())
            };
            if let Some(focus) = input_focus {
                window.handle_input(&focus, ElementInputHandler::new(bounds, editor.clone()), cx);
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
    dots: &mut DotTile,
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
            let stride = dot_stride(grid.xs.len(), grid.ys.len(), MAX_DOTS);
            // Anchor the thinning to canvas grid indices so the kept dots
            // stay on the same canvas points while panning.
            let first_x = grid.xs.get(stride_offset(grid.first_ix.0, stride));
            let first_y = grid.ys.get(stride_offset(grid.first_ix.1, stride));
            if let (Some(&x), Some(&y)) = (first_x, first_y) {
                let first = point(origin.x + px(x), origin.y + px(y));
                let period = grid.spacing_px * stride as f32;
                dots.paint(bounds, first, period, settings.dot_size, color, window);
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
                editor.finish_size_steps(cx);
                editor.commit_focused_shape_number(window, cx);
                editor.commit_focused_text_size(window, cx);
                editor.close_link_popover(window, cx);
                editor.focus_handle.focus(window, cx);
                let pans = match event.button {
                    MouseButton::Middle => true,
                    MouseButton::Left => editor.left_drag_pans(),
                    _ => false,
                };
                // While editing text, a left press on it places the caret
                // or selects; anywhere else it stops editing and then does
                // what it would have done. A note is edited in its card,
                // which takes the presses on it: a press here closes it.
                if !pans && event.button == MouseButton::Left && editor.note_card.is_some() {
                    let at = editor.to_canvas(local(event.position, bounds));
                    if editor.press_outside_note_card(at, event.click_count, window, cx) {
                        return;
                    }
                }
                if !pans && event.button == MouseButton::Left && editor.text_edit.is_some() {
                    let at = editor.to_canvas(local(event.position, bounds));
                    let pressed = editor.text_press(
                        at,
                        event.click_count,
                        event.modifiers.shift,
                        event.modifiers.platform,
                        window,
                        cx,
                    );
                    if pressed {
                        return;
                    }
                    editor.end_text_edit(window, cx);
                }
                if pans {
                    editor.pan_drag = Some(event.position);
                    cx.notify();
                } else if event.button == MouseButton::Right {
                    let at = editor.to_canvas(local(event.position, bounds));
                    editor.select_for_menu(at, cx);
                } else if event.button == MouseButton::Left {
                    let at = editor.to_canvas(local(event.position, bounds));
                    // The second click of a double click on a gallery
                    // shape, which closed the gallery, places nothing.
                    let stray = matches!(editor.tools.tool, Tool::Shape { .. })
                        && shape_gallery::is_stray_second_click(
                            event.click_count,
                            editor.ribbon.gallery_closed_at.map(|at| at.elapsed()),
                        );
                    if stray {
                        return;
                    }
                    // A double click (with any tool but Hand, which pans)
                    // opens a topic's equation card on its equation, else
                    // edits text: the first click did what it does.
                    // The later clicks of a double click on a note badge
                    // add nothing: the first opened or closed its card.
                    if event.click_count >= 2 && editor.note_badge_at(at).is_some() {
                        return;
                    }
                    // A double click on an artboard's name renames it.
                    let board = (event.click_count == 2)
                        .then(|| editor.artboard_name_at(at))
                        .flatten();
                    if let Some(board) = board {
                        editor.cancel_gesture(cx);
                        editor.begin_rename(RenameTarget::Artboard(board), window, cx);
                        return;
                    }
                    if event.click_count == 2 {
                        match editor.equation_at(at) {
                            Some(topic) => editor.open_equation_card(topic, window, cx),
                            None => editor.edit_text_at(at, window, cx),
                        }
                        return;
                    }
                    match editor.tools.tool {
                        Tool::Pointer => {
                            editor.begin_pointer_press(at, event.modifiers, window, cx)
                        }
                        Tool::Shape { .. } | Tool::Relationship | Tool::Artboard => {
                            editor.begin_placing(at, event.modifiers, window, cx)
                        }
                        Tool::Text => editor.place_text_box(at, window, cx),
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
                if event.pressed_button == Some(MouseButton::Left) {
                    // A drag selection may have begun in the note card.
                    let at = editor.edit_point(local(event.position, bounds));
                    editor.text_drag_to(at, window, cx);
                } else {
                    // The button was released where we could not see it.
                    editor.end_text_drag();
                }
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
                if event.button == MouseButton::Left {
                    editor.end_text_drag();
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
        let canvas = canvas_background(
            self.session.page().canvas.background.as_deref(),
            cx.theme().background,
        );
        let theme = cx.theme();
        let links = link_color(theme.link, theme.primary);
        self.text_layouts.begin_frame(window, links);
        self.equation_cache.begin_frame();
        self.layout_text_boxes(window, canvas, cx);
        let page = self.session.page();
        let viewport = page.viewport;
        let auto = AutoColors::new(page.canvas.background.as_deref(), cx);
        let mut text = TextPaint {
            layouts: &mut self.text_layouts,
            glyphs: &mut self.glyph_outlines,
            canvas,
        };
        paint_shapes(
            window,
            cx,
            bounds,
            page,
            &libs,
            &viewport,
            &mut self.geometry_cache,
            &mut self.image_cache,
            &mut self.equation_cache,
            &mut text,
        );
        self.artboard_names =
            paint_artboard_names(window, cx, bounds, page, &viewport, &self.selection);
        self.equation_cache.end_frame(window);
        self.paint_text_edit(bounds, window, cx);
        self.text_layouts.end_frame();
        let preview = match self.gesture {
            Gesture::Placing { .. } => {
                let remembered = &cx.global::<crate::app_state::AppState>().settings;
                self.placement_preview(&libs, remembered)
            }
            _ => None,
        };
        if let Some(shape) = preview {
            paint_preview(
                window,
                cx,
                bounds,
                &shape,
                &libs,
                &viewport,
                PREVIEW_OPACITY,
                auto,
            );
        }
        let accent = cx.theme().primary;
        if let Some(shape) = self
            .attach_highlight
            .and_then(|id| self.session.find_shape(id))
        {
            let outline = self.geometry_cache.outline(shape, &libs).geometry.clone();
            paint_highlight(window, bounds, &viewport, &outline, accent);
        }
        if let Some(ends) = self.selected_line_ends() {
            paint_line_ends(window, bounds, &viewport, &ends, accent);
        }
        if let Some(chrome) = self.selection_chrome() {
            let handles = self.chrome_has_handles();
            let resize = if handles {
                visible_handles(
                    &chrome,
                    HANDLE_GRAB_PX / viewport.zoom,
                    self.chrome_width_only(),
                )
            } else {
                Vec::new()
            };
            paint_selection(
                window,
                bounds,
                &viewport,
                &chrome,
                (handles && self.chrome_rotates()).then_some(ROTATE_HANDLE_OFFSET_PX),
                &resize,
                accent,
            );
        }
        // Handles to bend the selected line and resize a topic's picture.
        let knobs: Vec<Pt> = self
            .bend_handle()
            .into_iter()
            .chain(self.picture_handle())
            .collect();
        if !knobs.is_empty() {
            paint_line_ends(window, bounds, &viewport, &knobs, accent);
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

    /// Lays out the active page's text boxes and stores the size their text
    /// gives each (see `DocumentSession::set_text_box_layout`). It runs
    /// every frame and compares with every box's frame, so sizes left stale
    /// by undo, redo or opening a file are corrected before painting.
    /// Sizes come from layouts at zoom 1, so zooming changes nothing here.
    /// Topics' equations are rendered and their sizes stored first, so
    /// the same pass fits each topic to its equation.
    fn layout_text_boxes(
        &mut self,
        window: &mut Window,
        canvas: (u8, u8, u8),
        cx: &mut Context<Self>,
    ) {
        let generation = self.session.layout_generation();
        self.measure_equations(canvas);
        let page = self.session.page();
        let mut changes = Vec::new();
        for shape in page.layers.iter().flat_map(|l| &l.objects) {
            let Some(label) = shape.label.as_ref().filter(|_| shape.is_text_box()) else {
                continue;
            };
            let background = text_background(shape.style.fill.as_deref(), canvas);
            let color = auto_color(background, shape.style.opacity);
            let wrap = text_box_wrap(shape);
            let (w, h) = self
                .text_layouts
                .size(window, shape.id, &label.text, wrap, color);
            let (w, h) = (w + 2. * TEXT_BOX_PADDING, h + 2. * TEXT_BOX_PADDING);
            let f = &shape.frame;
            if (shape.auto_width() && f.w != w) || f.h != h {
                changes.push((shape.id, w, h));
            }
        }
        // Topics fit their text (wrapped at TOPIC_TEXT_MAX), icon, image,
        // badges and equation.
        let mut topics = Vec::new();
        for shape in page.layers.iter().flat_map(|l| &l.objects) {
            let (Some(t), Some(label)) = (&shape.topic, &shape.label) else {
                continue;
            };
            let background = text_background(shape.style.fill.as_deref(), canvas);
            let color = auto_color(background, shape.style.opacity);
            // Measured under an id of its own: painting lays the text out
            // at the topic's width, and one cache entry per id would
            // otherwise be laid out twice a frame.
            // Unwrapped first, so a short text gets a snug topic; wrapped
            // at TOPIC_TEXT_MAX only when longer than that.
            let measure = Uuid::from_u128(shape.id.as_u128() ^ TOPIC_MEASURE_ID);
            let (mut tw, mut th) =
                self.text_layouts
                    .size(window, measure, &label.text, None, color);
            if tw > TOPIC_TEXT_MAX {
                let wrapped = Uuid::from_u128(shape.id.as_u128() ^ TOPIC_MEASURE_ID.rotate_left(1));
                (tw, th) = self.text_layouts.size(
                    window,
                    wrapped,
                    &label.text,
                    Some(TOPIC_TEXT_MAX),
                    color,
                );
            }
            let (w, h) = t.size_for_text((tw, th));
            if (shape.frame.w, shape.frame.h) != (w, h) {
                topics.push((shape.id, w, h));
            }
        }
        for (id, w, h) in changes {
            self.session.set_text_box_layout(id, w, h);
        }
        for (id, w, h) in topics {
            self.session.set_topic_size(id, w, h);
        }
        // The selection chrome and the sidebar were drawn with the old
        // size. A size the frame cannot hold (past its clamp) changes
        // nothing, and must not repaint every frame.
        if self.session.layout_generation() != generation {
            cx.notify();
        }
    }

    /// Renders the active page's equations at their topic's text size, in
    /// its text colour (as painting does, so the render is reused),
    /// and stores the sizes that changed; one that does not render is
    /// `(0, 0)`, shown as nothing.
    fn measure_equations(&mut self, canvas: (u8, u8, u8)) {
        let page = self.session.page();
        let mut sizes = Vec::new();
        for shape in page.layers.iter().flat_map(|l| &l.objects) {
            let Some(eq) = shape.topic.as_ref().and_then(|t| t.equation.as_ref()) else {
                continue;
            };
            let font_size = equation_font_size(shape.label.as_ref().map(|l| &l.text));
            let color = equation_color(shape, canvas);
            let size = self
                .equation_cache
                .get(&eq.latex, color, font_size)
                .as_ref()
                .map_or((0., 0.), |r| r.size);
            if size != eq.size {
                sizes.push((shape.id, size));
            }
        }
        for (id, size) in sizes {
            self.session.set_topic_equation_size(id, size);
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
