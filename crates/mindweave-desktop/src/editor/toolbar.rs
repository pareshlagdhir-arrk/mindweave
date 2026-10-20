//! The toolbar under the title bar: tabs (Home and one per library), a
//! divider, then the tools of the shown tab.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, ActiveTheme as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::geometry::{Geometry, PathCmd};
use mindweave_core::Icon;

use super::tool::{shape_key, Tool, HOME_TAB};
use super::{EditorWindow, SMALL_TEXT_SIZE};
use crate::libraries::{lucide_icon_path, Libraries};

const TOOLBAR_HEIGHT: f32 = 32.;
const BUTTON_SIZE: f32 = 28.;
const ICON_SIZE: f32 = 16.;
/// Stroke width of an outline icon.
const OUTLINE_STROKE: f32 = 1.25;
/// Space between an outline icon's unit box and the 16px icon box, like
/// the padding Lucide icons have.
const OUTLINE_INSET: f32 = 2.;
const LOCK_DOT_SIZE: f32 = 4.;
const LOCK_DOT_INSET: f32 = 3.;
const DIVIDER_HEIGHT: f32 = 16.;

/// One toolbar button: its tool, icon and tooltip.
struct ToolButton {
    id: ElementId,
    tool: Tool,
    icon: ButtonIcon,
    tooltip: SharedString,
}

enum ButtonIcon {
    Svg(SharedString),
    Outline(Geometry),
}

pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let libraries = &cx.global::<Libraries>().0;
    let mut tabs = vec![(SharedString::from(HOME_TAB), SharedString::from("Home"))];
    tabs.extend(
        libraries
            .libraries()
            .iter()
            .map(|l| (l.id.clone().into(), l.name.clone().into())),
    );
    let buttons = tab_buttons(&editor.tools.tab, cx);

    let tab_elements: Vec<AnyElement> = tabs
        .into_iter()
        .enumerate()
        .map(|(ix, (id, name))| {
            let active = *id == editor.tools.tab;
            tab(ix, id, name, active, cx).into_any_element()
        })
        .collect();
    let button_elements: Vec<AnyElement> = buttons
        .into_iter()
        .map(|b| tool_button(editor, b, cx).into_any_element())
        .collect();
    let theme = cx.theme();

    h_flex()
        .h(px(TOOLBAR_HEIGHT))
        .flex_none()
        .px(px(8.))
        .bg(theme.title_bar)
        .border_b_1()
        .border_color(theme.border)
        .text_size(px(SMALL_TEXT_SIZE))
        .child(h_flex().h_full().flex_none().children(tab_elements))
        .child(
            div()
                .flex_none()
                .w(px(1.))
                .h(px(DIVIDER_HEIGHT))
                .mx(px(8.))
                .bg(theme.border),
        )
        .child(
            h_flex()
                .id("toolbar-tools")
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .overflow_x_scroll()
                .children(button_elements),
        )
}

/// The buttons of tab `tab`: Pointer and Hand on Home, else the library's
/// shapes. An unknown tab shows Home's.
fn tab_buttons(tab: &str, cx: &App) -> Vec<ToolButton> {
    let library = cx
        .global::<Libraries>()
        .0
        .libraries()
        .iter()
        .find(|l| l.id == tab);
    let Some(library) = library else {
        return vec![
            ToolButton {
                id: "tool-pointer".into(),
                tool: Tool::Pointer,
                icon: ButtonIcon::Svg("icons/mouse-pointer-2.svg".into()),
                tooltip: "Pointer (V)".into(),
            },
            ToolButton {
                id: "tool-hand".into(),
                tool: Tool::Hand,
                icon: ButtonIcon::Svg("icons/hand.svg".into()),
                tooltip: "Hand (H)".into(),
            },
        ];
    };
    library
        .shapes
        .iter()
        .enumerate()
        .map(|(ix, shape)| ToolButton {
            id: ("tool-shape", ix).into(),
            tool: Tool::shape(&library.id, &shape.id),
            icon: match &shape.icon {
                Icon::Lucide(name) => ButtonIcon::Svg(lucide_icon_path(name, cx)),
                Icon::Outline(geometry) => ButtonIcon::Outline(geometry.clone()),
            },
            tooltip: match shape_key(&library.id, &shape.id) {
                Some(key) => format!("{} ({key})", shape.name).into(),
                None => shape.name.clone().into(),
            },
        })
        .collect()
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
    div()
        .id(("toolbar-tab", ix))
        .h_full()
        .flex()
        .items_center()
        .flex_none()
        .px(px(8.))
        .border_b_1()
        .when(active, |tab| {
            tab.text_color(foreground).border_color(theme.primary)
        })
        .when(!active, |tab| {
            tab.text_color(theme.muted_foreground)
                .border_color(transparent_black())
                .hover(move |style| style.text_color(foreground))
        })
        .child(name)
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.tools.tab != *id {
                this.tools.tab = id.to_string();
                cx.notify();
            }
        }))
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
    let icon: AnyElement = match button.icon {
        ButtonIcon::Svg(path) => svg()
            .path(path)
            .size(px(ICON_SIZE))
            .flex_none()
            .text_color(icon_color)
            .into_any_element(),
        ButtonIcon::Outline(geometry) => outline_icon(geometry, icon_color).into_any_element(),
    };
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
        .when(locked, |b| {
            b.child(
                div()
                    .absolute()
                    .top(px(LOCK_DOT_INSET))
                    .right(px(LOCK_DOT_INSET))
                    .size(px(LOCK_DOT_SIZE))
                    .rounded_full()
                    .bg(theme.primary),
            )
        })
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            this.tools.select(tool.clone(), event.click_count() >= 2);
            // Keep tool keys working after a click on the toolbar.
            this.focus_handle.focus(window, cx);
            cx.notify();
        }))
}

/// A library outline drawn into a 16px box with a thin stroke.
fn outline_icon(geometry: Geometry, color: Hsla) -> impl IntoElement {
    gpui_kit::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let span = ICON_SIZE - 2. * OUTLINE_INSET;
            let at = |(x, y): (f32, f32)| {
                point(
                    bounds.origin.x + px(OUTLINE_INSET + x * span),
                    bounds.origin.y + px(OUTLINE_INSET + y * span),
                )
            };
            let mut builder = PathBuilder::stroke(px(OUTLINE_STROKE));
            for cmd in geometry.paths.iter().flat_map(|p| &p.cmds) {
                match *cmd {
                    PathCmd::MoveTo(p) => builder.move_to(at(p)),
                    PathCmd::LineTo(p) => builder.line_to(at(p)),
                    PathCmd::QuadTo { ctrl, to } => builder.curve_to(at(to), at(ctrl)),
                    PathCmd::CubicTo { c1, c2, to } => {
                        builder.cubic_bezier_to(at(to), at(c1), at(c2))
                    }
                    PathCmd::Close => builder.close(),
                }
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(ICON_SIZE))
    .flex_none()
}
