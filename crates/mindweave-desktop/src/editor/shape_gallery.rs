//! The More gallery of the ribbon: every shape of a library, filtered by a
//! search field.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    v_flex, ActiveTheme as _, Icon as ComponentIcon, IconName, Selectable as _, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::{Library, ShapeDef};

use super::toolbar::{button_icon, lock_dot, shape_button, ToolButton, MORE_BUTTON_WIDTH};
use super::EditorWindow;

/// The gallery's width and height limits, padding included.
const GALLERY_WIDTH: f32 = 360.;
const GALLERY_MAX_HEIGHT: f32 = 400.;
/// The popover's padding on each side.
const GALLERY_PADDING: f32 = 12.;
const SEARCH_HEIGHT: f32 = 28.;
const SECTION_GAP: f32 = 8.;
const CELL_SIZE: f32 = 64.;
const CELL_LABEL_SIZE: f32 = 11.;
/// How long a single click waits for a second one before closing the
/// gallery, so a double click can still lock the tool.
const CLOSE_DELAY: Duration = Duration::from_millis(300);

/// The More button and its gallery of `library`'s shapes. A click on a
/// shape selects its tool, a double click also locks it; either closes the
/// gallery, as does Escape. The button shows as selected when
/// `active_hidden`: the active tool is one the row has no room for.
pub(super) fn more_button(
    editor: &EditorWindow,
    library: &Library,
    active_hidden: bool,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let search = editor.ribbon.search.clone();
    let query = search.read(cx).value();
    let cells: Vec<ToolButton> = filter_shapes(&library.shapes, &query)
        .into_iter()
        .enumerate()
        .map(|(ix, shape)| shape_button(&library.id, ("gallery-shape", ix), shape, cx))
        .collect();
    let cells = Rc::new(cells);
    let selected = editor.tools.tool.clone();
    let locked = editor.tools.locked;
    let this = cx.entity().downgrade();
    let search_focus = search.focus_handle(cx);
    let clear = search.clone();

    Popover::new("shape-gallery")
        .anchor(Anchor::TopLeft)
        .track_focus(&search_focus)
        .on_open_change(move |open, window, cx| {
            if *open {
                clear.update(cx, |input, cx| input.set_value("", window, cx));
            }
        })
        .trigger(
            Button::new("ribbon-more")
                .ghost()
                .xsmall()
                .label("More")
                .dropdown_caret(true)
                .selected(active_hidden)
                .w(px(MORE_BUTTON_WIDTH))
                .flex_none()
                .tooltip("All Shapes"),
        )
        .content(move |_, _, cx| {
            let popover = cx.entity().downgrade();
            let theme = cx.theme();
            let muted = theme.muted_foreground;
            let close_on_escape = {
                let (this, popover) = (this.clone(), popover.clone());
                move |_: &Escape, window: &mut Window, cx: &mut App| {
                    close(&this, &popover, window, cx)
                }
            };
            let grid: Vec<AnyElement> = cells
                .iter()
                .map(|cell| {
                    let on = selected == cell.tool;
                    gallery_cell(cell.clone(), on, on && locked, &this, &popover, cx)
                        .into_any_element()
                })
                .collect();
            v_flex()
                .w(px(GALLERY_WIDTH - 2. * GALLERY_PADDING))
                .gap(px(SECTION_GAP))
                .child(
                    div().on_action(close_on_escape).child(
                        Input::new(&search).small().prefix(
                            ComponentIcon::new(IconName::Search)
                                .small()
                                .text_color(muted),
                        ),
                    ),
                )
                .child(
                    div()
                        .id("gallery-grid")
                        .max_h(px(GALLERY_MAX_HEIGHT
                            - 2. * GALLERY_PADDING
                            - SEARCH_HEIGHT
                            - SECTION_GAP))
                        .overflow_y_scroll()
                        .when(grid.is_empty(), |grid| {
                            grid.py(px(SECTION_GAP))
                                .text_color(muted)
                                .child("No shapes match")
                        })
                        .child(div().flex().flex_wrap().children(grid)),
                )
        })
}

/// One gallery shape: its icon over its name.
fn gallery_cell(
    cell: ToolButton,
    selected: bool,
    locked: bool,
    editor: &WeakEntity<EditorWindow>,
    popover: &WeakEntity<PopoverState>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let color = if selected {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    let hover_bg = theme.list_hover;
    let tooltip = cell.tooltip;
    let tool = cell.tool;
    let (editor, popover) = (editor.clone(), popover.clone());
    v_flex()
        .id(cell.id)
        .relative()
        .flex_none()
        .size(px(CELL_SIZE))
        .p(px(4.))
        .gap(px(4.))
        .items_center()
        .justify_center()
        .rounded(theme.radius)
        .text_color(color)
        .when(selected, |c| c.bg(theme.list_active))
        .when(!selected, |c| c.hover(move |style| style.bg(hover_bg)))
        .child(button_icon(cell.icon, color))
        .child(
            div()
                .w_full()
                .text_center()
                .text_size(px(CELL_LABEL_SIZE))
                .truncate()
                .child(cell.name),
        )
        .when(locked, |c| c.child(lock_dot(theme.primary)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(move |event, window, cx| {
            let lock = event.click_count() >= 2;
            editor
                .update(cx, |editor, cx| {
                    editor.tools.select(tool.clone(), lock);
                    cx.notify();
                })
                .ok();
            if lock {
                close(&editor, &popover, window, cx);
                return;
            }
            // Wait for a possible second click before closing.
            let (editor, popover) = (editor.clone(), popover.clone());
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(CLOSE_DELAY).await;
                    cx.update(|window, cx| close(&editor, &popover, window, cx))
                        .ok();
                })
                .detach();
        })
}

/// Closes the gallery, if still open, and gives the editor back its focus
/// so tool keys work.
fn close(
    editor: &WeakEntity<EditorWindow>,
    popover: &WeakEntity<PopoverState>,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok(open) = popover.read_with(cx, |popover, _| popover.is_open()) else {
        return;
    };
    if !open {
        return;
    }
    popover
        .update(cx, |popover, cx| popover.dismiss(window, cx))
        .ok();
    editor
        .update(cx, |editor, cx| {
            editor.focus_handle.focus(window, cx);
            cx.notify();
        })
        .ok();
}

/// The shapes whose name contains `query`, ignoring case and surrounding
/// spaces, in library order. A blank query keeps them all.
fn filter_shapes<'a>(shapes: &'a [ShapeDef], query: &str) -> Vec<&'a ShapeDef> {
    let query = query.trim().to_lowercase();
    shapes
        .iter()
        .filter(|shape| shape.name.to_lowercase().contains(&query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::filter_shapes;
    use mindweave_core::geometry::Primitive;
    use mindweave_core::{Icon, ShapeDef, ShapeSource, StyleDefaults};

    fn shape(name: &str) -> ShapeDef {
        ShapeDef {
            id: name.to_lowercase(),
            name: name.into(),
            icon: Icon::Lucide("square".into()),
            default_size: (1.0, 1.0),
            style: StyleDefaults::default(),
            source: ShapeSource::Primitive(Primitive::Rect),
            params: Vec::new(),
        }
    }

    fn names(shapes: &[&ShapeDef]) -> Vec<String> {
        shapes.iter().map(|s| s.name.clone()).collect()
    }

    #[test]
    fn blank_query_keeps_every_shape() {
        let shapes = [shape("Rectangle"), shape("Ellipse"), shape("Star")];
        assert_eq!(filter_shapes(&shapes, "").len(), 3);
        assert_eq!(filter_shapes(&shapes, "   ").len(), 3);
    }

    #[test]
    fn query_matches_names_ignoring_case_and_spaces() {
        let shapes = [
            shape("Rectangle"),
            shape("Rounded Rectangle"),
            shape("Ellipse"),
            shape("Right Arrow"),
        ];
        assert_eq!(
            names(&filter_shapes(&shapes, "RECT")),
            ["Rectangle", "Rounded Rectangle"]
        );
        assert_eq!(
            names(&filter_shapes(&shapes, "  ed re ")),
            ["Rounded Rectangle"]
        );
        assert_eq!(names(&filter_shapes(&shapes, "ow")), ["Right Arrow"]);
        assert!(filter_shapes(&shapes, "hexagon").is_empty());
    }
}
