//! The editor status bar. Left: the left sidebar button, page and
//! layer. Right: pointer position, zoom (resets it), theme name (opens the
//! theme picker) and the Properties sidebar button.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, ActiveTheme as _, Icon, Selectable as _, Sizable as _};
use gpui_kit::*;

use super::{EditorWindow, SMALL_TEXT_SIZE};
use crate::actions::{ToggleLeftSidebar, ToggleProperties, ToggleThemePicker, ZoomReset};
use crate::app_state::AppState;
use crate::libraries::lucide_icon_path;

const STATUS_BAR_HEIGHT: f32 = 24.;
const ITEM_GAP: f32 = 16.;

pub(super) fn render(editor: &EditorWindow, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let foreground = theme.foreground;
    let doc = &editor.session().doc();
    let page = editor.session().page();
    let layer_name = page
        .layers
        .get(page.active_layer)
        .map(|l| l.name.as_str())
        .unwrap_or_default();

    let settings = &cx.global::<AppState>().settings;
    let layers = sidebar_button(
        "toggle-left-sidebar",
        "panel-left",
        settings.left_sidebar_shown(),
        "Sidebar",
        "⌘B",
        || Box::new(ToggleLeftSidebar),
        cx,
    );
    let properties = sidebar_button(
        "toggle-properties",
        "panel-right",
        settings.show_properties,
        "Properties",
        "⌥⌘B",
        || Box::new(ToggleProperties),
        cx,
    );

    h_flex()
        .h(px(STATUS_BAR_HEIGHT))
        .flex_none()
        .px(px(6.))
        .gap(px(ITEM_GAP))
        .bg(theme.status_bar)
        .border_t_1()
        .border_color(theme.status_bar_border)
        .text_size(px(SMALL_TEXT_SIZE))
        .text_color(theme.muted_foreground)
        .child(layers)
        .child(page_label(doc.active_page, doc.pages.len()))
        .child(format!("Layer: {layer_name}"))
        .child(div().flex_1())
        .children(cursor_label(editor.cursor_canvas))
        .child(
            div()
                .id("zoom")
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .child(zoom_label(page.viewport.zoom_percent()))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ZoomReset), cx)),
        )
        .child(
            div()
                .id("theme-name")
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .child(theme.theme_name().clone())
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleThemePicker), cx)),
        )
        .child(properties)
}

/// A button showing or hiding sidebar `name`, highlighted while shown.
fn sidebar_button(
    id: &'static str,
    icon: &str,
    shown: bool,
    name: &str,
    keys: &str,
    action: fn() -> Box<dyn Action>,
    cx: &App,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(Icon::empty().path(lucide_icon_path(icon, cx)))
        .selected(shown)
        .tooltip(sidebar_tooltip(name, shown, keys))
        .on_click(move |_, window, cx| window.dispatch_action(action(), cx))
}

/// "Hide Layers (⌘B)" or "Show Layers (⌘B)".
fn sidebar_tooltip(name: &str, shown: bool, keys: &str) -> String {
    let verb = if shown { "Hide" } else { "Show" };
    format!("{verb} {name} ({keys})")
}

/// "Page {i} of {n}" for the zero-based `active` page.
fn page_label(active: usize, count: usize) -> String {
    format!("Page {} of {count}", active + 1)
}

/// "x {X} · y {Y}" in whole canvas units, or nothing off the canvas.
fn cursor_label(cursor: Option<(f32, f32)>) -> Option<String> {
    cursor.map(|(x, y)| format!("x {} · y {}", x.round() as i64, y.round() as i64))
}

fn zoom_label(percent: u32) -> String {
    format!("{percent}%")
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{cursor_label, page_label, sidebar_tooltip, zoom_label};

    #[test]
    fn sidebar_tooltips_say_what_a_click_does() {
        assert_eq!(sidebar_tooltip("Layers", true, "⌘B"), "Hide Layers (⌘B)");
        assert_eq!(
            sidebar_tooltip("Properties", false, "⌥⌘B"),
            "Show Properties (⌥⌘B)"
        );
    }

    #[test]
    fn page_label_is_one_based() {
        assert_eq!(page_label(0, 3), "Page 1 of 3");
        assert_eq!(page_label(2, 3), "Page 3 of 3");
    }

    #[test]
    fn cursor_label_rounds_and_hides_off_canvas() {
        assert_eq!(cursor_label(None), None);
        assert_eq!(
            cursor_label(Some((1240.4, -320.6))).as_deref(),
            Some("x 1240 · y -321")
        );
        assert_eq!(
            cursor_label(Some((-0.2, 0.0))).as_deref(),
            Some("x 0 · y 0")
        );
    }

    #[test]
    fn zoom_label_is_a_percentage() {
        assert_eq!(zoom_label(100), "100%");
        assert_eq!(zoom_label(25), "25%");
    }
}
