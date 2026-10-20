//! The editor window for one document: Layers panel, canvas, page tabs,
//! Properties panel and status bar.

mod canvas;
mod layers_panel;
mod page_tabs;
mod properties_panel;
mod status_bar;

use std::future::Future;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::{v_flex, ActiveTheme as _, Sizable as _, WindowExt as _};
use gpui_kit::*;
use mindweave_core::{format_hex_color, parse_hex_color, DocumentSession};

use crate::actions::{CloseWindow, Save, ToggleLayers, ToggleProperties};
use crate::app_state::AppState;
use crate::menu_bar::{menu_bar, WindowKind};

const TEXT_SIZE: f32 = 13.;
const SMALL_TEXT_SIZE: f32 = 12.;
const SECTION_LABEL_SIZE: f32 = 10.;
const LAYERS_WIDTH: f32 = 220.;
const PROPERTIES_WIDTH: f32 = 240.;
const PANEL_MIN_WIDTH: f32 = 160.;
const PANEL_MAX_WIDTH: f32 = 480.;

/// The user's answer to "save changes before closing?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveChoice {
    Save,
    DontSave,
    Cancel,
}

/// Asks whether to save `file_name` before it closes. Escape and closing
/// the prompt count as Cancel.
pub fn prompt_to_save(
    file_name: &str,
    window: &mut Window,
    cx: &mut App,
) -> impl Future<Output = SaveChoice> {
    let answer = window.prompt(
        PromptLevel::Warning,
        &format!("Do you want to save the changes you made to “{file_name}”?"),
        Some("Your changes will be lost if you don't save them."),
        &["Save", "Don't Save", "Cancel"],
        cx,
    );
    async move {
        match answer.await {
            Ok(0) => SaveChoice::Save,
            Ok(1) => SaveChoice::DontSave,
            _ => SaveChoice::Cancel,
        }
    }
}

/// What an inline rename edits: a page, or a layer of the active page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameTarget {
    Page(usize),
    Layer(usize),
}

/// What a click on a page tab or layer row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowClick {
    /// Make the page or layer active.
    Activate,
    /// Start renaming it.
    Rename,
    /// Nothing: the click landed in the row's own open rename field, where
    /// it places the caret or selects text.
    Ignore,
}

/// Decides what a click on a tab or row does, given whether that tab or row
/// is being renamed and the click count.
pub(crate) fn row_click(renaming: bool, click_count: usize) -> RowClick {
    if renaming {
        RowClick::Ignore
    } else if click_count >= 2 {
        RowClick::Rename
    } else {
        RowClick::Activate
    }
}

/// An inline rename in progress.
struct Rename {
    target: RenameTarget,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub struct EditorWindow {
    session: DocumentSession,
    /// The pointer position in canvas coordinates, or `None` when the
    /// pointer is off the canvas.
    cursor_canvas: Option<(f32, f32)>,
    focus_handle: FocusHandle,
    /// The canvas's bounds when it was last painted; zoom actions anchor on
    /// its centre.
    canvas_bounds: Option<Bounds<Pixels>>,
    /// The pointer position relative to the canvas origin, or `None` when
    /// the pointer is off the canvas.
    pointer_local: Option<(f32, f32)>,
    /// Whether Space is held down while the editor has focus (left drag
    /// then pans).
    space_held: bool,
    /// The pointer position at the last step of a pan drag in progress.
    pan_drag: Option<Point<Pixels>>,
    rename: Option<Rename>,
    properties: properties_panel::PropertyFields,
    /// The title last given to the window.
    window_title: String,
    _subscriptions: Vec<Subscription>,
}

impl EditorWindow {
    pub fn new(session: DocumentSession, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let (properties, mut _subscriptions) =
            properties_panel::PropertyFields::new(session.page().canvas.pattern, window, cx);
        _subscriptions.extend([
            // Panel visibility and the theme name live in the settings.
            cx.observe_global::<AppState>(|_, cx| cx.notify()),
            // A Space released while the window is inactive or the editor
            // is unfocused never reaches `on_key_up`; forget it instead.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.release_space(cx);
                }
            }),
            cx.on_blur(&focus_handle, window, |this, _, cx| this.release_space(cx)),
        ]);
        let window_title = session.title();
        Self {
            session,
            cursor_canvas: None,
            focus_handle,
            canvas_bounds: None,
            pointer_local: None,
            space_held: false,
            pan_drag: None,
            rename: None,
            properties,
            window_title,
            _subscriptions,
        }
    }

    pub fn session(&self) -> &DocumentSession {
        &self.session
    }

    /// Applies `f` to the session, redraws, and refreshes the window title
    /// (on the next render).
    pub fn update_session(&mut self, f: impl FnOnce(&mut DocumentSession), cx: &mut Context<Self>) {
        f(&mut self.session);
        cx.notify();
    }

    /// Sets the pointer position shown in the status bar, in canvas
    /// coordinates; `None` hides it.
    pub fn set_cursor(&mut self, p: Option<(f32, f32)>, cx: &mut Context<Self>) {
        if self.cursor_canvas != p {
            self.cursor_canvas = p;
            cx.notify();
        }
    }

    /// The document's file name without extension.
    pub fn file_name(&self) -> String {
        self.session
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    /// Saves the document. On failure shows a notification and returns
    /// `false`.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.commit_pending_edits(window, cx);
        let saved = match self.session.save() {
            Ok(()) => true,
            Err(err) => {
                window.push_notification(Notification::error(err.to_string()), cx);
                false
            }
        };
        self.sync_window_title(window);
        cx.notify();
        saved
    }

    /// Closes the window, first asking to save when there are unsaved
    /// changes. Used by Close Window and the window's close button.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_pending_edits(window, cx);
        if !self.session.is_dirty() {
            close(window, cx);
            return;
        }
        let choice = prompt_to_save(&self.file_name(), window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let close_now = match choice.await {
                SaveChoice::Save => this
                    .update_in(cx, |this, window, cx| this.save(window, cx))
                    .unwrap_or(false),
                SaveChoice::DontSave => true,
                SaveChoice::Cancel => false,
            };
            if close_now {
                this.update_in(cx, |_, window, cx| close(window, cx)).ok();
            }
        })
        .detach();
    }

    /// Applies edits that otherwise wait for Enter or blur: an open rename
    /// and a Properties number being typed. Save, Close and Quit call this first so
    /// keyboard shortcuts do not skip them.
    pub fn commit_pending_edits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            self.finish_rename(true, cx);
            // The rename field is gone; keep keyboard shortcuts working.
            self.focus_handle.focus(window, cx);
        }
        self.commit_focused_number(window, cx);
    }

    /// Starts an inline rename of `target` with its current name selected.
    pub(crate) fn begin_rename(
        &mut self,
        target: RenameTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = match target {
            RenameTarget::Page(ix) => self.session.doc.pages.get(ix).map(|p| p.name.clone()),
            RenameTarget::Layer(ix) => self.session.page().layers.get(ix).map(|l| l.name.clone()),
        };
        let Some(name) = name else {
            return;
        };
        self.finish_rename(true, cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |input, cx| {
            input.select_all(window, cx);
            input.focus(window, cx);
        });
        let _subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            match event {
                InputEvent::PressEnter { .. } => {
                    this.finish_rename(true, cx);
                    this.focus_handle.focus(window, cx);
                }
                // Focus has already gone elsewhere; leave it there.
                InputEvent::Blur => this.finish_rename(true, cx),
                _ => {}
            }
        });
        self.rename = Some(Rename {
            target,
            input,
            _subscription,
        });
        cx.notify();
    }

    /// Ends the inline rename, applying the typed name when `commit` is set.
    /// A blank name keeps the old one.
    fn finish_rename(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        if commit {
            let name = rename.input.read(cx).value().to_string();
            self.update_session(
                |s| match rename.target {
                    RenameTarget::Page(ix) => s.rename_page(ix, &name),
                    RenameTarget::Layer(ix) => s.rename_layer(ix, &name),
                },
                cx,
            );
        }
        cx.notify();
    }

    /// The rename input when `target` is being renamed.
    pub(crate) fn rename_input(&self, target: RenameTarget) -> Option<&Entity<InputState>> {
        self.rename
            .as_ref()
            .filter(|r| r.target == target)
            .map(|r| &r.input)
    }

    /// The inline rename field: Enter or leaving it commits, Escape cancels.
    pub(crate) fn render_rename_input(
        &self,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                this.finish_rename(false, cx);
                this.focus_handle.focus(window, cx);
            }))
            .child(Input::new(input).small())
    }

    /// Switches pages, committing any rename in progress first.
    pub(crate) fn activate_page(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.finish_rename(true, cx);
        self.update_session(|s| s.set_active_page(ix), cx);
        // The new page has its own viewport.
        self.refresh_cursor(cx);
    }

    /// Adds a page, committing any rename in progress first.
    pub(crate) fn add_page(&mut self, cx: &mut Context<Self>) {
        self.finish_rename(true, cx);
        self.update_session(
            |s| {
                s.add_page();
            },
            cx,
        );
        // The new page has its own viewport.
        self.refresh_cursor(cx);
    }

    /// Gives the window the document's title, and shows the unsaved-changes
    /// dot in its close button while the document is dirty.
    fn sync_window_title(&mut self, window: &mut Window) {
        let title = self.session.title();
        if title != self.window_title {
            window.set_window_title(&title);
            window.set_window_edited(self.session.is_dirty());
            self.window_title = title;
        }
    }

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        self.save(window, cx);
    }

    fn on_close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close(window, cx);
    }

    fn on_toggle_layers(&mut self, _: &ToggleLayers, _: &mut Window, cx: &mut Context<Self>) {
        AppState::update_settings(cx, |s| s.show_layers = !s.show_layers);
    }

    fn on_toggle_properties(
        &mut self,
        _: &ToggleProperties,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        AppState::update_settings(cx, |s| s.show_properties = !s.show_properties);
    }

    /// Space held over a focused editor turns left drags into pans.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if is_plain_space(&event.keystroke)
            && !self.space_held
            && self.focus_handle.is_focused(window)
        {
            self.space_held = true;
            cx.notify();
        }
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "space" {
            self.release_space(cx);
        }
    }

    fn release_space(&mut self, cx: &mut Context<Self>) {
        if self.space_held {
            self.space_held = false;
            cx.notify();
        }
    }

    /// The canvas area.
    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .child(canvas::canvas(cx.entity(), self.session.page()))
    }
}

/// Space with no modifiers.
fn is_plain_space(keystroke: &Keystroke) -> bool {
    keystroke.key == "space" && !keystroke.modifiers.modified()
}

fn close(window: &mut Window, cx: &mut App) {
    AppState::remember_window_size(WindowKind::Editor, window, cx);
    window.remove_window();
}

/// A number typed into the Properties panel, or `None` when it is not a
/// finite number.
pub(crate) fn parse_number(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok().filter(|n| n.is_finite())
}

/// An opaque colour for a `#RRGGBB` string.
pub(crate) fn hex_to_hsla(hex: &str) -> Option<Hsla> {
    let (r, g, b) = parse_hex_color(hex)?;
    Some(rgb(u32::from_be_bytes([0, r, g, b])).into())
}

/// `color` as an uppercase `#RRGGBB` string; alpha is dropped.
pub(crate) fn hsla_to_hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format_hex_color((channel(c.r), channel(c.g), channel(c.b)))
}

/// `text` with hair spaces between its characters, standing in for letter
/// spacing, which gpui text does not support.
pub(crate) fn letter_spaced(text: &str) -> String {
    let chars: Vec<String> = text.chars().map(String::from).collect();
    chars.join("\u{200A}")
}

/// A panel section heading: 10px, uppercase, muted, letter-spaced.
pub(crate) fn section_label(text: &str, cx: &App) -> Div {
    div()
        .text_size(px(SECTION_LABEL_SIZE))
        .text_color(cx.theme().muted_foreground)
        .child(letter_spaced(&text.to_uppercase()))
}

/// Element ids for the panel group, one per sidebar combination, so each
/// combination keeps its own panel widths.
fn panel_group_id(show_layers: bool, show_properties: bool) -> &'static str {
    match (show_layers, show_properties) {
        (true, true) => "editor-panels-both",
        (true, false) => "editor-panels-layers",
        (false, true) => "editor-panels-properties",
        (false, false) => "editor-panels-none",
    }
}

impl Focusable for EditorWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_window_title(window);
        self.sync_properties(window, cx);

        let (show_layers, show_properties) = {
            let settings = &cx.global::<AppState>().settings;
            (settings.show_layers, settings.show_properties)
        };
        let panel_range = px(PANEL_MIN_WIDTH)..px(PANEL_MAX_WIDTH);

        let mut panels = h_resizable(panel_group_id(show_layers, show_properties));
        if show_layers {
            panels = panels.child(
                resizable_panel()
                    .size(px(LAYERS_WIDTH))
                    .size_range(panel_range.clone())
                    .fixed()
                    .child(layers_panel::render(self, cx)),
            );
        }
        panels = panels.child(
            resizable_panel().child(
                v_flex()
                    .size_full()
                    .min_w_0()
                    .child(self.render_canvas(cx))
                    .child(page_tabs::render(self, cx)),
            ),
        );
        if show_properties {
            panels = panels.child(
                resizable_panel()
                    .size(px(PROPERTIES_WIDTH))
                    .size_range(panel_range)
                    .fixed()
                    .child(properties_panel::render(self, cx)),
            );
        }

        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_toggle_layers))
            .on_action(cx.listener(Self::on_toggle_properties))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(TEXT_SIZE))
            .child(menu_bar(
                WindowKind::Editor,
                Some(self.window_title.clone().into()),
                cx,
            ))
            .child(div().flex_1().min_h_0().child(panels))
            .child(status_bar::render(self, cx))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        hex_to_hsla, hsla_to_hex, is_plain_space, letter_spaced, panel_group_id, parse_number,
        row_click, RowClick,
    };
    use gpui_kit::Keystroke;

    #[test]
    fn only_unmodified_space_arms_panning() {
        let key = |s: &str| Keystroke::parse(s).unwrap();
        assert!(is_plain_space(&key("space")));
        assert!(!is_plain_space(&key("cmd-space")));
        assert!(!is_plain_space(&key("shift-space")));
        assert!(!is_plain_space(&key("a")));
    }

    #[test]
    fn row_clicks_activate_rename_or_leave_an_open_rename_alone() {
        assert_eq!(row_click(false, 1), RowClick::Activate);
        assert_eq!(row_click(false, 2), RowClick::Rename);
        assert_eq!(row_click(false, 3), RowClick::Rename);
        assert_eq!(row_click(true, 1), RowClick::Ignore);
        assert_eq!(row_click(true, 2), RowClick::Ignore);
    }

    #[test]
    fn numbers_parse_trimmed() {
        assert_eq!(parse_number("20"), Some(20.0));
        assert_eq!(parse_number("  12.5 "), Some(12.5));
        assert_eq!(parse_number("0"), Some(0.0));
        assert_eq!(parse_number("-3"), Some(-3.0));
    }

    #[test]
    fn non_numbers_and_non_finite_numbers_are_rejected() {
        for bad in ["abc", "", "20px", "NaN", "inf", "-inf"] {
            assert_eq!(parse_number(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hex_colours_convert_to_and_from_hsla() {
        for hex in ["#000000", "#FFFFFF", "#FF8000", "#1A2B3C", "#808080"] {
            let hsla = hex_to_hsla(hex).unwrap();
            assert_eq!(hsla.a, 1.0);
            assert_eq!(hsla_to_hex(hsla), hex);
        }
        assert_eq!(hex_to_hsla("ff8000"), hex_to_hsla("#FF8000"));
        assert_eq!(hex_to_hsla("nope"), None);
    }

    #[test]
    fn letter_spacing_uses_hair_spaces() {
        assert_eq!(letter_spaced("ABC"), "A\u{200A}B\u{200A}C");
        assert_eq!(letter_spaced(""), "");
    }

    #[test]
    fn each_sidebar_combination_has_its_own_panel_group() {
        let ids = [
            panel_group_id(true, true),
            panel_group_id(true, false),
            panel_group_id(false, true),
            panel_group_id(false, false),
        ];
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
