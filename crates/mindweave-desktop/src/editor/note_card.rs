//! The note card: a topic's note, or its task's description, edited in
//! place in a card beside the topic. The card shows a text-editing session
//! on the note ([`EditTarget::Note`]) or description
//! ([`EditTarget::Description`]), so the canvas editor's keys, clipboard,
//! input methods, links and undo work in it, and the session (open to
//! close) is one undo step. A text left blank is removed when the card
//! closes.
//!
//! The note is laid out at zoom 1 whatever the canvas zoom, and its
//! geometry ([`EditorWindow::note_geometry`]) is in pixels from the
//! canvas's top left, where the card shows it.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _, WindowExt as _};
use gpui_kit::*;
use mindweave_core::geometry::Pt;
use mindweave_core::{DocumentSession, RichText, LINE_BREAK};
use uuid::Uuid;

use super::cards::{card_origin, card_topic_present, topic_on_screen, MARGIN};
use super::color_palette::palette_popover;
use super::text_edit::{
    target_text, Area, EditGeometry, EditTarget, TextCommand, Toggle, NOTE_TEXT_WIDTH,
};
use super::text_paint::{paint_upright_text, TextFrame};
use super::EditorWindow;
use crate::libraries::lucide_icon_path;

/// The card's width, in pixels.
const CARD_WIDTH: f32 = 320.;
/// The tallest the card grows; longer notes scroll.
const CARD_MAX_HEIGHT: f32 = 360.;
/// The card's height until it is first laid out.
const CARD_HEIGHT_GUESS: f32 = 140.;
/// Space between the card and its topic.
const CARD_GAP: f32 = 12.;
/// Padding beside the text, inside the 1 px border: the text fills the
/// card's width at the note's wrap width.
const CARD_PADDING_X: f32 = (CARD_WIDTH - NOTE_TEXT_WIDTH) / 2. - 1.;
/// Padding above and below the card's content.
const CARD_PADDING_Y: f32 = 8.;
/// Space between the formatting bar, the text and the hint.
const CARD_SPACING: f32 = 6.;
/// Space above and below the text, inside its scrolling area.
const TEXT_PADDING_Y: f32 = 4.;
/// The least height of the text area, so an empty note is easy to click.
const MIN_TEXT_HEIGHT: f32 = 48.;
/// The size of the formatting bar's icons.
const ICON_SIZE: f32 = 14.;
/// The text size of the hint.
const HINT_TEXT_SIZE: f32 = 11.;
/// The hint at the bottom of the card.
const HINT: &str = "⌘-click a link to open it";
/// The most characters of a note its badge's tooltip shows.
const PREVIEW_CHARS: usize = 80;
/// The badge tooltip's offset from the pointer.
const TOOLTIP_OFFSET: (f32, f32) = (12., 18.);
/// The badge tooltip's widest.
const TOOLTIP_WIDTH: f32 = 260.;
/// The badge tooltip's text size.
const TOOLTIP_TEXT_SIZE: f32 = 12.;

/// The note card open on a topic.
pub(super) struct NoteCard {
    /// The topic whose note or task description it edits.
    pub topic: Uuid,
    /// What it edits: the topic's note or its task's description.
    pub target: EditTarget,
    /// The height of the note's text, laid out, in pixels.
    text_height: f32,
    /// The card's height when it was last laid out, in pixels.
    height: Option<f32>,
    /// The text area's scroll position.
    scroll: ScrollHandle,
    /// The caret and text length last scrolled into view.
    revealed: Option<(usize, usize)>,
}

/// What a press on the canvas does to the open note card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutsidePress {
    /// Nothing: the second click of the double click that opened it.
    Keep,
    /// Closes it and goes on as a press on the canvas.
    Close,
    /// Closes it and does nothing else: a press on its topic's badge.
    CloseOnly,
}

/// What a press with `clicks` clicks on the canvas does to the note card
/// open on topic `open`, given the topic whose note badge it lands on.
pub(super) fn outside_press(open: Uuid, badge: Option<Uuid>, clicks: usize) -> OutsidePress {
    match badge {
        Some(topic) if topic == open && clicks > 1 => OutsidePress::Keep,
        Some(topic) if topic == open => OutsidePress::CloseOnly,
        _ => OutsidePress::Close,
    }
}

/// Whether the note card open on `target` stays open: its topic is on the
/// active page's active layer and `target` is the text being edited
/// (`editing`).
pub(super) fn note_card_stays(
    session: &DocumentSession,
    editing: Option<EditTarget>,
    target: EditTarget,
) -> bool {
    editing == Some(target) && card_topic_present(session, target.id())
}

/// The note's first paragraph with any text, for its badge's tooltip:
/// line breaks read as spaces, cut at [`PREVIEW_CHARS`] characters with
/// "…". Empty for a blank note.
pub(super) fn note_preview(note: &RichText) -> String {
    let first = note
        .paragraphs
        .iter()
        .map(|p| {
            let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            text.replace(LINE_BREAK, " ").trim().to_string()
        })
        .find(|text| !text.is_empty())
        .unwrap_or_default();
    if first.chars().count() <= PREVIEW_CHARS {
        return first;
    }
    let cut: String = first.chars().take(PREVIEW_CHARS).collect();
    format!("{cut}…")
}

impl EditorWindow {
    /// Cmd+Shift+N and the ribbon's Note button: opens the note card on the
    /// one selected topic; with any other selection, says what to select.
    pub(super) fn edit_selected_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_topic() {
            Some(topic) => self.open_note_card(topic, window, cx),
            None => window
                .push_notification(Notification::warning("Select one topic to add a note."), cx),
        }
    }

    /// Opens the note card on `topic`: selects the topic and edits its note
    /// (an empty one when it has none), with the caret at its end. Already
    /// open there, the card gets the keys back.
    pub(super) fn open_note_card(
        &mut self,
        topic: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_text_card(EditTarget::Note(topic), window, cx);
    }

    /// Opens the card on the description of `topic`'s task, as
    /// [`Self::open_note_card`] does on its note.
    pub(super) fn open_description_card(
        &mut self,
        topic: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_text_card(EditTarget::Description(topic), window, cx);
    }

    /// Opens the card on `target`, a note or a task description.
    fn open_text_card(&mut self, target: EditTarget, window: &mut Window, cx: &mut Context<Self>) {
        let topic = target.id();
        let editing = self.text_edit.as_ref().map(|e| e.target);
        if self.note_card.as_ref().is_some_and(|c| c.target == target) && editing == Some(target) {
            self.focus_handle.focus(window, cx);
            return;
        }
        // Another card's text ends its session first, as one undo step.
        self.close_note_card(window, cx);
        self.hand_over_equation_card(window, cx);
        self.begin_text_edit(target, window, cx);
        let editing = self.text_edit.as_ref().map(|e| e.target);
        if editing == Some(target) {
            self.note_card = Some(NoteCard {
                topic,
                target,
                text_height: 0.,
                height: None,
                scroll: ScrollHandle::new(),
                revealed: None,
            });
        }
        cx.notify();
    }

    /// Closes the note card, if open, ending the note's session: a blank
    /// note is removed, and the session's edits become one undo step (none
    /// when nothing changed).
    pub(super) fn close_note_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_card.is_none() {
            return;
        }
        let editing_note = self.text_edit.as_ref().is_some_and(|e| e.target.in_card());
        if editing_note {
            self.end_text_edit(window, cx);
        }
        self.note_card = None;
        self.note_geometry = None;
        cx.notify();
    }

    /// A left press on the canvas, at canvas point `at`, while the note
    /// card is open (the card itself takes presses on it): closes the card
    /// (see [`outside_press`]). Returns whether the press is used up.
    pub(super) fn press_outside_note_card(
        &mut self,
        at: Pt,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(card) = self.note_card.as_ref() else {
            return false;
        };
        // Only a note's card closes from its badge.
        let badge = match card.target {
            EditTarget::Note(_) => self.note_badge_at(at),
            _ => None,
        };
        match outside_press(card.topic, badge, clicks) {
            OutsidePress::Keep => true,
            OutsidePress::Close => {
                self.close_note_card(window, cx);
                false
            }
            OutsidePress::CloseOnly => {
                self.close_note_card(window, cx);
                true
            }
        }
    }

    /// Closes the card once its topic is no longer on the active page's
    /// active layer (deleted, undone, or another page or layer shown) or
    /// its note is no longer edited; else lays the note out for the card's
    /// height.
    pub(super) fn sync_note_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.note_card.as_ref() else {
            return;
        };
        let target = card.target;
        let editing = self.text_edit.as_ref().map(|e| e.target);
        if !note_card_stays(&self.session, editing, target) {
            self.close_note_card(window, cx);
            return;
        }
        let Some((text, wrap)) = target_text(&self.session, target) else {
            return;
        };
        let color = cx.theme().popover_foreground;
        let (_, height) = self
            .text_layouts
            .size(window, target.layout_id(), &text, wrap, color);
        if let Some(card) = self.note_card.as_mut() {
            card.text_height = height;
        }
    }

    /// Records the card's laid-out height, which places it next frame.
    fn set_note_card_height(&mut self, height: f32, cx: &mut Context<Self>) {
        if let Some(card) = self.note_card.as_mut() {
            if card.height != Some(height) {
                card.height = Some(height);
                cx.notify();
            }
        }
    }

    /// Gives the editor the keys back after a press in the card, closing
    /// the link popover.
    fn focus_note_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_link_popover(window, cx);
        self.focus_handle.focus(window, cx);
    }

    /// A left press in the card's text area: places the caret (Shift:
    /// extends the selection) or selects a word or paragraph, starting a
    /// drag selection; with the platform key, opens the link there.
    fn note_text_press(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_note_card(window, cx);
        let Some(canvas) = self.canvas_bounds else {
            return;
        };
        let p = event.position - canvas.origin;
        self.text_press(
            (f32::from(p.x), f32::from(p.y)),
            event.click_count,
            event.modifiers.shift,
            event.modifiers.platform,
            window,
            cx,
        );
    }

    /// Runs `command` on the note from the card's formatting bar.
    fn note_command(&mut self, command: TextCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.text_command(command, window, cx);
    }

    /// Paints the note into the card's text area, `bounds` (its text at
    /// zoom 1, top left at the origin), with its selection and caret, and
    /// records where it is for presses, input methods and the link popover.
    fn paint_note_text(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.note_card.as_ref() else {
            return;
        };
        let Some(canvas) = self.canvas_bounds else {
            return;
        };
        let target = card.target;
        let view = card.scroll.bounds();
        let local = |p: Point<Pixels>| {
            let p = p - canvas.origin;
            (f32::from(p.x), f32::from(p.y))
        };
        let centre = local(view.center());
        self.note_geometry = Some(EditGeometry {
            frame: TextFrame {
                centre: local(bounds.origin),
                offset: (0., 0.),
                angle: 0.,
            },
            area: Area {
                centre,
                half: (
                    f32::from(view.size.width) / 2.,
                    f32::from(view.size.height) / 2.,
                ),
                angle: 0.,
            },
        });
        let color = cx.theme().popover_foreground;
        let Some((text, wrap)) = target_text(&self.session, target) else {
            return;
        };
        paint_upright_text(
            window,
            cx,
            &mut self.text_layouts,
            &mut self.glyph_outlines,
            target.layout_id(),
            &text,
            wrap,
            color,
            bounds.origin,
        );
        self.paint_note_marks(bounds.origin, window, cx);
        self.reveal_note_caret(bounds.origin, window);
    }

    /// Scrolls the text area so the caret, with the text's top left at
    /// `origin`, is in view after it moved or the text changed.
    fn reveal_note_caret(&mut self, origin: Point<Pixels>, window: &mut Window) {
        let Some(edit) = self.text_edit.as_ref() else {
            return;
        };
        let Some(card) = self.note_card.as_mut() else {
            return;
        };
        let target = edit.target;
        let Some((text, wrap)) = target_text(&self.session, target) else {
            return;
        };
        let key = (edit.caret, text.len());
        if card.revealed == Some(key) {
            return;
        }
        let Some(layout) = self.text_layouts.cached(target.layout_id(), &text, wrap) else {
            return;
        };
        card.revealed = Some(key);
        let caret = layout.caret_rect(edit.caret);
        let view = card.scroll.bounds();
        let top = origin.y + px(caret.y - TEXT_PADDING_Y);
        let bottom = origin.y + px(caret.y + caret.h + TEXT_PADDING_Y);
        let mut offset = card.scroll.offset();
        if top < view.top() {
            offset.y += view.top() - top;
        } else if bottom > view.bottom() {
            offset.y -= bottom - view.bottom();
        } else {
            return;
        }
        card.scroll.set_offset(offset);
        window.refresh();
    }

    /// A tooltip with the start of the note whose badge the pointer is
    /// over (not the one open in the card).
    pub(super) fn render_note_tooltip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let at = self.cursor_canvas?;
        let pointer = self.pointer_local?;
        let topic = self.note_badge_at(at)?;
        if self
            .note_card
            .as_ref()
            .is_some_and(|c| c.target == EditTarget::Note(topic))
        {
            return None;
        }
        let note = self
            .session
            .find_shape(topic)?
            .topic
            .as_ref()?
            .note
            .as_ref()?;
        let preview = note_preview(note);
        if preview.is_empty() {
            return None;
        }
        let canvas_width = f32::from(self.canvas_bounds?.size.width);
        let left = (pointer.0 + TOOLTIP_OFFSET.0)
            .min(canvas_width - TOOLTIP_WIDTH - MARGIN)
            .max(MARGIN);
        let theme = cx.theme();
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(pointer.1 + TOOLTIP_OFFSET.1))
                .max_w(px(TOOLTIP_WIDTH))
                .px(px(6.))
                .py(px(3.))
                .bg(theme.popover)
                .text_color(theme.popover_foreground)
                .text_size(px(TOOLTIP_TEXT_SIZE))
                .border_1()
                .border_color(theme.border)
                .rounded(theme.radius)
                .shadow_md()
                .child(preview)
                .into_any_element(),
        )
    }

    /// The card, beside its topic on the canvas, when one is open.
    pub(super) fn render_note_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.note_card.as_ref()?;
        let frame = self.session.find_shape(card.topic)?.frame;
        let canvas_size = self.canvas_bounds?.size;
        let topic = topic_on_screen(&frame, &self.session.page().viewport);
        let window_size = (f32::from(canvas_size.width), f32::from(canvas_size.height));
        let max_height = CARD_MAX_HEIGHT.min(window_size.1 - 2. * MARGIN).max(0.);
        let height = card.height.unwrap_or(CARD_HEIGHT_GUESS).min(max_height);
        let (left, top) = card_origin(topic, (CARD_WIDTH, height), window_size, CARD_GAP);
        let measure = cx.entity();
        let painter = cx.entity();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let icon = |name: &str, cx: &App| {
            svg()
                .path(lucide_icon_path(name, cx))
                .size(px(ICON_SIZE))
                .text_color(cx.theme().popover_foreground)
        };
        let command = |id: &'static str, name: &str, tip: &'static str, c: TextCommand| {
            Button::new(id)
                .ghost()
                .xsmall()
                .child(icon(name, cx))
                .tooltip(tip)
                .on_click(cx.listener(move |this, _, window, cx| this.note_command(c, window, cx)))
        };
        let color = palette_popover(
            "note-color-palette",
            &self.properties.text.palette,
            Button::new("note-color")
                .ghost()
                .xsmall()
                .child(icon("baseline", cx))
                .tooltip("Colour"),
        );
        let bar = h_flex()
            .flex_none()
            .gap(px(2.))
            .child(command(
                "note-bold",
                "bold",
                "Bold (⌘B)",
                TextCommand::Toggle(Toggle::Bold),
            ))
            .child(command(
                "note-italic",
                "italic",
                "Italic (⌘I)",
                TextCommand::Toggle(Toggle::Italic),
            ))
            .child(command(
                "note-underline",
                "underline",
                "Underline (⌘U)",
                TextCommand::Toggle(Toggle::Underline),
            ))
            .child(command(
                "note-strike",
                "strikethrough",
                "Strikethrough (⌘⇧X)",
                TextCommand::Toggle(Toggle::Strike),
            ))
            .child(color)
            .child(command(
                "note-bullets",
                "list",
                "Bulleted List",
                TextCommand::ToggleBullets,
            ))
            .child(command(
                "note-numbers",
                "list-ordered",
                "Numbered List",
                TextCommand::ToggleNumbered,
            ))
            .child(command(
                "note-link",
                "link",
                "Link (⌘K)",
                TextCommand::EditLink,
            ));
        let title = matches!(card.target, EditTarget::Description(_)).then(|| {
            div()
                .flex_none()
                .text_size(px(HINT_TEXT_SIZE))
                .text_color(muted)
                .child("Task description")
        });
        let text_height = card.text_height;
        let text_area = div()
            .id("note-text")
            .h(px(text_height.max(MIN_TEXT_HEIGHT) + 2. * TEXT_PADDING_Y))
            .min_h(px(0.))
            .flex_shrink(1.)
            .py(px(TEXT_PADDING_Y))
            .overflow_y_scroll()
            .track_scroll(&card.scroll)
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.note_text_press(event, window, cx);
                }),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        painter.update(cx, |this, cx| this.paint_note_text(bounds, window, cx));
                    },
                )
                .w(px(NOTE_TEXT_WIDTH))
                .h(px(text_height)),
            );
        let card_element = v_flex()
            .id("note-card")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(CARD_WIDTH))
            .max_h(px(max_height))
            .px(px(CARD_PADDING_X))
            .py(px(CARD_PADDING_Y))
            .gap(px(CARD_SPACING))
            .occlude()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_note_card(window, cx)),
            )
            // Measures the card (its border box less the border).
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let height = f32::from(bounds.size.height) + 2.;
                        measure.update(cx, |this, cx| this.set_note_card_height(height, cx));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(title)
            .child(bar)
            .child(text_area)
            .child(
                div()
                    .flex_none()
                    .text_size(px(HINT_TEXT_SIZE))
                    .text_color(muted)
                    .child(HINT),
            );
        Some(card_element.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{note_card_stays, note_preview, outside_press, OutsidePress};
    use crate::editor::text_edit::{
        finish_text_session, start_text_session, store_target_text, EditTarget,
    };
    use crate::editor::SessionGesture;
    use mindweave_core::{DocumentSession, HAlign, RichText, RunStyle};

    fn plain(s: &str) -> RichText {
        let mut t = RichText::new(RunStyle::default(), HAlign::Left);
        t.insert(0, s, &RunStyle::default());
        t
    }

    fn session() -> (tempfile::TempDir, DocumentSession) {
        let dir = tempfile::tempdir().unwrap();
        let session = DocumentSession::create(dir.path().join("n.mindweave")).unwrap();
        (dir, session)
    }

    #[test]
    fn note_preview_is_the_first_non_empty_paragraph() {
        assert_eq!(note_preview(&plain("\n  \nBuy milk\nand eggs")), "Buy milk");
        assert_eq!(note_preview(&plain("")), "");
        // A line break inside the paragraph reads as a space.
        assert_eq!(note_preview(&plain("a\u{2028}b")), "a b");
    }

    #[test]
    fn note_preview_truncates_long_paragraphs() {
        let long = "x".repeat(81);
        assert_eq!(note_preview(&plain(&long)), format!("{}…", "x".repeat(80)));
        let exact = "é".repeat(80);
        assert_eq!(note_preview(&plain(&exact)), exact);
    }

    #[test]
    fn a_press_on_the_open_cards_badge_only_closes_it() {
        let (open, other) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        assert_eq!(outside_press(open, Some(open), 1), OutsidePress::CloseOnly);
        // The second click of the double click that opened it.
        assert_eq!(outside_press(open, Some(open), 2), OutsidePress::Keep);
        assert_eq!(outside_press(open, Some(other), 1), OutsidePress::Close);
        assert_eq!(outside_press(open, None, 2), OutsidePress::Close);
    }

    #[test]
    fn note_card_closes_when_its_topic_goes() {
        let (_dir, mut s) = session();
        let id = s.add_central_topic((0., 0.));
        let target = EditTarget::Note(id);
        let note = Some(target);
        assert!(note_card_stays(&s, note, target));
        // Its session ended (or another text is edited): it closes.
        assert!(!note_card_stays(&s, None, target));
        assert!(!note_card_stays(&s, Some(EditTarget::Label(id)), target));
        assert!(!note_card_stays(
            &s,
            Some(EditTarget::Description(id)),
            target
        ));
        s.delete_with_branches(&[id]);
        assert!(!note_card_stays(&s, note, target));
        assert!(s.undo());
        assert!(note_card_stays(&s, note, target));
        // Another page does not show it either.
        s.add_page();
        assert!(!note_card_stays(&s, note, target));
    }

    #[test]
    fn closing_a_new_empty_note_leaves_no_note() {
        let (_dir, mut s) = session();
        let id = s.add_central_topic((0., 0.));
        let target = EditTarget::Note(id);
        let note = |s: &DocumentSession| s.find_shape(id).unwrap().topic.clone().unwrap().note;
        let mut g = SessionGesture::default();
        // Opened and closed untouched.
        assert!(start_text_session(&mut s, &mut g, target));
        assert!(finish_text_session(&mut s, &mut g, target));
        assert_eq!(note(&s), None);
        // Typed into, then emptied again.
        assert!(start_text_session(&mut s, &mut g, target));
        store_target_text(&mut s, target, &plain("x"));
        store_target_text(&mut s, target, &plain(" "));
        assert!(finish_text_session(&mut s, &mut g, target));
        assert_eq!(note(&s), None);
        // No undo step either: undo takes the topic away.
        assert!(s.undo());
        assert!(s.find_shape(id).is_none());
    }

    #[test]
    fn a_task_description_is_rich_text_and_a_blank_one_is_removed() {
        let (_dir, mut s) = session();
        let id = s.add_central_topic((0., 0.));
        let target = EditTarget::Description(id);
        let mut g = SessionGesture::default();
        // Not a task yet: nothing to edit.
        assert!(!start_text_session(&mut s, &mut g, target));
        s.update_shapes(&[id], |shape| {
            shape.topic.as_mut().unwrap().task = Some(mindweave_core::Task::default());
        });
        let task = |s: &DocumentSession| {
            s.find_shape(id)
                .unwrap()
                .topic
                .clone()
                .unwrap()
                .task
                .unwrap()
        };
        assert!(start_text_session(&mut s, &mut g, target));
        store_target_text(&mut s, target, &plain("Ship **it**"));
        assert!(finish_text_session(&mut s, &mut g, target));
        assert_eq!(task(&s).description.unwrap().plain(), "Ship **it**");
        assert!(start_text_session(&mut s, &mut g, target));
        store_target_text(&mut s, target, &plain("  "));
        assert!(finish_text_session(&mut s, &mut g, target));
        assert_eq!(task(&s).description, None);
        // Each session is one undo step.
        assert!(s.undo());
        assert!(task(&s).description.is_some());
    }
}
