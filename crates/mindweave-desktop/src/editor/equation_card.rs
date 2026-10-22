//! The equation card: a LaTeX field beside a topic with a live preview,
//! insert snippets, and Done and Cancel. Only the preview follows typing;
//! the topic changes once, when Done applies (one undo step).

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, InputEvent, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use uuid::Uuid;

use super::cards::{card_origin, card_topic_present, topic_on_screen, MARGIN};
use super::{section_label, EditorWindow};
use crate::actions::{ApplyEquation, EQUATION_CARD_CONTEXT};
use crate::equation::{rasterise, render_equation, RenderedEquation};

/// The card's width, in pixels.
const CARD_WIDTH: f32 = 340.;
/// The card's height until it is first laid out (it fits its content).
const CARD_HEIGHT_GUESS: f32 = 300.;
/// Space between the card and its topic.
const CARD_GAP: f32 = 12.;
const CARD_PADDING: f32 = 12.;
const CARD_SPACING: f32 = 8.;
/// Height of the LaTeX field (about four lines).
const INPUT_HEIGHT: f32 = 84.;
const INPUT_TEXT_SIZE: f32 = 13.;
/// The preview area's height, and the em of the preview.
const PREVIEW_HEIGHT: f32 = 84.;
const PREVIEW_FONT_SIZE: f32 = 20.;
const SNIPPET_GAP: f32 = 4.;

/// The equation card open on a topic.
pub(super) struct EquationCard {
    /// The topic whose equation it edits.
    pub topic: Uuid,
    /// The LaTeX being typed.
    pub input: Entity<TextareaState>,
    /// The typed LaTeX rendered.
    preview: Preview,
    /// The card's height when it was last laid out, in pixels.
    height: Option<f32>,
    _subscription: Subscription,
}

/// The card's preview of the typed LaTeX.
struct Preview {
    /// Whether the input is blank (nothing to preview).
    blank: bool,
    /// The colour it is drawn in: the theme's text colour.
    color: Hsla,
    render: Result<RenderedEquation, String>,
    /// The render rasterised for the window's scale, and the size it is
    /// shown at, in pixels.
    image: Option<(Arc<RenderImage>, (f32, f32))>,
}

impl EditorWindow {
    /// Opens the equation card on the one selected topic; with any other
    /// selection, says what to select. The ribbon's Equation button.
    pub(super) fn edit_selected_equation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_topic() {
            Some(topic) => self.open_equation_card(topic, window, cx),
            None => window.push_notification(
                Notification::warning("Select one topic to add an equation."),
                cx,
            ),
        }
    }

    /// Opens the equation card on `topic`, its field holding the topic's
    /// LaTeX, in place of any card open.
    pub(super) fn open_equation_card(
        &mut self,
        topic: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Opened again on the same topic, the card keeps what was typed.
        if let Some(card) = self
            .equation_card
            .as_ref()
            .filter(|card| card_is_open_on(Some(card.topic), topic))
        {
            let input = card.input.clone();
            input.update(cx, |state, cx| state.focus(window, cx));
            return;
        }
        self.begin_edit_action(window, cx);
        self.close_equation_card(window);
        let Some(shape) = self.session.find_shape(topic) else {
            return;
        };
        let latex = shape
            .topic
            .as_ref()
            .and_then(|t| t.equation.as_ref())
            .map(|eq| eq.latex.clone())
            .unwrap_or_default();
        let end = latex.len();
        let input = cx.new(|cx| TextareaState::new(window, cx).default_value(latex));
        input.update(cx, |state, cx| {
            state.set_selected_range(end..end, cx);
            state.focus(window, cx);
        });
        let _subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh_equation_preview(window, cx);
            }
        });
        self.equation_card = Some(EquationCard {
            topic,
            input,
            preview: Preview {
                blank: true,
                color: cx.theme().foreground,
                render: Err(String::new()),
                image: None,
            },
            height: None,
            _subscription,
        });
        self.refresh_equation_preview(window, cx);
    }

    /// Done: sets the topic's equation to the typed LaTeX (removes it when
    /// the field is blank) as one undo step, and closes the card. Does
    /// nothing while the LaTeX does not render.
    pub(super) fn apply_equation_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.equation_card.as_ref() else {
            return;
        };
        let typed = card.input.read(cx).value();
        let Some(latex) = equation_to_apply(&typed, &card.preview.render) else {
            return;
        };
        let topic = card.topic;
        let current = self
            .session
            .find_shape(topic)
            .and_then(|s| s.topic.as_ref())
            .and_then(|t| t.equation.as_ref())
            .map(|eq| eq.latex.clone());
        if latex != current {
            self.update_session(|s| s.set_topic_equation(topic, latex), cx);
        }
        self.cancel_equation_card(window, cx);
    }

    /// Cancel: closes the card, leaving the topic as it was.
    pub(super) fn cancel_equation_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.equation_card.is_none() {
            return;
        }
        self.close_equation_card(window);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Closes the card, if open, and releases its preview's texture.
    fn close_equation_card(&mut self, window: &mut Window) {
        if let Some(card) = self.equation_card.take() {
            release_preview(card.preview, window);
        }
    }

    /// Closes the card once its topic is no longer on the active page's
    /// active layer (deleted, undone, or another page or layer shown),
    /// giving the editor back its focus; redraws the preview after a theme
    /// change.
    pub(super) fn sync_equation_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.equation_card.as_ref() else {
            return;
        };
        if !card_topic_present(&self.session, card.topic) {
            self.close_equation_card(window);
            self.focus_handle.focus(window, cx);
        } else if card.preview.color != cx.theme().foreground {
            self.refresh_equation_preview(window, cx);
        }
    }

    /// Records the card's laid-out height, which places it next frame.
    fn set_equation_card_height(&mut self, height: f32, cx: &mut Context<Self>) {
        if let Some(card) = self.equation_card.as_mut() {
            if card.height != Some(height) {
                card.height = Some(height);
                cx.notify();
            }
        }
    }

    /// Renders the typed LaTeX for the preview, in the theme's text colour.
    fn refresh_equation_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.equation_card.as_mut() else {
            return;
        };
        let latex = card.input.read(cx).value().trim().to_string();
        let color = cx.theme().foreground;
        let render = render_equation(&latex, color, PREVIEW_FONT_SIZE);
        let room = (CARD_WIDTH - 2. * CARD_PADDING, PREVIEW_HEIGHT);
        let image = render.as_ref().ok().and_then(|r| {
            let shown = fitted(r.size, room)?;
            let image = rasterise(&r.svg, window.scale_factor() * shown.0 / r.size.0, cx)?;
            Some((image, shown))
        });
        let old = std::mem::replace(
            &mut card.preview,
            Preview {
                blank: latex.is_empty(),
                color,
                render,
                image,
            },
        );
        release_preview(old, window);
        cx.notify();
    }

    /// Inserts `snippet` at the caret (in place of the selection) and puts
    /// the caret where typing goes next.
    fn insert_equation_snippet(
        &mut self,
        snippet: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.equation_card.as_ref() else {
            return;
        };
        card.input.clone().update(cx, |state, cx| {
            let start = state.selected_range().start;
            state.replace(snippet, window, cx);
            let caret = start + snippet_caret(snippet);
            state.set_selected_range(caret..caret, cx);
            state.focus(window, cx);
        });
        self.refresh_equation_preview(window, cx);
    }

    /// The card, beside its topic on the canvas, when one is open.
    pub(super) fn render_equation_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.equation_card.as_ref()?;
        let frame = self.session.find_shape(card.topic)?.frame;
        let canvas_size = self.canvas_bounds?.size;
        let topic = topic_on_screen(&frame, &self.session.page().viewport);
        let window_size = (f32::from(canvas_size.width), f32::from(canvas_size.height));
        let max_height = (window_size.1 - 2. * MARGIN).max(0.);
        let height = card.height.unwrap_or(CARD_HEIGHT_GUESS).min(max_height);
        let (left, top) = card_origin(topic, (CARD_WIDTH, height), window_size, CARD_GAP);
        let editor = cx.entity();
        let theme = cx.theme();
        let can_apply =
            equation_to_apply(&card.input.read(cx).value(), &card.preview.render).is_some();
        let snippets = SNIPPETS.iter().enumerate().map(|(ix, &(label, snippet))| {
            Button::new(("equation-snippet", ix))
                .outline()
                .xsmall()
                .label(label)
                .tooltip(snippet)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.insert_equation_snippet(snippet, window, cx)
                }))
        });
        // In a short canvas the preview gives up its height first.
        let preview = div()
            .h(px(PREVIEW_HEIGHT))
            .min_h(px(0.))
            .flex_shrink(1.)
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .map(|area| match (&card.preview.render, &card.preview.image) {
                _ if card.preview.blank => area
                    .text_color(theme.muted_foreground)
                    .child("Type LaTeX to see the equation."),
                (Ok(_), Some((image, (w, h)))) => {
                    area.child(img(image.clone()).w(px(*w)).h(px(*h)))
                }
                (Ok(_), None) => area,
                (Err(message), _) => area
                    .px(px(CARD_SPACING))
                    .text_color(theme.danger)
                    .child(message.clone()),
            });
        let card_element = v_flex()
            .id("equation-card")
            .key_context(EQUATION_CARD_CONTEXT)
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(CARD_WIDTH))
            .max_h(px(max_height))
            .p(px(CARD_PADDING))
            .gap(px(CARD_SPACING))
            .occlude()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_action(
                cx.listener(|this, _: &Escape, window, cx| this.cancel_equation_card(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ApplyEquation, window, cx| {
                this.apply_equation_card(window, cx)
            }))
            // Measures the card (its border box less the border).
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let height = f32::from(bounds.size.height) + 2.;
                        editor.update(cx, |this, cx| this.set_equation_card_height(height, cx));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(section_label("Equation", cx).flex_none())
            .child(
                div().flex_none().child(
                    Textarea::new(&card.input)
                        .h(px(INPUT_HEIGHT))
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(INPUT_TEXT_SIZE)),
                ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .flex_wrap()
                    .gap(px(SNIPPET_GAP))
                    .children(snippets),
            )
            .child(preview)
            .child(
                h_flex()
                    .flex_none()
                    .justify_end()
                    .gap(px(CARD_SPACING))
                    .child(
                        Button::new("equation-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel_equation_card(window, cx)
                            })),
                    )
                    .child(
                        Button::new("equation-done")
                            .primary()
                            .small()
                            .label("Done")
                            .tooltip("Done (⌘Return)")
                            .disabled(!can_apply)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_equation_card(window, cx)
                            })),
                    ),
            );
        Some(card_element.into_any_element())
    }
}

/// Whether opening the card on `topic` finds it already open there
/// (`open` is the topic of the card open, if any), so it keeps its draft.
fn card_is_open_on(open: Option<Uuid>, topic: Uuid) -> bool {
    open == Some(topic)
}

/// Releases `preview`'s texture.
fn release_preview(preview: Preview, window: &mut Window) {
    if let Some((image, _)) = preview.image {
        window.drop_image(image).ok();
    }
}

/// The size an equation `size` big is shown at in `room`: its own, scaled
/// down to fit; `None` when it has no area.
fn fitted(size: (f32, f32), room: (f32, f32)) -> Option<(f32, f32)> {
    if !(size.0 > 0. && size.1 > 0.) {
        return None;
    }
    let scale = (room.0 / size.0).min(room.1 / size.1).min(1.);
    Some((size.0 * scale, size.1 * scale))
}

/// The insert snippets: button label and the LaTeX inserted at the caret.
const SNIPPETS: [(&str, &str); 9] = [
    ("a/b", r"\frac{}{}"),
    ("√", r"\sqrt{}"),
    ("xⁿ", "^{}"),
    ("xₙ", "_{}"),
    ("∑", r"\sum_{}^{}"),
    ("∫", r"\int_{}^{}"),
    ("( )", r"\begin{pmatrix}  \end{pmatrix}"),
    ("α", r"\alpha"),
    ("→", r"\ce{ -> }"),
];

/// What Done does with the typed LaTeX `input`, given its preview:
/// `Some(None)` removes the equation (the input is blank), `Some(Some(latex))`
/// sets the trimmed LaTeX (it renders), and `None` means Done is unavailable
/// (it does not render).
pub(super) fn equation_to_apply(
    input: &str,
    preview: &Result<RenderedEquation, String>,
) -> Option<Option<String>> {
    let latex = input.trim();
    if latex.is_empty() {
        return Some(None);
    }
    preview.is_ok().then(|| Some(latex.to_string()))
}

/// Where the caret goes in `snippet` once it is inserted, in bytes from its
/// start: inside its first empty braces, between the spaces of a matrix,
/// at the start of a reaction (before its arrow), else after it.
fn snippet_caret(snippet: &str) -> usize {
    ["{}", "  ", "{ "]
        .iter()
        .filter_map(|mark| snippet.find(mark).map(|at| at + 1))
        .min()
        .unwrap_or(snippet.len())
}

#[cfg(test)]
mod tests {
    use super::{card_is_open_on, equation_to_apply, fitted, snippet_caret, SNIPPETS};
    use crate::equation::render_equation;
    use gpui_kit::hsla;

    #[test]
    fn a_blank_input_removes_the_equation() {
        let preview = Err("anything".to_string());
        assert_eq!(equation_to_apply("  \n ", &preview), Some(None));
    }

    #[test]
    fn a_valid_input_is_applied_trimmed() {
        let preview = render_equation(r"\frac{a}{b}", hsla(0., 0., 0., 1.), 20.);
        assert_eq!(
            equation_to_apply(" \\frac{a}{b}\n", &preview),
            Some(Some(r"\frac{a}{b}".to_string()))
        );
    }

    #[test]
    fn an_invalid_input_cannot_be_applied() {
        let preview = render_equation(r"\frac{a", hsla(0., 0., 0., 1.), 20.);
        assert!(preview.is_err());
        assert_eq!(equation_to_apply(r"\frac{a", &preview), None);
    }

    #[test]
    fn snippets_put_the_caret_where_typing_goes_next() {
        let carets: Vec<(&str, usize)> = SNIPPETS
            .iter()
            .map(|(_, s)| (*s, snippet_caret(s)))
            .collect();
        assert_eq!(
            carets,
            [
                (r"\frac{}{}", 6),
                (r"\sqrt{}", 6),
                ("^{}", 2),
                ("_{}", 2),
                (r"\sum_{}^{}", 6),
                (r"\int_{}^{}", 6),
                (r"\begin{pmatrix}  \end{pmatrix}", 16),
                (r"\alpha", 6),
                (r"\ce{ -> }", 4),
            ]
        );
    }

    #[test]
    fn previews_shrink_to_fit_but_never_grow() {
        assert_eq!(fitted((40., 20.), (300., 80.)), Some((40., 20.)));
        assert_eq!(fitted((600., 40.), (300., 80.)), Some((300., 20.)));
        assert_eq!(fitted((100., 160.), (300., 80.)), Some((50., 80.)));
        assert_eq!(fitted((0., 20.), (300., 80.)), None);
    }

    #[test]
    fn opening_the_card_again_on_its_topic_keeps_the_draft() {
        let (a, b) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        assert!(card_is_open_on(Some(a), a));
        assert!(!card_is_open_on(Some(a), b));
        assert!(!card_is_open_on(None, a));
    }
}
