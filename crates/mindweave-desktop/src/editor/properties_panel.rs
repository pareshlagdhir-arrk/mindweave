//! The Properties panel: the active page's canvas settings, and the
//! (always empty) selection.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, IconName, IndexPath, Sizable as _, Theme,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::{BackgroundPattern, CanvasSettings, DocumentSession};

use super::{hex_to_hsla, hsla_to_hex, parse_number, section_label, EditorWindow, SMALL_TEXT_SIZE};

const PATTERNS: [(BackgroundPattern, &str); 3] = [
    (BackgroundPattern::Lines, "Lines"),
    (BackgroundPattern::Dots, "Dots"),
    (BackgroundPattern::None, "None"),
];
const NUMBER_INPUT_WIDTH: f32 = 72.;
const FIELD_GAP: f32 = 6.;
const ROW_HEIGHT: f32 = 28.;
const SWATCH_SIZE: f32 = 14.;
/// Room kept for the colour reset button, so the hex text does not move
/// when it appears.
const RESET_WIDTH: f32 = 20.;

/// The pattern dropdown's items.
type PatternItems = Vec<&'static str>;

/// A numeric canvas setting, edited in a text field that applies on Enter
/// or blur and reverts to the stored value when the text is not a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NumberSetting {
    /// Line spacing (Lines) or dot spacing (Dots).
    GridSize,
    MajorEvery,
    DotSize,
}

impl NumberSetting {
    const ALL: [Self; 3] = [Self::GridSize, Self::MajorEvery, Self::DotSize];

    fn label(self, pattern: BackgroundPattern) -> &'static str {
        match (self, pattern) {
            (Self::GridSize, BackgroundPattern::Dots) => "Spacing",
            (Self::GridSize, _) => "Grid size",
            (Self::MajorEvery, _) => "Major line every",
            (Self::DotSize, _) => "Dot size",
        }
    }

    fn tooltip(self) -> Option<&'static str> {
        match self {
            Self::MajorEvery => Some("Every Nth grid line is drawn stronger; 0 turns this off"),
            _ => None,
        }
    }

    fn suffix(self) -> Option<&'static str> {
        match self {
            Self::GridSize | Self::DotSize => Some("px"),
            Self::MajorEvery => None,
        }
    }

    fn value(self, canvas: &CanvasSettings) -> f32 {
        match self {
            Self::GridSize => canvas.grid_size,
            Self::MajorEvery => canvas.major_every as f32,
            Self::DotSize => canvas.dot_size,
        }
    }

    /// `value` as the field shows it: whole numbers, except dot sizes,
    /// which keep their half pixel.
    fn text(self, value: f32) -> String {
        match self {
            Self::GridSize | Self::MajorEvery => format!("{}", value.round() as i64),
            Self::DotSize => format!("{value}"),
        }
    }

    fn apply(self, session: &mut DocumentSession, value: f32) {
        match self {
            Self::GridSize => session.set_grid_size(value),
            Self::MajorEvery => session.set_major_every(major_every_from(value)),
            Self::DotSize => session.set_dot_size(value),
        }
    }
}

/// A typed major-line count as a whole, non-negative number (saturating).
fn major_every_from(value: f32) -> u32 {
    value.round().max(0.) as u32
}

/// A canvas colour: a swatch that opens a colour picker, the hex value, and
/// a button that goes back to the theme's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ColorSetting {
    Background,
    Line,
    Dot,
}

impl ColorSetting {
    const ALL: [Self; 3] = [Self::Background, Self::Line, Self::Dot];

    fn label(self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::Line => "Line colour",
            Self::Dot => "Dot colour",
        }
    }

    fn value(self, canvas: &CanvasSettings) -> Option<&str> {
        match self {
            Self::Background => canvas.background.as_deref(),
            Self::Line => canvas.line_color.as_deref(),
            Self::Dot => canvas.dot_color.as_deref(),
        }
    }

    /// The colour used while the page sets none (the canvas paints the
    /// same theme colours).
    fn theme_color(self, theme: &Theme) -> Hsla {
        match self {
            Self::Background => theme.background,
            Self::Line | Self::Dot => theme.border,
        }
    }

    fn apply(self, session: &mut DocumentSession, color: Option<&str>) {
        match self {
            Self::Background => session.set_background(color),
            Self::Line => session.set_line_color(color),
            Self::Dot => session.set_dot_color(color),
        }
    }
}

/// A row of the Canvas section below the pattern dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Number(NumberSetting),
    Color(ColorSetting),
}

/// The settings shown for `pattern`, in order.
fn pattern_rows(pattern: BackgroundPattern) -> &'static [Row] {
    use ColorSetting as C;
    use NumberSetting as N;
    match pattern {
        BackgroundPattern::Lines => &[
            Row::Number(N::GridSize),
            Row::Color(C::Line),
            Row::Number(N::MajorEvery),
        ],
        BackgroundPattern::Dots => &[
            Row::Number(N::GridSize),
            Row::Number(N::DotSize),
            Row::Color(C::Dot),
        ],
        BackgroundPattern::None => &[],
    }
}

struct NumberField {
    setting: NumberSetting,
    input: Entity<InputState>,
    /// The value the input currently shows.
    shows: Option<f32>,
}

struct ColorField {
    setting: ColorSetting,
    state: Entity<ColorPickerState>,
    /// The colour the picker currently shows: the page's, else the
    /// theme's.
    shows: Option<Hsla>,
}

/// The Properties panel's controls, kept in step with the active page.
pub(super) struct PropertyFields {
    pattern: Entity<SelectState<PatternItems>>,
    numbers: Vec<NumberField>,
    colors: Vec<ColorField>,
}

impl PropertyFields {
    /// Creates the controls, and the subscriptions that apply their edits
    /// to the editor's session.
    pub(super) fn new(
        pattern: BackgroundPattern,
        window: &mut Window,
        cx: &mut Context<EditorWindow>,
    ) -> (Self, Vec<Subscription>) {
        let mut subscriptions = Vec::new();

        let items: PatternItems = PATTERNS.iter().map(|(_, label)| *label).collect();
        let selected = Some(IndexPath::new(pattern_index(pattern)));
        let pattern = cx.new(|cx| SelectState::new(items, selected, window, cx));
        subscriptions.push(cx.subscribe_in(
            &pattern,
            window,
            |this, _, event: &SelectEvent<PatternItems>, _, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                if let Some((p, _)) = PATTERNS.iter().find(|(_, l)| l == label) {
                    this.update_session(|s| s.set_pattern(*p), cx);
                }
            },
        ));

        let numbers = NumberSetting::ALL
            .into_iter()
            .map(|setting| {
                let input = cx.new(|cx| InputState::new(window, cx));
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this, _, event, window, cx| {
                        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                            this.commit_number(setting, window, cx);
                        }
                    },
                ));
                NumberField {
                    setting,
                    input,
                    shows: None,
                }
            })
            .collect();

        let colors = ColorSetting::ALL
            .into_iter()
            .map(|setting| {
                let state = cx.new(|cx| ColorPickerState::new(window, cx));
                subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, _, event: &ColorPickerEvent, _, cx| {
                        // The picker never clears a colour itself; the reset
                        // button does that.
                        if let ColorPickerEvent::Change(Some(color)) = event {
                            let hex = hsla_to_hex(*color);
                            this.update_session(|s| setting.apply(s, Some(&hex)), cx);
                        }
                    },
                ));
                ColorField {
                    setting,
                    state,
                    shows: None,
                }
            })
            .collect();

        (
            Self {
                pattern,
                numbers,
                colors,
            },
            subscriptions,
        )
    }

    fn number(&self, setting: NumberSetting) -> &NumberField {
        self.numbers
            .iter()
            .find(|f| f.setting == setting)
            .expect("a field for every number setting")
    }

    fn color(&self, setting: ColorSetting) -> &ColorField {
        self.colors
            .iter()
            .find(|f| f.setting == setting)
            .expect("a field for every colour setting")
    }
}

impl EditorWindow {
    /// Applies a number field on Enter or blur; whatever happens, the field
    /// then shows the page's value.
    fn commit_number(
        &mut self,
        setting: NumberSetting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self
            .properties
            .number(setting)
            .input
            .read(cx)
            .value()
            .to_string();
        if let Some(value) = parse_number(&text) {
            self.update_session(|s| setting.apply(s, value), cx);
        }
        self.show_number(setting, window, cx);
    }

    /// Puts the active page's value in a number field.
    fn show_number(&mut self, setting: NumberSetting, window: &mut Window, cx: &mut Context<Self>) {
        let value = setting.value(&self.session.page().canvas);
        let Some(field) = self
            .properties
            .numbers
            .iter_mut()
            .find(|f| f.setting == setting)
        else {
            return;
        };
        field.input.update(cx, |input, cx| {
            input.set_value(setting.text(value), window, cx)
        });
        field.shows = Some(value);
    }

    /// Applies the number being typed, if any. Only a focused field can
    /// hold an unapplied value; an unfocused one shows the page's value
    /// rounded, which must not be written back.
    pub(super) fn commit_focused_number(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .properties
            .numbers
            .iter()
            .find(|f| f.input.read(cx).focus_handle(cx).is_focused(window))
            .map(|f| f.setting);
        if let Some(setting) = focused {
            self.commit_number(setting, window, cx);
        }
    }

    /// Brings the panel's controls up to date with the active page (and
    /// theme), leaving alone any the user is editing.
    pub(super) fn sync_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let canvas = self.session.page().canvas.clone();

        let ix = IndexPath::new(pattern_index(canvas.pattern));
        let pattern = self.properties.pattern.clone();
        if pattern.read(cx).selected_index(cx) != Some(ix) {
            pattern.update(cx, |state, cx| {
                state.set_selected_index(Some(ix), window, cx)
            });
        }

        for setting in NumberSetting::ALL {
            let field = self.properties.number(setting);
            let value = setting.value(&canvas);
            if field.shows != Some(value)
                && !field.input.read(cx).focus_handle(cx).is_focused(window)
            {
                self.show_number(setting, window, cx);
            }
        }

        let theme = cx.theme().clone();
        for field in &mut self.properties.colors {
            let color = field
                .setting
                .value(&canvas)
                .and_then(hex_to_hsla)
                .unwrap_or_else(|| field.setting.theme_color(&theme));
            // An open picker owns its colour until it closes.
            if field.shows != Some(color) && !field.state.read(cx).is_open() {
                field
                    .state
                    .update(cx, |state, cx| state.set_value(color, window, cx));
                field.shows = Some(color);
            }
        }
    }
}

pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let canvas = &editor.session().page().canvas;

    let pattern = v_flex()
        .gap(px(FIELD_GAP))
        .child("Background pattern")
        .child(
            Select::new(&editor.properties.pattern)
                .id("background-pattern")
                .small()
                .w_full(),
        );

    let rows: Vec<AnyElement> = pattern_rows(canvas.pattern)
        .iter()
        .map(|row| match *row {
            Row::Number(setting) => number_row(editor, setting, canvas.pattern, muted),
            Row::Color(setting) => color_row(editor, setting, cx),
        })
        .collect();

    v_flex()
        .size_full()
        .bg(cx.theme().sidebar)
        .px(px(12.))
        .pt(px(10.))
        .gap(px(4.))
        .text_size(px(SMALL_TEXT_SIZE))
        .child(section_label("Canvas", cx))
        .child(color_row(editor, ColorSetting::Background, cx))
        .child(pattern.mt(px(4.)))
        .children(rows)
        .child(section_label("Selection", cx).mt(px(16.)))
        .child(div().text_color(muted).child("Nothing selected"))
}

/// A label on the left and a control on the right.
fn field_row(label: &'static str, tooltip: Option<&'static str>, control: impl IntoElement) -> Div {
    let label = div()
        .id(label)
        .child(label)
        .when_some(tooltip, |label, text| {
            label.tooltip(move |window, cx| Tooltip::new(text).build(window, cx))
        });
    h_flex()
        .h(px(ROW_HEIGHT))
        .flex_none()
        .justify_between()
        .gap(px(FIELD_GAP))
        .child(label)
        .child(control)
}

fn number_row(
    editor: &EditorWindow,
    setting: NumberSetting,
    pattern: BackgroundPattern,
    muted: Hsla,
) -> AnyElement {
    let input = Input::new(&editor.properties.number(setting).input)
        .small()
        .when_some(setting.suffix(), |input, suffix| {
            input.suffix(div().text_color(muted).child(suffix))
        });
    field_row(
        setting.label(pattern),
        setting.tooltip(),
        div().w(px(NUMBER_INPUT_WIDTH)).child(input),
    )
    .into_any_element()
}

fn color_row(
    editor: &EditorWindow,
    setting: ColorSetting,
    cx: &mut Context<EditorWindow>,
) -> AnyElement {
    let field = editor.properties.color(setting);
    let custom = setting.value(&editor.session().page().canvas).is_some();
    let hex = field.shows.map(hsla_to_hex).unwrap_or_default();
    let reset = div()
        .w(px(RESET_WIDTH))
        .flex()
        .justify_end()
        .when(custom, |slot| {
            slot.child(
                Button::new(("reset-color", setting as usize))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip("Use Theme Colour")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_session(|s| setting.apply(s, None), cx);
                    })),
            )
        });
    let picker = h_flex()
        .gap(px(2.))
        // A theme colour's hex is shown muted.
        .when(!custom, |picker| {
            picker.text_color(cx.theme().muted_foreground)
        })
        .child(
            ColorPicker::new(&field.state)
                .with_size(px(SWATCH_SIZE))
                .label(hex)
                .anchor(Anchor::TopRight),
        )
        .child(reset);
    field_row(setting.label(), None, picker).into_any_element()
}

/// The dropdown item shown selected for `pattern`.
fn pattern_index(pattern: BackgroundPattern) -> usize {
    PATTERNS
        .iter()
        .position(|(p, _)| *p == pattern)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{major_every_from, pattern_index, pattern_rows, NumberSetting, Row, PATTERNS};
    use super::{ColorSetting as C, NumberSetting as N};
    use mindweave_core::BackgroundPattern;

    #[test]
    fn dropdown_lists_lines_dots_none() {
        let labels: Vec<_> = PATTERNS.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, ["Lines", "Dots", "None"]);
        for (ix, (pattern, _)) in PATTERNS.iter().enumerate() {
            assert_eq!(pattern_index(*pattern), ix);
        }
        assert_eq!(pattern_index(BackgroundPattern::Dots), 1);
    }

    #[test]
    fn each_pattern_shows_its_own_settings() {
        assert_eq!(
            pattern_rows(BackgroundPattern::Lines),
            [
                Row::Number(N::GridSize),
                Row::Color(C::Line),
                Row::Number(N::MajorEvery)
            ]
        );
        assert_eq!(
            pattern_rows(BackgroundPattern::Dots),
            [
                Row::Number(N::GridSize),
                Row::Number(N::DotSize),
                Row::Color(C::Dot)
            ]
        );
        assert!(pattern_rows(BackgroundPattern::None).is_empty());
        assert_eq!(N::GridSize.label(BackgroundPattern::Lines), "Grid size");
        assert_eq!(N::GridSize.label(BackgroundPattern::Dots), "Spacing");
    }

    #[test]
    fn numbers_are_shown_as_their_setting_stores_them() {
        assert_eq!(NumberSetting::GridSize.text(20.0), "20");
        assert_eq!(NumberSetting::GridSize.text(12.6), "13");
        assert_eq!(NumberSetting::MajorEvery.text(5.0), "5");
        assert_eq!(NumberSetting::DotSize.text(1.5), "1.5");
        assert_eq!(NumberSetting::DotSize.text(2.0), "2");
    }

    #[test]
    fn major_every_takes_whole_non_negative_counts() {
        assert_eq!(major_every_from(4.0), 4);
        assert_eq!(major_every_from(4.6), 5);
        assert_eq!(major_every_from(-3.0), 0);
        assert_eq!(major_every_from(1e12), u32::MAX);
    }
}
