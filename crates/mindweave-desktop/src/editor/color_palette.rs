//! The colour palette popover: an Auto choice, preset swatches, the
//! recently picked colours, and a custom picker (hue, saturation and
//! lightness sliders plus a hex field).
//!
//! A caller owns an `Entity<ColorPalette>` and shows it with
//! [`palette_popover`]. Every choice reaches the caller through the
//! palette's `on_pick` callback as a [`ColorPick`]:
//!
//! - [`ColorPick::Set`]: Auto (`None`), a preset or recent swatch, or a hex
//!   typed into the field and confirmed with Enter. A single change: the
//!   caller applies it as one undo step. The palette then closes.
//! - [`ColorPick::Preview`]: the custom sliders moved. More previews follow
//!   while the user drags; the caller applies each and groups them into one
//!   undo step, starting the step on the first preview (as the Properties
//!   panel does with `begin_color_edit(true)`).
//! - [`ColorPick::EndPreview`]: the palette closed after previews; the last
//!   preview is the final colour. The caller ends its grouped step (as
//!   `finish_color_edit` does).
//!
//! A `Set` that follows previews in the same opening joins their group,
//! which `EndPreview` then ends. Picking a colour (but not Auto) puts it at
//! the front of the recent colours in the app settings; a drag adds only
//! its final colour, when the palette closes.

use std::rc::Rc;

use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Escape, Input};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Selectable, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::color::normalize_hex_color;

use super::{hex_to_hsla, hsla_to_hex, SMALL_TEXT_SIZE};
use crate::app_state::AppState;

/// The preset swatches, five per row: neutrals, then hues around the
/// wheel, then pink, sky and brown. The hues are mid-to-dark tones that read
/// on light and dark backgrounds alike; the neutrals cannot all do that
/// (black and dark grey vanish on a dark canvas, white and light grey on a
/// light one), so they are there for contrast with a chosen fill.
pub(crate) const PRESET_COLORS: [&str; 20] = [
    "#000000", "#666666", "#808080", "#C8C8C8", "#FFFFFF", // neutrals
    "#DC2626", "#EA580C", "#D97706", "#EAB308", "#65A30D", // red .. lime
    "#16A34A", "#0D9488", "#0891B2", "#2563EB", "#4F46E5", // green .. indigo
    "#7C3AED", "#9333EA", "#DB2777", "#38BDF8", "#92400E", // violet, pink, sky, brown
];
const COLUMNS: usize = 5;
/// How many recent colours the palette shows.
const MAX_RECENT_SWATCHES: usize = mindweave_core::MAX_RECENT_COLORS;
const SWATCH: f32 = 20.;
const GAP: f32 = 4.;
/// The content width: five swatches and the gaps between them.
const WIDTH: f32 = COLUMNS as f32 * SWATCH + (COLUMNS - 1) as f32 * GAP;
const SLIDER_HEIGHT: f32 = 16.;
const TRACK_HEIGHT: f32 = 8.;
/// Colour stops drawn along the hue and lightness tracks.
const TRACK_STEPS: usize = 24;

/// A choice made in the palette; see the module docs for how a caller
/// applies each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColorPick {
    /// One finished change: `None` is Auto, else an uppercase `#RRGGBB`.
    Set(Option<String>),
    /// A custom-picker drag reached this uppercase `#RRGGBB`.
    Preview(String),
    /// The palette closed after previews; the last one stands.
    EndPreview,
}

pub(crate) type OnPick = Rc<dyn Fn(ColorPick, &mut Window, &mut App)>;

/// What the palette owes its caller for the previews of one opening.
#[derive(Debug, Default)]
struct Previews {
    /// The last colour previewed since the palette opened, unless a later
    /// choice replaced it.
    previewed: Option<String>,
    /// Whether the palette previewed since it opened, so the caller is owed
    /// an [`ColorPick::EndPreview`].
    previewing: bool,
}

impl Previews {
    /// The palette opened: nothing previewed yet.
    fn open(&mut self) {
        *self = Self::default();
    }

    /// A custom-picker drag reached `hex`.
    fn preview(&mut self, hex: &str) {
        self.previewed = Some(hex.to_string());
        self.previewing = true;
    }

    /// A choice (`Set`) was made: it replaces the drag's colour.
    fn chose(&mut self) {
        self.previewed = None;
    }

    /// The palette closed: whether an `EndPreview` is owed, and the colour
    /// the last drag ended on unless a later choice replaced it.
    fn close(&mut self) -> (bool, Option<String>) {
        let this = std::mem::take(self);
        (this.previewing, this.previewed)
    }
}

/// The colour a palette shows as chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaletteColor {
    /// The objects' colours differ: nothing is marked.
    Mixed,
    /// No colour set: Auto is marked.
    Auto,
    /// An uppercase `#RRGGBB`.
    Color(String),
}

impl PaletteColor {
    /// From a shared value (`None`: mixed) that is a colour or Auto
    /// (`None`); text that is not a colour counts as Auto.
    pub(crate) fn from_mixed(value: Option<Option<String>>) -> Self {
        match value {
            None => PaletteColor::Mixed,
            Some(color) => match color.and_then(|c| normalize_hex_color(&c)) {
                Some(hex) => PaletteColor::Color(hex),
                None => PaletteColor::Auto,
            },
        }
    }

    fn is_auto(&self) -> bool {
        *self == PaletteColor::Auto
    }

    fn hex(&self) -> Option<&str> {
        match self {
            PaletteColor::Color(hex) => Some(hex),
            _ => None,
        }
    }
}

/// The palette's state; render it with [`palette_popover`].
pub(crate) struct ColorPalette {
    /// The custom picker: its sliders and hex field.
    picker: Entity<ColorPickerState>,
    /// The colour shown as chosen.
    current: PaletteColor,
    on_pick: OnPick,
    previews: Previews,
    /// The popover showing the palette, while one is.
    popover: Option<WeakEntity<PopoverState>>,
    _subscription: Subscription,
}

impl ColorPalette {
    pub(crate) fn new(
        on_pick: impl Fn(ColorPick, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let subscription = cx.subscribe_in(
            &picker,
            window,
            |this, picker, event: &ColorPickerEvent, window, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                let hex = hsla_to_hex(*color);
                let palette = cx.entity().downgrade();
                // The picker stays open while the palette is; only a hex
                // committed with Enter closes it.
                if picker.read(cx).is_open() {
                    this.previews.preview(&hex);
                    let on_pick = this.on_pick.clone();
                    // Outside this palette's update, so the caller may read
                    // or update it.
                    window.defer(cx, move |window, cx| {
                        on_pick(ColorPick::Preview(hex), window, cx)
                    });
                } else {
                    window.defer(cx, move |window, cx| {
                        choose(&palette, Some(hex), window, cx)
                    });
                }
            },
        );
        Self {
            picker,
            current: PaletteColor::Auto,
            on_pick: Rc::new(on_pick),
            previews: Previews::default(),
            popover: None,
            _subscription: subscription,
        }
    }

    /// Shows `color` as the chosen one. The custom picker takes it up when
    /// the palette next opens.
    pub(crate) fn set_current(&mut self, color: PaletteColor, cx: &mut Context<Self>) {
        if self.current != color {
            self.current = color;
            cx.notify();
        }
    }

    /// Whether the palette is showing.
    pub(crate) fn is_open(&self, cx: &App) -> bool {
        self.popover
            .as_ref()
            .and_then(|p| p.upgrade())
            .is_some_and(|p| p.read(cx).is_open())
    }

    /// Readies the custom picker as the popover opens.
    fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.previews.open();
        let start = self
            .current
            .hex()
            .and_then(hex_to_hsla)
            .unwrap_or_else(|| hsla(0., 0., 0., 1.));
        self.picker.update(cx, |picker, cx| {
            picker.set_value(start, window, cx);
            picker.set_open(true, cx);
        });
    }

    /// Closes the custom picker as the popover closes. Returns whether the
    /// palette previewed while open, and the colour the last drag ended on
    /// unless a later choice replaced it.
    fn closed(&mut self, cx: &mut Context<Self>) -> (bool, Option<String>) {
        self.picker
            .update(cx, |picker, cx| picker.set_open(false, cx));
        self.popover = None;
        let (previewing, previewed) = self.previews.close();
        if let Some(hex) = &previewed {
            self.current = PaletteColor::Color(hex.clone());
        }
        (previewing, previewed)
    }
}

/// A popover opened by `trigger` that shows `palette`. Style it further
/// (e.g. its anchor) as any [`Popover`].
pub(crate) fn palette_popover(
    id: impl Into<ElementId>,
    palette: &Entity<ColorPalette>,
    trigger: impl Selectable + IntoElement + 'static,
) -> Popover {
    let opener = palette.downgrade();
    let content = palette.clone();
    Popover::new(id)
        .on_open_change(move |open, window, cx| {
            if *open {
                opener
                    .update(cx, |palette, cx| palette.opened(window, cx))
                    .ok();
                return;
            }
            let Ok(((previewing, last), on_pick)) = opener.update(cx, |palette, cx| {
                (palette.closed(cx), palette.on_pick.clone())
            }) else {
                return;
            };
            if let Some(hex) = last {
                remember(&hex, cx);
            }
            if previewing {
                on_pick(ColorPick::EndPreview, window, cx);
            }
        })
        .trigger(trigger)
        .content(move |_, _, cx| {
            let popover = cx.entity().downgrade();
            content.update(cx, |palette, _| palette.popover = Some(popover));
            content.clone()
        })
}

/// Makes `color` the palette's choice: tells the caller, remembers it among
/// the recent colours, and closes the palette.
fn choose(
    palette: &WeakEntity<ColorPalette>,
    color: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok((on_pick, popover)) = palette.update(cx, |palette, cx| {
        palette.current = PaletteColor::from_mixed(Some(color.clone()));
        // A choice after a drag replaces the drag's colour.
        palette.previews.chose();
        cx.notify();
        (palette.on_pick.clone(), palette.popover.clone())
    }) else {
        return;
    };
    if let Some(hex) = &color {
        remember(hex, cx);
    }
    on_pick(ColorPick::Set(color), window, cx);
    dismiss(popover, window, cx);
}

/// Closes the popover showing `palette`, if one is.
fn close(palette: &WeakEntity<ColorPalette>, window: &mut Window, cx: &mut App) {
    if let Ok(popover) = palette.read_with(cx, |palette, _| palette.popover.clone()) {
        dismiss(popover, window, cx);
    }
}

fn dismiss(popover: Option<WeakEntity<PopoverState>>, window: &mut Window, cx: &mut App) {
    if let Some(popover) = popover {
        popover
            .update(cx, |popover, cx| popover.dismiss(window, cx))
            .ok();
    }
}

/// Puts `hex` at the front of the recent colours in the app settings.
fn remember(hex: &str, cx: &mut App) {
    if cx.has_global::<AppState>() {
        AppState::update_settings(cx, |settings| settings.push_recent_color(hex));
    }
}

/// Whether the swatch for `swatch` shows as the chosen colour `current`.
fn is_current(current: &PaletteColor, swatch: &str) -> bool {
    current
        .hex()
        .is_some_and(|c| c.eq_ignore_ascii_case(swatch))
}

/// The recent colours the palette can show: valid ones, uppercase, in
/// order, each once, at most [`MAX_RECENT_SWATCHES`] (the settings file may
/// have been edited by hand).
fn recent_swatches(recent: &[String]) -> Vec<String> {
    let mut shown: Vec<String> = Vec::new();
    for color in recent.iter().filter_map(|c| normalize_hex_color(c)) {
        if shown.len() == MAX_RECENT_SWATCHES {
            break;
        }
        if !shown.contains(&color) {
            shown.push(color);
        }
    }
    shown
}

impl Render for ColorPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.entity().downgrade();
        let current = self.current.clone();
        let recent = if cx.has_global::<AppState>() {
            recent_swatches(&cx.global::<AppState>().settings.recent_colors)
        } else {
            Vec::new()
        };
        let swatch = |id: ElementId, hex: &str, cx: &App| {
            swatch(id, hex, is_current(&current, hex), &palette, cx)
        };
        let presets = PRESET_COLORS
            .iter()
            .enumerate()
            .map(|(ix, hex)| swatch(("palette-preset", ix).into(), hex, cx));
        let recents: Vec<AnyElement> = recent
            .iter()
            .enumerate()
            .map(|(ix, hex)| swatch(("palette-recent", ix).into(), hex, cx).into_any_element())
            .collect();
        let muted = cx.theme().muted_foreground;
        let caption = |text: &'static str| {
            div()
                .text_size(px(SMALL_TEXT_SIZE - 1.))
                .text_color(muted)
                .child(text)
        };

        v_flex()
            .w(px(WIDTH))
            .gap(px(8.))
            .text_size(px(SMALL_TEXT_SIZE))
            .child(auto_button(current.is_auto(), &palette, cx))
            .child(grid().children(presets))
            .when(!recents.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap(px(GAP))
                        .child(caption("Recent"))
                        .child(grid().children(recents)),
                )
            })
            .child(
                v_flex()
                    .gap(px(GAP))
                    .child(caption("Custom"))
                    .child(custom_picker(&self.picker, &palette, cx)),
            )
    }
}

/// Swatches laid out five to a row.
fn grid() -> Div {
    div().flex().flex_wrap().gap(px(GAP))
}

/// One colour square with a hairline border; a ring marks the chosen one.
fn swatch(
    id: ElementId,
    hex: &str,
    selected: bool,
    palette: &WeakEntity<ColorPalette>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let fill = hex_to_hsla(hex).unwrap_or_default();
    let hex = hex.to_string();
    let palette = palette.clone();
    div()
        .id(id)
        .flex_none()
        .size(px(SWATCH))
        .rounded(px(3.))
        .bg(fill)
        .border_1()
        .border_color(theme.border)
        .when(selected, |s| s.border_2().border_color(theme.ring))
        .when(!selected, |s| {
            s.hover(|style| style.border_color(theme.muted_foreground))
        })
        .tooltip({
            let hex = hex.clone();
            move |window, cx| Tooltip::new(hex.clone()).build(window, cx)
        })
        .on_click(move |_, window, cx| choose(&palette, Some(hex.clone()), window, cx))
}

/// The Auto choice: a swatch half black, half white, and its name.
fn auto_button(selected: bool, palette: &WeakEntity<ColorPalette>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let palette = palette.clone();
    let hover = theme.list_hover;
    h_flex()
        .id("palette-auto")
        .h(px(24.))
        .px(px(4.))
        .gap(px(6.))
        .rounded(theme.radius)
        .when(selected, |b| b.bg(theme.list_active))
        .when(!selected, |b| b.hover(move |style| style.bg(hover)))
        .child(
            h_flex()
                .flex_none()
                .size(px(16.))
                .rounded(px(3.))
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .child(div().h_full().flex_1().bg(gpui::black()))
                .child(div().h_full().flex_1().bg(gpui::white())),
        )
        .child("Auto")
        .on_click(move |_, window, cx| choose(&palette, None, window, cx))
}

/// The custom picker: hue, saturation and lightness sliders over tracks
/// showing their range, and the hex field (Enter applies it).
fn custom_picker(
    picker: &Entity<ColorPickerState>,
    palette: &WeakEntity<ColorPalette>,
    cx: &App,
) -> impl IntoElement {
    let palette = palette.clone();
    let state = picker.read(cx);
    let color = state
        .displayed_color()
        .unwrap_or_else(|| hsla(0., 0., 0., 1.));
    let sliders = state.sliders().clone();
    let hex_input = state.hex_input().clone();
    let stepped = |f: &dyn Fn(f32) -> Hsla| -> Vec<Hsla> {
        (0..TRACK_STEPS)
            .map(|ix| f(ix as f32 / (TRACK_STEPS - 1) as f32))
            .collect()
    };
    let hue = stepped(&|h| hsla(h, 1., 0.5, 1.));
    let saturation = stepped(&|s| hsla(color.h, s, color.l, 1.));
    let lightness = stepped(&|l| hsla(color.h, color.s.max(0.5), l, 1.));
    let border = cx.theme().border;
    let row = |slider: &Entity<SliderState>, track: Vec<Hsla>| {
        div()
            .relative()
            .flex()
            .items_center()
            .h(px(SLIDER_HEIGHT))
            .child(
                h_flex()
                    .absolute()
                    .left_0()
                    .right_0()
                    .h(px(TRACK_HEIGHT))
                    .rounded(px(TRACK_HEIGHT / 2.))
                    .overflow_hidden()
                    .children(track.into_iter().map(|c| div().flex_1().h_full().bg(c))),
            )
            .child(Slider::new(slider).flex_1().bg(transparent_black()))
    };
    v_flex()
        .gap(px(2.))
        .child(row(sliders.hue(), hue))
        .child(row(sliders.saturation(), saturation))
        .child(row(sliders.lightness(), lightness))
        .child(
            // The field keeps Escape for itself; close the palette instead.
            div()
                .mt(px(4.))
                .on_action(move |_: &Escape, window, cx| close(&palette, window, cx))
                .child(
                    Input::new(&hex_input).xsmall().prefix(
                        div()
                            .size(px(12.))
                            .rounded(px(2.))
                            .border_1()
                            .border_color(border)
                            .bg(color.opacity(1.)),
                    ),
                ),
        )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{is_current, recent_swatches, PaletteColor, Previews, PRESET_COLORS};
    use mindweave_core::color::normalize_hex_color;

    #[test]
    fn presets_are_twenty_distinct_uppercase_colours() {
        for hex in PRESET_COLORS {
            assert_eq!(normalize_hex_color(hex).as_deref(), Some(hex));
        }
        let mut sorted = PRESET_COLORS.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 20);
        // The dark grey sits between black and mid grey; one rose-red only.
        assert!(PRESET_COLORS.contains(&"#666666"));
        assert!(!PRESET_COLORS.contains(&"#4D4D4D"));
        assert!(!PRESET_COLORS.contains(&"#E11D48"));
    }

    #[test]
    fn current_swatch_matches_ignoring_case() {
        let red = PaletteColor::Color("#dc2626".into());
        assert!(is_current(&red, "#DC2626"));
        assert!(!is_current(&red, "#000000"));
        assert!(!is_current(&PaletteColor::Auto, "#000000"));
        assert!(!red.is_auto());
    }

    #[test]
    fn mixed_colours_highlight_nothing() {
        assert!(PaletteColor::Auto.is_auto());
        assert!(!PaletteColor::Mixed.is_auto());
        for hex in PRESET_COLORS {
            assert!(!is_current(&PaletteColor::Mixed, hex));
        }
        assert_eq!(PaletteColor::from_mixed(None), PaletteColor::Mixed);
        assert_eq!(PaletteColor::from_mixed(Some(None)), PaletteColor::Auto);
        assert_eq!(
            PaletteColor::from_mixed(Some(Some("#abcdef".into()))),
            PaletteColor::Color("#ABCDEF".into())
        );
    }

    #[test]
    fn recent_swatches_skip_invalid_entries() {
        let recent = ["#abcdef".to_string(), "oops".into(), "#000000".into()];
        assert_eq!(recent_swatches(&recent), ["#ABCDEF", "#000000"]);
    }

    #[test]
    fn recent_swatches_are_deduped_and_capped_at_eight() {
        // A hand-edited settings file may repeat colours or hold too many.
        let recent: Vec<String> = ["#ABCDEF", "#abcdef", "#000000"]
            .into_iter()
            .map(String::from)
            .chain((0..10).map(|i| format!("#1111{i:02}")))
            .collect();
        let shown = recent_swatches(&recent);
        assert_eq!(shown.len(), 8);
        assert_eq!(&shown[..3], ["#ABCDEF", "#000000", "#111100"]);
    }

    #[test]
    fn previews_end_when_the_palette_closes() {
        // Preview, preview, close: the caller is owed an EndPreview, and the
        // drag's last colour is remembered.
        let mut p = Previews::default();
        p.open();
        p.preview("#110000");
        p.preview("#220000");
        assert_eq!(p.close(), (true, Some("#220000".to_string())));
        // Nothing carries over to the next opening.
        p.open();
        assert_eq!(p.close(), (false, None));

        // A swatch after a drag: its Set replaces the drag's colour, and
        // closing still ends the previews.
        p.open();
        p.preview("#330000");
        p.chose();
        assert_eq!(p.close(), (true, None));

        // A swatch alone: no EndPreview.
        p.open();
        p.chose();
        assert_eq!(p.close(), (false, None));
    }
}
