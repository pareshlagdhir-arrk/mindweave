//! The page tabs under the canvas, with a trailing button to add a page.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{EditorWindow, RenameTarget, SMALL_TEXT_SIZE};

const TABS_HEIGHT: f32 = 28.;
const TAB_HEIGHT: f32 = 24.;
const RENAME_WIDTH: f32 = 140.;

pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let doc = &editor.session().doc;
    let tabs: Vec<AnyElement> = doc
        .pages
        .iter()
        .enumerate()
        .map(|(ix, page)| {
            tab(editor, ix, page.name.clone(), ix == doc.active_page, cx).into_any_element()
        })
        .collect();
    let theme = cx.theme();

    h_flex()
        .id("page-tabs")
        .h(px(TABS_HEIGHT))
        .flex_none()
        .items_end()
        .px(px(8.))
        .gap(px(2.))
        .overflow_x_scroll()
        .bg(theme.tab_bar)
        .border_t_1()
        .border_color(theme.border)
        .text_size(px(SMALL_TEXT_SIZE))
        .children(tabs)
        .child(
            h_flex().h(px(TABS_HEIGHT)).flex_none().child(
                Button::new("add-page")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("Add Page")
                    .on_click(cx.listener(|this, _, _, cx| this.add_page(cx))),
            ),
        )
}

fn tab(
    editor: &EditorWindow,
    ix: usize,
    name: String,
    active: bool,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let content: AnyElement = match editor.rename_input(RenameTarget::Page(ix)) {
        Some(input) => div()
            .w(px(RENAME_WIDTH))
            .flex()
            .child(editor.render_rename_input(input, cx))
            .into_any_element(),
        None => div().child(name).into_any_element(),
    };
    let theme = cx.theme();
    let foreground = theme.foreground;

    h_flex()
        .id(("page-tab", ix))
        .h(px(TAB_HEIGHT))
        .flex_none()
        .px(px(12.))
        .rounded_t(theme.radius)
        .when(active, |tab| {
            tab.bg(theme.background).text_color(foreground)
        })
        .when(!active, |tab| {
            tab.text_color(theme.muted_foreground)
                .hover(move |style| style.text_color(foreground))
        })
        .child(content)
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                this.begin_rename(RenameTarget::Page(ix), window, cx);
            } else {
                this.activate_page(ix, cx);
            }
        }))
}
