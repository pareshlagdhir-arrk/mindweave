//! The ribbon under the title bar: a row of tabs (Home, Mind Map, Artboard
//! and one per library), and under it a card with the tools of the shown
//! tab, in groups split by dividers. A library's shapes sit in a gallery
//! strip whose expander opens the shape gallery; shapes that do not fit
//! wait there, as artboard presets that do not fit wait in a menu.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Icon as ComponentIcon, IconName, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::geometry::{Geometry, PathCmd};
use mindweave_core::{ArtboardPreset, Icon, PresetCategory, ShapeDef, ARTBOARD_PRESETS};

use super::mindmap_edit::NewTopic;
use super::tool::{shape_key, Tool, ARTBOARD_TAB, BASIC_SHAPES, HOME_TAB, MINDMAP_TAB};
use super::{shape_gallery, EditorWindow, SMALL_TEXT_SIZE};
use crate::assets;
use crate::libraries::{lucide_icon_path, Libraries};

const TAB_ROW_HEIGHT: f32 = 28.;
const TAB_PADDING: f32 = 10.;
/// The active tab's underline.
const TAB_INDICATOR_HEIGHT: f32 = 3.;
const TAB_INDICATOR_INSET: f32 = 2.;
/// The card holding the tools, and its margin to the ribbon's edges.
const CARD_HEIGHT: f32 = 38.;
const CARD_MARGIN: f32 = 6.;
const ROW_PADDING: f32 = 8.;
const BUTTON_SIZE: f32 = 28.;
const ICON_SIZE: f32 = 16.;
const DIVIDER_HEIGHT: f32 = 20.;
const DIVIDER_MARGIN: f32 = 6.;
/// The gallery strip's border and inner padding, on each side.
const STRIP_BORDER: f32 = 1.;
const STRIP_PADDING: f32 = 1.;
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
/// Room a ribbon button takes in the tool row, with the gap after it.
const BUTTON_STEP: f32 = BUTTON_SIZE + TOOL_GAP;
/// Room a divider takes in the tool row, with its margins and the gap
/// after it.
const DIVIDER_STEP: f32 = 1. + 2. * DIVIDER_MARGIN + TOOL_GAP;
/// Width of the gallery strip's expander, which opens the shape gallery.
pub(super) const MORE_BUTTON_WIDTH: f32 = 20.;
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
    /// A Lucide icon over its accent layers, each in its own colour.
    Svg {
        path: SharedString,
        accents: Rc<[(SharedString, Hsla)]>,
    },
    /// Built at the origin of the 16px icon box.
    Outline(Option<Rc<Path<Pixels>>>),
}

impl ButtonIcon {
    /// Bundled Lucide icon `name` over its accent layers, if it has any.
    fn lucide(name: &str) -> Self {
        let accents = assets::accents(name)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(ix, accent)| (assets::accent_path(name, ix), rgb(accent.color).into()))
            .collect();
        ButtonIcon::Svg {
            path: format!("icons/{name}.svg").into(),
            accents,
        }
    }

    /// Lucide icon `name`, or the fallback icon when it is not bundled.
    fn lucide_or_fallback(name: &str, cx: &App) -> Self {
        let path = lucide_icon_path(name, cx);
        let bundled = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"))
            .unwrap_or(name);
        Self::lucide(bundled)
    }
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

/// A thin upright line between two groups of tools.
fn divider(cx: &App) -> AnyElement {
    div()
        .flex_none()
        .w(px(1.))
        .h(px(DIVIDER_HEIGHT))
        .mx(px(DIVIDER_MARGIN))
        .bg(cx.theme().border)
        .into_any_element()
}

/// `groups` side by side, split by dividers; empty groups are left out.
fn with_dividers(groups: Vec<Vec<AnyElement>>, cx: &App) -> Vec<AnyElement> {
    let mut row = Vec::new();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        if !row.is_empty() {
            row.push(divider(cx));
        }
        row.extend(group);
    }
    row
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
    let tabs = tab_order(
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
        .pl(px(ROW_PADDING - TAB_PADDING / 2.))
        .pr(px(4.))
        .gap(px(2.))
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

/// The tabs, as `(id, name)`: Home first, then Basic Shapes, Mind Map,
/// Artboard and the other libraries in `libraries`' order.
fn tab_order(
    libraries: impl IntoIterator<Item = (SharedString, SharedString)>,
) -> Vec<(SharedString, SharedString)> {
    let (basic, rest): (Vec<_>, Vec<_>) = libraries
        .into_iter()
        .partition(|(id, _)| id == BASIC_SHAPES);
    std::iter::once((HOME_TAB.into(), "Home".into()))
        .chain(basic)
        .chain([
            (MINDMAP_TAB.into(), "Mind Map".into()),
            (ARTBOARD_TAB.into(), "Artboard".into()),
        ])
        .chain(rest)
        .collect()
}

/// Row 2: a card with the tools of the shown tab. A library's shapes fill
/// a gallery strip, as many as fit, ending in the gallery's expander.
fn tool_row(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let libraries = cx.global::<Libraries>().0.clone();
    let library = libraries
        .libraries()
        .iter()
        .find(|l| l.id == editor.tools.tab);
    let content: Vec<AnyElement> = match library {
        Some(library) => {
            let buttons: Vec<ToolButton> = library
                .shapes
                .iter()
                .enumerate()
                .map(|(ix, shape)| shape_button(&library.id, ("tool-shape", ix), shape, cx))
                .collect();
            // The expander takes room only when some shapes do not fit.
            let shown = match editor.ribbon.tool_row_width {
                Some(width) => visible_count(
                    f32::from(width) - 2. * (STRIP_BORDER + STRIP_PADDING),
                    BUTTON_SIZE,
                    TOOL_GAP,
                    MORE_BUTTON_WIDTH + TOOL_GAP,
                    buttons.len(),
                ),
                // Not measured yet: show them all, cut off by the row.
                None => buttons.len(),
            };
            let active_hidden = active_tool_hidden(&buttons, shown, &editor.tools.tool);
            let more = (shown < buttons.len())
                .then(|| shape_gallery::more_button(editor, library, active_hidden, cx));
            let tiles: Vec<AnyElement> = buttons
                .into_iter()
                .take(shown)
                .map(|b| tool_button(editor, b, cx).into_any_element())
                .collect();
            vec![gallery_strip(tiles, more, cx).into_any_element()]
        }
        None if editor.tools.tab == MINDMAP_TAB => {
            type Run = fn(&mut EditorWindow, &mut Window, &mut Context<EditorWindow>);
            let topics: [(&'static str, &str, &'static str, Run); 3] = [
                (
                    "mm-central",
                    "network",
                    "New Mind Map",
                    |this, window, cx| this.add_central_topic(window, cx),
                ),
                (
                    "mm-sub",
                    "corner-down-right",
                    "Subtopic (Tab)",
                    |this, window, cx| this.add_topic(NewTopic::Sub, window, cx),
                ),
                (
                    "mm-sibling",
                    "list-plus",
                    "Topic (Enter; Shift+Enter before)",
                    |this, window, cx| this.add_topic(NewTopic::After, window, cx),
                ),
            ];
            let content: [(&'static str, &str, &'static str, Run); 2] = [
                ("mm-equation", "sigma", "Equation", |this, window, cx| {
                    this.edit_selected_equation(window, cx)
                }),
                (
                    "mm-note",
                    "sticky-note",
                    "Note (⌘⇧N)",
                    |this, window, cx| this.edit_selected_note(window, cx),
                ),
            ];
            let marks: [(&'static str, &str, &'static str, Run); 2] = [
                (
                    "mm-summary",
                    "braces",
                    "Summary of the Selected Topics",
                    |this, window, cx| this.add_summary(window, cx),
                ),
                (
                    "mm-boundary",
                    "square-dashed",
                    "Boundary round the Selected Topics",
                    |this, window, cx| this.toggle_boundary(window, cx),
                ),
            ];
            let mut actions = |commands: &[(&'static str, &str, &'static str, Run)], enabled| {
                commands
                    .iter()
                    .map(|&(id, icon, tip, run)| {
                        action_button(id, icon, tip, enabled, run, cx).into_any_element()
                    })
                    .collect::<Vec<_>>()
            };
            // Note and Equation work on exactly one topic.
            let one_topic = editor.selected_topic().is_some();
            let (topics, content, marks) = (
                actions(&topics, true),
                actions(&content, one_topic),
                actions(&marks, true),
            );
            let relationship = ToolButton {
                id: "tool-relationship".into(),
                tool: Tool::Relationship,
                icon: ButtonIcon::lucide("spline"),
                name: "Relationship".into(),
                tooltip: "Relationship: drag from one topic to another".into(),
            };
            let links = vec![tool_button(editor, relationship, cx).into_any_element()];
            with_dividers(vec![topics, content, marks, links], cx)
        }
        None if editor.tools.tab == ARTBOARD_TAB => artboard_row(editor, cx),
        None => {
            let tools = home_buttons()
                .into_iter()
                .map(|b| tool_button(editor, b, cx).into_any_element())
                .collect();
            let insert = vec![image_button(cx).into_any_element()];
            with_dividers(vec![tools, insert], cx)
        }
    };
    let this = cx.entity().downgrade();
    let theme = cx.theme();

    h_flex()
        .h(px(CARD_HEIGHT))
        .mx(px(CARD_MARGIN))
        .mb(px(CARD_MARGIN))
        .px(px(ROW_PADDING - CARD_MARGIN / 2.))
        .rounded(theme.radius_lg)
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    // Measures the row; a new width fits the shapes again.
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
                            // A notify while drawing is dropped; ask for
                            // another frame, which renders with the new width.
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
                        .children(content),
                ),
        )
}

/// A library's shapes in a bordered strip, ending in `more`, the expander
/// that opens the whole gallery, when some shapes do not fit.
fn gallery_strip(
    tiles: Vec<AnyElement>,
    more: Option<impl IntoElement>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .max_w_full()
        .h(px(BUTTON_SIZE + 2. * (STRIP_BORDER + STRIP_PADDING)))
        .p(px(STRIP_PADDING))
        .gap(px(TOOL_GAP))
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.title_bar)
        .child(
            h_flex()
                .min_w_0()
                .gap(px(TOOL_GAP))
                .overflow_hidden()
                .children(tiles),
        )
        .when_some(more, |strip, more| {
            strip.child(
                div()
                    .flex_none()
                    .h_full()
                    .flex()
                    .items_center()
                    .border_l_1()
                    .border_color(theme.border)
                    .child(more),
            )
        })
}

/// The Artboard tab: the Custom Artboard tool, then a button per preset,
/// a group per category. Presets the row has no room for wait in a menu
/// at its end.
fn artboard_row(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> Vec<AnyElement> {
    let custom = ToolButton {
        id: "tool-artboard".into(),
        tool: Tool::Artboard,
        icon: ButtonIcon::lucide("frame"),
        name: "Custom Artboard".into(),
        tooltip: "Custom Artboard (A): drag to draw one of any size".into(),
    };
    let categories: Vec<PresetCategory> = ARTBOARD_PRESETS.iter().map(|p| p.category).collect();
    let shown = match editor.ribbon.tool_row_width {
        Some(width) => presets_shown(f32::from(width), &categories),
        // Not measured yet: show them all, cut off by the row.
        None => ARTBOARD_PRESETS.len(),
    };
    let mut groups = vec![vec![tool_button(editor, custom, cx).into_any_element()]];
    for (ix, preset) in ARTBOARD_PRESETS.iter().enumerate().take(shown) {
        let button = preset_button(ix, preset, cx).into_any_element();
        match groups.last_mut() {
            Some(group) if ix > 0 && ARTBOARD_PRESETS[ix - 1].category == preset.category => {
                group.push(button)
            }
            _ => groups.push(vec![button]),
        }
    }
    let mut row = with_dividers(groups, cx);
    if shown < ARTBOARD_PRESETS.len() {
        row.push(more_presets(&ARTBOARD_PRESETS[shown..], cx).into_any_element());
    }
    row
}

/// A preset's tooltip and menu item: its name and size.
pub(super) fn preset_tooltip(preset: &ArtboardPreset) -> String {
    let (w, h) = preset.size;
    format!("{} — {w} × {h}", preset.name)
}

/// How many presets, of `categories` in order, fit in a tool row
/// `row_width` wide after the Custom Artboard button and its divider, a
/// divider between categories. When they do not all fit, room is kept at
/// the end for the menu holding the rest.
fn presets_shown(row_width: f32, categories: &[PresetCategory]) -> usize {
    let mut used = BUTTON_STEP + DIVIDER_STEP;
    // The room taken up to and including each preset, gap after it too.
    let mut ends = Vec::with_capacity(categories.len());
    for (ix, category) in categories.iter().enumerate() {
        if ix > 0 && categories[ix - 1] != *category {
            used += DIVIDER_STEP;
        }
        used += BUTTON_STEP;
        ends.push(used);
    }
    if ends.last().is_some_and(|&all| all - TOOL_GAP <= row_width) {
        return categories.len();
    }
    ends.iter()
        .take_while(|&&end| end + MORE_BUTTON_WIDTH <= row_width)
        .count()
}

/// The button placing an artboard of `preset`, the `ix`th.
fn preset_button(
    ix: usize,
    preset: &'static ArtboardPreset,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let tooltip: SharedString = preset_tooltip(preset).into();
    ribbon_button(
        ("artboard-preset", ix),
        ButtonIcon::lucide_or_fallback(preset.icon, cx),
        false,
        cx,
    )
    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
    .on_click(cx.listener(move |this, _, window, cx| {
        this.focus_handle.focus(window, cx);
        this.place_preset(preset, window, cx);
    }))
}

/// The ⌄ menu at the end of the Artboard tab holding the presets the row
/// has no room for, a separator between categories.
fn more_presets(
    hidden: &'static [ArtboardPreset],
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let this = cx.entity().downgrade();
    Button::new("artboard-more")
        .ghost()
        .xsmall()
        .icon(IconName::ChevronDown)
        .w(px(MORE_BUTTON_WIDTH))
        .h(px(BUTTON_SIZE))
        .flex_none()
        .tooltip("More Artboards")
        .dropdown_menu(move |mut menu, _, cx| {
            for (ix, preset) in hidden.iter().enumerate() {
                if ix > 0 && hidden[ix - 1].category != preset.category {
                    menu = menu.separator();
                }
                let this = this.clone();
                menu = menu.item(
                    PopupMenuItem::new(preset_tooltip(preset))
                        .icon(ComponentIcon::empty().path(lucide_icon_path(preset.icon, cx)))
                        .on_click(move |_, window, cx| {
                            this.update(cx, |this, cx| {
                                this.focus_handle.focus(window, cx);
                                this.place_preset(preset, window, cx);
                            })
                            .ok();
                        }),
                );
            }
            menu
        })
}

/// Home's tools: Pointer, Hand and Text.
fn home_buttons() -> Vec<ToolButton> {
    vec![
        ToolButton {
            id: "tool-pointer".into(),
            tool: Tool::Pointer,
            icon: ButtonIcon::lucide("mouse-pointer-2"),
            name: "Pointer".into(),
            tooltip: "Pointer (V)".into(),
        },
        ToolButton {
            id: "tool-hand".into(),
            tool: Tool::Hand,
            icon: ButtonIcon::lucide("move"),
            name: "Hand".into(),
            tooltip: "Hand (H)".into(),
        },
        ToolButton {
            id: "tool-text".into(),
            tool: Tool::Text,
            icon: ButtonIcon::lucide("type"),
            name: "Text".into(),
            tooltip: "Text (T)".into(),
        },
    ]
}

/// A 28px ribbon button with `icon`; the caller adds what a click does.
fn ribbon_button(
    id: impl Into<ElementId>,
    icon: ButtonIcon,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    ribbon_button_with(id, icon, selected, true, cx)
}

/// [`ribbon_button`]; not `enabled`, it is drawn muted, without a hover.
fn ribbon_button_with(
    id: impl Into<ElementId>,
    icon: ButtonIcon,
    selected: bool,
    enabled: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let hover_bg = theme.list_hover;
    let color = if enabled {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(BUTTON_SIZE))
        .rounded(theme.radius)
        .border_1()
        .when(selected, |b| {
            b.bg(theme.list_active)
                .border_color(theme.list_active_border)
        })
        .when(!selected, |b| b.border_color(transparent_black()))
        .when(!selected && enabled, |b| {
            b.hover(move |style| style.bg(hover_bg))
        })
        .child(button_icon(icon, color))
}

/// A ribbon button that runs `run` (an action, not a tool). Not
/// `enabled`, it looks disabled; a click still runs `run`, which says
/// what to select.
fn action_button(
    id: &'static str,
    icon: &str,
    tooltip: &'static str,
    enabled: bool,
    run: impl Fn(&mut EditorWindow, &mut Window, &mut Context<EditorWindow>) + 'static,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    ribbon_button_with(
        id,
        ButtonIcon::lucide_or_fallback(icon, cx),
        false,
        enabled,
        cx,
    )
    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
    .on_click(cx.listener(move |this, _, window, cx| {
        this.focus_handle.focus(window, cx);
        run(this, window, cx);
    }))
}

/// Home's Image button: imports picture files. An action, not a tool.
fn image_button(cx: &mut Context<EditorWindow>) -> impl IntoElement {
    ribbon_button(
        "import-image",
        ButtonIcon::lucide_or_fallback("image", cx),
        false,
        cx,
    )
    .tooltip(|window, cx| {
        Tooltip::new("Import Image (PNG, JPEG, GIF, WebP, SVG…)").build(window, cx)
    })
    .on_click(cx.listener(|this, _, window, cx| {
        this.import_images(window, cx);
        this.focus_handle.focus(window, cx);
    }))
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
            Icon::Lucide(name) => ButtonIcon::lucide_or_fallback(name, cx),
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

/// A tab: its name, and an underline in the accent colour while shown.
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
        .relative()
        .h_full()
        .flex()
        .items_center()
        .flex_none()
        .max_w(px(TAB_MAX_WIDTH))
        .px(px(TAB_PADDING))
        .when(active, |tab| {
            tab.text_color(foreground).child(
                div()
                    .absolute()
                    .bottom(px(TAB_INDICATOR_INSET))
                    .left(px(TAB_PADDING))
                    .right(px(TAB_PADDING))
                    .h(px(TAB_INDICATOR_HEIGHT))
                    .rounded_full()
                    .bg(theme.primary),
            )
        })
        .when(!active, |tab| {
            tab.text_color(theme.muted_foreground)
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
    let primary = cx.theme().primary;
    let tooltip = button.tooltip;
    let tool = button.tool;

    ribbon_button(button.id, button.icon, selected, cx)
        .when(locked, |b| b.child(lock_dot(primary)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            this.choose_tool(tool.clone(), event.click_count() >= 2, window, cx);
            // Keep tool keys working after a click on the toolbar.
            this.focus_handle.focus(window, cx);
            cx.notify();
        }))
}

/// A tool's 16px icon: its outline in `color` over its accent colours.
pub(super) fn button_icon(icon: ButtonIcon, color: Hsla) -> AnyElement {
    match icon {
        ButtonIcon::Svg { path, accents } => div()
            .relative()
            .size(px(ICON_SIZE))
            .flex_none()
            .children(accents.iter().map(|(layer, accent)| {
                svg()
                    .path(layer.clone())
                    .absolute()
                    .size(px(ICON_SIZE))
                    .text_color(*accent)
            }))
            .child(
                svg()
                    .path(path)
                    .absolute()
                    .size(px(ICON_SIZE))
                    .text_color(color),
            )
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
    use super::{
        active_tool_hidden, home_buttons, outline_icon_path, preset_tooltip, presets_shown,
        tab_order, visible_count, BUTTON_STEP, DIVIDER_STEP, ICON_SIZE, MORE_BUTTON_WIDTH,
        TOOL_GAP,
    };
    use crate::editor::tool::Tool;
    use gpui_kit::{px, SharedString};
    use mindweave_core::geometry::Geometry;
    use mindweave_core::{builtin_libraries, PresetCategory, ShapeSource, ARTBOARD_PRESETS};

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
    fn home_then_basic_shapes_lead_the_tabs() {
        let lib = |id: &str| (SharedString::from(id), SharedString::from(id));
        let ids = |tabs: Vec<(SharedString, SharedString)>| {
            tabs.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
        };
        assert_eq!(
            ids(tab_order([
                lib("flowchart"),
                lib("basic-shapes"),
                lib("aws")
            ])),
            [
                "home",
                "basic-shapes",
                "mindmap",
                "artboard",
                "flowchart",
                "aws"
            ]
        );
        assert_eq!(
            ids(tab_order([lib("aws")])),
            ["home", "mindmap", "artboard", "aws"]
        );
    }

    #[test]
    fn preset_tooltips_give_name_and_size() {
        assert_eq!(
            preset_tooltip(&ARTBOARD_PRESETS[0]),
            "iPhone 16 — 393 × 852"
        );
        let banner = ARTBOARD_PRESETS
            .iter()
            .find(|p| p.name == "Banner")
            .unwrap();
        assert_eq!(preset_tooltip(banner), "Banner — 1584 × 396");
    }

    #[test]
    fn presets_that_do_not_fit_wait_behind_the_menu() {
        use PresetCategory::*;
        let cats = [Device, Device, Desktop, Paper];
        // Custom Artboard and its divider, then two phones, a divider and
        // a desktop, a divider and a paper preset; no gap after the last.
        let all = BUTTON_STEP
            + DIVIDER_STEP
            + 2. * BUTTON_STEP
            + DIVIDER_STEP
            + BUTTON_STEP
            + DIVIDER_STEP
            + BUTTON_STEP
            - TOOL_GAP;
        assert_eq!(presets_shown(all, &cats), 4);
        assert_eq!(presets_shown(all + 100., &cats), 4);
        // Short of that, room is kept for the menu after the shown ones.
        let two = BUTTON_STEP + DIVIDER_STEP + 2. * BUTTON_STEP + MORE_BUTTON_WIDTH;
        assert_eq!(presets_shown(all - 1., &cats), 3);
        assert_eq!(presets_shown(two, &cats), 2);
        assert_eq!(presets_shown(two - 1., &cats), 1);
        // A preset after a divider needs the divider's room too.
        assert_eq!(
            presets_shown(two + BUTTON_STEP, &cats),
            2,
            "the divider before the desktop preset does not fit"
        );
        assert_eq!(presets_shown(two + BUTTON_STEP + DIVIDER_STEP, &cats), 3);
        assert_eq!(presets_shown(0., &cats), 0);
        assert_eq!(presets_shown(10., &[]), 0);
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
        // Home: Pointer, Hand, then Text.
        let buttons = home_buttons();
        assert!(active_tool_hidden(&buttons, 2, &Tool::Text));
        assert!(!active_tool_hidden(&buttons, 3, &Tool::Text));
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
