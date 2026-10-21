//! The on-canvas text editor: caret, selection, typing, clipboard and
//! word-grouped undo inside one text-editing session, and the platform
//! text input (IME, dead keys) that feeds it.
//!
//! [`TextEditor`] holds the session state and edits a copy of the target's
//! text; [`EditorWindow`] writes each change back to the target through the
//! session, inside the gesture the session was begun with, so the whole
//! session is one document undo step.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::{Pt, Rect};
use mindweave_core::{
    from_clipboard, DocumentSession, EditKind, EditSnapshot, HAlign, Label, RichText, RunStyle,
    Shape, StyleChange, StyleSummary, TextHistory, LINE_BREAK, MAX_CLIPBOARD_BYTES,
    MAX_PASTE_CHARS,
};
use serde_json::{json, Value};
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use super::text_layout::TextLayout;
use super::text_paint::{
    canvas_background, paint_polygon, text_color, text_frame, text_wrap, TextFrame,
    LINE_LABEL_PADDING,
};
use super::{EditorWindow, GestureOwner, SessionGesture};
use crate::actions::*;

/// Value of the `"mindweave"` key that marks a rich-text clipboard payload.
pub const TEXT_PAYLOAD_MARKER: &str = "text";
/// Rich-text payload version this build writes and reads.
pub const TEXT_PAYLOAD_VERSION: u64 = 1;
/// How long the caret stays on, then off.
const CARET_BLINK: Duration = Duration::from_millis(500);
/// Width of the painted caret, in pixels.
const CARET_PX: f32 = 2.;
/// Thickness of the line under text being composed, in pixels.
const MARKED_UNDERLINE_PX: f32 = 1.5;

/// Byte offset in `text` of UTF-16 offset `n` (inside a surrogate pair:
/// the character's start; past the end: the end).
pub fn utf16_to_byte(text: &str, n: usize) -> usize {
    let mut units = 0;
    for (i, ch) in text.char_indices() {
        let next = units + ch.len_utf16();
        if next > n {
            return i;
        }
        units = next;
    }
    text.len()
}

/// UTF-16 offset of byte offset `n` in `text` (inside a character: its
/// start).
pub fn byte_to_utf16(text: &str, n: usize) -> usize {
    text.char_indices()
        .take_while(|(i, ch)| i + ch.len_utf8() <= n)
        .map(|(_, ch)| ch.len_utf16())
        .sum()
}

/// `text` as a rich-text clipboard payload:
/// `{"mindweave":"text","version":1,"text":{...}}`.
pub fn text_payload(text: &RichText) -> String {
    json!({
        "mindweave": TEXT_PAYLOAD_MARKER,
        "version": TEXT_PAYLOAD_VERSION,
        "text": text,
    })
    .to_string()
}

/// The rich text in clipboard payload `payload`, normalised, or `None` when
/// it is not a rich-text payload of a version this build reads or is too
/// large.
pub fn parse_text_payload(payload: &str) -> Option<RichText> {
    if payload.len() > MAX_CLIPBOARD_BYTES {
        return None;
    }
    let mut value: Value = serde_json::from_str(payload).ok()?;
    let obj = value.as_object_mut()?;
    if obj.get("mindweave")?.as_str()? != TEXT_PAYLOAD_MARKER
        || obj.get("version")?.as_u64()? != TEXT_PAYLOAD_VERSION
    {
        return None;
    }
    // Deserialising normalises.
    serde_json::from_value(obj.remove("text")?).ok()
}

/// What Paste puts into the text.
#[derive(Clone, Debug, PartialEq)]
pub enum Pasted {
    Rich(RichText),
    Plain(String),
}

/// What to paste from a clipboard entry with `metadata` and `text`: the
/// rich text of a Mindweave payload, else the plain text, except copied
/// shapes, which are not text.
pub fn pasted(metadata: Option<&str>, text: Option<&str>) -> Option<Pasted> {
    if let Some(rich) = metadata.and_then(parse_text_payload) {
        return Some(Pasted::Rich(rich));
    }
    let text = text?;
    if from_clipboard(text).is_some() {
        return None;
    }
    Some(Pasted::Plain(text.to_string()))
}

/// `text` cut to its first [`MAX_PASTE_CHARS`] graphemes.
fn capped(text: &str) -> &str {
    let end = text
        .grapheme_indices(true)
        .nth(MAX_PASTE_CHARS)
        .map_or(text.len(), |(i, _)| i);
    &text[..end]
}

/// `at` moved back to a grapheme boundary of `text` (the end past it).
fn floor_boundary(text: &RichText, at: usize) -> usize {
    if at >= text.len() {
        text.len()
    } else {
        text.prev_grapheme(text.next_grapheme(at))
    }
}

/// What a press on the canvas hits while editing, from `current` (inside
/// the target's area, and the caret position, by its current layout) or,
/// before the text is laid out, whether the last `painted` area holds it:
/// `None` outside the text, else the caret position when known.
pub fn press_hit(current: Option<(bool, usize)>, painted: bool) -> Option<Option<usize>> {
    match current {
        Some((inside, offset)) => inside.then_some(Some(offset)),
        None => painted.then_some(None),
    }
}

/// The line geometry caret moves follow: a text layout.
pub trait Lines {
    /// The caret's x at `offset`, in canvas units from the text's left.
    fn caret_x(&self, offset: usize) -> f32;
    /// See [`TextLayout::vertical`].
    fn vertical(&self, offset: usize, x: f32, down: bool) -> Option<usize>;
    /// See [`TextLayout::line_range`].
    fn line_range(&self, offset: usize) -> Option<Range<usize>>;
}

impl Lines for TextLayout {
    fn caret_x(&self, offset: usize) -> f32 {
        self.caret_rect(offset).x
    }

    fn vertical(&self, offset: usize, x: f32, down: bool) -> Option<usize> {
        TextLayout::vertical(self, offset, x, down)
    }

    fn line_range(&self, offset: usize) -> Option<Range<usize>> {
        TextLayout::line_range(self, offset)
    }
}

/// How typing `s` groups for undo: spaces and breaks start a step, other
/// text joins the word being typed.
fn typing_kind(s: &str) -> EditKind {
    if !s.is_empty() && s.chars().all(char::is_whitespace) {
        EditKind::InsertSpace
    } else {
        EditKind::InsertWord
    }
}

/// A style toggle (Cmd+B, Cmd+I, Cmd+U, Cmd+Shift+X).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Toggle {
    fn flag(self, style: &mut RunStyle) -> &mut bool {
        match self {
            Toggle::Bold => &mut style.bold,
            Toggle::Italic => &mut style.italic,
            Toggle::Underline => &mut style.underline,
            Toggle::Strike => &mut style.strike,
        }
    }

    /// The value turning it on or off over a range with `summary` sets.
    pub(super) fn value(self, summary: &StyleSummary) -> bool {
        StyleSummary::toggle_value(match self {
            Toggle::Bold => &summary.bold,
            Toggle::Italic => &summary.italic,
            Toggle::Underline => &summary.underline,
            Toggle::Strike => &summary.strike,
        })
    }

    pub(super) fn change(self, on: bool) -> StyleChange {
        match self {
            Toggle::Bold => StyleChange::Bold(on),
            Toggle::Italic => StyleChange::Italic(on),
            Toggle::Underline => StyleChange::Underline(on),
            Toggle::Strike => StyleChange::Strike(on),
        }
    }
}

/// A caret movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

/// What a text key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCommand {
    /// A movement; `true` extends the selection.
    Move(Motion, bool),
    /// Deletes before the caret; `true`: a word.
    Backspace(bool),
    /// Deletes after the caret; `true`: a word.
    Delete(bool),
    NewParagraph,
    LineBreak,
    SelectAll,
    Toggle(Toggle),
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    Exit,
}

/// What a mouse drag selects by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Grapheme,
    Word,
    Paragraph,
}

/// One text-editing session on the label or text of shape `target`.
/// Offsets are byte offsets into the text's `plain()`.
pub struct TextEditor {
    pub target: Uuid,
    pub caret: usize,
    pub anchor: usize,
    /// Text being composed by an input method.
    pub marked: Option<Range<usize>>,
    /// The style typing uses, set by a toggle with nothing selected.
    pub typing_style: Option<RunStyle>,
    pub history: TextHistory,
    /// The state before the composition in progress began.
    composing_from: Option<EditSnapshot>,
    /// Where Up and Down aim, in canvas units from the text's left edge.
    goal_x: Option<f32>,
    /// A colour drag from the Properties panel is being previewed: its
    /// changes join the undo step its first preview recorded.
    style_preview: bool,
}

impl TextEditor {
    /// A session on `text` with the caret at its end.
    pub fn new(target: Uuid, text: &RichText) -> Self {
        let caret = text.len();
        TextEditor {
            target,
            caret,
            anchor: caret,
            marked: None,
            typing_style: None,
            history: TextHistory::new(EditSnapshot {
                text: text.clone(),
                caret,
                anchor: caret,
            }),
            composing_from: None,
            goal_x: None,
            style_preview: false,
        }
    }

    /// The selected bytes, in order (empty at the caret).
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    fn snapshot(&self, text: &RichText) -> EditSnapshot {
        EditSnapshot {
            text: text.clone(),
            caret: self.caret,
            anchor: self.anchor,
        }
    }

    /// Keeps the caret, selection and marked text inside `text`, which may
    /// have changed outside the editor.
    fn clamp(&mut self, text: &RichText) {
        let len = text.len();
        self.caret = self.caret.min(len);
        self.anchor = self.anchor.min(len);
        if self.marked.as_ref().is_some_and(|m| m.end > len) {
            self.marked = None;
        }
    }

    /// Selects from `anchor` to `caret`. The typing style is forgotten and
    /// the next edit starts a new undo step.
    pub fn select(&mut self, caret: usize, anchor: usize) {
        self.caret = caret;
        self.anchor = anchor;
        self.typing_style = None;
        self.goal_x = None;
        self.style_preview = false;
        self.history.break_group();
    }

    /// Moves the caret to `to`, extending the selection or not.
    fn move_to(&mut self, to: usize, extend: bool) {
        let anchor = if extend { self.anchor } else { to };
        self.select(to, anchor);
    }

    /// Replaces `range` with `s`, typed in the typing style, else in the
    /// style of the replaced text's first character, else in the style at
    /// `range.start`; the caret goes after it. Not recorded for undo.
    /// Returns the inserted bytes.
    fn splice(&mut self, text: &mut RichText, range: Range<usize>, s: &str) -> Range<usize> {
        let style = self.typing_style.take().unwrap_or_else(|| {
            let at = if range.is_empty() {
                range.start
            } else {
                text.next_grapheme(range.start)
            };
            text.style_at(at)
        });
        // Where the text model really cuts: `delete` widens the range to
        // grapheme boundaries and `insert` snaps back to one.
        let start = floor_boundary(text, range.start);
        text.delete(range);
        let start = floor_boundary(text, start);
        let before = text.len();
        let caret = text.insert(start, s, &style);
        let inserted = start..start + (text.len() - before);
        self.caret = caret;
        self.anchor = caret;
        self.goal_x = None;
        inserted
    }

    /// Types `s` over the selection.
    pub fn type_text(&mut self, text: &mut RichText, s: &str) {
        let before = self.snapshot(text);
        self.splice(text, self.selection(), s);
        if *text != before.text {
            self.history.record(before, typing_kind(s));
        }
    }

    /// Removes `range` as one undo step (a selection), or merging with the
    /// deletions before it.
    fn delete_range(&mut self, text: &mut RichText, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let before = self.snapshot(text);
        let kind = if self.selection().is_empty() {
            EditKind::Delete
        } else {
            EditKind::Other
        };
        text.delete(range.clone());
        let at = range.start.min(text.len());
        self.caret = at;
        self.anchor = at;
        self.typing_style = None;
        self.goal_x = None;
        self.history.record(before, kind);
    }

    /// Backspace: the selection, else the grapheme (or word) before the
    /// caret.
    fn delete_backward(&mut self, text: &mut RichText, word: bool) {
        let sel = self.selection();
        let range = if !sel.is_empty() {
            sel
        } else if word {
            text.prev_word(self.caret)..self.caret
        } else {
            text.prev_grapheme(self.caret)..self.caret
        };
        self.delete_range(text, range);
    }

    /// Delete: the selection, else the grapheme (or word) after the caret.
    fn delete_forward(&mut self, text: &mut RichText, word: bool) {
        let sel = self.selection();
        let range = if !sel.is_empty() {
            sel
        } else if word {
            self.caret..text.next_word(self.caret)
        } else {
            self.caret..text.next_grapheme(self.caret)
        };
        self.delete_range(text, range);
    }

    /// Turns a style on or off over the selection (one undo step), or with
    /// nothing selected, for the text typed next.
    pub fn toggle(&mut self, text: &mut RichText, which: Toggle) {
        let sel = self.selection();
        if sel.is_empty() {
            let mut style = self
                .typing_style
                .take()
                .unwrap_or_else(|| text.style_at(self.caret));
            let flag = which.flag(&mut style);
            *flag = !*flag;
            self.typing_style = Some(style);
            return;
        }
        let before = self.snapshot(text);
        let on = which.value(&text.summary(sel.clone()));
        text.apply(sel, &which.change(on));
        if *text != before.text {
            self.history.record(before, EditKind::Style);
        }
    }

    /// Sets one property from the Properties panel over the selection (one
    /// undo step), or with nothing selected, for the text typed next. A
    /// caret in an empty paragraph also gives the paragraph the style, so
    /// it survives a click back into the text. With `preview` (a colour
    /// drag), changes up to [`Self::end_style_preview`] are one undo step.
    pub fn apply_style(&mut self, text: &mut RichText, change: &StyleChange, preview: bool) {
        let sel = self.selection();
        let before = self.snapshot(text);
        if sel.is_empty() {
            let mut style = self
                .typing_style
                .take()
                .unwrap_or_else(|| text.style_at(self.caret));
            change.apply_to(&mut style);
            self.typing_style = Some(style);
            if !text.paragraph_range(self.caret).is_empty() {
                return;
            }
        }
        text.apply(sel, change);
        if *text != before.text {
            self.record_style(before, preview);
        }
    }

    /// Aligns the paragraphs the selection touches (a caret: its
    /// paragraph), as one undo step.
    pub fn set_align(&mut self, text: &mut RichText, align: HAlign) {
        let before = self.snapshot(text);
        text.set_align(self.selection(), align);
        if *text != before.text {
            self.record_style(before, false);
        }
    }

    /// Ends a colour drag's preview: the next change is a step of its own.
    pub fn end_style_preview(&mut self) {
        self.style_preview = false;
    }

    /// Records a style change made from `before`; a preview after another
    /// joins its step.
    fn record_style(&mut self, before: EditSnapshot, preview: bool) {
        if !(preview && self.style_preview) {
            self.history.record(before, EditKind::Style);
        }
        self.style_preview = preview;
    }

    /// The selection as plain text (line breaks as newlines) and as a
    /// rich-text payload; `None` when nothing is selected.
    pub fn copy(&self, text: &RichText) -> Option<(String, String)> {
        let sel = self.selection();
        if sel.is_empty() {
            return None;
        }
        let slice = text.slice(sel);
        let plain = slice.plain().replace(LINE_BREAK, "\n");
        Some((plain, text_payload(&slice)))
    }

    /// Copies, then removes the selection.
    fn cut(&mut self, text: &mut RichText) -> Option<(String, String)> {
        let copied = self.copy(text)?;
        self.delete_range(text, self.selection());
        Some(copied)
    }

    /// Replaces the selection with `rich` (at most [`MAX_PASTE_CHARS`]
    /// graphemes of it), keeping its styles.
    pub fn paste_rich(&mut self, text: &mut RichText, rich: &RichText) {
        if rich.is_empty() {
            return;
        }
        let plain = rich.plain();
        let capped = capped(&plain).len();
        let cut;
        let rich = if capped < plain.len() {
            cut = rich.slice(0..capped);
            &cut
        } else {
            rich
        };
        let before = self.snapshot(text);
        let sel = self.selection();
        text.delete(sel.clone());
        let caret = text.insert_rich(sel.start.min(text.len()), rich);
        self.caret = caret;
        self.anchor = caret;
        self.typing_style = None;
        self.goal_x = None;
        self.history.record(before, EditKind::Paste);
    }

    /// Replaces the selection with plain `s` (at most [`MAX_PASTE_CHARS`]
    /// graphemes of it), in the typing style.
    pub fn paste_plain(&mut self, text: &mut RichText, s: &str) {
        let s = capped(s);
        if s.is_empty() {
            return;
        }
        let before = self.snapshot(text);
        self.splice(text, self.selection(), s);
        self.history.record(before, EditKind::Paste);
    }

    /// Input method composition: replaces `range` (else the marked text,
    /// else the selection) with `s`, marked, selecting `selected` (bytes
    /// of `s`; else the caret after it). Nothing is recorded until the
    /// composition is committed.
    pub fn replace_and_mark(
        &mut self,
        text: &mut RichText,
        range: Option<Range<usize>>,
        s: &str,
        selected: Option<Range<usize>>,
    ) {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        if self.composing_from.is_none() {
            self.composing_from = Some(self.snapshot(text));
        }
        let inserted = self.splice(text, range, s);
        self.marked = (!inserted.is_empty()).then(|| inserted.clone());
        if let Some(sel) = selected {
            let at = |n: usize| (inserted.start + n).min(inserted.end);
            self.anchor = at(sel.start);
            self.caret = at(sel.end);
        }
    }

    /// Text input: replaces `range` (else the marked text, else the
    /// selection) with `s`. Ends a composition in progress as one undo
    /// step.
    pub fn replace_text(&mut self, text: &mut RichText, range: Option<Range<usize>>, s: &str) {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection());
        self.marked = None;
        let before = match self.composing_from.take() {
            Some(before) => {
                self.splice(text, range, s);
                return self.commit(text, before, s);
            }
            None => self.snapshot(text),
        };
        self.splice(text, range, s);
        if *text != before.text {
            self.history.record(before, typing_kind(s));
        }
    }

    /// Ends a composition, keeping the marked text.
    pub fn unmark(&mut self, text: &mut RichText) {
        let committed = self
            .marked
            .take()
            .and_then(|m| text.plain().get(m).map(str::to_string))
            .unwrap_or_default();
        if let Some(before) = self.composing_from.take() {
            self.commit(text, before, &committed);
        }
    }

    /// Records a finished composition that started at `before` and put in
    /// `committed`: a single letter (a dead key) joins the word being
    /// typed; longer text (an input method's word) is a step of its own.
    fn commit(&mut self, text: &RichText, before: EditSnapshot, committed: &str) {
        if *text == before.text {
            return;
        }
        let kind = if committed.graphemes(true).count() > 1 {
            EditKind::Other
        } else {
            typing_kind(committed)
        };
        self.history.record(before, kind);
    }

    /// Steps back through the session's edits; `false` with none left.
    pub fn undo(&mut self, text: &mut RichText) -> bool {
        self.unmark(text);
        let current = self.snapshot(text);
        match self.history.undo(current) {
            Some(snapshot) => {
                self.restore(text, snapshot);
                true
            }
            None => false,
        }
    }

    /// Steps forward again; `false` with nothing undone.
    pub fn redo(&mut self, text: &mut RichText) -> bool {
        self.unmark(text);
        let current = self.snapshot(text);
        match self.history.redo(current) {
            Some(snapshot) => {
                self.restore(text, snapshot);
                true
            }
            None => false,
        }
    }

    fn restore(&mut self, text: &mut RichText, snapshot: EditSnapshot) {
        *text = snapshot.text;
        self.caret = snapshot.caret;
        self.anchor = snapshot.anchor;
        self.clamp(text);
        self.typing_style = None;
        self.goal_x = None;
    }

    /// Selects from `origin` (the range first pressed) to the `unit` at
    /// `at`, for a mouse drag.
    pub fn drag_select(&mut self, text: &RichText, origin: &Range<usize>, unit: Unit, at: usize) {
        let here = match unit {
            Unit::Grapheme => at..at,
            Unit::Word => text.word_range(at),
            Unit::Paragraph => text.paragraph_range(at),
        };
        if here.start < origin.start {
            self.select(here.start, origin.end);
        } else {
            self.select(here.end.max(origin.end), origin.start);
        }
    }

    /// Moves the caret; `layout` (the text's current layout) places line
    /// moves.
    fn motion(&mut self, text: &RichText, layout: Option<&dyn Lines>, m: Motion, extend: bool) {
        let sel = self.selection();
        let collapse = !extend && !sel.is_empty();
        let from = match m {
            _ if !collapse => self.caret,
            Motion::Left | Motion::WordLeft | Motion::Up | Motion::LineStart | Motion::DocStart => {
                sel.start
            }
            _ => sel.end,
        };
        let to = match m {
            Motion::Left if collapse => sel.start,
            Motion::Right if collapse => sel.end,
            Motion::Left => text.prev_grapheme(from),
            Motion::Right => text.next_grapheme(from),
            Motion::WordLeft => text.prev_word(from),
            Motion::WordRight => text.next_word(from),
            Motion::Up | Motion::Down => {
                let down = m == Motion::Down;
                let goal = self.goal_x.or_else(|| layout.map(|l| l.caret_x(from)));
                let to = layout
                    .zip(goal)
                    .and_then(|(l, x)| l.vertical(from, x, down))
                    .unwrap_or(if down { text.len() } else { 0 });
                self.move_to(to, extend);
                self.goal_x = goal;
                return;
            }
            Motion::LineStart | Motion::LineEnd => {
                let line = layout
                    .and_then(|l| l.line_range(from))
                    .unwrap_or_else(|| text.paragraph_range(from));
                if m == Motion::LineStart {
                    line.start
                } else {
                    line.end
                }
            }
            Motion::DocStart => 0,
            Motion::DocEnd => text.len(),
        };
        self.move_to(to, extend);
    }

    /// Runs an editing key's command (clipboard and exit are the
    /// window's).
    fn command(&mut self, text: &mut RichText, layout: Option<&dyn Lines>, c: TextCommand) {
        match c {
            TextCommand::Move(m, extend) => self.motion(text, layout, m, extend),
            TextCommand::Backspace(word) => self.delete_backward(text, word),
            TextCommand::Delete(word) => self.delete_forward(text, word),
            TextCommand::NewParagraph => self.type_text(text, "\n"),
            TextCommand::LineBreak => self.type_text(text, LINE_BREAK.encode_utf8(&mut [0; 4])),
            TextCommand::SelectAll => self.select(text.len(), 0),
            TextCommand::Toggle(t) => self.toggle(text, t),
            TextCommand::Undo => {
                self.undo(text);
            }
            TextCommand::Redo => {
                self.redo(text);
            }
            TextCommand::Copy | TextCommand::Cut | TextCommand::Paste | TextCommand::Exit => {}
        }
    }
}

/// What a Text-tool click or a double click edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextTarget {
    /// The label of a shape, or a text box.
    Existing(Uuid),
    /// A new text box at this canvas point.
    NewBox(Pt),
}

/// What a Text-tool click or a double click at canvas point `at` edits,
/// given the shape `hit` there (the topmost unlocked shape of the visible
/// active layer): that shape's text, else a new text box at the point.
pub fn text_target_at(hit: Option<Uuid>, at: Pt) -> TextTarget {
    match hit {
        Some(id) => TextTarget::Existing(id),
        None => TextTarget::NewBox(at),
    }
}

/// What Return edits: the selection's one shape, when it is a text box or
/// has a label.
pub fn enter_target(session: &DocumentSession, selection: &[Uuid]) -> Option<Uuid> {
    let [id] = selection else {
        return None;
    };
    let shape = session.find_shape(*id)?;
    (shape.is_text_box() || shape.label.is_some()).then_some(*id)
}

/// Opens a text-editing session's gesture and adds a new, empty text box
/// with its top-left corner at `at` to the active layer inside it, so that
/// leaving it empty leaves no undo step. Returns the box's id.
pub fn start_new_text_box(
    session: &mut DocumentSession,
    gesture: &mut SessionGesture,
    at: Pt,
) -> Uuid {
    let began = gesture.begin(session, GestureOwner::Text);
    debug_assert!(began, "the gesture is free when a new text box is added");
    let shape = Shape::new_text_box(at);
    let id = shape.id;
    session.add_shapes(vec![shape]);
    id
}

/// Opens a text-editing session's gesture on shape `target` of the active
/// layer, giving a shape without a label an empty one
/// ([`Label::new_shape_label`]). Returns `false`, opening nothing, when
/// there is no such shape.
pub fn start_text_session(
    session: &mut DocumentSession,
    gesture: &mut SessionGesture,
    target: Uuid,
) -> bool {
    let Some(shape) = session.find_shape(target) else {
        return false;
    };
    let needs_label = shape.label.is_none();
    let began = gesture.begin(session, GestureOwner::Text);
    // A new text box's session opened the gesture already.
    debug_assert!(
        began || gesture.owner() == Some(GestureOwner::Text),
        "a text session's gesture is its own"
    );
    if needs_label {
        session.set_label(target, Some(Label::new_shape_label()));
    }
    true
}

/// Ends the text-editing session on `target`: an empty text box is removed
/// (a new one leaves no undo step), an empty label is cleared, and the
/// session's edits become one undo step (none when nothing changed).
/// Returns whether the target is still there.
pub fn finish_text_session(
    session: &mut DocumentSession,
    gesture: &mut SessionGesture,
    target: Uuid,
) -> bool {
    let empty = |s: &Shape| s.label.as_ref().is_none_or(|l| l.text.is_empty());
    match session.find_shape(target) {
        Some(shape) if shape.is_text_box() && empty(shape) => session.remove_shape_silently(target),
        Some(shape) if shape.label.is_some() && empty(shape) => session.set_label(target, None),
        _ => {}
    }
    gesture.end(session, GestureOwner::Text);
    session.find_shape(target).is_some()
}

/// A rectangle `2 * half` big centred on `centre`, turned by `angle`
/// degrees: where clicks edit the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub centre: Pt,
    pub half: Pt,
    pub angle: f32,
}

impl Area {
    pub fn contains(&self, (x, y): Pt) -> bool {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let (dx, dy) = (x - self.centre.0, y - self.centre.1);
        let (lx, ly) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        lx.abs() <= self.half.0 && ly.abs() <= self.half.1
    }
}

/// Where clicks edit `shape`'s text, laid out `size` big at `frame`: a
/// line label's text and its background patch, else the whole shape.
pub fn target_area(shape: &Shape, size: (f32, f32), frame: &TextFrame) -> Area {
    if shape.is_line_like_label() {
        let pad = LINE_LABEL_PADDING;
        return Area {
            centre: frame.to_canvas((size.0 / 2., size.1 / 2.)),
            half: (size.0 / 2. + pad, size.1 / 2. + pad),
            angle: frame.angle,
        };
    }
    let f = &shape.frame;
    Area {
        centre: (f.x + f.w / 2., f.y + f.h / 2.),
        half: (f.w / 2., f.h / 2.),
        angle: f.rotation,
    }
}

/// The target's text placement and click area, as last painted.
#[derive(Clone, Copy, Debug)]
struct EditGeometry {
    frame: TextFrame,
    area: Area,
}

/// A mouse selection in progress.
#[derive(Clone, Debug)]
struct TextDrag {
    origin: Range<usize>,
    unit: Unit,
}

/// The editor's on-screen state: caret blink, mouse drag and the target's
/// geometry.
#[derive(Default)]
pub struct TextEditUi {
    caret_visible: bool,
    blink: Option<Task<()>>,
    drag: Option<TextDrag>,
    geometry: Option<EditGeometry>,
}

impl EditorWindow {
    /// Readies the editor for a text-editing session: ends the one in
    /// progress, puts back a pointer gesture and ends a colour drag's step,
    /// so the session's gesture is its own.
    fn prepare_text_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_text_edit(window, cx);
        self.cancel_gesture(cx);
        self.finish_color_edit(cx);
    }

    /// Starts editing the label (or text) of shape `target` on the active
    /// layer, with the caret at its end; a shape without a label gets an
    /// empty one. The session's edits form one undo step. Ends any edit in
    /// progress first.
    ///
    /// A new text box is added inside the session's gesture (see
    /// [`start_new_text_box`]), so leaving it empty removes it without an
    /// undo step; no flag for new boxes is needed.
    pub fn begin_text_edit(&mut self, target: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_text_edit(window, cx);
        let scope = self.selection_scope();
        if !start_text_session(&mut self.session, &mut self.session_gesture, target) {
            return;
        }
        self.prune_selection(scope);
        self.selection = vec![target];
        let Some(text) = self
            .session
            .find_shape(target)
            .and_then(|s| s.label.as_ref())
            .map(|l| l.text.clone())
        else {
            // Not reached: the session gave the target a label.
            finish_text_session(&mut self.session, &mut self.session_gesture, target);
            return;
        };
        self.text_edit = Some(TextEditor::new(target, &text));
        self.focus_handle.focus(window, cx);
        self.restart_blink(window, cx);
        cx.notify();
    }

    /// Adds a new text box at canvas point `at` on the active layer and
    /// starts editing it. A hidden active layer refuses, as it refuses
    /// placing shapes. Left empty, the box goes away without an undo step.
    pub(super) fn begin_new_text_box(
        &mut self,
        at: Pt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prepare_text_edit(window, cx);
        if !self.session.active_layer_selectable() {
            window.push_notification(Notification::warning("Show the layer to draw on it."), cx);
            return;
        }
        let id = start_new_text_box(&mut self.session, &mut self.session_gesture, at);
        self.begin_text_edit(id, window, cx);
    }

    /// A click with the Text tool at canvas point `at`: edits the text of
    /// the shape there, like a double click, else adds a text box there;
    /// the tool then goes back to Pointer unless locked. A refused box
    /// (hidden layer) leaves the tool as it is.
    pub(super) fn place_text_box(&mut self, at: Pt, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_text_at(at, window, cx);
        if self.text_edit.is_some() {
            self.tools.after_place();
        }
        cx.notify();
    }

    /// A double click or Text-tool click at canvas point `at`: edits the
    /// text of the shape there (the caret at the click when it lands on
    /// the text), else adds a new text box there.
    pub(super) fn edit_text_at(&mut self, at: Pt, window: &mut Window, cx: &mut Context<Self>) {
        // A move begun by the first click whose mouse-up was missed is put
        // back before hit-testing.
        self.cancel_gesture(cx);
        let hit = self.shape_at(at, cx);
        match text_target_at(hit, at) {
            TextTarget::NewBox(at) => self.begin_new_text_box(at, window, cx),
            TextTarget::Existing(id) => {
                self.begin_text_edit(id, window, cx);
                if self.text_edit.is_some() {
                    self.text_press(at, 1, false, window, cx);
                    // A press, not a drag: the mouse-up may come anywhere.
                    self.end_text_drag();
                }
            }
        }
    }

    /// Return: edits the one selected text box or labelled shape, with all
    /// its text selected.
    pub(super) fn edit_selected_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = enter_target(&self.session, &self.selection) else {
            return;
        };
        self.begin_text_edit(id, window, cx);
        self.text_command(TextCommand::SelectAll, window, cx);
    }

    /// Stops editing text: an empty text box is removed and an emptied
    /// label cleared, the session's edits become one undo step (none when
    /// nothing changed) and the keys go back to the shapes. The target, if
    /// still there, becomes the selection.
    pub fn end_text_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A composition in progress is kept, as its own undo step.
        if self.text_edit.as_ref().is_some_and(|e| e.marked.is_some()) {
            self.with_text(window, cx, |ed, text, _| ed.unmark(text));
        }
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        self.text_ui = TextEditUi::default();
        let scope = self.selection_scope();
        let kept = finish_text_session(&mut self.session, &mut self.session_gesture, edit.target);
        if kept {
            self.selection = vec![edit.target];
        }
        self.prune_selection(scope);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Runs `f` on the editor and a copy of the target's text with its
    /// current layout, then stores the text back in the target when it
    /// changed. `None` when not editing or the target is gone.
    pub(super) fn with_text<R>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut TextEditor, &mut RichText, Option<&TextLayout>) -> R,
    ) -> Option<R> {
        let edit = self.text_edit.as_mut()?;
        let target = edit.target;
        let shape = self.session.find_shape(target)?;
        let wrap = text_wrap(shape);
        let mut text = shape.label.as_ref()?.text.clone();
        edit.clamp(&text);
        let layout = self.text_layouts.cached(target, &text, wrap);
        let result = f(edit, &mut text, layout);
        let changed = self
            .session
            .find_shape(target)
            .and_then(|s| s.label.as_ref())
            .is_some_and(|l| l.text != text);
        if changed {
            self.update_session(
                |s| {
                    s.update_shapes(&[target], |shape| {
                        if let Some(label) = shape.label.as_mut() {
                            label.text = text.clone();
                        }
                    })
                },
                cx,
            );
        }
        self.restart_blink(window, cx);
        cx.notify();
        Some(result)
    }

    /// Runs a text key's command while editing.
    pub(super) fn text_command(
        &mut self,
        command: TextCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text_edit.is_none() {
            return;
        }
        match command {
            TextCommand::Exit => self.end_text_edit(window, cx),
            TextCommand::Copy | TextCommand::Cut => {
                let cut = command == TextCommand::Cut;
                let copied = self
                    .with_text(
                        window,
                        cx,
                        |ed, text, _| {
                            if cut {
                                ed.cut(text)
                            } else {
                                ed.copy(text)
                            }
                        },
                    )
                    .flatten();
                if let Some((plain, payload)) = copied {
                    cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(plain, payload));
                }
            }
            TextCommand::Paste => {
                let Some(item) = cx.read_from_clipboard() else {
                    return;
                };
                let Some(content) =
                    pasted(item.metadata().map(String::as_str), item.text().as_deref())
                else {
                    return;
                };
                self.with_text(window, cx, |ed, text, _| match &content {
                    Pasted::Rich(rich) => ed.paste_rich(text, rich),
                    Pasted::Plain(plain) => ed.paste_plain(text, plain),
                });
            }
            _ => {
                self.with_text(window, cx, |ed, text, layout| {
                    ed.command(text, layout.map(|l| l as &dyn Lines), command)
                });
            }
        }
    }

    /// Shows the caret and restarts its blink, so it stays on while typing.
    /// While the editor does not have focus (a Properties field has it)
    /// the caret is hidden and the blink, with its repaints, stops; it
    /// starts again when the editor gets focus back.
    pub(super) fn restart_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.text_ui.caret_visible = true;
        self.text_ui.blink = Some(cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(CARET_BLINK).await;
            let blinking = this
                .update_in(cx, |this, window, cx| {
                    if this.text_edit.is_none() {
                        return false;
                    }
                    let focused = this.focus_handle.is_focused(window);
                    this.text_ui.caret_visible = focused && !this.text_ui.caret_visible;
                    cx.notify();
                    focused
                })
                .unwrap_or(false);
            if !blinking {
                break;
            }
        }));
    }

    /// The target's caret position under canvas point `at`.
    fn text_offset_at(&mut self, at: Pt) -> Option<usize> {
        let (geometry, layout) = self.edit_layout()?;
        Some(layout.offset_at(geometry.frame.to_local(at)))
    }

    /// The placement and click area of the text being edited, from its
    /// current layout, with that layout (`None` before it is laid out).
    fn edit_layout(&mut self) -> Option<(EditGeometry, &TextLayout)> {
        let edit = self.text_edit.as_ref()?;
        let shape = self.session.find_shape(edit.target)?;
        let text = &shape.label.as_ref()?.text;
        let layout = self
            .text_layouts
            .cached(edit.target, text, text_wrap(shape))?;
        let size = layout.size();
        let frame = text_frame(shape, size);
        let area = target_area(shape, size, &frame);
        Some((EditGeometry { frame, area }, layout))
    }

    /// Whether canvas point `at` is on the text being edited.
    pub(super) fn over_edited_text(&self, at: Pt) -> bool {
        self.text_edit.is_some() && self.text_ui.geometry.is_some_and(|g| g.area.contains(at))
    }

    /// A left press at canvas point `at` while editing. On the target it
    /// places the caret (Shift: extends the selection), or with a double
    /// or triple click selects a word or paragraph, and starts a drag
    /// selection; returns `false` elsewhere.
    pub(super) fn text_press(
        &mut self,
        at: Pt,
        clicks: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self
            .edit_layout()
            .map(|(g, layout)| (g.area.contains(at), layout.offset_at(g.frame.to_local(at))));
        let painted = self.over_edited_text(at);
        let Some(offset) = press_hit(current, painted) else {
            return false;
        };
        let unit = match clicks {
            0 | 1 => Unit::Grapheme,
            2 => Unit::Word,
            _ => Unit::Paragraph,
        };
        self.text_ui.drag = self.with_text(window, cx, |ed, text, _| {
            let offset = offset.unwrap_or(ed.caret);
            let origin = match unit {
                Unit::Grapheme if shift => ed.anchor..ed.anchor,
                Unit::Grapheme => offset..offset,
                Unit::Word => text.word_range(offset),
                Unit::Paragraph => text.paragraph_range(offset),
            };
            ed.drag_select(text, &origin, unit, offset);
            TextDrag { origin, unit }
        });
        true
    }

    /// Continues a drag selection to canvas point `at`; `false` when none
    /// is in progress.
    pub(super) fn text_drag_to(
        &mut self,
        at: Pt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(drag) = self.text_ui.drag.clone() else {
            return false;
        };
        if let Some(offset) = self.text_offset_at(at) {
            self.with_text(window, cx, |ed, text, _| {
                ed.drag_select(text, &drag.origin, drag.unit, offset)
            });
        }
        true
    }

    /// Ends a drag selection; `false` when none was in progress.
    pub(super) fn end_text_drag(&mut self) -> bool {
        self.text_ui.drag.take().is_some()
    }

    /// Paints the selection, the text being composed and the caret of the
    /// text being edited, turned with it, and remembers where it is.
    pub(super) fn paint_text_edit(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.text_edit.as_ref() else {
            return;
        };
        let theme = cx.theme();
        let (selection_color, caret_color) = (theme.selection, theme.caret);
        let canvas = canvas_background(
            self.session.page().canvas.background.as_deref(),
            theme.background,
        );
        let Some(shape) = self.session.find_shape(edit.target) else {
            return;
        };
        let Some(label) = &shape.label else {
            return;
        };
        let wrap = text_wrap(shape);
        let color = text_color(shape, canvas);
        let size = self
            .text_layouts
            .size(window, shape.id, &label.text, wrap, color);
        let frame = text_frame(shape, size);
        self.text_ui.geometry = Some(EditGeometry {
            frame,
            area: target_area(shape, size, &frame),
        });
        let Some(layout) = self.text_layouts.cached(shape.id, &label.text, wrap) else {
            return;
        };
        let vp = self.session.page().viewport;
        let zoom = vp.zoom;
        let paint = |window: &mut Window, r: Rect, color: Hsla| {
            let corners = [
                (r.x, r.y),
                (r.x + r.w, r.y),
                (r.x + r.w, r.y + r.h),
                (r.x, r.y + r.h),
            ]
            .map(|p| {
                let (x, y) = vp.canvas_to_screen(frame.to_canvas(p));
                point(bounds.origin.x + px(x), bounds.origin.y + px(y))
            });
            if frame.angle.rem_euclid(360.) == 0. {
                window.paint_quad(fill(Bounds::from_corners(corners[0], corners[2]), color));
            } else {
                paint_polygon(window, &corners, color);
            }
        };
        let sel = edit.selection();
        if !sel.is_empty() {
            for r in layout.selection_rects(sel.clone()) {
                paint(window, r, selection_color);
            }
        }
        if let Some(marked) = edit.marked.clone() {
            let thickness = MARKED_UNDERLINE_PX / zoom;
            for r in layout.selection_rects(marked) {
                let bar = Rect {
                    y: r.y + r.h - thickness,
                    h: thickness,
                    ..r
                };
                paint(window, bar, color);
            }
        }
        if sel.is_empty() && self.text_ui.caret_visible && self.focus_handle.is_focused(window) {
            let r = layout.caret_rect(edit.caret);
            let w = CARET_PX / zoom;
            let caret = Rect {
                x: r.x - w / 2.,
                w,
                ..r
            };
            paint(window, caret, caret_color);
        }
    }

    /// The plain text of the target being edited.
    fn edited_plain(&self) -> Option<String> {
        let edit = self.text_edit.as_ref()?;
        let shape = self.session.find_shape(edit.target)?;
        Some(shape.label.as_ref()?.text.plain())
    }

    /// `range` (UTF-16, of the edited text) in bytes.
    fn bytes_of(&self, range: Range<usize>) -> Option<Range<usize>> {
        let plain = self.edited_plain()?;
        Some(utf16_to_byte(&plain, range.start)..utf16_to_byte(&plain, range.end))
    }
}

/// Registers the text editor's key handlers on the editor root.
pub(super) fn on_text_actions(root: Div, cx: &mut Context<EditorWindow>) -> Div {
    use Motion as M;
    use TextCommand as C;
    macro_rules! bind {
        ($root:ident, $($action:ty => $command:expr),* $(,)?) => {{
            $(
                let $root = $root.on_action(cx.listener(|this, _: &$action, window, cx| {
                    this.text_command($command, window, cx)
                }));
            )*
            $root
        }};
    }
    bind!(root,
        TextLeft => C::Move(M::Left, false),
        TextRight => C::Move(M::Right, false),
        TextUp => C::Move(M::Up, false),
        TextDown => C::Move(M::Down, false),
        TextWordLeft => C::Move(M::WordLeft, false),
        TextWordRight => C::Move(M::WordRight, false),
        TextLineStart => C::Move(M::LineStart, false),
        TextLineEnd => C::Move(M::LineEnd, false),
        TextDocStart => C::Move(M::DocStart, false),
        TextDocEnd => C::Move(M::DocEnd, false),
        TextSelectLeft => C::Move(M::Left, true),
        TextSelectRight => C::Move(M::Right, true),
        TextSelectUp => C::Move(M::Up, true),
        TextSelectDown => C::Move(M::Down, true),
        TextSelectWordLeft => C::Move(M::WordLeft, true),
        TextSelectWordRight => C::Move(M::WordRight, true),
        TextSelectLineStart => C::Move(M::LineStart, true),
        TextSelectLineEnd => C::Move(M::LineEnd, true),
        TextSelectDocStart => C::Move(M::DocStart, true),
        TextSelectDocEnd => C::Move(M::DocEnd, true),
        TextBackspace => C::Backspace(false),
        TextDelete => C::Delete(false),
        TextBackspaceWord => C::Backspace(true),
        TextDeleteWord => C::Delete(true),
        TextNewParagraph => C::NewParagraph,
        TextLineBreak => C::LineBreak,
        TextSelectAll => C::SelectAll,
        TextBold => C::Toggle(Toggle::Bold),
        TextItalic => C::Toggle(Toggle::Italic),
        TextUnderline => C::Toggle(Toggle::Underline),
        TextStrike => C::Toggle(Toggle::Strike),
        TextCopy => C::Copy,
        TextCut => C::Cut,
        TextPaste => C::Paste,
        TextUndo => C::Undo,
        TextRedo => C::Redo,
        TextExit => C::Exit,
    )
}

/// Platform text input (typing, dead keys, input methods) for the text
/// being edited. gpui speaks UTF-16 offsets; the editor bytes.
impl EntityInputHandler for EditorWindow {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let plain = self.edited_plain()?;
        let range =
            utf16_to_byte(&plain, range_utf16.start)..utf16_to_byte(&plain, range_utf16.end);
        *adjusted_range =
            Some(byte_to_utf16(&plain, range.start)..byte_to_utf16(&plain, range.end));
        plain.get(range).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let plain = self.edited_plain()?;
        let edit = self.text_edit.as_ref()?;
        let sel = edit.selection();
        Some(UTF16Selection {
            range: byte_to_utf16(&plain, sel.start)..byte_to_utf16(&plain, sel.end),
            reversed: edit.caret < edit.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let marked = self.text_edit.as_ref()?.marked.clone()?;
        let plain = self.edited_plain()?;
        Some(byte_to_utf16(&plain, marked.start)..byte_to_utf16(&plain, marked.end))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.with_text(window, cx, |ed, text, _| ed.unmark(text));
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.and_then(|r| self.bytes_of(r));
        self.with_text(window, cx, |ed, rich, _| ed.replace_text(rich, range, text));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.and_then(|r| self.bytes_of(r));
        let selected = new_selected_range
            .map(|r| utf16_to_byte(new_text, r.start)..utf16_to_byte(new_text, r.end));
        self.with_text(window, cx, |ed, rich, _| {
            ed.replace_and_mark(rich, range, new_text, selected)
        });
    }

    /// The window bounds of the first line of `range_utf16` (or of the
    /// caret at its start), around the turned box.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.bytes_of(range_utf16)?;
        let vp = self.session.page().viewport;
        let (geometry, layout) = self.edit_layout()?;
        let r = layout
            .selection_rects(range.clone())
            .into_iter()
            .next()
            .unwrap_or_else(|| layout.caret_rect(range.start));
        let corners = [
            (r.x, r.y),
            (r.x + r.w, r.y),
            (r.x + r.w, r.y + r.h),
            (r.x, r.y + r.h),
        ]
        .map(|p| vp.canvas_to_screen(geometry.frame.to_canvas(p)));
        let (mut lo, mut hi) = (corners[0], corners[0]);
        for (x, y) in corners {
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let origin = element_bounds.origin;
        Some(Bounds::from_corners(
            point(origin.x + px(lo.0), origin.y + px(lo.1)),
            point(origin.x + px(hi.0), origin.y + px(hi.1)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.canvas_bounds?;
        let local = point - bounds.origin;
        let at = self
            .session
            .page()
            .viewport
            .screen_to_canvas((f32::from(local.x), f32::from(local.y)));
        let offset = self.text_offset_at(at)?;
        let plain = self.edited_plain()?;
        Some(byte_to_utf16(&plain, offset))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        byte_to_utf16, enter_target, finish_text_session, parse_text_payload, pasted, press_hit,
        start_new_text_box, start_text_session, target_area, text_payload, text_target_at,
        utf16_to_byte, Lines, Motion, Pasted, TextEditor, TextTarget, Toggle, Unit,
    };
    use crate::editor::interaction::hit_shape;
    use crate::editor::{GestureOwner, SessionGesture};
    use mindweave_core::geometry::Frame;
    use mindweave_core::{
        builtin_libraries, DocumentSession, HAlign, Label, LibraryRegistry, RichText, RunStyle,
        Shape, StyleChange, MAX_PASTE_CHARS,
    };
    use std::ops::Range;
    use uuid::Uuid;

    fn plain(s: &str) -> RichText {
        let mut t = RichText::new(RunStyle::default(), HAlign::Left);
        t.insert(0, s, &RunStyle::default());
        t
    }

    fn editor(text: &RichText) -> TextEditor {
        TextEditor::new(Uuid::new_v4(), text)
    }

    #[test]
    fn utf16_byte_conversion_roundtrip() {
        // 'a' 1 byte / 1 unit, '😀' 4 / 2, 'é' 2 / 1, '日' 3 / 1, line break 3 / 1.
        let s = "a😀é日\u{2028}x";
        let pairs = [(0, 0), (1, 1), (5, 3), (7, 4), (10, 5), (13, 6), (14, 7)];
        for (byte, unit) in pairs {
            assert_eq!(byte_to_utf16(s, byte), unit, "byte {byte}");
            assert_eq!(utf16_to_byte(s, unit), byte, "unit {unit}");
        }
        for (i, _) in s.char_indices() {
            assert_eq!(utf16_to_byte(s, byte_to_utf16(s, i)), i);
        }
        // Inside the emoji's surrogate pair: its start.
        assert_eq!(utf16_to_byte(s, 2), 1);
        // Inside a character's bytes: its start.
        assert_eq!(byte_to_utf16(s, 3), 1);
        // Past the end: the end.
        assert_eq!(utf16_to_byte(s, 100), s.len());
        assert_eq!(byte_to_utf16(s, 100), 7);
        assert_eq!(utf16_to_byte("", 3), 0);
    }

    #[test]
    fn ime_compose_commit_and_cancel() {
        // Compose, then commit: one undo step that removes the word.
        let mut text = plain("ab ");
        let mut ed = editor(&text);
        ed.replace_and_mark(&mut text, None, "に", None);
        assert_eq!(text.plain(), "ab に");
        assert_eq!(ed.marked, Some(3..6));
        assert_eq!(ed.caret, 6);
        ed.replace_and_mark(&mut text, None, "にほん", Some(3..6));
        assert_eq!(text.plain(), "ab にほん");
        assert_eq!(ed.marked, Some(3..12));
        assert_eq!((ed.anchor, ed.caret), (6, 9));
        ed.replace_text(&mut text, None, "日本");
        assert_eq!(text.plain(), "ab 日本");
        assert_eq!(ed.marked, None);
        assert_eq!((ed.anchor, ed.caret), (9, 9));
        assert!(ed.undo(&mut text));
        assert_eq!(text.plain(), "ab ");
        assert_eq!(ed.caret, 3);
        assert!(!ed.undo(&mut text), "the commit is a single step");
        assert!(ed.redo(&mut text));
        assert_eq!(text.plain(), "ab 日本");

        // Compose, then cancel (empty marked text, then unmark): no change
        // and no step.
        let mut text = plain("ab ");
        let mut ed = editor(&text);
        ed.replace_and_mark(&mut text, None, "に", None);
        ed.replace_and_mark(&mut text, None, "", None);
        assert_eq!(ed.marked, None);
        ed.unmark(&mut text);
        assert_eq!(text.plain(), "ab ");
        assert_eq!(ed.caret, 3);
        assert!(!ed.undo(&mut text));

        // A dead key: the accent is marked, then replaced by the letter,
        // joining the word being typed.
        let mut text = plain("");
        let mut ed = editor(&text);
        ed.type_text(&mut text, "c");
        ed.type_text(&mut text, "a");
        ed.type_text(&mut text, "f");
        ed.replace_and_mark(&mut text, None, "´", None);
        ed.replace_text(&mut text, None, "é");
        assert_eq!(text.plain(), "café");
        assert!(ed.undo(&mut text));
        assert_eq!(text.plain(), "");

        // Unmark commits what is marked.
        let mut text = plain("");
        let mut ed = editor(&text);
        ed.replace_and_mark(&mut text, None, "日", None);
        ed.unmark(&mut text);
        assert_eq!((text.plain().as_str(), ed.marked.clone()), ("日", None));
        assert!(ed.undo(&mut text));
        assert_eq!(text.plain(), "");
    }

    #[test]
    fn toggle_without_selection_sets_typing_style() {
        let mut text = plain("ab");
        let mut ed = editor(&text);
        let before = text.clone();
        ed.toggle(&mut text, Toggle::Bold);
        assert_eq!(text, before, "no text changes");
        assert!(ed.typing_style.as_ref().is_some_and(|s| s.bold));
        ed.type_text(&mut text, "c");
        assert!(text.style_at(3).bold);
        assert!(!text.style_at(2).bold);
        // The typed text carries the style now.
        assert_eq!(ed.typing_style, None);
        ed.type_text(&mut text, "d");
        assert!(text.style_at(4).bold);
        // Toggling twice with nothing selected leaves the style as it was.
        ed.toggle(&mut text, Toggle::Italic);
        ed.toggle(&mut text, Toggle::Italic);
        assert!(ed
            .typing_style
            .as_ref()
            .is_some_and(|s| !s.italic && s.bold));
        // Moving the caret forgets the typing style.
        ed.toggle(&mut text, Toggle::Underline);
        ed.select(0, 0);
        assert_eq!(ed.typing_style, None);
        // With a selection the toggle applies to it, as one undo step.
        ed.select(2, 0);
        ed.toggle(&mut text, Toggle::Strike);
        assert!(text.style_at(1).strike && text.style_at(2).strike);
        assert!(!text.style_at(3).strike);
        assert!(ed.undo(&mut text));
        assert!(!text.style_at(1).strike);
    }

    #[test]
    fn sidebar_styles_apply_to_the_selection_or_the_typing_style() {
        let mut text = plain("abc");
        let mut ed = editor(&text);
        // A caret in text: the typing style only, no text change.
        let before = text.clone();
        ed.apply_style(&mut text, &StyleChange::Size(30.), false);
        assert_eq!(text, before);
        assert_eq!(ed.typing_style.as_ref().map(|s| s.size), Some(30.));
        ed.apply_style(&mut text, &StyleChange::Bold(true), false);
        let typing = ed.typing_style.clone().unwrap();
        assert!(typing.bold && typing.size == 30.);
        ed.type_text(&mut text, "d");
        assert_eq!(text.style_at(4).size, 30.);

        // A selection: one undo step per change.
        let mut text = plain("abc");
        let mut ed = editor(&text);
        ed.select(2, 0);
        ed.apply_style(&mut text, &StyleChange::Font(Some("Georgia".into())), false);
        assert_eq!(text.style_at(1).font.as_deref(), Some("Georgia"));
        assert_eq!(text.style_at(3).font, None);
        ed.apply_style(&mut text, &StyleChange::Size(20.), false);
        assert!(ed.undo(&mut text));
        assert_eq!(text.style_at(1).size, RunStyle::default().size);
        assert_eq!(text.style_at(1).font.as_deref(), Some("Georgia"));
        assert!(ed.undo(&mut text));
        assert_eq!(text.style_at(1).font, None);

        // Colour previews from a drag are one undo step until it ends.
        let mut text = plain("abc");
        let mut ed = editor(&text);
        ed.select(3, 0);
        for hex in ["#110000", "#220000", "#330000"] {
            ed.apply_style(&mut text, &StyleChange::Color(Some(hex.into())), true);
        }
        ed.end_style_preview();
        ed.apply_style(&mut text, &StyleChange::Color(Some("#440000".into())), true);
        assert_eq!(text.style_at(1).color.as_deref(), Some("#440000"));
        assert!(ed.undo(&mut text));
        assert_eq!(text.style_at(1).color.as_deref(), Some("#330000"));
        assert!(ed.undo(&mut text));
        assert_eq!(text.style_at(1).color, None);
        assert!(!ed.undo(&mut text));
    }

    #[test]
    fn a_caret_in_an_empty_paragraph_keeps_the_sidebar_style() {
        // An empty new box: the style stays with the paragraph, so a click
        // into the box before typing does not lose it.
        let mut text = plain("");
        let mut ed = editor(&text);
        ed.apply_style(&mut text, &StyleChange::Size(40.), false);
        ed.select(0, 0);
        assert_eq!(ed.typing_style, None);
        ed.type_text(&mut text, "x");
        assert_eq!(text.style_at(1).size, 40.);
    }

    #[test]
    fn sidebar_alignment_applies_to_the_touched_paragraphs() {
        let mut text = plain("one");
        text.insert(3, "\ntwo", &RunStyle::default());
        let mut ed = editor(&text);
        ed.select(1, 1);
        ed.set_align(&mut text, HAlign::Center);
        assert_eq!(text.paragraphs[0].align, HAlign::Center);
        assert_eq!(text.paragraphs[1].align, HAlign::Left);
        ed.select(5, 0);
        ed.set_align(&mut text, HAlign::Right);
        assert!(text.paragraphs.iter().all(|p| p.align == HAlign::Right));
        assert!(ed.undo(&mut text));
        assert_eq!(text.paragraphs[1].align, HAlign::Left);
    }

    #[test]
    fn rich_copy_paste_keeps_styles() {
        let mut text = plain("plain ");
        let bold = RunStyle {
            bold: true,
            ..RunStyle::default()
        };
        text.insert(6, "bold", &bold);
        text.insert(10, "\u{2028}next", &RunStyle::default());
        let mut ed = editor(&text);
        ed.select(14, 4);
        let (plain_text, payload) = ed.copy(&text).unwrap();
        // Line breaks are plain newlines for other applications.
        assert_eq!(plain_text, "n bold\nn");
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["mindweave"], "text");
        assert_eq!(value["version"], 1);
        let rich = parse_text_payload(&payload).unwrap();
        assert_eq!(rich, text.slice(4..14));

        let mut other = plain("x");
        let mut ed2 = editor(&other);
        ed2.paste_rich(&mut other, &rich);
        assert_eq!(other.plain(), "xn bold\u{2028}n");
        assert!(!other.style_at(3).bold);
        assert!(other.style_at(4).bold && other.style_at(7).bold);
        assert_eq!(ed2.caret, other.len());
        assert!(ed2.undo(&mut other));
        assert_eq!(other.plain(), "x");

        // Nothing selected: nothing to copy.
        ed.select(3, 3);
        assert_eq!(ed.copy(&text), None);
        // Other payloads are not rich text.
        assert_eq!(parse_text_payload("{}"), None);
        assert_eq!(parse_text_payload("hello"), None);
        assert_eq!(
            parse_text_payload(r#"{"mindweave":"shapes","version":2,"shapes":[]}"#),
            None
        );
        let future = payload.replace("\"version\":1", "\"version\":2");
        assert_eq!(parse_text_payload(&future), None);
        // A selection replaced by a paste, styles kept.
        let mut t = plain("abc");
        t.apply(0..3, &StyleChange::Italic(true));
        let mut ed3 = editor(&t);
        ed3.select(3, 0);
        ed3.paste_rich(&mut t, &rich);
        assert_eq!(t.plain(), "n bold\u{2028}n");
        assert!(!t.style_at(1).italic);
    }

    #[test]
    fn plain_paste_capped() {
        let mut text = plain("");
        let mut ed = editor(&text);
        let long = "x".repeat(MAX_PASTE_CHARS + 10);
        ed.paste_plain(&mut text, &long);
        assert_eq!(text.len(), MAX_PASTE_CHARS);
        assert_eq!(ed.caret, MAX_PASTE_CHARS);
        assert!(ed.undo(&mut text));
        assert!(text.is_empty());

        // Graphemes are counted, not bytes: an emoji family is one.
        let family = "👨‍👩‍👧";
        let many = family.repeat(MAX_PASTE_CHARS + 1);
        let mut text = plain("");
        let mut ed = editor(&text);
        ed.paste_plain(&mut text, &many);
        assert_eq!(text.len(), family.len() * MAX_PASTE_CHARS);

        // Newlines start paragraphs; the paste replaces the selection.
        let mut text = plain("abc");
        let mut ed = editor(&text);
        ed.select(2, 1);
        ed.paste_plain(&mut text, "1\r\n2");
        assert_eq!(text.plain(), "a1\n2c");
        assert_eq!(text.paragraphs.len(), 2);
        assert_eq!(ed.caret, 4);
        // Empty text pastes nothing and records nothing.
        let mut text = plain("abc");
        let mut ed = editor(&text);
        ed.paste_plain(&mut text, "");
        assert!(!ed.undo(&mut text));
    }

    #[test]
    fn paste_prefers_rich_text_and_skips_copied_shapes() {
        let rich = plain("hi");
        let payload = text_payload(&rich);
        assert_eq!(pasted(Some(&payload), Some("hi")), Some(Pasted::Rich(rich)));
        assert_eq!(
            pasted(Some("not a payload"), Some("x")),
            Some(Pasted::Plain("x".into()))
        );
        assert_eq!(pasted(None, Some("x")), Some(Pasted::Plain("x".into())));
        let shapes = mindweave_core::to_clipboard(&[Shape::new_text_box((0., 0.))]);
        assert_eq!(pasted(None, Some(&shapes)), None);
        assert_eq!(pasted(None, None), None);
    }

    #[test]
    fn mouse_selection_by_grapheme_word_and_paragraph() {
        let text = plain("one two\nthree");
        let mut ed = editor(&text);
        // Drag left from inside "two".
        ed.drag_select(&text, &(5..5), Unit::Grapheme, 1);
        assert_eq!((ed.anchor, ed.caret), (5, 1));
        ed.drag_select(&text, &(5..5), Unit::Grapheme, 6);
        assert_eq!((ed.anchor, ed.caret), (5, 6));
        // Double-click drag: whole words.
        let word = text.word_range(5);
        assert_eq!(word, 4..7);
        ed.drag_select(&text, &word, Unit::Word, 1);
        assert_eq!((ed.anchor, ed.caret), (7, 0));
        ed.drag_select(&text, &word, Unit::Word, 5);
        assert_eq!((ed.anchor, ed.caret), (4, 7));
        // Triple-click drag: whole paragraphs.
        let para = text.paragraph_range(2);
        ed.drag_select(&text, &para, Unit::Paragraph, 10);
        assert_eq!((ed.anchor, ed.caret), (0, 13));
    }

    #[test]
    fn clicks_hit_the_turned_target() {
        use super::super::text_paint::text_frame;
        let mut shape = Shape::new_text_box((100., 50.));
        shape.frame.w = 40.;
        shape.frame.h = 20.;
        shape.frame.rotation = 90.;
        let frame = text_frame(&shape, (32., 12.));
        let area = target_area(&shape, (32., 12.), &frame);
        // Centre (120, 60); turned a quarter, the box stands 20 wide, 40 tall.
        assert!(area.contains((120., 60.)));
        assert!(area.contains((125., 78.)));
        assert!(!area.contains((138., 60.)));
        // The text's top-left corner, inside the padding, maps both ways.
        let corner = frame.to_canvas((0., 0.));
        assert!((corner.0 - 126.).abs() < 1e-3 && (corner.1 - 44.).abs() < 1e-3);
        let back = frame.to_local(corner);
        assert!(back.0.abs() < 1e-3 && back.1.abs() < 1e-3);
    }

    #[test]
    fn rich_paste_capped() {
        let rich = plain(&"x".repeat(MAX_PASTE_CHARS + 5));
        let mut text = plain("");
        let mut ed = editor(&text);
        ed.paste_rich(&mut text, &rich);
        assert_eq!(text.len(), MAX_PASTE_CHARS);
        assert_eq!(ed.caret, MAX_PASTE_CHARS);
    }

    #[test]
    fn marked_text_follows_grapheme_snapping() {
        // "e" + combining acute (2 bytes) + "x": a range starting inside
        // the grapheme is widened to it by the text model.
        let mut text = plain("e\u{301}x");
        let mut ed = editor(&text);
        ed.replace_and_mark(&mut text, Some(1..3), "に", None);
        assert_eq!(text.plain(), "にx");
        assert_eq!(ed.marked, Some(0..3));
        assert_eq!(ed.caret, 3);
    }

    /// Lines of a monospace text, one unit per byte: the lines of
    /// `text.plain()` split at paragraph breaks.
    struct Mono(Vec<Range<usize>>);

    impl Mono {
        fn of(text: &RichText) -> Self {
            let plain = text.plain();
            let mut start = 0;
            let mut lines = Vec::new();
            for part in plain.split('\n') {
                lines.push(start..start + part.len());
                start += part.len() + 1;
            }
            Mono(lines)
        }

        fn index(&self, offset: usize) -> usize {
            self.0.iter().rposition(|l| l.start <= offset).unwrap_or(0)
        }
    }

    impl Lines for Mono {
        fn caret_x(&self, offset: usize) -> f32 {
            (offset - self.0[self.index(offset)].start) as f32
        }

        fn vertical(&self, offset: usize, x: f32, down: bool) -> Option<usize> {
            let i = self.index(offset);
            let j = if down { i + 1 } else { i.checked_sub(1)? };
            let line = self.0.get(j)?;
            Some((line.start + x as usize).min(line.end))
        }

        fn line_range(&self, offset: usize) -> Option<Range<usize>> {
            self.0.get(self.index(offset)).cloned()
        }
    }

    #[test]
    fn up_and_down_keep_the_goal_column() {
        let text = plain("abcdef\nab\nabcdef");
        let lines = Mono::of(&text);
        let mut ed = editor(&text);
        ed.select(5, 5);
        let go = |ed: &mut TextEditor, m: Motion, extend: bool| {
            ed.motion(&text, Some(&lines), m, extend);
            (ed.anchor, ed.caret)
        };
        // Through a short line and back out at the same column.
        assert_eq!(go(&mut ed, Motion::Down, false), (9, 9));
        assert_eq!(go(&mut ed, Motion::Down, false), (15, 15));
        assert_eq!(go(&mut ed, Motion::Up, false), (9, 9));
        assert_eq!(go(&mut ed, Motion::Up, false), (5, 5));
        // Past the first line: the start.
        assert_eq!(go(&mut ed, Motion::Up, false), (0, 0));
        // A sideways move sets a new column.
        ed.select(9, 9);
        assert_eq!(go(&mut ed, Motion::Left, false), (8, 8));
        assert_eq!(go(&mut ed, Motion::Down, false), (11, 11));
        // Shift extends from the anchor.
        assert_eq!(go(&mut ed, Motion::Up, true), (11, 8));
    }

    #[test]
    fn moves_with_a_selection_start_from_its_ends() {
        let text = plain("ab\ncd\nef");
        let lines = Mono::of(&text);
        let mut ed = editor(&text);
        let mut from_selection = |m: Motion, layout: bool| {
            ed.select(4, 1);
            let lines: Option<&dyn Lines> = if layout { Some(&lines) } else { None };
            ed.motion(&text, lines, m, false);
            ed.caret
        };
        for layout in [true, false] {
            // Cmd+Left and Cmd+Up: from the selection's start.
            assert_eq!(from_selection(Motion::LineStart, layout), 0);
            assert_eq!(from_selection(Motion::DocStart, layout), 0);
            // Cmd+Right: from its end.
            assert_eq!(from_selection(Motion::LineEnd, layout), 5);
            assert_eq!(from_selection(Motion::Left, layout), 1);
            assert_eq!(from_selection(Motion::Right, layout), 4);
        }
        // Up from a selection starts at its start (line 1, column 1).
        assert_eq!(from_selection(Motion::Up, true), 0);
        assert_eq!(from_selection(Motion::Down, true), 7);
    }

    #[test]
    fn word_moves() {
        let text = plain("one two, three");
        let mut ed = editor(&text);
        let go = |ed: &mut TextEditor, m: Motion, extend: bool| {
            ed.motion(&text, None, m, extend);
            (ed.anchor, ed.caret)
        };
        assert_eq!(go(&mut ed, Motion::WordLeft, false), (9, 9));
        assert_eq!(go(&mut ed, Motion::WordLeft, false), (4, 4));
        assert_eq!(go(&mut ed, Motion::WordRight, false), (7, 7));
        assert_eq!(go(&mut ed, Motion::WordRight, true), (7, 14));
        // A selection collapses towards the move, then moves a word.
        assert_eq!(go(&mut ed, Motion::WordLeft, false), (4, 4));
    }

    fn session() -> (tempfile::TempDir, DocumentSession) {
        let dir = tempfile::tempdir().unwrap();
        let session = DocumentSession::create(dir.path().join("t.mindweave")).unwrap();
        (dir, session)
    }

    fn rectangle(libs: &LibraryRegistry, at: (f32, f32)) -> Shape {
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        Shape::new(
            def,
            "basic-shapes",
            Frame {
                x: at.0,
                y: at.1,
                w: 100.,
                h: 50.,
                ..Frame::default()
            },
        )
    }

    #[test]
    fn text_target_at_resolution() {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let (_dir, mut s) = session();
        let target =
            |s: &DocumentSession, at: (f32, f32)| text_target_at(hit_shape(s, &libs, at, 1.), at);
        // Empty canvas: a new box at the point.
        assert_eq!(target(&s, (500., 500.)), TextTarget::NewBox((500., 500.)));
        // A shape: its label.
        let shape = rectangle(&libs, (0., 0.));
        let shape_id = shape.id;
        let mut text_box = Shape::new_text_box((200., 0.));
        text_box.frame.w = 60.;
        let box_id = text_box.id;
        s.add_shapes(vec![shape, text_box]);
        assert_eq!(target(&s, (50., 25.)), TextTarget::Existing(shape_id));
        // A text box: itself.
        assert_eq!(target(&s, (230., 10.)), TextTarget::Existing(box_id));
        // A line: its label.
        let def = libs.shape("basic-shapes", "line").unwrap();
        let line = Shape::new(
            def,
            "basic-shapes",
            Frame {
                x: 0.,
                y: 300.,
                w: 100.,
                h: 0.,
                ..Frame::default()
            },
        );
        let line_id = line.id;
        s.add_shapes(vec![line]);
        assert_eq!(target(&s, (50., 300.)), TextTarget::Existing(line_id));
        // Locked: a new box.
        s.update_shapes(&[shape_id], |sh| sh.locked = true);
        assert_eq!(target(&s, (50., 25.)), TextTarget::NewBox((50., 25.)));
        // On another layer: a new box.
        s.add_layer();
        assert_eq!(target(&s, (230., 10.)), TextTarget::NewBox((230., 10.)));
        // Hidden active layer: a new box (which adding then refuses).
        let lower = s.page().layers.len() - 1;
        s.set_active_layer(lower);
        assert_eq!(target(&s, (230., 10.)), TextTarget::Existing(box_id));
        s.toggle_layer_visibility(lower);
        assert_eq!(target(&s, (230., 10.)), TextTarget::NewBox((230., 10.)));
    }

    #[test]
    fn empty_new_box_leaves_nothing() {
        let (_dir, mut s) = session();
        let mut g = SessionGesture::default();
        let id = start_new_text_box(&mut s, &mut g, (10., 20.));
        let shape = s.find_shape(id).unwrap();
        assert!(shape.is_text_box());
        assert_eq!((shape.frame.x, shape.frame.y), (10., 20.));
        assert!(!finish_text_session(&mut s, &mut g, id));
        assert!(s.find_shape(id).is_none());
        assert!(!s.can_undo());
        assert!(!s.is_dirty());
        // The gesture is closed: a pointer gesture can open.
        assert!(g.begin(&mut s, GestureOwner::Pointer));
        g.end(&mut s, GestureOwner::Pointer);

        // A box that got text stays, as one undo step.
        let id = start_new_text_box(&mut s, &mut g, (0., 0.));
        s.update_shapes(&[id], |sh| {
            sh.label
                .as_mut()
                .unwrap()
                .text
                .insert(0, "hi", &RunStyle::default());
        });
        assert!(finish_text_session(&mut s, &mut g, id));
        assert!(s.is_dirty());
        assert!(s.undo());
        assert!(s.find_shape(id).is_none());
        assert!(!s.can_undo());
    }

    #[test]
    fn emptied_label_is_cleared() {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let (_dir, mut s) = session();
        let shape = rectangle(&libs, (0., 0.));
        let id = shape.id;
        s.add_shapes(vec![shape]);
        let steps_before = s.can_undo();
        let mut g = SessionGesture::default();

        // Editing a shape without a label gives it an empty default one;
        // leaving it empty takes it away again, leaving no step.
        assert!(start_text_session(&mut s, &mut g, id));
        let label = s.find_shape(id).unwrap().label.clone().unwrap();
        assert_eq!(label, Label::new_shape_label());
        assert!(finish_text_session(&mut s, &mut g, id));
        assert_eq!(s.find_shape(id).unwrap().label, None);
        s.undo();
        assert!(s.find_shape(id).is_none(), "the only step left adds it");
        assert!(s.redo());
        assert_eq!(steps_before, s.can_undo());

        // A label emptied while editing is cleared, as one step.
        s.set_label(id, Some(Label::new_shape_label()));
        s.update_shapes(&[id], |sh| {
            sh.label
                .as_mut()
                .unwrap()
                .text
                .insert(0, "x", &RunStyle::default());
        });
        assert!(start_text_session(&mut s, &mut g, id));
        s.update_shapes(&[id], |sh| sh.label.as_mut().unwrap().text.delete(0..1));
        assert!(finish_text_session(&mut s, &mut g, id));
        assert_eq!(s.find_shape(id).unwrap().label, None);
        assert!(s.undo());
        assert_eq!(
            s.find_shape(id)
                .unwrap()
                .label
                .as_ref()
                .unwrap()
                .text
                .plain(),
            "x"
        );

        // An existing text box emptied while editing is removed, as a step.
        let text_box = mindweave_core::plain_text_box("hello", (0., 0.)).unwrap();
        let box_id = text_box.id;
        s.add_shapes(vec![text_box]);
        assert!(start_text_session(&mut s, &mut g, box_id));
        s.update_shapes(&[box_id], |sh| sh.label.as_mut().unwrap().text.delete(0..5));
        assert!(!finish_text_session(&mut s, &mut g, box_id));
        assert!(s.find_shape(box_id).is_none());
        assert!(s.undo());
        assert!(s.find_shape(box_id).is_some());

        // A missing shape starts nothing.
        assert!(!start_text_session(&mut s, &mut g, Uuid::new_v4()));
        assert!(g.begin(&mut s, GestureOwner::Pointer));
    }

    #[test]
    fn enter_edits_one_selected_text_object() {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let (_dir, mut s) = session();
        let plain_shape = rectangle(&libs, (0., 0.));
        let mut labelled = rectangle(&libs, (0., 100.));
        labelled.label = Some(Label::new_shape_label());
        let text_box = Shape::new_text_box((0., 200.));
        let ids = [plain_shape.id, labelled.id, text_box.id];
        s.add_shapes(vec![plain_shape, labelled, text_box]);
        assert_eq!(enter_target(&s, &[ids[0]]), None);
        assert_eq!(enter_target(&s, &[ids[1]]), Some(ids[1]));
        assert_eq!(enter_target(&s, &[ids[2]]), Some(ids[2]));
        assert_eq!(enter_target(&s, &[ids[1], ids[2]]), None);
        assert_eq!(enter_target(&s, &[]), None);
    }

    #[test]
    fn presses_use_the_current_layout_before_any_paint() {
        // Laid out but never painted: the current layout decides.
        assert_eq!(press_hit(Some((true, 7)), false), Some(Some(7)));
        // A stale painted area does not keep a press outside the text.
        assert_eq!(press_hit(Some((false, 7)), true), None);
        // Not laid out yet: the painted area, without an offset.
        assert_eq!(press_hit(None, true), Some(None));
        assert_eq!(press_hit(None, false), None);
    }
}
