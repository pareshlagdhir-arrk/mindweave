//! The Properties panel's Text section: font, size, bold / italic /
//! underline / strikethrough, colour, alignment and (for shape labels)
//! vertical alignment.
//!
//! While text is being edited the section shows, and changes, the selected
//! text, or with nothing selected the style typing will use; its changes
//! join the editing session's undo step. Otherwise it shows the text of the
//! selected text boxes and labelled shapes (unlabelled shapes are skipped),
//! and each change sets that one property in all of it as one undo step.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, v_virtual_list, ActiveTheme as _, Icon as ComponentIcon, IconName,
    Sizable as _, VirtualListScrollHandle,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::{
    DocumentSession, HAlign, Mixed, RunStyle, Shape, StyleChange, StyleSummary, VAlign,
    MAX_FONT_SIZE, MIN_FONT_SIZE,
};
use uuid::Uuid;

use super::color_palette::{palette_popover, ColorPalette, ColorPick, PaletteColor};
use super::properties_panel::{field_row, FIELD_GAP, SWATCH_SIZE};
use super::shape_properties::{format_number, SECTION_GAP};
use super::text_edit::{EditTarget, TextEditor, Toggle};
use super::text_layout::{installed_font_families, refresh_installed_fonts};
use super::{hex_to_hsla, parse_number, section_label, EditorWindow, GestureOwner, SessionGesture};
use crate::app_state::AppState;
use crate::libraries::lucide_icon_path;

/// The sizes the ▾ menu offers.
pub(super) const SIZE_PRESETS: [f32; 9] = [8., 10., 12., 14., 18., 24., 32., 48., 72.];
/// Wheel travel, in pixels, for one size step: a mouse wheel notch (one
/// line) is one step, and trackpad pixels add up to steps.
pub(super) const WHEEL_STEP_PX: f32 = 20.;
/// The size box, stepper and presets included.
const SIZE_BOX_WIDTH: f32 = 100.;
/// Each half of the ▴▾ stepper, and its chevrons.
const STEPPER_WIDTH: f32 = 14.;
const STEPPER_HEIGHT: f32 = 10.;
const STEPPER_ICON_SIZE: f32 = 10.;
/// The side of the style toggles and alignment buttons.
const TOGGLE_SIZE: f32 = 24.;
const ICON_SIZE: f32 = 14.;
const FONT_BUTTON_WIDTH: f32 = 150.;
const FONT_LIST_WIDTH: f32 = 240.;
const FONT_ROW_HEIGHT: f32 = 26.;
const FONT_ROW_TEXT_SIZE: f32 = 13.;
/// The font list shows this many rows before it scrolls.
const FONT_LIST_ROWS: usize = 12;
/// Size steps this close together are one undo step (see
/// [`GestureOwner::Size`]).
const SIZE_STEPS_IDLE: Duration = Duration::from_millis(500);
/// What the font control calls the app's default font (`None`).
const DEFAULT_FONT: &str = "Default";

/// What the section shows.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TextSummary {
    /// The objects whose text it shows: the one being edited, or the
    /// selected ones with text, back to front.
    pub targets: Vec<Uuid>,
    pub style: StyleSummary,
    /// The size the steppers and the wheel start from: the shared size,
    /// else the first text's.
    pub base_size: f32,
    /// The vertical alignment of the shape labels among the targets;
    /// `None` when there are none (text boxes and line labels have none).
    pub valign: Option<Mixed<VAlign>>,
}

/// `a` where `b` agrees, else mixed.
fn merge<T: PartialEq>(a: Mixed<T>, b: &Mixed<T>) -> Mixed<T> {
    match (a, b) {
        (Mixed::Same(x), Mixed::Same(y)) if x == *y => Mixed::Same(x),
        _ => Mixed::Mixed,
    }
}

/// The styles of two texts together: each value they share, else mixed.
pub(super) fn merge_summaries(a: StyleSummary, b: &StyleSummary) -> StyleSummary {
    StyleSummary {
        font: merge(a.font, &b.font),
        size: merge(a.size, &b.size),
        bold: merge(a.bold, &b.bold),
        italic: merge(a.italic, &b.italic),
        underline: merge(a.underline, &b.underline),
        strike: merge(a.strike, &b.strike),
        color: merge(a.color, &b.color),
        link: merge(a.link, &b.link),
        align: merge(a.align, &b.align),
    }
}

/// Whether `shape`'s label has a vertical alignment: shape labels do; text
/// boxes fit their text and line labels sit on the line.
fn has_valign(shape: &Shape) -> bool {
    !shape.is_text_box() && !shape.is_line_like_label()
}

/// The section's view of all the text of `shapes`, skipping those without
/// text; `None` when none has any.
pub(super) fn summarize_objects(shapes: &[&Shape]) -> Option<TextSummary> {
    let mut targets = Vec::new();
    let mut style: Option<StyleSummary> = None;
    let mut base_size = None;
    let mut valign: Option<Mixed<VAlign>> = None;
    for shape in shapes {
        let Some(label) = &shape.label else {
            continue;
        };
        let text = &label.text;
        let summary = text.summary(0..text.len());
        style = Some(match style {
            None => summary,
            Some(merged) => merge_summaries(merged, &summary),
        });
        base_size.get_or_insert(text.style_at(0).size);
        if has_valign(shape) {
            let this = Mixed::Same(label.valign);
            valign = Some(match valign {
                None => this,
                Some(v) => merge(v, &this),
            });
        }
        targets.push(shape.id);
    }
    Some(TextSummary {
        targets,
        style: style?,
        base_size: base_size?,
        valign,
    })
}

/// A summary of one style (and paragraph alignment `align`).
fn style_summary(style: &RunStyle, align: Mixed<HAlign>) -> StyleSummary {
    StyleSummary {
        font: Mixed::Same(style.font.clone()),
        size: Mixed::Same(style.size),
        bold: Mixed::Same(style.bold),
        italic: Mixed::Same(style.italic),
        underline: Mixed::Same(style.underline),
        strike: Mixed::Same(style.strike),
        color: Mixed::Same(style.color.clone()),
        link: Mixed::Same(style.link.clone()),
        align,
    }
}

/// The section's view of `shape`'s text being edited with `editor`: the
/// selection's styles, or with nothing selected the style typing will use.
fn summarize_editing(shape: &Shape, editor: &TextEditor) -> Option<TextSummary> {
    let label = shape.label.as_ref()?;
    let text = &label.text;
    let len = text.len();
    let sel = editor.selection();
    let sel = sel.start.min(len)..sel.end.min(len);
    let mut style = text.summary(sel.clone());
    if let Some(typing) = editor.typing_style.as_ref().filter(|_| sel.is_empty()) {
        style = style_summary(typing, style.align);
    }
    let base_size = match style.size {
        Mixed::Same(size) => size,
        Mixed::Mixed => text.style_at(text.next_grapheme(sel.start)).size,
    };
    Some(TextSummary {
        targets: vec![shape.id],
        style,
        base_size,
        valign: has_valign(shape).then_some(Mixed::Same(label.valign)),
    })
}

/// Size steps for wheel travel `delta` (pixels, up is positive) on top of
/// the travel `carry` left from before: whole [`WHEEL_STEP_PX`]s, and the
/// travel left over.
pub(super) fn wheel_steps(carry: f32, delta: f32) -> (i32, f32) {
    let travel = carry + delta;
    let steps = (travel / WHEEL_STEP_PX).trunc();
    (steps as i32, travel - steps * WHEEL_STEP_PX)
}

/// `base` moved by `steps` points, within the allowed sizes.
pub(super) fn step_size(base: f32, steps: i32) -> f32 {
    (base + steps as f32).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

/// What the size field shows for a text's `size` (`None`: no text): the
/// size, or empty with the placeholder "Mixed" (or none).
pub(super) fn size_display(size: Option<&Mixed<f32>>) -> (Option<f32>, &'static str) {
    match size {
        Some(Mixed::Same(size)) => (Some(*size), ""),
        Some(Mixed::Mixed) => (None, "Mixed"),
        None => (None, ""),
    }
}

/// Commits size field text `text`, which the user `typed` (or not) over
/// `shown`: `apply` gets the size to apply (`None`: nothing to apply) and
/// returns the text's size afterwards. Returns what the field must show
/// then, even while it keeps focus (the per-frame sync leaves a focused
/// field alone).
#[must_use = "the size field must show the result"]
pub(super) fn commit_size(
    text: &str,
    typed: bool,
    shown: Option<f32>,
    apply: impl FnOnce(Option<f32>) -> Option<Mixed<f32>>,
) -> (Option<f32>, &'static str) {
    let size = typed.then(|| typed_size(text, shown)).flatten();
    size_display(apply(size).as_ref())
}

/// The undo step that groups size steps (wheel, ↑ and ↓) made while not
/// editing: the session's gesture, held by [`GestureOwner::Size`].
#[derive(Debug, Default)]
pub(super) struct SizeSteps {
    /// Whether the group holds the gesture.
    open: bool,
    /// Counts steps, so only the last one's idle timer ends the group.
    count: u64,
}

impl SizeSteps {
    /// Opens the group unless it is open (or another gesture is), and
    /// counts a step; returns the step's number while the group is open.
    pub(super) fn begin(
        &mut self,
        gesture: &mut SessionGesture,
        session: &mut DocumentSession,
    ) -> Option<u64> {
        if !self.open {
            self.open = gesture.begin(session, GestureOwner::Size);
        }
        self.open.then(|| {
            self.count += 1;
            self.count
        })
    }

    /// Ends the group if it is open; returns whether it was.
    pub(super) fn end(
        &mut self,
        gesture: &mut SessionGesture,
        session: &mut DocumentSession,
    ) -> bool {
        std::mem::take(&mut self.open) && gesture.end(session, GestureOwner::Size)
    }

    /// Whether step number `count` is still the last one.
    fn is_last(&self, count: u64) -> bool {
        self.open && self.count == count
    }
}

/// Applies edit `f` to `session`. Any edit but a size step (`size_step`)
/// first ends the size steps' group, so it is an undo step of its own.
pub(super) fn edit_session(
    steps: &mut SizeSteps,
    gesture: &mut SessionGesture,
    session: &mut DocumentSession,
    size_step: bool,
    f: impl FnOnce(&mut DocumentSession),
) {
    if !size_step {
        steps.end(gesture, session);
    }
    f(session);
}

/// The size to apply from size field text `text` when it showed `shown`
/// (`None`: "Mixed"): a number, clamped to the allowed sizes, unless it is
/// what was shown.
pub(super) fn typed_size(text: &str, shown: Option<f32>) -> Option<f32> {
    parse_number(text)
        .map(|v| v.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE))
        .filter(|v| Some(*v) != shown)
}

/// The font list: the `recent` families still `installed` first, newest
/// first, then the other installed families sorted ignoring case, each
/// once. Hidden system families (names starting with ".") are left out.
/// Also returns how many recent ones lead.
pub(super) fn font_rows(installed: Vec<String>, recent: &[String]) -> (Vec<SharedString>, usize) {
    let mut all = installed;
    all.retain(|name| !name.starts_with('.'));
    all.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    all.dedup();
    let available: HashSet<&str> = all.iter().map(String::as_str).collect();
    let mut leading: Vec<&str> = Vec::new();
    for name in recent {
        if available.contains(name.as_str()) && !leading.contains(&name.as_str()) {
            leading.push(name);
        }
    }
    let mut rows: Vec<SharedString> = leading
        .iter()
        .map(|n| SharedString::from(n.to_string()))
        .collect();
    let count = rows.len();
    rows.extend(
        all.iter()
            .filter(|n| !leading.contains(&n.as_str()))
            .map(|n| SharedString::from(n.clone())),
    );
    (rows, count)
}

/// Indexes of the `rows` whose name contains `query`, ignoring case and
/// surrounding spaces; a blank query keeps them all.
pub(super) fn filter_fonts(rows: &[SharedString], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, name)| name.to_lowercase().contains(&query))
        .map(|(ix, _)| ix)
        .collect()
}

/// A change made in the section.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum TextChange {
    Style(StyleChange),
    /// Bold, italic, underline or strikethrough: on unless all of it is on.
    Toggle(Toggle),
    Align(HAlign),
    VAlign(VAlign),
}

/// Applies `change` to all the text of the shapes with `ids` (not while
/// editing) as one undo step: only that property changes, in every run;
/// shapes without text are skipped.
pub(super) fn apply_to_objects(session: &mut DocumentSession, ids: &[Uuid], change: &TextChange) {
    let change = match change {
        TextChange::Toggle(toggle) => {
            let wanted: HashSet<&Uuid> = ids.iter().collect();
            let shapes: Vec<&Shape> = session
                .shapes_on_active_layer()
                .iter()
                .filter(|s| wanted.contains(&s.id))
                .collect();
            let Some(summary) = summarize_objects(&shapes) else {
                return;
            };
            TextChange::Style(toggle.change(toggle.value(&summary.style)))
        }
        other => other.clone(),
    };
    session.update_shapes(ids, |shape| {
        let valign = has_valign(shape);
        let Some(label) = shape.label.as_mut() else {
            return;
        };
        match &change {
            TextChange::Style(style) => label.text.apply_all(style),
            TextChange::Align(align) => {
                let len = label.text.len();
                label.text.set_align(0..len, *align);
            }
            TextChange::VAlign(v) if valign => label.valign = *v,
            TextChange::VAlign(_) | TextChange::Toggle(_) => {}
        }
    });
}

/// What the section was last synced from: the content revision, the
/// selection, and the edit's target, caret, selection and typing style
/// (hashed, so comparing is cheap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SyncKey {
    revision: u64,
    selection: u64,
    editing: u64,
}

impl SyncKey {
    fn new(revision: u64, selection: &[Uuid], edit: Option<&TextEditor>) -> Self {
        let mut h = DefaultHasher::new();
        selection.hash(&mut h);
        let selection = h.finish();
        let mut h = DefaultHasher::new();
        if let Some(edit) = edit {
            (edit.target, edit.caret, edit.anchor).hash(&mut h);
            if let Some(s) = &edit.typing_style {
                s.font.hash(&mut h);
                s.size.to_bits().hash(&mut h);
                (s.bold, s.italic, s.underline, s.strike).hash(&mut h);
                s.color.hash(&mut h);
            }
        }
        SyncKey {
            revision,
            selection,
            editing: h.finish(),
        }
    }
}

/// The section's controls, kept in step with the text they show.
pub(super) struct TextFields {
    size: Entity<InputState>,
    /// The size the field shows (`None`: "Mixed" or nothing).
    size_shows: Option<f32>,
    /// The user has typed in the size field since it was last shown.
    size_typed: bool,
    /// Wheel travel over the size field not yet turned into a step.
    wheel_carry: f32,
    palette: Entity<ColorPalette>,
    /// The palette has previewed a colour drag since it opened.
    previewing: bool,
    /// The undo step grouping size steps.
    pub(super) size_steps: SizeSteps,
    /// A size step is being applied (see [`edit_session`]).
    pub(super) size_step_edit: bool,
    /// The size field had focus at the last sync, so it was not updated.
    size_left_alone: bool,
    fonts: Entity<FontPicker>,
    pub(super) summary: Option<TextSummary>,
    synced: Option<SyncKey>,
    _subscriptions: Vec<Subscription>,
}

impl TextFields {
    pub(super) fn new(window: &mut Window, cx: &mut Context<EditorWindow>) -> Self {
        let size = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(&size, window, |this, _, event, window, cx| {
            match event {
                // Showing a value sets it silently, so this is typing.
                InputEvent::Change => this.properties.text.size_typed = true,
                InputEvent::PressEnter { .. } => {
                    this.commit_text_size(window, cx);
                    // Typing goes back to the text being edited.
                    if this.text_edit.is_some() {
                        this.focus_handle.focus(window, cx);
                    }
                }
                InputEvent::Blur => this.commit_text_size(window, cx),
                _ => {}
            }
        });
        let editor = cx.entity().downgrade();
        let palette = cx.new(|cx| {
            ColorPalette::new(
                move |pick, window, cx| {
                    editor
                        .update(cx, |editor, cx| editor.pick_text_color(pick, window, cx))
                        .ok();
                },
                window,
                cx,
            )
        });
        let editor = cx.entity().downgrade();
        let fonts = cx.new(|cx| {
            FontPicker::new(
                move |family, window, cx| {
                    editor
                        .update(cx, |editor, cx| editor.pick_font(family, window, cx))
                        .ok();
                },
                window,
                cx,
            )
        });
        TextFields {
            size,
            size_shows: None,
            size_typed: false,
            wheel_carry: 0.,
            palette,
            previewing: false,
            size_steps: SizeSteps::default(),
            size_step_edit: false,
            size_left_alone: false,
            fonts,
            summary: None,
            synced: None,
            _subscriptions: vec![subscription],
        }
    }

    /// Puts `size` (`None`: "Mixed", or nothing when `placeholder` is
    /// empty) in the size field.
    fn show_size(
        &mut self,
        size: Option<f32>,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let text = size.map(format_number).unwrap_or_default();
        self.size.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        self.size_shows = size;
        self.size_typed = false;
    }
}

impl EditorWindow {
    /// What the Text section shows now.
    fn text_summary(&self) -> Option<TextSummary> {
        match &self.text_edit {
            Some(edit) => match edit.target {
                EditTarget::Label(id) => summarize_editing(self.session.find_shape(id)?, edit),
                // A note's formatting is the note card's, not this section's.
                EditTarget::Note(_) => None,
            },
            None => summarize_objects(&self.shapes_among(&self.selection)),
        }
    }

    /// Brings the Text section up to date. Runs every frame, so it does
    /// nothing unless the content, the selection or the text selection
    /// changed since the last full sync; a size being typed is left alone.
    pub(super) fn sync_text_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = SyncKey::new(
            self.session.revision(),
            &self.selection,
            self.text_edit.as_ref(),
        );
        let fields = &mut self.properties.text;
        let typing = fields.size.read(cx).focus_handle(cx).is_focused(window);
        if fields.synced == Some(key) {
            // Only the size field may be behind: it was left alone while
            // being typed in.
            if fields.size_left_alone && !typing {
                let (size, placeholder) =
                    size_display(fields.summary.as_ref().map(|s| &s.style.size));
                fields.show_size(size, placeholder, window, cx);
                fields.size_left_alone = false;
            }
            return;
        }
        let summary = self.text_summary();
        let fields = &mut self.properties.text;
        if !typing {
            let (size, placeholder) = size_display(summary.as_ref().map(|s| &s.style.size));
            fields.show_size(size, placeholder, window, cx);
        }
        fields.size_left_alone = typing;
        let style = summary.as_ref().map(|s| &s.style);
        let color = PaletteColor::from_mixed(match style.map(|s| &s.color) {
            Some(Mixed::Same(color)) => Some(color.clone()),
            Some(Mixed::Mixed) => None,
            None => Some(None),
        });
        fields
            .palette
            .update(cx, |palette, cx| palette.set_current(color, cx));
        let font = match style.map(|s| &s.font) {
            Some(Mixed::Same(font)) => Some(font.clone()),
            _ => None,
        };
        fields
            .fonts
            .update(cx, |fonts, cx| fonts.set_current(font, cx));
        fields.summary = summary;
        fields.synced = Some(key);
    }

    /// Applies a change from the Text section: while editing, to the
    /// selection (or the typing style) inside the editing session, with
    /// `preview` grouping a colour drag; otherwise to all the text of the
    /// selected objects as one undo step (or joining an open colour drag).
    fn apply_text_change(
        &mut self,
        change: TextChange,
        preview: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.text_edit.as_ref() else {
            let ids = self.selection.clone();
            self.update_session(|s| apply_to_objects(s, &ids, &change), cx);
            self.properties.text.synced = None;
            return;
        };
        let target = edit.target;
        match change {
            TextChange::Style(style) => {
                self.with_text(window, cx, |ed, text, _| {
                    ed.apply_style(text, &style, preview)
                });
            }
            TextChange::Toggle(toggle) => {
                // The value the section shows the toggle with: the typing
                // style with nothing selected.
                let Some(summary) = self.text_summary() else {
                    return;
                };
                let style = toggle.change(toggle.value(&summary.style));
                self.with_text(window, cx, |ed, text, _| {
                    ed.apply_style(text, &style, false)
                });
            }
            TextChange::Align(align) => {
                self.with_text(window, cx, |ed, text, _| ed.set_align(text, align));
            }
            TextChange::VAlign(v) => {
                // Only a label has a vertical alignment.
                if let EditTarget::Label(id) = target {
                    self.update_session(|s| apply_to_objects(s, &[id], &TextChange::VAlign(v)), cx);
                }
            }
        }
        self.properties.text.synced = None;
    }

    /// Applies the size typed in the size field, if any; the field then
    /// shows the text's size (the clamped size applied, or for text that is
    /// not a number the size it had), even while it keeps focus.
    fn commit_text_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fields = &self.properties.text;
        let typed = fields.size_typed;
        let text = fields.size.read(cx).value().to_string();
        let shows = fields.size_shows;
        self.properties.text.size_typed = false;
        let (size, placeholder) = commit_size(&text, typed, shows, |size| {
            if let Some(size) = size.filter(|_| self.text_summary().is_some()) {
                let change = TextChange::Style(StyleChange::Size(size));
                self.apply_text_change(change, false, window, cx);
            }
            Some(self.text_summary()?.style.size)
        });
        self.properties
            .text
            .show_size(size, placeholder, window, cx);
        self.properties.text.synced = None;
        cx.notify();
    }

    /// Applies the size being typed, if any. The canvas and the edit
    /// actions call this before they change what the section shows (and
    /// before text editing ends), so the size goes where it was typed.
    pub(super) fn commit_focused_text_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .properties
            .text
            .size
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if focused {
            self.commit_text_size(window, cx);
        }
    }

    /// Moves the size by `steps` points from the size shown (the first
    /// text's when mixed), and shows it.
    fn step_text_size(&mut self, steps: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(base) = self.properties.text.summary.as_ref().map(|s| s.base_size) else {
            return;
        };
        let size = step_size(base, steps);
        self.begin_size_steps(window, cx);
        self.properties.text.size_step_edit = true;
        self.apply_text_change(
            TextChange::Style(StyleChange::Size(size)),
            false,
            window,
            cx,
        );
        self.properties.text.size_step_edit = false;
        let fields = &mut self.properties.text;
        fields.show_size(Some(size), "", window, cx);
        // Steps arriving before the next sync go on from this size.
        if let Some(summary) = fields.summary.as_mut() {
            summary.base_size = size;
        }
    }

    /// Opens (or keeps open) the undo step that groups size steps made
    /// while not editing; it ends [`SIZE_STEPS_IDLE`] after the last step,
    /// when the pointer leaves the size control, or before any other edit.
    fn begin_size_steps(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_edit.is_some() {
            return;
        }
        let Some(count) = self
            .properties
            .text
            .size_steps
            .begin(&mut self.session_gesture, &mut self.session)
        else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SIZE_STEPS_IDLE).await;
            this.update(cx, |this, cx| {
                if this.properties.text.size_steps.is_last(count) {
                    this.finish_size_steps(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Ends the undo step of size steps, if one is open, without
    /// notifying (see [`Self::finish_size_steps`]).
    pub(super) fn end_size_steps(&mut self) -> bool {
        self.properties
            .text
            .size_steps
            .end(&mut self.session_gesture, &mut self.session)
    }

    /// Ends the undo step of size steps, if one is open.
    pub(super) fn finish_size_steps(&mut self, cx: &mut Context<Self>) {
        if self.end_size_steps() {
            self.update_session(|_| {}, cx);
        }
    }

    /// The wheel over the size field steps the size by one per notch.
    fn on_size_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let delta = event.delta.pixel_delta(px(WHEEL_STEP_PX)).y.as_f32();
        let (steps, carry) = wheel_steps(self.properties.text.wheel_carry, delta);
        self.properties.text.wheel_carry = carry;
        if steps != 0 {
            self.step_text_size(steps, window, cx);
        }
    }

    /// A choice in the colour palette (see [`ColorPick`]).
    fn pick_text_color(&mut self, pick: ColorPick, window: &mut Window, cx: &mut Context<Self>) {
        match pick {
            ColorPick::Set(color) => {
                let previewing = self.properties.text.previewing;
                let change = TextChange::Style(StyleChange::Color(color));
                self.apply_text_change(change, previewing, window, cx);
            }
            ColorPick::Preview(hex) => {
                self.properties.text.previewing = true;
                self.begin_color_edit(true);
                let change = TextChange::Style(StyleChange::Color(Some(hex)));
                self.apply_text_change(change, true, window, cx);
            }
            ColorPick::EndPreview => {
                self.properties.text.previewing = false;
                if let Some(edit) = self.text_edit.as_mut() {
                    edit.end_style_preview();
                }
                self.finish_color_edit(cx);
            }
        }
    }

    /// A family picked in the font list (`None`: the default font).
    fn pick_font(&mut self, family: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = &family {
            if cx.has_global::<AppState>() {
                AppState::update_settings(cx, |s| s.push_recent_font(name));
            }
        }
        self.apply_text_change(
            TextChange::Style(StyleChange::Font(family)),
            false,
            window,
            cx,
        );
    }

    /// Whether the Text section's colour palette is open.
    pub(super) fn text_palette_open(&self, cx: &App) -> bool {
        self.properties.text.palette.read(cx).is_open(cx)
    }
}

/// The Text section, when there is text to show: the text being edited,
/// or the selected text boxes and labelled shapes.
/// Whether the selection's one font family `font` is not installed (it is
/// drawn in the default font).
fn font_missing(font: &Mixed<Option<String>>, installed: &HashSet<String>) -> bool {
    matches!(font, Mixed::Same(Some(name)) if !installed.contains(name))
}

pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> Option<AnyElement> {
    let fields = &editor.properties.text;
    let summary = fields.summary.as_ref()?;
    let style = &summary.style;
    let toggles = h_flex()
        .gap(px(2.))
        .child(toggle_button(Toggle::Bold, "bold", "Bold", &style.bold, cx))
        .child(toggle_button(
            Toggle::Italic,
            "italic",
            "Italic",
            &style.italic,
            cx,
        ))
        .child(toggle_button(
            Toggle::Underline,
            "underline",
            "Underline",
            &style.underline,
            cx,
        ))
        .child(toggle_button(
            Toggle::Strike,
            "strikethrough",
            "Strikethrough",
            &style.strike,
            cx,
        ));
    let aligns = h_flex().gap(px(2.)).children(
        [
            (HAlign::Left, "text-align-start", "Align Left"),
            (HAlign::Center, "text-align-center", "Align Centre"),
            (HAlign::Right, "text-align-end", "Align Right"),
        ]
        .map(|(align, icon, tooltip)| {
            let on = style.align == Mixed::Same(align);
            choice_button(("text-align", align as usize), icon, tooltip, on, cx).on_click(
                cx.listener(move |this, _, window, cx| {
                    this.apply_text_change(TextChange::Align(align), false, window, cx)
                }),
            )
        }),
    );
    let valigns = summary.valign.map(|valign| {
        h_flex().gap(px(2.)).children(
            [
                (VAlign::Top, "align-vertical-justify-start", "Align Top"),
                (
                    VAlign::Middle,
                    "align-vertical-justify-center",
                    "Align Middle",
                ),
                (VAlign::Bottom, "align-vertical-justify-end", "Align Bottom"),
            ]
            .map(|(v, icon, tooltip)| {
                let on = valign == Mixed::Same(v);
                choice_button(("text-valign", v as usize), icon, tooltip, on, cx).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.apply_text_change(TextChange::VAlign(v), false, window, cx)
                    }),
                )
            }),
        )
    });
    // From the installed fonts, not the layouts: objects off screen have
    // none.
    let missing = font_missing(&style.font, &installed_font_families(cx));

    Some(
        v_flex()
            .gap(px(4.))
            .child(section_label("Text", cx).mt(px(SECTION_GAP)))
            .child(field_row(
                "Font",
                None,
                font_button(fields, &style.font, missing, cx),
            ))
            .child(field_row("Size", None, size_control(fields, cx)))
            .child(field_row("Style", None, toggles))
            .child(field_row(
                "Colour",
                None,
                color_button(fields, &style.color, cx),
            ))
            .child(field_row("Align", None, aligns))
            .when_some(valigns, |section, valigns| {
                section.child(field_row("Vertical", None, valigns))
            })
            .into_any_element(),
    )
}

/// A lucide icon at the section's icon size.
fn icon(name: &str, color: Hsla, cx: &App) -> Svg {
    svg()
        .path(lucide_icon_path(name, cx))
        .size(px(ICON_SIZE))
        .flex_none()
        .text_color(color)
}

/// A square button showing `icon`, marked when `on`.
fn choice_button(
    id: impl Into<ElementId>,
    icon_name: &str,
    tooltip: &'static str,
    on: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let hover = theme.list_hover;
    let color = if on {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    div()
        .id(id)
        .flex_none()
        .size(px(TOGGLE_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme.radius)
        .when(on, |b| b.bg(theme.list_active))
        .when(!on, |b| b.hover(move |s| s.bg(hover)))
        .child(icon(icon_name, color, cx))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
}

/// A style toggle: marked when all the text has the style, half-marked
/// when some of it has.
fn toggle_button(
    toggle: Toggle,
    icon_name: &str,
    tooltip: &'static str,
    state: &Mixed<bool>,
    cx: &mut Context<EditorWindow>,
) -> Stateful<Div> {
    let on = *state == Mixed::Same(true);
    let mixed = *state == Mixed::Mixed;
    let partial = cx.theme().list_active.opacity(0.45);
    choice_button(("text-toggle", toggle as usize), icon_name, tooltip, on, cx)
        .when(mixed, |b| b.bg(partial))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.apply_text_change(TextChange::Toggle(toggle), false, window, cx)
        }))
}

/// The size box: the size field (Enter or leaving it applies; the wheel
/// over it steps), a small ▴▾ stepper, and the ▾ preset sizes, in one
/// border.
fn size_control(fields: &TextFields, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let theme = cx.theme();
    let (hover, muted, border) = (theme.list_hover, theme.muted_foreground, theme.border);
    let step = |id: &'static str, name: &str, tooltip: &'static str, steps: i32| {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(STEPPER_WIDTH))
            .h(px(STEPPER_HEIGHT))
            .rounded(px(2.))
            .hover(move |s| s.bg(hover))
            .child(
                svg()
                    .path(lucide_icon_path(name, cx))
                    .size(px(STEPPER_ICON_SIZE))
                    .text_color(muted),
            )
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .on_click(
                cx.listener(move |this, _, window, cx| this.step_text_size(steps, window, cx)),
            )
    };
    let stepper = v_flex()
        .flex_none()
        .child(step("text-size-up", "chevron-up", "Larger", 1))
        .child(step("text-size-down", "chevron-down", "Smaller", -1));
    let presets = h_flex()
        .flex_none()
        .h_full()
        .pl(px(2.))
        .border_l_1()
        .border_color(border)
        .child(size_presets(cx));
    div()
        .id("text-size")
        .w(px(SIZE_BOX_WIDTH))
        // Leaving the field ends a run of wheel steps.
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !*hovered {
                this.finish_size_steps(cx);
            }
        }))
        .on_scroll_wheel(cx.listener(EditorWindow::on_size_wheel))
        // Arrow keys in the field step too.
        .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
            cx.stop_propagation();
            this.step_text_size(1, window, cx);
        }))
        .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
            cx.stop_propagation();
            this.step_text_size(-1, window, cx);
        }))
        .child(
            Input::new(&fields.size)
                .small()
                .suffix(h_flex().h_full().gap(px(2.)).child(stepper).child(presets)),
        )
}

/// The ▾ menu of preset sizes.
fn size_presets(cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let editor = cx.entity().downgrade();
    Popover::new("text-size-presets")
        .anchor(Anchor::TopRight)
        .trigger(
            Button::new("text-size-presets-button")
                .ghost()
                .xsmall()
                .icon(IconName::ChevronDown)
                .tooltip("Preset Sizes"),
        )
        .content(move |_, _, cx| {
            let popover = cx.entity().downgrade();
            let hover = cx.theme().list_hover;
            let radius = cx.theme().radius;
            v_flex()
                .w(px(64.))
                .children(SIZE_PRESETS.iter().map(|&size| {
                    let (editor, popover) = (editor.clone(), popover.clone());
                    div()
                        .id(("text-size-preset", size as usize))
                        .h(px(FONT_ROW_HEIGHT))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(radius)
                        .hover(move |s| s.bg(hover))
                        .child(format_number(size))
                        .on_click(move |_, window, cx| {
                            popover
                                .update(cx, |popover, cx| popover.dismiss(window, cx))
                                .ok();
                            editor
                                .update(cx, |editor, cx| {
                                    let change = TextChange::Style(StyleChange::Size(size));
                                    editor.apply_text_change(change, false, window, cx);
                                })
                                .ok();
                        })
                }))
        })
}

/// The colour swatch (Auto: half black, half white) and its name, opening
/// the palette.
fn color_button(fields: &TextFields, color: &Mixed<Option<String>>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let swatch = div()
        .flex_none()
        .size(px(SWATCH_SIZE))
        .rounded(px(3.))
        .overflow_hidden()
        .border_1()
        .border_color(theme.border);
    let (swatch, label) = match color {
        Mixed::Same(Some(hex)) => (swatch.bg(hex_to_hsla(hex).unwrap_or_default()), hex.clone()),
        Mixed::Same(None) => (
            swatch
                .flex()
                .child(div().h_full().flex_1().bg(gpui::black()))
                .child(div().h_full().flex_1().bg(gpui::white())),
            "Auto".to_string(),
        ),
        Mixed::Mixed => (swatch, "Mixed".to_string()),
    };
    let set = matches!(color, Mixed::Same(Some(_)));
    let trigger = Button::new("text-color").ghost().xsmall().child(
        h_flex().gap(px(FIELD_GAP)).child(swatch).child(
            div()
                .when(!set, |l| l.text_color(theme.muted_foreground))
                .child(label),
        ),
    );
    palette_popover("text-color-palette", &fields.palette, trigger).anchor(Anchor::TopRight)
}

/// The font control: the family in its own font ("Mixed"; a family that
/// is not installed is marked missing), opening the font list.
fn font_button(
    fields: &TextFields,
    font: &Mixed<Option<String>>,
    missing: bool,
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let name = match font {
        Mixed::Same(Some(name)) => div()
            .min_w_0()
            .truncate()
            .when(!missing, |d| {
                d.font_family(SharedString::from(name.clone()))
            })
            .child(name.clone()),
        Mixed::Same(None) => div().child(DEFAULT_FONT),
        Mixed::Mixed => div().text_color(muted).child("Mixed"),
    };
    let label = h_flex()
        .min_w_0()
        .flex_1()
        .gap(px(4.))
        .child(name)
        .when(missing, |l| {
            l.child(div().flex_none().text_color(muted).child("(missing)"))
        });
    let trigger = Button::new("text-font")
        .outline()
        .small()
        .w(px(FONT_BUTTON_WIDTH))
        .dropdown_caret(true)
        .child(label);
    let picker = fields.fonts.clone();
    let opener = fields.fonts.downgrade();
    let search_focus = fields.fonts.read(cx).search.focus_handle(cx);
    Popover::new("text-font-list")
        .anchor(Anchor::TopRight)
        .track_focus(&search_focus)
        .on_open_change(move |open, window, cx| {
            if *open {
                opener
                    .update(cx, |picker, cx| picker.opened(window, cx))
                    .ok();
            }
        })
        .trigger(trigger)
        .content(move |_, _, cx| {
            let popover = cx.entity().downgrade();
            picker.update(cx, |picker, _| picker.popover = Some(popover));
            picker.clone()
        })
}

pub(super) type OnFont = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;

/// The font list: a search field over every installed family (recently
/// used first), each drawn in its own font. Virtualised, so only the rows
/// in view are laid out.
pub(super) struct FontPicker {
    search: Entity<InputState>,
    /// The installed families, recent ones first (see [`font_rows`]).
    rows: Vec<SharedString>,
    recent_count: usize,
    /// What the list shows: `None` for the default font, then the
    /// families matching the search.
    shown: Vec<Option<SharedString>>,
    item_sizes: Rc<Vec<Size<Pixels>>>,
    /// The row Enter picks, in `shown`.
    highlighted: usize,
    /// The family shown as chosen: `Some(None)` is the default font,
    /// `None` mixed.
    current: Option<Option<String>>,
    scroll: VirtualListScrollHandle,
    on_pick: OnFont,
    /// The popover showing the list, while one is.
    popover: Option<WeakEntity<PopoverState>>,
    _subscription: Subscription,
}

impl FontPicker {
    fn new(
        on_pick: impl Fn(Option<String>, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search fonts"));
        let subscription =
            cx.subscribe_in(&search, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.refilter(cx),
                InputEvent::PressEnter { .. } => this.pick(this.highlighted, window, cx),
                _ => {}
            });
        FontPicker {
            search,
            rows: Vec::new(),
            recent_count: 0,
            shown: Vec::new(),
            item_sizes: Rc::new(Vec::new()),
            highlighted: 0,
            current: None,
            scroll: VirtualListScrollHandle::new(),
            on_pick: Rc::new(on_pick),
            popover: None,
            _subscription: subscription,
        }
    }

    fn set_current(&mut self, current: Option<Option<String>>, cx: &mut Context<Self>) {
        if self.current != current {
            self.current = current;
            cx.notify();
        }
    }

    /// Reads the installed families as the list opens (fonts installed
    /// since are then listed and drawn), clears the search and brings the
    /// chosen family into view.
    fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = window.text_system().all_font_names();
        refresh_installed_fonts(&names);
        let recent = if cx.has_global::<AppState>() {
            cx.global::<AppState>().settings.recent_fonts.clone()
        } else {
            Vec::new()
        };
        (self.rows, self.recent_count) = font_rows(names, &recent);
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refilter(cx);
        let current = self
            .shown
            .iter()
            .position(|row| self.is_current(row.as_deref()));
        if let Some(ix) = current {
            self.highlighted = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
    }

    /// Shows the rows matching the search; the first is highlighted.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        let default = query.trim().is_empty()
            || DEFAULT_FONT
                .to_lowercase()
                .contains(&query.trim().to_lowercase());
        self.shown = default
            .then_some(None)
            .into_iter()
            .chain(
                filter_fonts(&self.rows, &query)
                    .into_iter()
                    .map(|ix| Some(self.rows[ix].clone())),
            )
            .collect();
        self.item_sizes = Rc::new(vec![
            size(px(FONT_LIST_WIDTH), px(FONT_ROW_HEIGHT));
            self.shown.len()
        ]);
        self.highlighted = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn is_current(&self, row: Option<&str>) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.as_deref() == row)
    }

    /// Moves the highlight by `by` rows.
    fn move_highlight(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.shown.is_empty() {
            return;
        }
        let last = self.shown.len() as isize - 1;
        self.highlighted = (self.highlighted as isize + by).clamp(0, last) as usize;
        self.scroll
            .scroll_to_item(self.highlighted, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Picks row `ix` of the list and closes it.
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.shown.get(ix) else {
            return;
        };
        let family = row.as_ref().map(|name| name.to_string());
        let on_pick = self.on_pick.clone();
        // Outside this update, so the caller may read or update the list.
        window.defer(cx, move |window, cx| on_pick(family, window, cx));
        self.close(window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popover) = self.popover.take().and_then(|p| p.upgrade()) {
            window.defer(cx, move |window, cx| {
                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
            });
        }
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.shown[ix];
        let current = self.is_current(row.as_deref());
        let highlighted = ix == self.highlighted;
        let hover = theme.list_hover;
        // A line under the recently used families, while all are listed.
        let last_recent = row.is_some()
            && self.shown.len() == self.rows.len() + 1
            && self.recent_count > 0
            && ix == self.recent_count;
        let name = match row {
            Some(family) => div()
                .min_w_0()
                .truncate()
                .font_family(family.clone())
                .child(family.clone()),
            None => div().child(DEFAULT_FONT),
        };
        div()
            .id(("font-row", ix))
            .h(px(FONT_ROW_HEIGHT))
            .px(px(8.))
            .flex()
            .items_center()
            .text_size(px(FONT_ROW_TEXT_SIZE))
            .rounded(theme.radius)
            .when(current, |r| r.bg(theme.list_active))
            .when(!current && highlighted, |r| r.bg(hover))
            .when(!current, |r| r.hover(move |s| s.bg(hover)))
            .when(last_recent, |r| r.border_b_1().border_color(theme.border))
            .child(name)
            .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, window, cx)))
            .into_any_element()
    }
}

impl Render for FontPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let rows = self.shown.len().min(FONT_LIST_ROWS);
        let list = if self.shown.is_empty() {
            div()
                .py(px(8.))
                .text_color(muted)
                .child("No fonts match")
                .into_any_element()
        } else {
            div()
                .h(px(rows as f32 * FONT_ROW_HEIGHT))
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "font-list",
                        self.item_sizes.clone(),
                        |this, range, _, cx| range.map(|ix| this.render_row(ix, cx)).collect(),
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                )
                .into_any_element()
        };
        v_flex()
            .w(px(FONT_LIST_WIDTH))
            .gap(px(6.))
            .child(
                div()
                    // The field keeps Escape and the arrows for itself.
                    .on_action(cx.listener(|this, _: &Escape, window, cx| this.close(window, cx)))
                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                        cx.stop_propagation();
                        this.move_highlight(-1, cx);
                    }))
                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                        cx.stop_propagation();
                        this.move_highlight(1, cx);
                    }))
                    .child(
                        Input::new(&self.search).small().prefix(
                            ComponentIcon::new(IconName::Search)
                                .small()
                                .text_color(muted),
                        ),
                    ),
            )
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        apply_to_objects, commit_size, edit_session, filter_fonts, font_missing, font_rows,
        merge_summaries, size_display, step_size, summarize_objects, typed_size, wheel_steps,
        SizeSteps, TextChange, SIZE_PRESETS, WHEEL_STEP_PX,
    };
    use crate::editor::text_edit::Toggle;
    use crate::editor::{GestureOwner, SessionGesture};
    use gpui_kit::SharedString;
    use mindweave_core::geometry::Frame;
    use mindweave_core::{
        builtin_libraries, DocumentSession, HAlign, Label, LibraryRegistry, Mixed, RichText,
        RunStyle, Shape, StyleChange, VAlign,
    };

    fn text_box(s: &str, style: RunStyle) -> Shape {
        let mut shape = Shape::new_text_box((0., 0.));
        let label = shape.label.as_mut().unwrap();
        label.text = RichText::new(style.clone(), HAlign::Left);
        label.text.insert(0, s, &style);
        shape
    }

    fn rectangle(label: Option<&str>) -> Shape {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        let mut shape = Shape::new(def, "basic-shapes", Frame::default());
        if let Some(s) = label {
            let mut l = Label::new_shape_label();
            let style = l.text.style_at(0);
            l.text.insert(0, s, &style);
            shape.label = Some(l);
        }
        shape
    }

    #[test]
    fn a_family_that_is_not_installed_is_marked_missing() {
        let installed: std::collections::HashSet<String> =
            ["Georgia", "Menlo"].map(String::from).into();
        let missing = |font: Mixed<Option<String>>| font_missing(&font, &installed);
        assert!(!missing(Mixed::Same(Some("Georgia".into()))));
        assert!(missing(Mixed::Same(Some("Nope Sans".into()))));
        // The default font, or several families: nothing to mark.
        assert!(!missing(Mixed::Same(None)));
        assert!(!missing(Mixed::Mixed));
    }

    #[test]
    fn summaries_merge_across_objects() {
        let a = text_box("one", RunStyle::default());
        let mut b = text_box(
            "two",
            RunStyle {
                bold: true,
                ..RunStyle::default()
            },
        );
        let one = summarize_objects(&[&a]).unwrap();
        assert_eq!(one.style.bold, Mixed::Same(false));
        assert_eq!(one.style.size, Mixed::Same(16.));
        assert_eq!(one.valign, None, "text boxes have no vertical alignment");

        let both = summarize_objects(&[&a, &b]).unwrap();
        assert_eq!(both.style.bold, Mixed::Mixed);
        assert_eq!(both.style.size, Mixed::Same(16.));
        assert_eq!(both.style.align, Mixed::Same(HAlign::Left));
        assert_eq!(both.targets, vec![a.id, b.id]);

        // Different sizes are mixed; steps start from the first text's.
        b.label
            .as_mut()
            .unwrap()
            .text
            .apply_all(&StyleChange::Size(30.));
        let both = summarize_objects(&[&b, &a]).unwrap();
        assert_eq!(both.style.size, Mixed::Mixed);
        assert_eq!(both.base_size, 30.);

        // Unlabelled shapes are skipped; labels bring vertical alignment.
        let plain = rectangle(None);
        let labelled = rectangle(Some("hi"));
        assert!(summarize_objects(&[&plain]).is_none());
        let s = summarize_objects(&[&plain, &a, &labelled]).unwrap();
        assert_eq!(s.targets, vec![a.id, labelled.id]);
        assert_eq!(s.valign, Some(Mixed::Same(VAlign::Middle)));
        assert_eq!(s.style.size, Mixed::Mixed, "labels default to 14");
        assert_eq!(s.style.align, Mixed::Mixed, "labels default to centre");
    }

    #[test]
    fn whole_object_changes_set_one_property_as_one_undo_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("t.mindweave")).unwrap();
        let mut boxed = text_box("ab", RunStyle::default());
        // Per-word styles stay: "a" is italic.
        boxed
            .label
            .as_mut()
            .unwrap()
            .text
            .apply(0..1, &StyleChange::Italic(true));
        let labelled = rectangle(Some("hi"));
        let plain = rectangle(None);
        let ids = [boxed.id, labelled.id, plain.id];
        s.add_shapes(vec![boxed.clone(), labelled.clone(), plain.clone()]);
        let label = |s: &DocumentSession, id| s.find_shape(id).unwrap().label.clone();

        apply_to_objects(
            &mut s,
            &ids,
            &TextChange::Style(StyleChange::Font(Some("Georgia".into()))),
        );
        let text = label(&s, boxed.id).unwrap().text;
        assert!(text.style_at(1).italic, "other styles are kept");
        assert_eq!(text.style_at(2).font.as_deref(), Some("Georgia"));
        let l = label(&s, labelled.id).unwrap().text;
        assert_eq!(l.style_at(1).font.as_deref(), Some("Georgia"));
        assert!(
            label(&s, plain.id).is_none(),
            "unlabelled shapes are skipped"
        );

        // A toggle turns bold on everywhere unless all of it is bold.
        apply_to_objects(&mut s, &ids, &TextChange::Toggle(Toggle::Bold));
        assert!(label(&s, boxed.id).unwrap().text.style_at(2).bold);
        apply_to_objects(&mut s, &ids, &TextChange::Toggle(Toggle::Bold));
        assert!(!label(&s, labelled.id).unwrap().text.style_at(1).bold);

        apply_to_objects(&mut s, &ids, &TextChange::Align(HAlign::Right));
        assert_eq!(
            label(&s, boxed.id).unwrap().text.paragraphs[0].align,
            HAlign::Right
        );
        // Vertical alignment: shape labels only.
        apply_to_objects(&mut s, &ids, &TextChange::VAlign(VAlign::Bottom));
        assert_eq!(label(&s, labelled.id).unwrap().valign, VAlign::Bottom);
        let unchanged = boxed.label.as_ref().unwrap().valign;
        assert_eq!(label(&s, boxed.id).unwrap().valign, unchanged);

        // One undo step per change.
        for _ in 0..4 {
            assert!(s.undo());
        }
        let text = label(&s, boxed.id).unwrap().text;
        assert_eq!(text.style_at(2).font.as_deref(), Some("Georgia"));
        assert!(s.undo());
        assert_eq!(label(&s, boxed.id).unwrap().text.style_at(2).font, None);
    }

    #[test]
    fn merging_keeps_only_shared_values() {
        let a = text_box("a", RunStyle::default()).label.unwrap().text;
        let b = text_box(
            "b",
            RunStyle {
                color: Some("#FF0000".into()),
                ..RunStyle::default()
            },
        )
        .label
        .unwrap()
        .text;
        let merged = merge_summaries(a.summary(0..1), &b.summary(0..1));
        assert_eq!(merged.color, Mixed::Mixed);
        assert_eq!(merged.font, Mixed::Same(None));
        assert_eq!(merged.italic, Mixed::Same(false));
        let mixed = merge_summaries(merged.clone(), &a.summary(0..1));
        assert_eq!(mixed.color, Mixed::Mixed, "mixed stays mixed");
    }

    #[test]
    fn the_wheel_steps_by_one_per_notch_and_accumulates_trackpad_pixels() {
        // A notch (one line, scaled to WHEEL_STEP_PX) is one step; up grows.
        assert_eq!(wheel_steps(0., WHEEL_STEP_PX), (1, 0.));
        assert_eq!(wheel_steps(0., -WHEEL_STEP_PX), (-1, 0.));
        assert_eq!(wheel_steps(0., 3. * WHEEL_STEP_PX), (3, 0.));
        // Small trackpad deltas add up before stepping.
        let (steps, carry) = wheel_steps(0., WHEEL_STEP_PX * 0.6);
        assert_eq!(steps, 0);
        let (steps, carry) = wheel_steps(carry, WHEEL_STEP_PX * 0.6);
        assert_eq!(steps, 1);
        assert!((carry - WHEEL_STEP_PX * 0.2).abs() < 1e-4);
        // A change of direction does not jump.
        let (steps, _) = wheel_steps(WHEEL_STEP_PX * 0.5, -WHEEL_STEP_PX * 0.6);
        assert_eq!(steps, 0);
    }

    #[test]
    fn sizes_step_and_clamp() {
        assert_eq!(step_size(14., 1), 15.);
        assert_eq!(step_size(14., -3), 11.);
        assert_eq!(step_size(1., -1), 1.);
        assert_eq!(step_size(1000., 2), 1000.);
        assert_eq!(step_size(12.5, 1), 13.5);
        // Typed sizes: numbers only, clamped to 1..=1000; unchanged → none.
        assert_eq!(typed_size("24", Some(16.)), Some(24.));
        assert_eq!(typed_size(" 0 ", Some(16.)), Some(1.));
        assert_eq!(typed_size("5000", None), Some(1000.));
        assert_eq!(typed_size("16", Some(16.)), None);
        assert_eq!(typed_size("abc", Some(16.)), None);
        assert_eq!(typed_size("", None), None);
    }

    #[test]
    fn the_size_field_shows_what_was_applied_after_a_commit() {
        // "5000" applies 1000, and the field then shows 1000 (it keeps focus
        // after Enter, so the per-frame sync would not correct it).
        let applied = typed_size("5000", Some(16.)).unwrap();
        assert_eq!(size_display(Some(&Mixed::Same(applied))), (Some(1000.), ""));
        // "abc" applies nothing: the field goes back to the text's size.
        assert_eq!(typed_size("abc", Some(16.)), None);
        assert_eq!(size_display(Some(&Mixed::Same(16.))), (Some(16.), ""));
        assert_eq!(size_display(Some(&Mixed::Mixed)), (None, "Mixed"));
        assert_eq!(size_display(None), (None, ""));
    }

    #[test]
    fn consecutive_size_steps_are_one_undo_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("w.mindweave")).unwrap();
        let boxed = text_box("ab", RunStyle::default());
        let ids = [boxed.id];
        s.add_shapes(vec![boxed.clone()]);
        let size = |s: &DocumentSession| {
            s.find_shape(boxed.id)
                .unwrap()
                .label
                .as_ref()
                .unwrap()
                .text
                .style_at(1)
                .size
        };
        let mut g = SessionGesture::default();
        assert!(g.begin(&mut s, GestureOwner::Size));
        for v in [17., 18., 19.] {
            apply_to_objects(&mut s, &ids, &TextChange::Style(StyleChange::Size(v)));
        }
        assert!(g.end(&mut s, GestureOwner::Size));
        assert_eq!(size(&s), 19.);
        assert!(s.undo());
        assert_eq!(size(&s), 16.);
    }

    #[test]
    fn a_commit_tells_the_field_what_to_show() {
        // "5000" applies 1000 and the field shows 1000.
        let mut applied = None;
        let shown = commit_size("5000", true, Some(16.), |size| {
            applied = size;
            Some(Mixed::Same(size.unwrap_or(16.)))
        });
        assert_eq!(applied, Some(1000.));
        assert_eq!(shown, (Some(1000.), ""));
        // "abc" applies nothing and the field goes back to the size.
        let shown = commit_size("abc", true, Some(16.), |size| {
            assert_eq!(size, None);
            Some(Mixed::Same(16.))
        });
        assert_eq!(shown, (Some(16.), ""));
        // Text only shown, not typed, is never applied.
        let shown = commit_size("20", false, Some(16.), |size| {
            assert_eq!(size, None);
            Some(Mixed::Mixed)
        });
        assert_eq!(shown, (None, "Mixed"));
    }

    #[test]
    fn an_unrelated_edit_ends_the_size_steps_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("u.mindweave")).unwrap();
        let boxed = text_box("ab", RunStyle::default());
        let ids = [boxed.id];
        s.add_shapes(vec![boxed.clone()]);
        let style = |s: &DocumentSession| {
            s.find_shape(boxed.id)
                .unwrap()
                .label
                .as_ref()
                .unwrap()
                .text
                .style_at(1)
        };
        let mut g = SessionGesture::default();
        let mut steps = SizeSteps::default();
        let size_to = |v: f32| {
            move |s: &mut DocumentSession| {
                apply_to_objects(s, &ids, &TextChange::Style(StyleChange::Size(v)))
            }
        };
        // Two ↑ clicks: one group.
        for v in [17., 18.] {
            assert!(steps.begin(&mut g, &mut s).is_some());
            edit_session(&mut steps, &mut g, &mut s, true, size_to(v));
        }
        // Then Bold, within the idle time: its own step.
        edit_session(&mut steps, &mut g, &mut s, false, |s| {
            apply_to_objects(s, &ids, &TextChange::Toggle(Toggle::Bold))
        });
        assert!(style(&s).bold && style(&s).size == 18.);
        assert!(s.undo());
        assert!(!style(&s).bold);
        assert_eq!(style(&s).size, 18.);
        assert!(s.undo());
        assert_eq!(style(&s).size, 16.);
        // Nothing is left holding the gesture.
        assert!(g.begin(&mut s, GestureOwner::Pointer));
    }

    #[test]
    fn size_presets() {
        assert_eq!(SIZE_PRESETS, [8., 10., 12., 14., 18., 24., 32., 48., 72.]);
    }

    #[test]
    fn font_rows_put_recent_installed_fonts_first() {
        let installed = [
            "Menlo", "Arial", "georgia", "Arial", "Zapfino", "Avenir", ".SF NS",
        ]
        .map(String::from)
        .to_vec();
        let recent = ["Zapfino", "Gone Font", "Arial"].map(String::from);
        let (rows, recent_count) = font_rows(installed.clone(), &recent);
        // Recent (still installed) first, then the rest sorted ignoring
        // case, each once.
        assert_eq!(
            rows,
            ["Zapfino", "Arial", "Avenir", "georgia", "Menlo"].map(SharedString::from)
        );
        assert_eq!(recent_count, 2);
        let (rows, recent_count) = font_rows(installed, &[]);
        assert_eq!(recent_count, 0);
        assert_eq!(rows[0], "Arial");
    }

    #[test]
    fn font_search_ignores_case_and_spaces() {
        let rows: Vec<_> = ["Arial", "Avenir Next", "Georgia"].map(Into::into).to_vec();
        assert_eq!(filter_fonts(&rows, ""), [0, 1, 2]);
        assert_eq!(filter_fonts(&rows, "  AV "), [1]);
        assert_eq!(filter_fonts(&rows, "i"), [0, 1, 2]);
        assert!(filter_fonts(&rows, "zzz").is_empty());
    }
}
