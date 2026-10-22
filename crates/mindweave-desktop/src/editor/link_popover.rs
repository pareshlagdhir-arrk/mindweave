//! The link popover (Cmd+K): one field, beside the text being edited, for
//! the link on the selection or, with only a caret, on the word at it.
//! Return applies it (an empty field removes the link); Escape closes it.
//! It works on any text being edited: a label on the canvas or a note.

use std::ops::Range;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _};
use gpui_kit::*;

use super::mindmap_edit::normalized_link;
use super::text_edit::target_text;
use super::EditorWindow;

/// The popover's width, in pixels.
const POPOVER_WIDTH: f32 = 260.;
/// Space between the popover and the text it is for.
const POPOVER_GAP: f32 = 6.;
const POPOVER_PADDING: f32 = 6.;
/// The least space kept between the popover and the canvas's edges.
const EDGE: f32 = 8.;

/// The popover open on the text being edited.
pub(super) struct LinkPopover {
    /// The link being typed.
    input: Entity<InputState>,
    /// The text the link applies to, fixed when the popover opened.
    range: Range<usize>,
    /// Its top left corner, in pixels from the canvas's top left.
    at: (f32, f32),
    _subscription: Subscription,
}

/// The link to store for what was typed: none when blank.
pub(super) fn typed_link(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| normalized_link(value))
}

/// Where the popover goes, below the text `lo`..`hi` (corners, in pixels
/// from the canvas's top left), or above it when there is no room below,
/// kept inside a canvas `canvas` big.
pub(super) fn popover_origin(
    lo: (f32, f32),
    hi: (f32, f32),
    size: (f32, f32),
    canvas: (f32, f32),
) -> (f32, f32) {
    let x = lo.0.min(canvas.0 - size.0 - EDGE).max(EDGE);
    let below = hi.1 + POPOVER_GAP;
    let y = if below + size.1 + EDGE <= canvas.1 {
        below
    } else {
        (lo.1 - POPOVER_GAP - size.1).max(EDGE)
    };
    (x, y)
}

impl EditorWindow {
    /// Cmd+K while editing text: opens the popover on the selection (or
    /// the word at the caret), its field holding the link there.
    pub(super) fn open_link_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.text_edit.as_ref() else {
            return;
        };
        let Some((text, _)) = target_text(&self.session, edit.target) else {
            return;
        };
        let range = edit.link_range(&text);
        if range.is_empty() {
            window.push_notification(Notification::warning("Select text to link."), cx);
            return;
        }
        let current = text.link_at(range.start).unwrap_or_default().to_string();
        let Some((lo, hi)) = self.range_bounds(range.clone()) else {
            return;
        };
        let canvas = self.canvas_bounds.map_or((f32::MAX, f32::MAX), |b| {
            (f32::from(b.size.width), f32::from(b.size.height))
        });
        let height = 2. * POPOVER_PADDING + 30.;
        let at = popover_origin(lo, hi, (POPOVER_WIDTH, height), canvas);
        let end = current.len();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Link (empty removes it)")
                .default_value(current)
        });
        input.update(cx, |state, cx| {
            state.set_selected_range(0..end, cx);
            state.focus(window, cx);
        });
        let _subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.apply_link_popover(window, cx);
            }
        });
        self.link_popover = Some(LinkPopover {
            input,
            range,
            at,
            _subscription,
        });
        cx.notify();
    }

    /// Return: sets the link on the popover's text to the typed value (none
    /// when blank) as one undo step, and closes the popover.
    fn apply_link_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popover) = self.link_popover.as_ref() else {
            return;
        };
        let link = typed_link(popover.input.read(cx).value().as_ref());
        let range = popover.range.clone();
        self.with_text(window, cx, |ed, text, _| {
            let end = range.end.min(text.len());
            ed.set_link(text, range.start.min(end)..end, link)
        });
        self.close_link_popover(window, cx);
    }

    /// Closes the popover, if open, giving the editor back its focus.
    pub(super) fn close_link_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link_popover.take().is_some() {
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    /// The popover, over the canvas beside its text, when one is open.
    pub(super) fn render_link_popover(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let popover = self.link_popover.as_ref()?;
        let theme = cx.theme();
        Some(
            div()
                .id("link-popover")
                .absolute()
                .left(px(popover.at.0))
                .top(px(popover.at.1))
                .w(px(POPOVER_WIDTH))
                .p(px(POPOVER_PADDING))
                .occlude()
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .border_1()
                .border_color(theme.border)
                .rounded(theme.radius_lg)
                .shadow_lg()
                .on_action(
                    cx.listener(|this, _: &Escape, window, cx| this.close_link_popover(window, cx)),
                )
                .child(Input::new(&popover.input).small())
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{popover_origin, typed_link};

    #[test]
    fn a_blank_value_removes_the_link_and_others_are_normalized() {
        assert_eq!(typed_link("  "), None);
        assert_eq!(typed_link(""), None);
        assert_eq!(
            typed_link(" example.com "),
            Some("https://example.com".into())
        );
    }

    #[test]
    fn the_popover_opens_below_its_text_or_above_when_cramped() {
        let size = (200., 40.);
        let canvas = (800., 600.);
        assert_eq!(
            popover_origin((100., 100.), (150., 120.), size, canvas),
            (100., 126.)
        );
        assert_eq!(
            popover_origin((100., 560.), (150., 580.), size, canvas),
            (100., 514.)
        );
        // Kept inside the canvas sideways.
        assert_eq!(
            popover_origin((700., 100.), (750., 120.), size, canvas),
            (592., 126.)
        );
    }
}
