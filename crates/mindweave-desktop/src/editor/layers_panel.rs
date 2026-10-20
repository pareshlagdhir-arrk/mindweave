//! The Layers panel: the active page's layers, top layer first.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{row_click, section_label, EditorWindow, RenameTarget, RowClick, SMALL_TEXT_SIZE};

const HEADER_HEIGHT: f32 = 30.;
const ROW_HEIGHT: f32 = 26.;

pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let header = h_flex()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .pl(px(12.))
        .pr(px(6.))
        .justify_between()
        .child(section_label("Layers", cx))
        .child(
            Button::new("add-layer")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .tooltip("Add Layer")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.update_session(
                        |s| {
                            s.add_layer();
                        },
                        cx,
                    )
                })),
        );

    let page = editor.session().page();
    let rows: Vec<AnyElement> = page
        .layers
        .iter()
        .enumerate()
        .map(|(ix, layer)| {
            row(
                editor,
                ix,
                layer.name.clone(),
                layer.visible,
                ix == page.active_layer,
                cx,
            )
            .into_any_element()
        })
        .collect();

    v_flex()
        .size_full()
        .bg(cx.theme().sidebar)
        .text_size(px(SMALL_TEXT_SIZE))
        .child(header)
        .child(
            v_flex()
                .id("layers")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(6.))
                .gap(px(1.))
                .children(rows),
        )
}

fn row(
    editor: &EditorWindow,
    ix: usize,
    name: String,
    visible: bool,
    active: bool,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let label: AnyElement = match editor.rename_input(RenameTarget::Layer(ix)) {
        Some(input) => editor.render_rename_input(input, cx).into_any_element(),
        None => div()
            .flex_1()
            .min_w_0()
            .truncate()
            .child(name)
            .into_any_element(),
    };
    let theme = cx.theme();
    let hover = theme.list_hover;
    let (icon, tooltip) = if visible {
        (IconName::Eye, "Hide Layer")
    } else {
        (IconName::EyeOff, "Show Layer")
    };

    h_flex()
        .id(("layer", ix))
        .h(px(ROW_HEIGHT))
        .flex_none()
        .pl(px(8.))
        .pr(px(1.))
        .gap(px(4.))
        .rounded(theme.radius)
        .when(active, |row| row.bg(theme.list_active))
        .when(!active, |row| row.hover(move |style| style.bg(hover)))
        .when(!visible, |row| row.text_color(theme.muted_foreground))
        .child(label)
        .child(
            Button::new(("layer-visibility", ix))
                .ghost()
                .small()
                .icon(icon)
                .tooltip(tooltip)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.update_session(|s| s.toggle_layer_visibility(ix), cx);
                })),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            let renaming = this.rename_input(RenameTarget::Layer(ix)).is_some();
            match row_click(renaming, event.click_count()) {
                RowClick::Activate => this.update_session(|s| s.set_active_layer(ix), cx),
                RowClick::Rename => this.begin_rename(RenameTarget::Layer(ix), window, cx),
                RowClick::Ignore => {}
            }
        }))
}
