//! The ribbon under the title bar: a row of tabs (Home and one per
//! library), and under it the tools of the shown tab. Tools that do not fit
//! give way to a More button that opens the shape gallery.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::geometry::{Geometry, PathCmd};
use mindweave_core::{Icon, ShapeDef};

use super::tool::{shape_key, Tool, HOME_TAB};
use super::{shape_gallery, EditorWindow, SMALL_TEXT_SIZE};
use crate::libraries::{lucide_icon_path, Libraries};

const TAB_ROW_HEIGHT: f32 = 26.;
const TOOL_ROW_HEIGHT: f32 = 34.;
const ROW_PADDING: f32 = 8.;
const BUTTON_SIZE: f32 = 28.;
const ICON_SIZE: f32 = 16.;
/// Stroke width of an outline icon.
const OUTLINE_STROKE: f32 = 1.25;
/// Space between an outline icon's unit box and the 16px icon box, like
/// the padding Lucide icons have.
const OUTLINE_INSET: f32 = 2.;
const LOCK_DOT_SIZE: f32 = 4.;
const LOCK_DOT_INSET: f32 = 3.;
/// Widest tab; longer library names are cut with an ellipsis.
const TAB_MAX_WIDTH: f32 = 160.;
const TOOL_GAP: f32 = 2.;
/// Width of the More button, which the row keeps free when tools overflow.
pub(super) const MORE_BUTTON_WIDTH: f32 = 60.;
const TAB_MENU_MAX_WIDTH: f32 = 280.;
const TAB_MENU_MAX_HEIGHT: f32 = 400.;

/// One tool button: its tool, icon, name and tooltip.
#[derive(Clone)]
pub(super) struct ToolButton {
    pub id: ElementId,
    pub tool: Tool,
    pub icon: ButtonIcon,
    pub name: SharedString,
    pub tooltip: SharedString,
}

#[derive(Clone)]
pub(super) enum ButtonIcon {
    Svg(SharedString),
    /// Built at the origin of the 16px icon box.
    Outline(Option<Rc<Path<Pixels>>>),
}

/// Outline icons built once, by library and shape id; the libraries do not
/// change while the app runs.
#[derive(Default)]
struct OutlineIcons(HashMap<(String, String), Option<Rc<Path<Pixels>>>>);

impl Global for OutlineIcons {}

/// Per-window ribbon state.
pub(super) struct Ribbon {
    tabs_scroll: ScrollHandle,
    /// The tab last scrolled into view.
    scrolled_to: String,
    /// The width of the tool row when it was last painted.
    tool_row_width: Option<Pixels>,
    /// The shape gallery's search field.
    pub search: Entity<InputState>,
    /// Counts the gallery's openings, so a close scheduled by a click only
    /// closes the opening it was made in.
    pub gallery_opening: Rc<Cell<u64>>,
    /// When the gallery last closed, to keep the second click of a double
    /// click on a gallery shape from placing a shape.
    pub gallery_closed_at: Option<Instant>,
}

impl Ribbon {
    pub(super) fn new(window: &mut Window, cx: &mut Context<EditorWindow>) -> (Self, Subscription) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search shapes"));
        // The gallery filters as the query changes.
        let subscription = cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let ribbon = Ribbon {
            tabs_scroll: ScrollHandle::new(),
            scrolled_to: HOME_TAB.to_string(),
            tool_row_width: None,
            search,
            gallery_opening: Rc::default(),
            gallery_closed_at: None,
        };
        (ribbon, subscription)
    }
}

pub(super) fn render(
    editor: &mut EditorWindow,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let theme = cx.theme();
    let (bg, border) = (theme.title_bar, theme.border);
    v_flex()
        .flex_none()
        .bg(bg)
        .border_b_1()
        .border_color(border)
        .text_size(px(SMALL_TEXT_SIZE))
        .child(tab_row(editor, cx))
        .child(tool_row(editor, cx))
}

impl EditorWindow {
    /// Shows tab `id` without changing the tool.
    fn show_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.tools.tab != id {
            self.tools.tab = id.to_string();
            cx.notify();
        }
    }
}

/// Row 1: the tabs, scrolling sideways when they do not fit, and a menu of
/// every tab at the right end.
fn tab_row(editor: &mut EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let libraries = &cx.global::<Libraries>().0;
    let mut tabs = vec![(SharedString::from(HOME_TAB), SharedString::from("Home"))];
    tabs.extend(
        libraries
            .libraries()
            .iter()
            .map(|l| (l.id.clone().into(), l.name.clone().into())),
    );
    let active = editor.tools.tab.clone();
    // Bring a tab chosen from the menu or by a tool key into view.
    if editor.ribbon.scrolled_to != active {
        if let Some(ix) = tabs.iter().position(|(id, _)| **id == active) {
            editor.ribbon.tabs_scroll.scroll_to_item(ix);
        }
        editor.ribbon.scrolled_to = active.clone();
    }

    let tab_elements: Vec<AnyElement> = tabs
        .iter()
        .enumerate()
        .map(|(ix, (id, name))| {
            tab(ix, id.clone(), name.clone(), **id == active, cx).into_any_element()
        })
        .collect();
    let tabs = Rc::new(tabs);
    let this = cx.entity().downgrade();

    h_flex()
        .h(px(TAB_ROW_HEIGHT))
        .pl(px(ROW_PADDING))
        .pr(px(4.))
        .gap(px(4.))
        .child(
            // A vertical wheel scrolls it sideways too.
            h_flex()
                .id("ribbon-tabs")
                .h_full()
                .flex_1()
                .min_w_0()
                .overflow_x_scroll()
                .track_scroll(&editor.ribbon.tabs_scroll)
                .children(tab_elements),
        )
        .child(
            Button::new("ribbon-tab-menu")
                .ghost()
                .xsmall()
                .icon(IconName::ChevronDown)
                .tooltip("All Libraries")
                .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
                    for (id, name) in tabs.iter() {
                        let this = this.clone();
                        let id = id.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(*id == active)
                                .on_click(move |_, _, cx| {
                                    this.update(cx, |this, cx| this.show_tab(&id, cx)).ok();
                                }),
                        );
                    }
                    menu.max_w(px(TAB_MENU_MAX_WIDTH))
                        .max_h(px(TAB_MENU_MAX_HEIGHT))
                        .scrollable(true)
                }),
        )
}

/// Row 2: the tools of the shown tab, as many as fit, then More.
fn tool_row(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let libraries = cx.global::<Libraries>().0.clone();
    let library = libraries
        .libraries()
        .iter()
        .find(|l| l.id == editor.tools.tab);
    let buttons = match library {
        Some(library) => library
            .shapes
            .iter()
            .enumerate()
            .map(|(ix, shape)| shape_button(&library.id, ("tool-shape", ix), shape, cx))
            .collect(),
        None => home_buttons(),
    };
    let shown = match editor.ribbon.tool_row_width {
        Some(width) => visible_count(
            width.into(),
            BUTTON_SIZE,
            TOOL_GAP,
            MORE_BUTTON_WIDTH,
            buttons.len(),
        ),
        // Not measured yet: show them all, cut off by the row.
        None => buttons.len(),
    };
    let active_hidden = active_tool_hidden(&buttons, shown, &editor.tools.tool);
    let more = library
        .filter(|_| shown < buttons.len())
        .map(|library| shape_gallery::more_button(editor, library, active_hidden, cx));
    let button_elements: Vec<AnyElement> = buttons
        .into_iter()
        .take(shown)
        .map(|b| tool_button(editor, b, cx).into_any_element())
        .collect();
    let this = cx.entity().downgrade();

    h_flex().h(px(TOOL_ROW_HEIGHT)).px(px(ROW_PADDING)).child(
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                // Measures the row; a new width fits the buttons again.
                canvas(
                    move |bounds, window, cx| {
                        let width = bounds.size.width;
                        let changed = this
                            .update(cx, |this, _| {
                                let changed = this.ribbon.tool_row_width != Some(width);
                                this.ribbon.tool_row_width = Some(width);
                                changed
                            })
                            .unwrap_or(false);
                        // A notify while drawing is dropped; ask for another
                        // frame, which renders with the new width.
                        if changed {
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                h_flex()
                    .size_full()
                    .gap(px(TOOL_GAP))
                    .overflow_hidden()
                    .children(button_elements)
                    .children(more),
            ),
    )
}

/// Home's buttons: Pointer and Hand.
fn home_buttons() -> Vec<ToolButton> {
    vec![
        ToolButton {
            id: "tool-pointer".into(),
            tool: Tool::Pointer,
            icon: ButtonIcon::Svg("icons/mouse-pointer-2.svg".into()),
            name: "Pointer".into(),
            tooltip: "Pointer (V)".into(),
        },
        ToolButton {
            id: "tool-hand".into(),
            tool: Tool::Hand,
            icon: ButtonIcon::Svg("icons/hand.svg".into()),
            name: "Hand".into(),
            tooltip: "Hand (H)".into(),
        },
    ]
}

/// The button placing `shape` of library `library`.
pub(super) fn shape_button(
    library: &str,
    id: impl Into<ElementId>,
    shape: &ShapeDef,
    cx: &mut App,
) -> ToolButton {
    ToolButton {
        id: id.into(),
        tool: Tool::shape(library, &shape.id),
        icon: match &shape.icon {
            Icon::Lucide(name) => ButtonIcon::Svg(lucide_icon_path(name, cx)),
            Icon::Outline(geometry) => ButtonIcon::Outline(
                cx.default_global::<OutlineIcons>()
                    .0
                    .entry((library.to_string(), shape.id.clone()))
                    .or_insert_with(|| outline_icon_path(geometry).map(Rc::new))
                    .clone(),
            ),
        },
        name: shape.name.clone().into(),
        tooltip: match shape_key(library, &shape.id) {
            Some(key) => format!("{} ({key})", shape.name).into(),
            None => shape.name.clone().into(),
        },
    }
}

fn tab(
    ix: usize,
    id: SharedString,
    name: SharedString,
    active: bool,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let theme = cx.theme();
    let foreground = theme.foreground;
    let tooltip = name.clone();
    div()
        .id(("ribbon-tab", ix))
        .h_full()
        .flex()
        .items_center()
        .flex_none()
        .max_w(px(TAB_MAX_WIDTH))
        .px(px(8.))
        .border_b_2()
        .when(active, |tab| {
            tab.text_color(foreground).border_color(theme.primary)
        })
        .when(!active, |tab| {
            tab.text_color(theme.muted_foreground)
                .border_color(transparent_black())
                .hover(move |style| style.text_color(foreground))
        })
        .child(div().min_w_0().truncate().child(name))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, _, cx| this.show_tab(&id, cx)))
}

/// A 28px tool button. A click selects the tool; a double click also
/// locks it, shown by a dot in the corner.
fn tool_button(
    editor: &EditorWindow,
    button: ToolButton,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let selected = editor.tools.tool == button.tool;
    let locked = selected && editor.tools.locked;
    let theme = cx.theme();
    let icon_color = if selected {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    let hover_bg = theme.list_hover;
    let icon = button_icon(button.icon, icon_color);
    let tooltip = button.tooltip;
    let tool = button.tool;

    div()
        .id(button.id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(BUTTON_SIZE))
        .rounded(theme.radius)
        .when(selected, |b| b.bg(theme.list_active))
        .when(!selected, |b| b.hover(move |style| style.bg(hover_bg)))
        .child(icon)
        .when(locked, |b| b.child(lock_dot(theme.primary)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            this.tools.select(tool.clone(), event.click_count() >= 2);
            // Keep tool keys working after a click on the toolbar.
            this.focus_handle.focus(window, cx);
            cx.notify();
        }))
}

/// A tool's 16px icon in `color`.
pub(super) fn button_icon(icon: ButtonIcon, color: Hsla) -> AnyElement {
    match icon {
        ButtonIcon::Svg(path) => svg()
            .path(path)
            .size(px(ICON_SIZE))
            .flex_none()
            .text_color(color)
            .into_any_element(),
        ButtonIcon::Outline(path) => outline_icon(path, color).into_any_element(),
    }
}

/// The dot in the top right corner of a locked tool's button.
pub(super) fn lock_dot(color: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .top(px(LOCK_DOT_INSET))
        .right(px(LOCK_DOT_INSET))
        .size(px(LOCK_DOT_SIZE))
        .rounded_full()
        .bg(color)
}

/// A library outline as a thin stroke in a 16px box at the origin; `None`
/// when it has nothing to draw.
fn outline_icon_path(geometry: &Geometry) -> Option<Path<Pixels>> {
    let span = ICON_SIZE - 2. * OUTLINE_INSET;
    let at = |(x, y): (f32, f32)| point(px(OUTLINE_INSET + x * span), px(OUTLINE_INSET + y * span));
    let mut builder = PathBuilder::stroke(px(OUTLINE_STROKE));
    for cmd in geometry.paths.iter().flat_map(|p| &p.cmds) {
        match *cmd {
            PathCmd::MoveTo(p) => builder.move_to(at(p)),
            PathCmd::LineTo(p) => builder.line_to(at(p)),
            PathCmd::QuadTo { ctrl, to } => builder.curve_to(at(to), at(ctrl)),
            PathCmd::CubicTo { c1, c2, to } => builder.cubic_bezier_to(at(to), at(c1), at(c2)),
            PathCmd::Close => builder.close(),
        }
    }
    builder
        .build()
        .ok()
        .filter(|path| !path.vertices.is_empty())
}

/// A prebuilt outline icon painted in a 16px box.
fn outline_icon(path: Option<Rc<Path<Pixels>>>, color: Hsla) -> impl IntoElement {
    gpui_kit::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let Some(path) = &path else {
                return;
            };
            let mut path = Path::clone(path);
            for v in &mut path.vertices {
                v.xy_position += bounds.origin;
            }
            path.bounds.origin += bounds.origin;
            window.paint_path(path, color);
        },
    )
    .size(px(ICON_SIZE))
    .flex_none()
}

/// How many of `total` buttons `button` wide, `gap` apart, fit in a row
/// `row_width` wide. When they do not all fit, room is kept at the end for
/// the More button, `more_button` wide after a gap.
fn visible_count(row_width: f32, button: f32, gap: f32, more_button: f32, total: usize) -> usize {
    let all = total as f32 * (button + gap) - gap;
    if total == 0 || all <= row_width {
        return total;
    }
    // Each shown button takes its width and the gap after it.
    let room = row_width - more_button;
    if room <= 0. {
        return 0;
    }
    ((room / (button + gap)).floor() as usize).min(total)
}

/// Whether `active` is the tool of a button after the first `shown`, which
/// the row has no room for; More then stands in for it.
fn active_tool_hidden(buttons: &[ToolButton], shown: usize, active: &Tool) -> bool {
    buttons.iter().skip(shown).any(|b| b.tool == *active)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{active_tool_hidden, home_buttons, outline_icon_path, visible_count, ICON_SIZE};
    use crate::editor::tool::Tool;
    use gpui_kit::px;
    use mindweave_core::geometry::Geometry;
    use mindweave_core::{builtin_libraries, ShapeSource};

    #[test]
    fn outline_icons_are_built_inside_the_icon_box() {
        let libs = builtin_libraries();
        let ShapeSource::Outline(diamond) = &libs[0].shape("diamond").unwrap().source else {
            panic!("the diamond is an SVG outline");
        };
        let path = outline_icon_path(diamond).unwrap();
        let b = path.bounds;
        assert!(b.origin.x >= px(0.) && b.origin.y >= px(0.));
        assert!(b.origin.x + b.size.width <= px(ICON_SIZE));
        assert!(b.origin.y + b.size.height <= px(ICON_SIZE));
        assert!(outline_icon_path(&Geometry { paths: vec![] }).is_none());
    }

    #[test]
    fn all_buttons_show_when_they_fit() {
        // 4 buttons of 28 with 3 gaps of 2: exactly 118.
        assert_eq!(visible_count(118., 28., 2., 60., 4), 4);
        assert_eq!(visible_count(500., 28., 2., 60., 4), 4);
        assert_eq!(visible_count(0., 28., 2., 60., 0), 0);
    }

    #[test]
    fn overflowing_buttons_leave_room_for_more() {
        // One short of fitting all 4: each shown button takes 30, then More.
        assert_eq!(visible_count(117., 28., 2., 60., 4), 1);
        assert_eq!(visible_count(120., 28., 2., 60., 10), 2);
        assert_eq!(visible_count(119.9, 28., 2., 60., 10), 1);
        // Only More fits, or not even that.
        assert_eq!(visible_count(60., 28., 2., 60., 10), 0);
        assert_eq!(visible_count(10., 28., 2., 60., 10), 0);
        assert_eq!(visible_count(-5., 28., 2., 60., 10), 0);
    }

    #[test]
    fn more_stands_in_for_a_hidden_active_tool() {
        // Home: Pointer, then Hand.
        let buttons = home_buttons();
        assert!(!active_tool_hidden(&buttons, 2, &Tool::Hand));
        assert!(active_tool_hidden(&buttons, 1, &Tool::Hand));
        assert!(!active_tool_hidden(&buttons, 1, &Tool::Pointer));
        assert!(active_tool_hidden(&buttons, 0, &Tool::Pointer));
        // A tool of another tab is not hidden here.
        assert!(!active_tool_hidden(
            &buttons,
            0,
            &Tool::shape("basic-shapes", "star")
        ));
        // A count past the end is treated as all shown.
        assert!(!active_tool_hidden(&buttons, 5, &Tool::Hand));
    }
}
