//! The Properties panel for selected shapes: Shape, Transform, Style and
//! Shape properties sections.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::geometry::{Frame, Rect};
use mindweave_core::{
    resize, selection_bounds, translate, DocumentSession, Handle, LibraryRegistry, ParamDef, Shape,
};
use uuid::Uuid;

use super::properties_panel::{
    field_row, FIELD_GAP, NUMBER_INPUT_WIDTH, RESET_WIDTH, ROW_HEIGHT, SWATCH_SIZE,
};
use super::{hex_to_hsla, hsla_to_hex, parse_number, section_label, EditorWindow, SMALL_TEXT_SIZE};
use crate::libraries::Libraries;

/// Stroke widths run from 0 to this, in steps of [`STROKE_WIDTH_STEP`].
const STROKE_WIDTH_MAX: f32 = 100.0;
const STROKE_WIDTH_STEP: f32 = 0.5;
/// Space above each section heading after the first.
const SECTION_GAP: f32 = 12.;

/// A value shared by every selected shape, or `Mixed` when they differ.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value<T> {
    Same(T),
    Mixed,
}

impl<T> Value<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Value<U> {
        match self {
            Value::Same(v) => Value::Same(f(v)),
            Value::Mixed => Value::Mixed,
        }
    }
}

/// The value all of `values` share; `None` when there are none.
pub(super) fn common<T: PartialEq>(values: impl IntoIterator<Item = T>) -> Option<Value<T>> {
    let mut values = values.into_iter();
    let first = values.next()?;
    Some(if values.all(|v| v == first) {
        Value::Same(first)
    } else {
        Value::Mixed
    })
}

/// The number all of `values` share as the panel shows them (`shown`
/// turns a value into the unit its field shows, then [`format_number`]
/// rounds it), so float noise below the shown precision is not "Mixed".
/// The first value stands for them all. `None` when there are none.
pub(super) fn common_number(
    values: impl IntoIterator<Item = f32>,
    shown: fn(f32) -> f32,
) -> Option<Value<f32>> {
    let mut values = values.into_iter();
    let first = values.next()?;
    let key = shown_hundredths(shown(first));
    Some(if values.all(|v| shown_hundredths(shown(v)) == key) {
        Value::Same(first)
    } else {
        Value::Mixed
    })
}

/// `value` in whole hundredths: two values format alike (see
/// [`format_number`]) exactly when these are equal, without building the
/// strings.
fn shown_hundredths(value: f32) -> f32 {
    (value * 100.0).round()
}

/// An opacity (0–1) as the percentage the panel shows.
pub(super) fn opacity_percent(opacity: f32) -> f32 {
    opacity * 100.0
}

/// A typed percentage as an opacity, clamped to 0–1.
pub(super) fn opacity_from_percent(percent: f32) -> f32 {
    (percent / 100.0).clamp(0.0, 1.0)
}

/// `value` clamped to `min..=max` and rounded to a whole number of
/// `step`s above `min` (no rounding when `step` is not positive).
pub(super) fn snap(value: f32, min: f32, max: f32, step: f32) -> f32 {
    let value = value.clamp(min, max);
    if step > 0.0 {
        (min + ((value - min) / step).round() * step).clamp(min, max)
    } else {
        value
    }
}

/// A number as a field shows it: at most two decimals, no trailing zeros.
pub(super) fn format_number(value: f32) -> String {
    let rounded = shown_hundredths(value) / 100.0;
    // `+ 0.0` turns -0 into 0.
    format!("{}", rounded + 0.0)
}

/// One edge or size of the Transform section's box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BoundsField {
    X,
    Y,
    W,
    H,
}

/// The box the Transform section shows: one shape's own frame box, or the
/// axis-aligned bounds of several.
pub(super) fn shown_bounds(frames: &[Frame]) -> Option<Rect> {
    match frames {
        [f] => Some(Rect {
            x: f.x,
            y: f.y,
            w: f.w,
            h: f.h,
        }),
        _ => selection_bounds(frames),
    }
}

/// `frames` after setting `field` of their [`shown_bounds`] to `value`.
/// One frame takes the value itself (sizes no less than 0). Several move
/// together (X, Y) or scale from the box's left or top edge as a resize
/// of its right or bottom handle would (W, H).
pub(super) fn edit_bounds(frames: &[Frame], field: BoundsField, value: f32) -> Vec<Frame> {
    if let [f] = frames {
        let mut f = *f;
        match field {
            BoundsField::X => f.x = value,
            BoundsField::Y => f.y = value,
            BoundsField::W => f.w = value.max(0.0),
            BoundsField::H => f.h = value.max(0.0),
        }
        return vec![f];
    }
    let Some(b) = selection_bounds(frames) else {
        return Vec::new();
    };
    let (right, bottom) = (b.x + b.w, b.y + b.h);
    match field {
        BoundsField::X => translate(frames, value - b.x, 0.0),
        BoundsField::Y => translate(frames, 0.0, value - b.y),
        BoundsField::W => resize(
            frames,
            Handle::E,
            (right, bottom),
            (b.x + value.max(0.0), bottom),
            false,
            false,
        ),
        BoundsField::H => resize(
            frames,
            Handle::S,
            (right, bottom),
            (right, b.y + value.max(0.0)),
            false,
            false,
        ),
    }
}

/// What the panel shows for the selected shapes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Summary {
    pub count: usize,
    /// "Library · Shape", "Missing library", or "Mixed" for different kinds.
    pub title: String,
    pub bounds: Rect,
    pub rotation: Value<f32>,
    pub fill: Value<Option<String>>,
    pub stroke: Value<Option<String>>,
    pub stroke_width: Value<f32>,
    pub opacity: Value<f32>,
    /// The shape's declared parameters, when every selected shape is the
    /// same library shape.
    pub params: Vec<(ParamDef, Value<f32>)>,
}

/// The panel's view of `shapes`; `None` when there are none.
pub(super) fn summarize(shapes: &[&Shape], libs: &LibraryRegistry) -> Option<Summary> {
    let first = *shapes.first()?;
    let frames: Vec<Frame> = shapes.iter().map(|s| s.frame).collect();
    let same_kind = shapes
        .iter()
        .all(|s| s.library == first.library && s.shape == first.shape);
    let def = libs.shape(&first.library, &first.shape);
    let title = if !same_kind {
        "Mixed".to_string()
    } else {
        match (libs.libraries().iter().find(|l| l.id == first.library), def) {
            (None, _) => "Missing library".to_string(),
            (Some(lib), None) => format!("{} · Missing shape", lib.name),
            (Some(lib), Some(def)) => format!("{} · {}", lib.name, def.name),
        }
    };
    let params = match def.filter(|_| same_kind) {
        Some(def) => def
            .params
            .iter()
            .filter_map(|p| {
                let values = shapes.iter().map(|s| s.param(&p.key).unwrap_or(p.default));
                Some((p.clone(), common_number(values, same)?))
            })
            .collect(),
        None => Vec::new(),
    };
    // Colours are compared borrowed and copied once.
    let color = |f: fn(&Shape) -> Option<&str>| {
        common(shapes.iter().map(|s| f(s))).map(|v| v.map(|c| c.map(str::to_owned)))
    };
    Some(Summary {
        count: shapes.len(),
        title,
        bounds: shown_bounds(&frames)?,
        rotation: common_number(frames.iter().map(|f| f.rotation), same)?,
        fill: color(|s| s.style.fill.as_deref())?,
        stroke: color(|s| s.style.stroke.as_deref())?,
        stroke_width: common_number(shapes.iter().map(|s| s.style.stroke_width), same)?,
        opacity: common_number(shapes.iter().map(|s| s.style.opacity), opacity_percent)?,
        params,
    })
}

/// A number shown as it is stored.
fn same(value: f32) -> f32 {
    value
}

/// What the shape panel was last synced from: the content revision, the
/// text-box layout generation (laid-out sizes the revision does not track)
/// and the selection (its ids hashed, so comparing is cheap). Equal keys
/// mean nothing the panel shows has changed, up to a 64-bit hash collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SyncKey {
    revision: u64,
    layout: u64,
    selection: u64,
}

impl SyncKey {
    pub(super) fn new(revision: u64, selection: &[Uuid]) -> Self {
        let mut hasher = DefaultHasher::new();
        selection.hash(&mut hasher);
        Self {
            revision,
            layout: 0,
            selection: hasher.finish(),
        }
    }

    /// The key with the session's text-box layout generation.
    pub(super) fn with_layout(self, layout: u64) -> Self {
        Self { layout, ..self }
    }

    /// Whether the panel synced at `last` (`None`: never, or a field was
    /// left alone then) needs syncing for this key.
    pub(super) fn needs_sync(self, last: Option<Self>) -> bool {
        last != Some(self)
    }
}

/// A numeric field of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NumberKind {
    Bounds(BoundsField),
    /// Degrees.
    Rotation,
    StrokeWidth,
    /// Shown as a percentage.
    Opacity,
    /// A shape parameter, by key.
    Param(String),
}

/// The fields every selection has, in panel order.
const FIXED_NUMBERS: [NumberKind; 7] = [
    NumberKind::Bounds(BoundsField::X),
    NumberKind::Bounds(BoundsField::Y),
    NumberKind::Bounds(BoundsField::W),
    NumberKind::Bounds(BoundsField::H),
    NumberKind::Rotation,
    NumberKind::StrokeWidth,
    NumberKind::Opacity,
];

impl Summary {
    /// The value field `kind` shows; `None` for a parameter the shapes do
    /// not declare.
    fn shown(&self, kind: &NumberKind) -> Option<Value<f32>> {
        let b = &self.bounds;
        Some(match kind {
            NumberKind::Bounds(BoundsField::X) => Value::Same(b.x),
            NumberKind::Bounds(BoundsField::Y) => Value::Same(b.y),
            NumberKind::Bounds(BoundsField::W) => Value::Same(b.w),
            NumberKind::Bounds(BoundsField::H) => Value::Same(b.h),
            NumberKind::Rotation => self.rotation.clone(),
            NumberKind::StrokeWidth => self.stroke_width.clone(),
            NumberKind::Opacity => match self.opacity {
                Value::Same(o) => Value::Same(opacity_percent(o)),
                Value::Mixed => Value::Mixed,
            },
            NumberKind::Param(key) => self.params.iter().find(|(p, _)| p.key == *key)?.1.clone(),
        })
    }
}

/// The text field `kind` shows for `summary`: `None` for "Mixed" (an empty
/// field with that placeholder).
fn shown_text(summary: Option<&Summary>, kind: &NumberKind) -> Option<String> {
    match summary.and_then(|s| s.shown(kind)) {
        Some(Value::Same(v)) => Some(format_number(v)),
        Some(Value::Mixed) => None,
        None => Some(String::new()),
    }
}

/// Sets `value` (typed into field `kind`) on the shapes with `ids` as one
/// undo step. Values are clamped (and parameters snapped) to their range.
pub(super) fn apply_number(
    session: &mut DocumentSession,
    ids: &[Uuid],
    kind: &NumberKind,
    value: f32,
    libs: &LibraryRegistry,
) {
    match kind {
        NumberKind::Bounds(field) => {
            let wanted: HashSet<Uuid> = ids.iter().copied().collect();
            let (found, frames): (Vec<Uuid>, Vec<Frame>) = session
                .shapes_on_active_layer()
                .iter()
                .filter(|s| wanted.contains(&s.id))
                .map(|s| (s.id, s.frame))
                .unzip();
            let edited: HashMap<Uuid, Frame> = found
                .into_iter()
                .zip(edit_bounds(&frames, *field, value))
                .collect();
            session.update_shapes(ids, |s| {
                if let Some(f) = edited.get(&s.id) {
                    s.frame = *f;
                }
            });
        }
        NumberKind::Rotation => session.update_shapes(ids, |s| s.frame.rotation = value),
        NumberKind::StrokeWidth => {
            let width = snap(value, 0.0, STROKE_WIDTH_MAX, STROKE_WIDTH_STEP);
            session.update_shapes(ids, |s| s.style.stroke_width = width);
        }
        NumberKind::Opacity => {
            let opacity = opacity_from_percent(value);
            session.update_shapes(ids, |s| s.style.opacity = opacity);
        }
        NumberKind::Param(key) => session.update_shapes(ids, |s| {
            let def = libs.shape(&s.library, &s.shape);
            if let Some(p) = def.and_then(|d| d.params.iter().find(|p| p.key == *key)) {
                s.set_param(key, snap(value, p.min, p.max, p.step));
            }
        }),
    }
}

/// A shape colour: a swatch that opens a colour picker, the hex value (or
/// "None" / "Mixed"), and a button that removes the colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StyleColor {
    Fill,
    Stroke,
}

impl StyleColor {
    const ALL: [Self; 2] = [Self::Fill, Self::Stroke];

    fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Stroke => "Stroke",
        }
    }

    fn none_tooltip(self) -> &'static str {
        match self {
            Self::Fill => "No Fill",
            Self::Stroke => "No Stroke",
        }
    }

    fn of(self, summary: &Summary) -> &Value<Option<String>> {
        match self {
            Self::Fill => &summary.fill,
            Self::Stroke => &summary.stroke,
        }
    }

    fn slot(self, shape: &mut Shape) -> &mut Option<String> {
        match self {
            Self::Fill => &mut shape.style.fill,
            Self::Stroke => &mut shape.style.stroke,
        }
    }
}

struct NumberField {
    kind: NumberKind,
    input: Entity<InputState>,
    /// The text the field shows (`None`: empty with the "Mixed"
    /// placeholder); `None` outside until first shown.
    shows: Option<Option<String>>,
    /// The shapes the shown value belongs to. A commit edits these, so a
    /// value typed before the selection changed goes where it was meant.
    ids: Vec<Uuid>,
    _subscription: Subscription,
}

struct ColorField {
    which: StyleColor,
    state: Entity<ColorPickerState>,
    /// The colour the picker shows (`None`: no colour, or mixed); `None`
    /// outside until first shown.
    shows: Option<Option<Hsla>>,
    _subscription: Subscription,
}

/// The shape panel's controls, kept in step with the selection.
pub(super) struct ShapeFields {
    /// [`FIXED_NUMBERS`], then one per parameter of the selected shape.
    numbers: Vec<NumberField>,
    colors: Vec<ColorField>,
    /// The selection as last synced.
    summary: Option<Summary>,
    /// What the fields were last synced from; `None` forces a sync.
    synced: Option<SyncKey>,
}

impl ShapeFields {
    pub(super) fn new(window: &mut Window, cx: &mut Context<EditorWindow>) -> Self {
        let numbers = FIXED_NUMBERS
            .into_iter()
            .map(|kind| number_field(kind, window, cx))
            .collect();
        let colors = StyleColor::ALL
            .into_iter()
            .map(|which| {
                let state = cx.new(|cx| ColorPickerState::new(window, cx));
                let _subscription = cx.subscribe_in(
                    &state,
                    window,
                    move |this, state, event: &ColorPickerEvent, _, cx| {
                        // The picker never clears a colour itself; the
                        // remove button does that.
                        if let ColorPickerEvent::Change(Some(color)) = event {
                            this.begin_color_edit(state.read(cx).is_open());
                            this.set_style_color(which, Some(hsla_to_hex(*color)), cx);
                        }
                    },
                );
                ColorField {
                    which,
                    state,
                    shows: None,
                    _subscription,
                }
            })
            .collect();
        Self {
            numbers,
            colors,
            summary: None,
            synced: None,
        }
    }

    fn number(&self, kind: &NumberKind) -> Option<&NumberField> {
        self.numbers.iter().find(|f| f.kind == *kind)
    }

    fn color(&self, which: StyleColor) -> &ColorField {
        self.colors
            .iter()
            .find(|f| f.which == which)
            .expect("a field for every style colour")
    }
}

/// A number field that applies on Enter or blur.
fn number_field(
    kind: NumberKind,
    window: &mut Window,
    cx: &mut Context<EditorWindow>,
) -> NumberField {
    let input = cx.new(|cx| InputState::new(window, cx));
    let commits = kind.clone();
    let _subscription = cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            this.commit_shape_number(&commits, window, cx);
        }
    });
    NumberField {
        kind,
        input,
        shows: None,
        ids: Vec::new(),
        _subscription,
    }
}

/// Puts `text` (`None`: "Mixed") in `field`.
fn show_text(field: &mut NumberField, text: Option<String>, window: &mut Window, cx: &mut App) {
    let placeholder = if text.is_none() { "Mixed" } else { "" };
    let value = text.clone().unwrap_or_default();
    field.input.update(cx, |input, cx| {
        input.set_value(value, window, cx);
        input.set_placeholder(placeholder, window, cx);
    });
    field.shows = Some(text);
}

impl EditorWindow {
    /// The shapes of the active layer with one of `ids`, back to front.
    pub(super) fn shapes_among(&self, ids: &[Uuid]) -> Vec<&Shape> {
        if ids.is_empty() {
            return Vec::new();
        }
        let ids: HashSet<&Uuid> = ids.iter().collect();
        self.session
            .shapes_on_active_layer()
            .iter()
            .filter(|s| ids.contains(&s.id))
            .collect()
    }

    fn summarize_shapes(&self, ids: &[Uuid], cx: &App) -> Option<Summary> {
        summarize(&self.shapes_among(ids), &cx.global::<Libraries>().0)
    }

    /// Brings the shape panel's controls up to date with the selection,
    /// leaving alone a field being typed in and an open colour picker.
    /// Runs every frame, so it does nothing unless the content or the
    /// selection changed since the last full sync.
    pub(super) fn sync_shape_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = SyncKey::new(self.session.revision(), &self.selection)
            .with_layout(self.session.layout_generation());
        if !key.needs_sync(self.properties.shape.synced) {
            return;
        }
        let summary = self.summarize_shapes(&self.selection, cx);
        self.sync_param_fields(summary.as_ref(), window, cx);
        let fields = &mut self.properties.shape;
        // A field left alone now is synced on a later frame.
        let mut left_alone = false;
        if summary.is_some() {
            for field in &mut fields.numbers {
                if field.input.read(cx).focus_handle(cx).is_focused(window) {
                    left_alone = true;
                    continue;
                }
                let text = shown_text(summary.as_ref(), &field.kind);
                if field.shows.as_ref() != Some(&text) || field.ids != self.selection {
                    show_text(field, text, window, cx);
                    field.ids.clone_from(&self.selection);
                }
            }
        }
        if let Some(summary) = &summary {
            for field in &mut fields.colors {
                let color = match field.which.of(summary) {
                    Value::Same(Some(hex)) => hex_to_hsla(hex),
                    _ => None,
                };
                // An open picker owns its colour until it closes.
                if field.state.read(cx).is_open() {
                    left_alone = true;
                } else if field.shows != Some(color) {
                    field.state.update(cx, |state, cx| match color {
                        Some(color) => state.set_value(color, window, cx),
                        None => state.clear_value(window, cx),
                    });
                    field.shows = Some(color);
                }
            }
        }
        fields.summary = summary;
        fields.synced = (!left_alone).then_some(key);
    }

    /// Whether one of the shape panel's colour pickers is open.
    pub(super) fn shape_picker_open(&self, cx: &App) -> bool {
        self.properties
            .shape
            .colors
            .iter()
            .any(|f| f.state.read(cx).is_open())
    }

    /// Gives the panel one field per parameter of the selected shape.
    fn sync_param_fields(
        &mut self,
        summary: Option<&Summary>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let wanted: Vec<String> = summary
            .map(|s| s.params.iter().map(|(p, _)| p.key.clone()).collect())
            .unwrap_or_default();
        let numbers = &mut self.properties.shape.numbers;
        let have = numbers.iter().filter_map(|f| match &f.kind {
            NumberKind::Param(key) => Some(key),
            _ => None,
        });
        if have.eq(wanted.iter()) {
            return;
        }
        numbers.retain(|f| !matches!(f.kind, NumberKind::Param(_)));
        for key in wanted {
            let field = number_field(NumberKind::Param(key), window, cx);
            self.properties.shape.numbers.push(field);
        }
    }

    /// Applies number field `kind` on Enter or blur, unless its text is
    /// unchanged; whatever happens, it then shows the stored value.
    fn commit_shape_number(
        &mut self,
        kind: &NumberKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.properties.shape.number(kind) else {
            return;
        };
        let text = field.input.read(cx).value().trim().to_string();
        let unchanged = field
            .shows
            .as_ref()
            .is_some_and(|shown| shown.as_deref().unwrap_or("") == text);
        let ids = field.ids.clone();
        if !unchanged {
            if let Some(value) = parse_number(&text) {
                let libs = cx.global::<Libraries>().0.clone();
                self.update_session(|s| apply_number(s, &ids, kind, value, &libs), cx);
            }
        }
        let text = shown_text(self.summarize_shapes(&ids, cx).as_ref(), kind);
        if let Some(field) = self
            .properties
            .shape
            .numbers
            .iter_mut()
            .find(|f| f.kind == *kind)
        {
            show_text(field, text, window, cx);
        }
    }

    /// Applies the shape number being typed, if any.
    pub(super) fn commit_focused_shape_number(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self
            .properties
            .shape
            .numbers
            .iter()
            .find(|f| f.input.read(cx).focus_handle(cx).is_focused(window))
            .map(|f| f.kind.clone());
        if let Some(kind) = focused {
            self.commit_shape_number(&kind, window, cx);
        }
    }

    /// Sets (or with `None` removes) the selected shapes' fill or stroke.
    fn set_style_color(
        &mut self,
        which: StyleColor,
        color: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let ids = self.selection.clone();
        self.update_session(
            |s| s.update_shapes(&ids, |shape| *which.slot(shape) = color.clone()),
            cx,
        );
    }
}

/// The panel for the selected shapes.
pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let fields = &editor.properties.shape;
    let muted = cx.theme().muted_foreground;
    let panel = v_flex()
        .id("shape-properties")
        .size_full()
        .overflow_y_scroll()
        .bg(cx.theme().sidebar)
        .px(px(12.))
        .pt(px(10.))
        .pb(px(12.))
        .gap(px(4.))
        .text_size(px(SMALL_TEXT_SIZE));
    let Some(summary) = &fields.summary else {
        return panel
            .child(section_label("Selection", cx))
            .child(div().text_color(muted).child("Nothing selected"));
    };

    let title = h_flex()
        .h(px(ROW_HEIGHT))
        .flex_none()
        .gap(px(FIELD_GAP))
        .child(div().min_w_0().truncate().child(summary.title.clone()))
        .when(summary.count > 1, |row| {
            row.child(
                div()
                    .flex_none()
                    .text_color(muted)
                    .child(format!("{} shapes", summary.count)),
            )
        });
    let pair = |a: BoundsField, b: BoundsField| {
        h_flex()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .gap(px(FIELD_GAP))
            .child(labelled_input(fields, a, muted))
            .child(labelled_input(fields, b, muted))
    };
    let number =
        |kind: NumberKind, suffix: Option<&'static str>| number_input(fields, &kind, suffix, muted);
    let params: Vec<AnyElement> = summary
        .params
        .iter()
        .map(|(p, _)| {
            field_row(
                p.label.clone(),
                None,
                number(NumberKind::Param(p.key.clone()), None),
            )
            .into_any_element()
        })
        .collect();

    panel
        .child(section_label("Shape", cx))
        .child(title)
        .child(section_label("Transform", cx).mt(px(SECTION_GAP)))
        .child(pair(BoundsField::X, BoundsField::Y))
        .child(pair(BoundsField::W, BoundsField::H))
        .child(field_row(
            "Rotation",
            None,
            number(NumberKind::Rotation, Some("°")),
        ))
        .child(section_label("Style", cx).mt(px(SECTION_GAP)))
        .child(color_row(editor, summary, StyleColor::Fill, cx))
        .child(color_row(editor, summary, StyleColor::Stroke, cx))
        .child(field_row(
            "Stroke width",
            None,
            number(NumberKind::StrokeWidth, Some("px")),
        ))
        .child(field_row(
            "Opacity",
            None,
            number(NumberKind::Opacity, Some("%")),
        ))
        .when(!params.is_empty(), |panel| {
            panel
                .child(section_label("Shape properties", cx).mt(px(SECTION_GAP)))
                .children(params)
        })
}

/// Half a row: an X, Y, W or H field with its letter inside.
fn labelled_input(fields: &ShapeFields, field: BoundsField, muted: Hsla) -> Div {
    let letter = match field {
        BoundsField::X => "X",
        BoundsField::Y => "Y",
        BoundsField::W => "W",
        BoundsField::H => "H",
    };
    div()
        .flex_1()
        .min_w_0()
        .when_some(fields.number(&NumberKind::Bounds(field)), |slot, field| {
            slot.child(
                Input::new(&field.input)
                    .small()
                    .prefix(div().text_color(muted).child(letter)),
            )
        })
}

/// A fixed-width number field with an optional unit.
fn number_input(
    fields: &ShapeFields,
    kind: &NumberKind,
    suffix: Option<&'static str>,
    muted: Hsla,
) -> Div {
    div()
        .w(px(NUMBER_INPUT_WIDTH))
        .when_some(fields.number(kind), |slot, field| {
            slot.child(
                Input::new(&field.input)
                    .small()
                    .when_some(suffix, |input, suffix| {
                        input.suffix(div().text_color(muted).child(suffix))
                    }),
            )
        })
}

fn color_row(
    editor: &EditorWindow,
    summary: &Summary,
    which: StyleColor,
    cx: &mut Context<EditorWindow>,
) -> AnyElement {
    let field = editor.properties.shape.color(which);
    let value = which.of(summary);
    let (label, set) = match value {
        Value::Same(Some(hex)) => (hex.clone(), true),
        Value::Same(None) => ("None".to_string(), false),
        Value::Mixed => ("Mixed".to_string(), false),
    };
    let removable = *value != Value::Same(None);
    let remove = div()
        .w(px(RESET_WIDTH))
        .flex()
        .justify_end()
        .when(removable, |slot| {
            slot.child(
                Button::new(("no-style-color", which as usize))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(which.none_tooltip())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_style_color(which, None, cx);
                    })),
            )
        });
    let picker = h_flex()
        .gap(px(2.))
        // "None" and "Mixed" are shown muted.
        .when(!set, |picker| {
            picker.text_color(cx.theme().muted_foreground)
        })
        .child(
            ColorPicker::new(&field.state)
                .with_size(px(SWATCH_SIZE))
                .label(label)
                .anchor(Anchor::TopRight),
        )
        .child(remove);
    field_row(which.label(), None, picker).into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        apply_number, common, common_number, edit_bounds, format_number, opacity_from_percent,
        opacity_percent, shown_bounds, shown_text, snap, summarize, BoundsField, NumberKind,
        SyncKey, Value,
    };
    use mindweave_core::geometry::{Frame, Rect};
    use mindweave_core::{builtin_libraries, DocumentSession, LibraryRegistry, Shape};
    use uuid::Uuid;

    fn frame(x: f32, y: f32, w: f32, h: f32) -> Frame {
        Frame {
            x,
            y,
            w,
            h,
            ..Frame::default()
        }
    }

    fn libs() -> LibraryRegistry {
        LibraryRegistry::new(builtin_libraries(), None).0
    }

    fn shape(libs: &LibraryRegistry, id: &str, f: Frame) -> Shape {
        Shape::new(libs.shape("basic-shapes", id).unwrap(), "basic-shapes", f)
    }

    #[test]
    fn common_values_are_same_or_mixed() {
        assert_eq!(common(Vec::<f32>::new()), None);
        assert_eq!(common([2.0]), Some(Value::Same(2.0)));
        assert_eq!(common([2.0, 2.0, 2.0]), Some(Value::Same(2.0)));
        assert_eq!(common([2.0, 3.0, 2.0]), Some(Value::Mixed));
        assert_eq!(common([None, None::<&str>]), Some(Value::Same(None)));
        assert_eq!(common([Some("#FF0000"), None]), Some(Value::Mixed));
    }

    #[test]
    fn numbers_that_show_alike_are_not_mixed() {
        let same = |a: f32, b: f32| common_number([a, b], |v| v) == Some(Value::Same(a));
        // Float noise below what the field shows.
        assert!(same(10.0, 10.000_001));
        assert!(same(0.1 + 0.2, 0.3));
        assert!(same(-0.001, 0.0));
        assert!(!same(10.0, 10.01));
        assert!(!same(1.0, 2.0));
        // Agrees with the shown text.
        for (a, b) in [(1.004, 1.0), (1.006, 1.0), (2.345, 2.35), (-0.004, 0.0)] {
            assert_eq!(same(a, b), format_number(a) == format_number(b), "{a} {b}");
        }
        assert_eq!(common_number([], |v| v), None);
        // Compared as shown: opacities as percentages.
        assert_eq!(
            common_number([0.5, 0.504], opacity_percent),
            Some(Value::Mixed)
        );
        assert_eq!(
            common_number([0.5, 0.500_000_1], opacity_percent),
            Some(Value::Same(0.5))
        );
    }

    #[test]
    fn summaries_ignore_float_noise() {
        let libs = libs();
        let a = shape(&libs, "rectangle", frame(0.0, 0.0, 10.0, 10.0));
        let mut b = a.clone();
        b.frame.rotation = 0.000_01;
        b.style.opacity = 1.0 - 1e-7;
        b.style.stroke_width = a.style.stroke_width + 1e-6;
        let both = summarize(&[&a, &b], &libs).unwrap();
        assert_eq!(both.rotation, Value::Same(0.0));
        assert_eq!(both.opacity, Value::Same(a.style.opacity));
        assert_eq!(both.stroke_width, Value::Same(a.style.stroke_width));
    }

    #[test]
    fn the_panel_resyncs_only_when_content_or_selection_change() {
        let [a, b] = [(); 2].map(|_| Uuid::new_v4());
        let key = SyncKey::new(7, &[a, b]);
        assert!(key.needs_sync(None));
        assert!(!key.needs_sync(Some(key)));
        assert!(!SyncKey::new(7, &[a, b]).needs_sync(Some(key)));
        assert!(SyncKey::new(8, &[a, b]).needs_sync(Some(key)));
        assert!(SyncKey::new(7, &[a]).needs_sync(Some(key)));
        assert!(SyncKey::new(7, &[b, a]).needs_sync(Some(key)));
        assert!(SyncKey::new(7, &[]).needs_sync(Some(key)));
        // A text box's laid-out size changing (H/W) resyncs too.
        let key = SyncKey::new(7, &[a]).with_layout(3);
        assert!(!SyncKey::new(7, &[a]).with_layout(3).needs_sync(Some(key)));
        assert!(SyncKey::new(7, &[a]).with_layout(4).needs_sync(Some(key)));
    }

    #[test]
    fn opacity_shows_as_a_percentage() {
        assert_eq!(opacity_percent(1.0), 100.0);
        assert_eq!(format_number(opacity_percent(0.6)), "60");
        assert_eq!(opacity_percent(0.0), 0.0);
        assert_eq!(opacity_from_percent(50.0), 0.5);
        assert_eq!(opacity_from_percent(100.0), 1.0);
        assert_eq!(opacity_from_percent(150.0), 1.0);
        assert_eq!(opacity_from_percent(-5.0), 0.0);
        assert_eq!(
            format_number(opacity_percent(opacity_from_percent(33.0))),
            "33"
        );
    }

    #[test]
    fn values_snap_to_their_range_and_step() {
        assert_eq!(snap(2.3, 0.0, 100.0, 0.5), 2.5);
        assert_eq!(snap(2.2, 0.0, 100.0, 0.5), 2.0);
        assert_eq!(snap(150.0, 0.0, 100.0, 0.5), 100.0);
        assert_eq!(snap(-1.0, 0.0, 100.0, 0.5), 0.0);
        assert_eq!(snap(5.6, 3.0, 12.0, 1.0), 6.0);
        assert_eq!(snap(0.37, 0.1, 0.9, 0.0), 0.37);
    }

    #[test]
    fn numbers_show_at_most_two_decimals() {
        assert_eq!(format_number(12.0), "12");
        assert_eq!(format_number(12.5), "12.5");
        assert_eq!(format_number(12.3456), "12.35");
        assert_eq!(format_number(-0.001), "0");
        assert_eq!(format_number(-4.25), "-4.25");
    }

    #[test]
    fn one_shape_edits_its_own_frame() {
        let f = Frame {
            rotation: 30.0,
            ..frame(10.0, 20.0, 30.0, 40.0)
        };
        assert_eq!(
            shown_bounds(&[f]),
            Some(Rect {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 40.0
            })
        );
        let edit = |field, v| edit_bounds(&[f], field, v)[0];
        assert_eq!(edit(BoundsField::X, 5.0), Frame { x: 5.0, ..f });
        assert_eq!(edit(BoundsField::Y, -5.0), Frame { y: -5.0, ..f });
        assert_eq!(edit(BoundsField::W, 50.0), Frame { w: 50.0, ..f });
        assert_eq!(edit(BoundsField::H, -1.0), Frame { h: 0.0, ..f });
        assert_eq!(shown_bounds(&[]), None);
        assert!(edit_bounds(&[], BoundsField::X, 1.0).is_empty());
    }

    #[test]
    fn several_shapes_edit_their_group_box() {
        let frames = [frame(0.0, 0.0, 10.0, 10.0), frame(30.0, 10.0, 10.0, 30.0)];
        assert_eq!(
            shown_bounds(&frames),
            Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 40.0,
                h: 40.0
            })
        );
        // X and Y move the group.
        let moved = edit_bounds(&frames, BoundsField::X, 100.0);
        assert_eq!(moved[0], frame(100.0, 0.0, 10.0, 10.0));
        assert_eq!(moved[1], frame(130.0, 10.0, 10.0, 30.0));
        let moved = edit_bounds(&frames, BoundsField::Y, -10.0);
        assert_eq!(moved[1], frame(30.0, 0.0, 10.0, 30.0));
        // W scales from the left edge; heights stay.
        let wide = edit_bounds(&frames, BoundsField::W, 80.0);
        assert_eq!(wide[0], frame(0.0, 0.0, 20.0, 10.0));
        assert_eq!(wide[1], frame(60.0, 10.0, 20.0, 30.0));
        // H scales from the top edge; widths stay.
        let tall = edit_bounds(&frames, BoundsField::H, 20.0);
        assert_eq!(tall[0], frame(0.0, 0.0, 10.0, 5.0));
        assert_eq!(tall[1], frame(30.0, 5.0, 10.0, 15.0));
        let b = shown_bounds(&tall).unwrap();
        assert_eq!((b.y, b.h), (0.0, 20.0));
    }

    #[test]
    fn summary_shows_shared_values_and_mixed_ones() {
        let libs = libs();
        let mut a = shape(&libs, "rectangle", frame(0.0, 0.0, 10.0, 10.0));
        let mut b = shape(&libs, "rectangle", frame(20.0, 0.0, 10.0, 20.0));
        a.style.opacity = 0.5;
        b.style.opacity = 0.5;
        b.style.fill = None;
        b.frame.rotation = 45.0;
        b.set_param("corner_radius", 4.0);

        let one = summarize(&[&a], &libs).unwrap();
        assert_eq!(one.count, 1);
        assert_eq!(one.title, "Basic Shapes · Rectangle");
        assert_eq!(one.rotation, Value::Same(0.0));
        assert_eq!(one.fill, Value::Same(a.style.fill.clone()));
        assert_eq!(one.params.len(), 1);
        assert_eq!(one.params[0].0.key, "corner_radius");
        assert_eq!(one.params[0].1, Value::Same(0.0));

        let both = summarize(&[&a, &b], &libs).unwrap();
        assert_eq!(both.count, 2);
        assert_eq!(both.title, "Basic Shapes · Rectangle");
        assert_eq!(both.opacity, Value::Same(0.5));
        assert_eq!(both.stroke, Value::Same(a.style.stroke.clone()));
        assert_eq!(both.fill, Value::Mixed);
        assert_eq!(both.rotation, Value::Mixed);
        assert_eq!(both.params[0].1, Value::Mixed);
        // The rotated shape's box counts by its turned corners.
        assert_eq!(both.bounds.x, 0.0);
        assert!(both.bounds.w > 30.0);

        assert_eq!(summarize(&[], &libs), None);
    }

    #[test]
    fn different_kinds_hide_shape_properties() {
        let libs = libs();
        let rect = shape(&libs, "rectangle", frame(0.0, 0.0, 10.0, 10.0));
        let ellipse = shape(&libs, "ellipse", frame(0.0, 0.0, 10.0, 10.0));
        let s = summarize(&[&rect, &ellipse], &libs).unwrap();
        assert_eq!(s.title, "Mixed");
        assert!(s.params.is_empty());
    }

    #[test]
    fn missing_libraries_and_shapes_are_named() {
        let libs = libs();
        let mut missing = shape(&libs, "rectangle", frame(0.0, 0.0, 10.0, 10.0));
        missing.library = "gone".into();
        let s = summarize(&[&missing], &libs).unwrap();
        assert_eq!(s.title, "Missing library");
        assert!(s.params.is_empty());
        missing.library = "basic-shapes".into();
        missing.shape = "gone".into();
        assert_eq!(
            summarize(&[&missing], &libs).unwrap().title,
            "Basic Shapes · Missing shape"
        );
    }

    #[test]
    fn each_applied_value_is_one_undo_step_on_every_shape() {
        let libs = libs();
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("a.mindweave")).unwrap();
        let mut a = shape(&libs, "rectangle", frame(0.0, 0.0, 10.0, 10.0));
        let b = shape(&libs, "rectangle", frame(30.0, 10.0, 10.0, 30.0));
        a.frame.rotation = 10.0;
        let ids = [a.id, b.id];
        s.add_shapes(vec![a, b]);
        let shapes = |s: &DocumentSession| s.shapes_on_active_layer().to_vec();

        let apply = |s: &mut DocumentSession, kind: NumberKind, v: f32| {
            apply_number(s, &ids, &kind, v, &libs);
        };
        apply(&mut s, NumberKind::Rotation, 30.0);
        assert!(shapes(&s).iter().all(|sh| sh.frame.rotation == 30.0));
        apply(&mut s, NumberKind::Opacity, 40.0);
        assert!(shapes(&s).iter().all(|sh| sh.style.opacity == 0.4));
        apply(&mut s, NumberKind::StrokeWidth, 2.3);
        assert!(shapes(&s).iter().all(|sh| sh.style.stroke_width == 2.5));
        // Clamped to the declared minimum, the default: nothing changes and
        // no step is recorded.
        apply(&mut s, NumberKind::Param("corner_radius".into()), -3.0);
        assert!(shapes(&s)
            .iter()
            .all(|sh| sh.param("corner_radius") == Some(0.0)));
        apply(&mut s, NumberKind::Param("corner_radius".into()), 6.0);
        assert!(shapes(&s)
            .iter()
            .all(|sh| sh.param("corner_radius") == Some(6.0)));
        apply(&mut s, NumberKind::Bounds(BoundsField::X), 100.0);
        let lib_summary = |s: &DocumentSession| {
            let shapes = s.shapes_on_active_layer();
            summarize(&shapes.iter().collect::<Vec<_>>(), &libs).unwrap()
        };
        assert_eq!(lib_summary(&s).bounds.x.round(), 100.0);

        // Undo goes back one value at a time.
        assert!(s.undo());
        assert!(s.undo());
        assert!(shapes(&s)
            .iter()
            .all(|sh| sh.param("corner_radius") == Some(0.0)));
        for _ in 0..2 {
            assert!(s.undo());
        }
        assert!(shapes(&s).iter().all(|sh| sh.frame.rotation == 30.0));
        assert!(s.undo());
        assert_eq!(shapes(&s)[0].frame.rotation, 10.0);
    }

    #[test]
    fn fields_show_values_or_mixed() {
        let libs = libs();
        let a = shape(&libs, "rectangle", frame(0.5, 0.0, 10.0, 10.0));
        let mut b = shape(&libs, "rectangle", frame(20.0, 0.0, 10.0, 20.0));
        b.style.opacity = 0.25;
        let one = summarize(&[&b], &libs);
        let both = summarize(&[&a, &b], &libs);
        let text = |s, kind| shown_text(s, &kind);
        assert_eq!(text(one.as_ref(), NumberKind::Opacity), Some("25".into()));
        assert_eq!(text(both.as_ref(), NumberKind::Opacity), None);
        assert_eq!(
            text(both.as_ref(), NumberKind::Bounds(BoundsField::X)),
            Some("0.5".into())
        );
        assert_eq!(
            text(both.as_ref(), NumberKind::Param("nope".into())),
            Some(String::new())
        );
        assert_eq!(text(None, NumberKind::Rotation), Some(String::new()));
    }
}
