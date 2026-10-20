//! The Properties panel: the active page's canvas settings, and the
//! (always empty) selection.

use gpui_kit::component::input::Input;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::BackgroundPattern;

use super::{section_label, EditorWindow, SMALL_TEXT_SIZE};

const PATTERNS: [(BackgroundPattern, &str); 3] = [
    (BackgroundPattern::Lines, "Lines"),
    (BackgroundPattern::Dots, "Dots"),
    (BackgroundPattern::None, "None"),
];
const GRID_INPUT_WIDTH: f32 = 72.;
const FIELD_GAP: f32 = 6.;

pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let theme = cx.theme();
    let canvas = &editor.session().page().canvas;
    let pattern_ix = pattern_index(canvas.pattern);
    let show_grid_size = canvas.pattern != BackgroundPattern::None;

    let pattern = v_flex()
        .gap(px(FIELD_GAP))
        .child("Background pattern")
        .child(
            TabBar::new("background-pattern")
                .segmented()
                .xsmall()
                .w_full()
                .selected_index(pattern_ix)
                .children(
                    PATTERNS
                        .iter()
                        .map(|(_, label)| Tab::new().label(*label).flex_1()),
                )
                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                    if let Some((pattern, _)) = PATTERNS.get(*ix) {
                        this.update_session(|s| s.set_pattern(*pattern), cx);
                    }
                })),
        );

    let grid_size = h_flex()
        .justify_between()
        .gap(px(FIELD_GAP))
        .child("Grid size")
        .child(
            div().w(px(GRID_INPUT_WIDTH)).child(
                Input::new(&editor.grid_input)
                    .small()
                    .suffix(div().text_color(theme.muted_foreground).child("px")),
            ),
        );

    v_flex()
        .size_full()
        .bg(theme.sidebar)
        .px(px(12.))
        .pt(px(10.))
        .gap(px(10.))
        .text_size(px(SMALL_TEXT_SIZE))
        .child(section_label("Canvas", cx))
        .child(pattern)
        .when(show_grid_size, |panel| panel.child(grid_size))
        .child(section_label("Selection", cx).mt(px(12.)))
        .child(
            div()
                .text_color(theme.muted_foreground)
                .child("Nothing selected"),
        )
}

/// The segment shown selected for `pattern`.
fn pattern_index(pattern: BackgroundPattern) -> usize {
    PATTERNS
        .iter()
        .position(|(p, _)| *p == pattern)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{pattern_index, PATTERNS};
    use mindweave_core::BackgroundPattern;

    #[test]
    fn segments_are_lines_dots_none() {
        let labels: Vec<_> = PATTERNS.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, ["Lines", "Dots", "None"]);
        for (ix, (pattern, _)) in PATTERNS.iter().enumerate() {
            assert_eq!(pattern_index(*pattern), ix);
        }
        assert_eq!(pattern_index(BackgroundPattern::Dots), 1);
    }
}
