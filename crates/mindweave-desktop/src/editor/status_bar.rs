//! The editor status bar. Left: page and layer. Right: pointer position,
//! zoom (resets it) and theme name (opens the theme picker).

use gpui_kit::component::{h_flex, ActiveTheme as _};
use gpui_kit::*;

use super::{EditorWindow, SMALL_TEXT_SIZE};
use crate::actions::{ToggleThemePicker, ZoomReset};

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

    h_flex()
        .h(px(STATUS_BAR_HEIGHT))
        .flex_none()
        .px(px(12.))
        .gap(px(ITEM_GAP))
        .bg(theme.status_bar)
        .border_t_1()
        .border_color(theme.status_bar_border)
        .text_size(px(SMALL_TEXT_SIZE))
        .text_color(theme.muted_foreground)
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
    use super::{cursor_label, page_label, zoom_label};

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
