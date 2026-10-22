//! The editor window for one document: ribbon, Layers panel, canvas, page
//! tabs, Properties panel and status bar.

mod canvas;
mod color_palette;
mod icons_panel;
mod images;
mod interaction;
mod layers_panel;
mod line_properties;
mod mindmap_edit;
mod page_tabs;
mod properties_panel;
mod shape_gallery;
mod shape_paint;
mod shape_properties;
mod status_bar;
mod text_edit;
mod text_layout;
mod text_paint;
mod text_properties;
pub mod tool;
mod toolbar;
mod topic_properties;

use std::collections::HashSet;
use std::future::Future;

use gpui_kit::component::input::{self, Escape, Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::{v_flex, ActiveTheme as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::geometry::{Pt, Rect};
use mindweave_core::{
    duplicate, format_hex_color, from_clipboard, is_mindweave_payload, is_newer_payload,
    parse_hex_color, plain_text_box, prepare_paste, to_clipboard, unit_members, Arrange,
    DocumentSession, Shape, Viewport, MAX_CLIPBOARD_SHAPES,
};

// `Copy` is used by path: importing it would hide the derivable trait.
use crate::actions::{
    self, AddCentralTopic, AddSiblingBefore, AddSubtopic, AddSummary, BringForward, BringToFront,
    Cancel, CloseWindow, Cut, Delete, Duplicate, EditLabel, EditText, Group, NudgeDown,
    NudgeDownFar, NudgeLeft, NudgeLeftFar, NudgeRight, NudgeRightFar, NudgeUp, NudgeUpFar, Paste,
    Redo, Save, SelectAll, SendBackward, SendToBack, ToggleBoundary, ToggleIcons, ToggleLayers,
    ToggleProperties, ToolEllipse, ToolHand, ToolLine, ToolPointer, ToolRectangle,
    ToolRelationship, ToolText, Undo, Ungroup, EDITOR_CONTEXT, TEXT_EDITING_CONTEXT,
};
use crate::app_state::AppState;
use crate::menu_bar::{menu_bar, WindowKind};
use interaction::{Gesture, Press, NUDGE_FAR_STEP, NUDGE_STEP};
use shape_paint::GeometryCache;
use text_edit::{TextCommand, TextEditUi, TextEditor};
use text_layout::TextLayoutCache;
use text_paint::{canvas_background, text_box_wrap, text_color, GlyphOutlines, TEXT_BOX_PADDING};
use tool::{Tool, ToolState, BASIC_SHAPES};
use uuid::Uuid;

const TEXT_SIZE: f32 = 13.;
const SMALL_TEXT_SIZE: f32 = 12.;
const SECTION_LABEL_SIZE: f32 = 10.;
const LAYERS_WIDTH: f32 = 220.;
/// Height of the Layers panel when the Icons pane is shown below it.
const LAYERS_WITH_ICONS_HEIGHT: f32 = 200.;
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

/// What opened the session's gesture (the undo step being gathered).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GestureOwner {
    /// A move, resize or rotate on the canvas.
    Pointer,
    /// A colour picker drag in the Properties panel.
    Color,
    /// A text-editing session.
    Text,
    /// Text size steps from the Properties panel's wheel and arrows, while
    /// not editing: consecutive steps are one undo step.
    Size,
}

/// Who holds the session's gesture: only the part of the editor that
/// opened it ends it, so, for example, a colour picker closed while text
/// is being edited does not split the editing session's undo step.
#[derive(Debug, Default)]
pub(crate) struct SessionGesture(Option<GestureOwner>);

impl SessionGesture {
    /// Opens the session's gesture for `owner` unless one is open; returns
    /// whether it did.
    pub(crate) fn begin(&mut self, session: &mut DocumentSession, owner: GestureOwner) -> bool {
        if self.0.is_some() {
            return false;
        }
        session.begin_gesture();
        self.0 = Some(owner);
        true
    }

    /// Ends the session's gesture if `owner` opened it; returns whether it
    /// did.
    pub(crate) fn end(&mut self, session: &mut DocumentSession, owner: GestureOwner) -> bool {
        if self.0 != Some(owner) {
            return false;
        }
        session.end_gesture();
        self.0 = None;
        true
    }

    /// What holds the gesture, if anything.
    pub(crate) fn owner(&self) -> Option<GestureOwner> {
        self.0
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
    /// The active tool and the ribbon tab being shown.
    pub tools: ToolState,
    /// The ribbon's scroll position, measured width and gallery search.
    ribbon: toolbar::Ribbon,
    /// The pointer gesture in progress on the canvas.
    pub gesture: Gesture,
    /// The left press behind a Pointer-tool gesture.
    press: Press,
    /// Selected shapes, all on the active layer.
    pub selection: Vec<Uuid>,
    /// While a line is drawn: the shapes its start and end attach to.
    line_ends: [Option<Uuid>; 2],
    /// The line being drawn is a mind map relationship.
    placing_relationship: bool,
    /// While a topic is dragged: its offset when the drag began.
    topic_drag_from: Option<Option<(f32, f32)>>,
    /// The shape a line end being drawn or dragged would attach to.
    attach_highlight: Option<Uuid>,
    /// Canvas outlines of the shapes painted last frame.
    geometry_cache: GeometryCache,
    /// Pictures decoded for painting.
    image_cache: images::ImageCache,
    /// The Icons pane's search and set filter.
    icons: icons_panel::IconsPane,
    /// The Topic section's link field.
    topic_fields: topic_properties::TopicFields,
    /// Text layouts of the active page's labels and text boxes.
    text_layouts: TextLayoutCache,
    /// Glyph outlines for drawing rotated text.
    glyph_outlines: GlyphOutlines,
    /// What holds the session's gesture, if anything.
    session_gesture: SessionGesture,
    /// The text being edited on the canvas, if any.
    pub text_edit: Option<TextEditor>,
    /// Caret blink, drag selection and placement of the text being edited.
    text_ui: TextEditUi,
    properties: properties_panel::PropertyFields,
    /// The title last given to the window.
    window_title: String,
    _subscriptions: Vec<Subscription>,
}

impl EditorWindow {
    pub fn new(session: DocumentSession, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let (ribbon, ribbon_subscription) = toolbar::Ribbon::new(window, cx);
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
            // The caret blinks again when typing comes back to the canvas.
            cx.on_focus(&focus_handle, window, |this, window, cx| {
                if this.text_edit.is_some() {
                    this.restart_blink(window, cx);
                }
            }),
            ribbon_subscription,
        ]);
        let (icons, icons_subscription) = icons_panel::IconsPane::new(window, cx);
        _subscriptions.push(icons_subscription);
        let (topic_fields, topic_subscription) = topic_properties::TopicFields::new(window, cx);
        _subscriptions.push(topic_subscription);
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
            tools: ToolState::default(),
            ribbon,
            gesture: Gesture::Idle,
            press: Press::default(),
            selection: Vec::new(),
            line_ends: [None; 2],
            placing_relationship: false,
            topic_drag_from: None,
            attach_highlight: None,
            geometry_cache: GeometryCache::default(),
            image_cache: images::ImageCache::default(),
            icons,
            topic_fields,
            text_layouts: TextLayoutCache::default(),
            glyph_outlines: GlyphOutlines::default(),
            session_gesture: SessionGesture::default(),
            text_edit: None,
            text_ui: TextEditUi::default(),
            properties,
            window_title,
            _subscriptions,
        }
    }

    pub fn session(&self) -> &DocumentSession {
        &self.session
    }

    /// Applies `f` to the session, redraws, and refreshes the window title
    /// (on the next render). The selection is cleared when the active page
    /// or layer changes, and otherwise loses shapes no longer there. Any
    /// edit but a Text section size step first ends a run of size steps,
    /// so it is an undo step of its own.
    pub fn update_session(&mut self, f: impl FnOnce(&mut DocumentSession), cx: &mut Context<Self>) {
        let scope = self.selection_scope();
        let text = &mut self.properties.text;
        text_properties::edit_session(
            &mut text.size_steps,
            &mut self.session_gesture,
            &mut self.session,
            text.size_step_edit,
            f,
        );
        self.prune_selection(scope);
        cx.notify();
    }

    /// Sets the active page's viewport and redraws. A view change leaves
    /// the selection alone, so panning does no selection bookkeeping.
    pub fn set_viewport(&mut self, viewport: Viewport, cx: &mut Context<Self>) {
        self.session.set_viewport(viewport);
        cx.notify();
    }

    /// The active page and layer, which the selection belongs to.
    fn selection_scope(&self) -> (Uuid, Uuid) {
        let page = self.session.page();
        let layer = page
            .layers
            .get(page.active_layer)
            .map_or(Uuid::nil(), |l| l.id);
        (page.id, layer)
    }

    /// Brings the selection in line with the session after an edit made
    /// in selection scope `before` (see [`pruned`]).
    fn prune_selection(&mut self, before: (Uuid, Uuid)) {
        if self.selection.is_empty() {
            return;
        }
        let active = selectable_ids(self.session.shapes_on_active_layer());
        self.selection = pruned(
            std::mem::take(&mut self.selection),
            before,
            self.selection_scope(),
            &active,
            self.session.active_layer_selectable(),
        );
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

    /// Saves the document. Text being edited stays in editing: its edits
    /// so far become one undo step (an empty text box or label ends its
    /// editing first, which clears it). On failure shows a notification and
    /// returns `false`.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.commit_pending(true, window, cx);
        let editing = self.text_edit.as_ref().map(|e| e.target);
        let kept = editing.and_then(|target| {
            text_edit::save_text_session(&mut self.session, &mut self.session_gesture, target)
        });
        let result = kept.unwrap_or_else(|| {
            self.end_text_edit(window, cx);
            self.session.save()
        });
        let saved = match result {
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

    /// Applies a text size being typed (to the text it was typed for), ends
    /// text editing (its undo step), ends a colour picker's undo
    /// step, then applies edits that otherwise wait for Enter or blur: an
    /// open rename and a Properties number being typed (each its own undo
    /// step). Undo, Redo, Save (which keeps text editing open), Close,
    /// Quit, the edit actions and page and layer switches call this first, so keyboard shortcuts and clicks do
    /// not skip or misplace them.
    pub fn commit_pending_edits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_pending(false, window, cx);
    }

    /// [`Self::commit_pending_edits`]; with `keep_text`, text editing is
    /// left open (Save commits its step itself and goes on editing).
    fn commit_pending(&mut self, keep_text: bool, window: &mut Window, cx: &mut Context<Self>) {
        // A text size typed while editing belongs to the edited text.
        self.commit_focused_text_size(window, cx);
        self.finish_size_steps(cx);
        if !keep_text {
            self.end_text_edit(window, cx);
        }
        self.finish_color_edit(cx);
        if self.rename.is_some() {
            self.finish_rename(true, cx);
            // The rename field is gone; keep keyboard shortcuts working.
            self.focus_handle.focus(window, cx);
        }
        self.commit_focused_number(window, cx);
        self.commit_focused_shape_number(window, cx);
    }

    /// Starts an inline rename of `target` with its current name selected.
    pub(crate) fn begin_rename(
        &mut self,
        target: RenameTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = match target {
            RenameTarget::Page(ix) => self.session.doc().pages.get(ix).map(|p| p.name.clone()),
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

    /// Applies pending edits to the page and layer they were made on, and
    /// takes focus from the field they were typed in, which would
    /// otherwise keep showing the old page's value.
    fn commit_before_switching(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_pending_edits(window, cx);
        self.focus_handle.focus(window, cx);
    }

    /// Switches pages, applying pending edits first.
    pub(crate) fn activate_page(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_before_switching(window, cx);
        self.update_session(|s| s.set_active_page(ix), cx);
        // The new page has its own viewport.
        self.refresh_cursor(cx);
    }

    /// Makes layer `ix` of the active page active, applying pending edits
    /// first.
    pub(crate) fn activate_layer(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_before_switching(window, cx);
        self.update_session(|s| s.set_active_layer(ix), cx);
    }

    /// Adds a layer above the others and makes it active, applying pending
    /// edits first.
    pub(crate) fn add_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_before_switching(window, cx);
        self.update_session(
            |s| {
                s.add_layer();
            },
            cx,
        );
    }

    /// Adds a page, applying pending edits first.
    pub(crate) fn add_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_before_switching(window, cx);
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

    fn on_group(&mut self, _: &Group, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let ids = self.selection.clone();
        self.update_session(
            |s| {
                s.group_shapes(&ids);
            },
            cx,
        );
    }

    fn on_ungroup(&mut self, _: &Ungroup, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let ids = self.selection.clone();
        self.update_session(|s| s.ungroup_shapes(&ids), cx);
    }

    /// Moves the selection in the stacking order as one undo step.
    fn arrange(&mut self, how: Arrange, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let ids = self.selection.clone();
        self.update_session(|s| s.arrange_shapes(&ids, how), cx);
    }

    fn on_bring_to_front(&mut self, _: &BringToFront, window: &mut Window, cx: &mut Context<Self>) {
        self.arrange(Arrange::ToFront, window, cx);
    }

    fn on_bring_forward(&mut self, _: &BringForward, window: &mut Window, cx: &mut Context<Self>) {
        self.arrange(Arrange::Forward, window, cx);
    }

    fn on_send_backward(&mut self, _: &SendBackward, window: &mut Window, cx: &mut Context<Self>) {
        self.arrange(Arrange::Backward, window, cx);
    }

    fn on_send_to_back(&mut self, _: &SendToBack, window: &mut Window, cx: &mut Context<Self>) {
        self.arrange(Arrange::ToBack, window, cx);
    }

    /// Remembers the selection's style for new shapes, after a style edit
    /// in Properties.
    pub(super) fn remember_selection_style(&mut self, cx: &mut Context<Self>) {
        let shapes: Vec<Shape> = self
            .selection
            .iter()
            .filter_map(|id| self.session.find_shape(*id).cloned())
            .collect();
        AppState::remember_styles(cx, &shapes);
    }

    /// A right press at canvas point `at`: a shape (its whole group)
    /// becomes the selection unless it is already selected, so the menu
    /// acts on it; empty canvas clears the selection.
    pub(super) fn select_for_menu(&mut self, at: Pt, cx: &mut Context<Self>) {
        match self.shape_at(at, cx) {
            Some(id) if self.selection.contains(&id) => {}
            Some(id) => {
                self.selection = unit_members(self.session.shapes_on_active_layer(), id);
            }
            None => self.selection.clear(),
        }
        cx.notify();
    }

    /// Shows or hides the Icons pane; shown, its search box has focus.
    fn on_toggle_icons(&mut self, _: &ToggleIcons, window: &mut Window, cx: &mut Context<Self>) {
        AppState::update_settings(cx, |s| s.show_icons = !s.show_icons);
        if cx.global::<AppState>().settings.show_icons {
            let search = self.icons.search.clone();
            search.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.focus_handle.focus(window, cx);
        }
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

    /// Makes `tool` the active tool, locked when `lock` is set. Choosing a
    /// tool stops text editing.
    pub(crate) fn choose_tool(
        &mut self,
        tool: Tool,
        lock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_text_edit(window, cx);
        self.tools.select(tool, lock);
        cx.notify();
    }

    /// Makes `tool` the active (unlocked) tool, from its key.
    fn select_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        self.choose_tool(tool, false, window, cx);
    }

    fn on_tool_pointer(&mut self, _: &ToolPointer, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::Pointer, window, cx);
    }

    fn on_tool_hand(&mut self, _: &ToolHand, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::Hand, window, cx);
    }

    fn on_tool_text(&mut self, _: &ToolText, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::Text, window, cx);
    }

    fn on_tool_rectangle(
        &mut self,
        _: &ToolRectangle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tool(Tool::shape(BASIC_SHAPES, "rectangle"), window, cx);
    }

    fn on_tool_ellipse(&mut self, _: &ToolEllipse, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::shape(BASIC_SHAPES, "ellipse"), window, cx);
    }

    fn on_tool_line(&mut self, _: &ToolLine, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::shape(BASIC_SHAPES, "line"), window, cx);
    }

    /// Return: edits the one selected text box or labelled shape.
    /// Return: adds a topic after the one selected topic (a subtopic of a
    /// central topic); otherwise edits the selected text.
    fn on_edit_text(&mut self, _: &EditText, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_topic().is_some() {
            self.add_topic(mindmap_edit::NewTopic::After, window, cx);
            return;
        }
        self.begin_edit_action(window, cx);
        self.edit_selected_text(window, cx);
    }

    fn on_edit_label(&mut self, _: &EditLabel, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        self.edit_selected_text(window, cx);
    }

    fn on_add_subtopic(&mut self, _: &AddSubtopic, window: &mut Window, cx: &mut Context<Self>) {
        self.add_topic(mindmap_edit::NewTopic::Sub, window, cx);
    }

    fn on_add_sibling_before(
        &mut self,
        _: &AddSiblingBefore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_topic(mindmap_edit::NewTopic::Before, window, cx);
    }

    fn on_add_central_topic(
        &mut self,
        _: &AddCentralTopic,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_central_topic(window, cx);
    }

    fn on_add_summary(&mut self, _: &AddSummary, window: &mut Window, cx: &mut Context<Self>) {
        self.add_summary(window, cx);
    }

    fn on_toggle_boundary(
        &mut self,
        _: &ToggleBoundary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_boundary(window, cx);
    }

    fn on_tool_relationship(
        &mut self,
        _: &ToolRelationship,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_tool(Tool::Relationship, window, cx);
    }

    /// Escape: aborts the gesture in progress; otherwise clears the
    /// selection and goes back to the Pointer tool, unlocked.
    fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if !self.cancel_gesture(cx) {
            self.selection.clear();
            self.tools.cancel();
        }
        cx.notify();
    }

    /// Readies the editor for an edit action: applies pending edits and
    /// abandons a pointer gesture in progress (putting moved shapes back).
    fn begin_edit_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_pending_edits(window, cx);
        self.cancel_gesture(cx);
    }

    /// Undo and Redo. With nothing to undo or redo they do nothing (the
    /// menu items stay enabled). Selection pruning runs through
    /// `update_session`: shapes that are gone drop out, and a change of
    /// active page or layer clears it.
    fn step_history(&mut self, undo: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        self.update_session(
            |s| {
                if undo {
                    s.undo();
                } else {
                    s.redo();
                }
            },
            cx,
        );
        // Undoing an added page can move to another page and viewport.
        self.refresh_cursor(cx);
    }

    /// Undo; while text is being edited, undoes typing in it (the Edit
    /// menu item, like Cmd+Z).
    fn on_undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_canvas_text(TextCommand::Undo, window, cx) {
            return;
        }
        self.step_history(true, window, cx);
    }

    /// Redo; while text is being edited, redoes typing in it.
    fn on_redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_canvas_text(TextCommand::Redo, window, cx) {
            return;
        }
        self.step_history(false, window, cx);
    }

    /// The selected shapes, back to front.
    fn selected_shapes(&self) -> Vec<Shape> {
        self.shapes_among(&self.selection)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Puts the selected shapes on the clipboard; returns whether it did.
    /// With none selected the clipboard is left alone; with more than
    /// paste accepts, a notification says so.
    fn copy_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match clipboard_text(&self.selected_shapes()) {
            Ok(Some(text)) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            Ok(None) => false,
            Err(message) => {
                window.push_notification(Notification::warning(message), cx);
                false
            }
        }
    }

    /// Removes the selected shapes as one undo step.
    fn delete_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        let ids = self.selection.clone();
        // A topic goes with its branch and its relationships.
        self.update_session(
            |s| {
                s.delete_with_branches(&ids);
            },
            cx,
        );
    }

    /// Adds `shapes` to the active layer as one undo step and selects
    /// them. A hidden active layer refuses them, as it refuses placing.
    fn add_and_select(&mut self, shapes: Vec<Shape>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.session.active_layer_selectable() {
            window.push_notification(Notification::warning("Show the layer to draw on it."), cx);
            return;
        }
        let selection = selectable_in_order(&shapes, true);
        self.update_session(|s| s.add_shapes(shapes), cx);
        self.selection = selection;
        cx.notify();
    }

    /// Sends a text edit to the focused text field, if one has focus, and
    /// returns whether it did. Its keys never reach the editor, but Edit
    /// menu items do; they must edit the text, not the shapes. (gpui's
    /// `MenuItem::os_action` does not help: macOS sends the selector back
    /// to the app, which dispatches the same action.)
    fn forward_to_text_field(
        &self,
        action: impl Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let typing = in_text_field(&window.context_stack());
        if typing {
            window.dispatch_action(Box::new(action), cx);
        }
        typing
    }

    /// Sends an Edit menu item to the text being edited on the canvas, if
    /// any, and returns whether it did: while editing, these edit the
    /// text, not the shapes.
    fn forward_to_canvas_text(
        &mut self,
        command: TextCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let editing = self.text_edit.is_some();
        if editing {
            self.text_command(command, window, cx);
        }
        editing
    }

    fn on_copy(&mut self, _: &actions::Copy, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Copy, window, cx)
            || self.forward_to_canvas_text(TextCommand::Copy, window, cx)
        {
            return;
        }
        self.begin_edit_action(window, cx);
        self.copy_selection(window, cx);
    }

    fn on_cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Cut, window, cx)
            || self.forward_to_canvas_text(TextCommand::Cut, window, cx)
        {
            return;
        }
        self.begin_edit_action(window, cx);
        if self.copy_selection(window, cx) {
            self.delete_selection(cx);
        }
    }

    /// Pastes shapes copied from any Mindweave window onto the active
    /// layer: in place when that is in view, else centred in view. Other
    /// clipboard text (from another application) becomes a new text box
    /// at the view centre.
    fn on_paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Paste, window, cx)
            || self.forward_to_canvas_text(TextCommand::Paste, window, cx)
        {
            return;
        }
        self.begin_edit_action(window, cx);
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        // A Mindweave payload is text; anything else that is a picture,
        // or picture files, is placed as images.
        let payload = item.text().filter(|t| is_mindweave_payload(t));
        if payload.is_none() {
            if let Some(files) = images::clipboard_pictures(&item) {
                self.place_pictures(files, window, cx);
                return;
            }
        }
        let Some(text) = item.text() else {
            return;
        };
        match paste_plan(&text, &self.visible_canvas()) {
            PastePlan::Shapes(shapes) => self.add_and_select(shapes, window, cx),
            PastePlan::TextBox(mut shape) => {
                if let Some(label) = &shape.label {
                    let canvas = canvas_background(
                        self.session.page().canvas.background.as_deref(),
                        cx.theme().background,
                    );
                    let size = self.text_layouts.size(
                        window,
                        shape.id,
                        &label.text,
                        text_box_wrap(&shape),
                        text_color(&shape, canvas),
                    );
                    let view = self.visible_canvas();
                    centre_text_box(
                        &mut shape,
                        size,
                        (view.x + view.w / 2., view.y + view.h / 2.),
                    );
                }
                self.add_and_select(vec![*shape], window, cx);
            }
            PastePlan::Newer => window.push_notification(
                Notification::warning("This was copied from a newer version of Mindweave."),
                cx,
            ),
            PastePlan::Nothing => {}
        }
    }

    fn on_duplicate(&mut self, _: &Duplicate, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let copies = duplicate(&self.selected_shapes());
        if !copies.is_empty() {
            self.add_and_select(copies, window, cx);
        }
    }

    fn on_delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Delete, window, cx)
            || self.forward_to_canvas_text(TextCommand::Delete(false), window, cx)
        {
            return;
        }
        self.begin_edit_action(window, cx);
        self.delete_selection(cx);
    }

    /// Selects every unlocked shape of the active layer (none when it is
    /// hidden).
    fn on_select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::SelectAll, window, cx)
            || self.forward_to_canvas_text(TextCommand::SelectAll, window, cx)
        {
            return;
        }
        self.begin_edit_action(window, cx);
        self.selection = selectable_in_order(
            self.session.shapes_on_active_layer(),
            self.session.active_layer_selectable(),
        );
        cx.notify();
    }

    /// Space held over a focused editor turns left drags into pans (but
    /// types a space while editing text).
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if is_plain_space(&event.keystroke)
            && self.text_edit.is_none()
            && !self.space_held
            && self.focus_handle.is_focused(window)
        {
            self.space_held = true;
            cx.notify();
        }
    }

    /// Shift and Alt change the shape being placed, resized or rotated
    /// without a pointer move.
    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_gesture(None, event.modifiers, cx);
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

    /// The canvas area, with its right-click menu.
    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus_handle.clone();
        div()
            .id("canvas-area")
            .flex_1()
            .min_h_0()
            .child(canvas::canvas(cx.entity(), self.session.page()))
            .on_drop(
                cx.listener(|this, drag: &icons_panel::IconDrag, window, cx| {
                    // Dropped where the pointer is.
                    let Some(bounds) = this.canvas_bounds else {
                        return;
                    };
                    let p = window.mouse_position() - bounds.origin;
                    let at = this.to_canvas((f32::from(p.x), f32::from(p.y)));
                    // Dropped on a topic, the icon becomes the topic's icon.
                    let topic = this.shape_at(at, cx).filter(|id| {
                        this.session
                            .find_shape(*id)
                            .is_some_and(|s| s.topic.is_some())
                    });
                    match topic {
                        Some(id) => this.set_topic_picture_file(
                            &[id],
                            &drag.path.clone(),
                            mindweave_core::TOPIC_ICON_SIZE,
                            window,
                            cx,
                        ),
                        None => this.place_icon(&drag.path.clone(), Some(at), window, cx),
                    }
                }),
            )
            .context_menu(move |menu, _, _| canvas_menu(menu, focus.clone()))
    }
}

/// The canvas's right-click menu, acting on the selection.
fn canvas_menu(menu: PopupMenu, focus: FocusHandle) -> PopupMenu {
    menu.action_context(focus)
        .menu("Cut", Box::new(Cut))
        .menu("Copy", Box::new(actions::Copy))
        .menu("Paste", Box::new(Paste))
        .menu("Duplicate", Box::new(Duplicate))
        .menu("Delete", Box::new(Delete))
        .separator()
        .menu("Bring to Front", Box::new(BringToFront))
        .menu("Bring Forward", Box::new(BringForward))
        .menu("Send Backward", Box::new(SendBackward))
        .menu("Send to Back", Box::new(SendToBack))
        .separator()
        .menu("Group", Box::new(Group))
        .menu("Ungroup", Box::new(Ungroup))
}

/// The selection after a session edit: empty when the active page or
/// layer changed (`before` → `after`) or the active layer is hidden,
/// otherwise `selection` without ids missing from the active layer's
/// selectable (unlocked) shapes `active`, in its order.
fn pruned(
    mut selection: Vec<Uuid>,
    before: (Uuid, Uuid),
    after: (Uuid, Uuid),
    active: &HashSet<Uuid>,
    layer_visible: bool,
) -> Vec<Uuid> {
    if before != after || !layer_visible {
        return Vec::new();
    }
    selection.retain(|id| active.contains(id));
    selection
}

/// What Paste does with clipboard text (not while editing text).
#[derive(Debug, PartialEq)]
enum PastePlan {
    /// Adds these shapes.
    Shapes(Vec<Shape>),
    /// Adds this text box of pasted text, its top-left corner at the view
    /// centre until [`centre_text_box`] centres it there.
    TextBox(Box<Shape>),
    /// Refuses a payload from a newer Mindweave.
    Newer,
    Nothing,
}

/// What pasting clipboard `text` into `view` (the visible canvas area)
/// does: shapes copied in Mindweave, ready to paste; else, for text from
/// another application, a new text box at the view centre. Mindweave
/// clipboard text this build cannot read (a newer version, too large,
/// malformed) is never pasted as text.
fn paste_plan(text: &str, view: &Rect) -> PastePlan {
    if let Some(shapes) = from_clipboard(text) {
        return PastePlan::Shapes(prepare_paste(shapes, view));
    }
    if is_newer_payload(text) {
        return PastePlan::Newer;
    }
    if is_mindweave_payload(text) {
        return PastePlan::Nothing;
    }
    let centre = (view.x + view.w / 2., view.y + view.h / 2.);
    match plain_text_box(text, centre) {
        Some(shape) => PastePlan::TextBox(Box::new(shape)),
        None => PastePlan::Nothing,
    }
}

/// Sizes text box `shape` to fit its text, laid out `text_size` canvas
/// units big, and centres it on `centre`.
fn centre_text_box(shape: &mut Shape, (w, h): (f32, f32), centre: Pt) {
    let (w, h) = (w + 2. * TEXT_BOX_PADDING, h + 2. * TEXT_BOX_PADDING);
    let f = &mut shape.frame;
    (f.w, f.h) = (w, h);
    (f.x, f.y) = (centre.0 - w / 2., centre.1 - h / 2.);
}

/// Ids of the unlocked shapes in `shapes`, in order; none when their
/// layer is not visible. What Select All, Paste and Duplicate select.
fn selectable_in_order(shapes: &[Shape], layer_visible: bool) -> Vec<Uuid> {
    if !layer_visible {
        return Vec::new();
    }
    shapes.iter().filter(|s| !s.locked).map(|s| s.id).collect()
}

/// `shapes` as clipboard text: `Ok(None)` for none, `Err` with a message
/// for more than paste accepts ([`MAX_CLIPBOARD_SHAPES`]).
fn clipboard_text(shapes: &[Shape]) -> Result<Option<String>, String> {
    if shapes.len() > MAX_CLIPBOARD_SHAPES {
        return Err("Copy at most 10,000 shapes at a time.".into());
    }
    Ok((!shapes.is_empty()).then(|| to_clipboard(shapes)))
}

/// Ids of the shapes in `shapes` that can stay selected: the unlocked
/// ones.
fn selectable_ids(shapes: &[Shape]) -> HashSet<Uuid> {
    selectable_in_order(shapes, true).into_iter().collect()
}

/// Whether the focus, with key contexts `contexts` from the root down, is
/// in a text field (gpui's `Input` context).
fn in_text_field(contexts: &[KeyContext]) -> bool {
    contexts.iter().any(|c| c.contains("Input"))
}

/// The editor root's key context: [`EDITOR_CONTEXT`], plus
/// [`TEXT_EDITING_CONTEXT`] while text is being edited on the canvas.
fn key_context(editing_text: bool) -> KeyContext {
    let mut context = KeyContext::default();
    context.add(EDITOR_CONTEXT);
    if editing_text {
        context.add(TEXT_EDITING_CONTEXT);
    }
    context
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
        self.sync_topic_fields(window, cx);

        let (show_layers, show_properties, show_icons) = {
            let settings = &cx.global::<AppState>().settings;
            (
                settings.show_layers,
                settings.show_properties,
                settings.show_icons,
            )
        };
        let show_left = show_layers || show_icons;
        let panel_range = px(PANEL_MIN_WIDTH)..px(PANEL_MAX_WIDTH);

        let mut panels = h_resizable(panel_group_id(show_left, show_properties));
        if show_left {
            // Layers on top, Icons below; either may be hidden.
            let left = v_flex()
                .size_full()
                .when(show_layers, |left| {
                    left.child(
                        div()
                            .when(show_icons, |d| {
                                d.h(px(LAYERS_WITH_ICONS_HEIGHT)).flex_none()
                            })
                            .when(!show_icons, |d| d.flex_1())
                            .min_h_0()
                            .overflow_hidden()
                            .child(layers_panel::render(self, cx)),
                    )
                })
                .when(show_icons, |left| {
                    left.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .when(show_layers, |d| {
                                d.border_t_1().border_color(cx.theme().border)
                            })
                            .bg(cx.theme().sidebar)
                            .child(icons_panel::render(self, cx)),
                    )
                });
            panels = panels.child(
                resizable_panel()
                    .size(px(LAYERS_WIDTH))
                    .size_range(panel_range.clone())
                    .fixed()
                    .child(left),
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

        let root = v_flex()
            .size_full()
            .key_context(key_context(self.text_edit.is_some()))
            .track_focus(&self.focus_handle);
        text_edit::on_text_actions(root, cx)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_group))
            .on_action(cx.listener(Self::on_ungroup))
            .on_action(cx.listener(Self::on_bring_to_front))
            .on_action(cx.listener(Self::on_bring_forward))
            .on_action(cx.listener(Self::on_send_backward))
            .on_action(cx.listener(Self::on_send_to_back))
            .on_action(cx.listener(Self::on_toggle_icons))
            .on_action(cx.listener(Self::on_toggle_layers))
            .on_action(cx.listener(Self::on_toggle_properties))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(Self::on_tool_pointer))
            .on_action(cx.listener(Self::on_tool_hand))
            .on_action(cx.listener(Self::on_tool_text))
            .on_action(cx.listener(Self::on_edit_text))
            .on_action(cx.listener(Self::on_edit_label))
            .on_action(cx.listener(Self::on_add_subtopic))
            .on_action(cx.listener(Self::on_add_sibling_before))
            .on_action(cx.listener(Self::on_add_central_topic))
            .on_action(cx.listener(Self::on_add_summary))
            .on_action(cx.listener(Self::on_toggle_boundary))
            .on_action(cx.listener(Self::on_tool_relationship))
            .on_action(cx.listener(Self::on_tool_rectangle))
            .on_action(cx.listener(Self::on_tool_ellipse))
            .on_action(cx.listener(Self::on_tool_line))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_duplicate))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(|this, _: &NudgeLeft, _, cx| this.nudge(-NUDGE_STEP, 0., cx)))
            .on_action(cx.listener(|this, _: &NudgeRight, _, cx| this.nudge(NUDGE_STEP, 0., cx)))
            .on_action(cx.listener(|this, _: &NudgeUp, _, cx| this.nudge(0., -NUDGE_STEP, cx)))
            .on_action(cx.listener(|this, _: &NudgeDown, _, cx| this.nudge(0., NUDGE_STEP, cx)))
            .on_action(
                cx.listener(|this, _: &NudgeLeftFar, _, cx| this.nudge(-NUDGE_FAR_STEP, 0., cx)),
            )
            .on_action(
                cx.listener(|this, _: &NudgeRightFar, _, cx| this.nudge(NUDGE_FAR_STEP, 0., cx)),
            )
            .on_action(
                cx.listener(|this, _: &NudgeUpFar, _, cx| this.nudge(0., -NUDGE_FAR_STEP, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NudgeDownFar, _, cx| this.nudge(0., NUDGE_FAR_STEP, cx)),
            )
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(TEXT_SIZE))
            .child(menu_bar(
                WindowKind::Editor,
                Some(self.window_title.clone().into()),
                cx,
            ))
            .child(toolbar::render(self, cx))
            .child(div().flex_1().min_h_0().child(panels))
            .child(status_bar::render(self, cx))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::properties_panel::begin_color_step;
    use super::{
        centre_text_box, clipboard_text, hex_to_hsla, hsla_to_hex, in_text_field, is_plain_space,
        key_context, letter_spaced, panel_group_id, parse_number, paste_plan, pruned, row_click,
        selectable_ids, selectable_in_order, GestureOwner, PastePlan, RowClick, SessionGesture,
    };
    use gpui_kit::Keystroke;
    use mindweave_core::geometry::Frame;
    use mindweave_core::{
        builtin_libraries, from_clipboard, LibraryRegistry, Shape, MAX_CLIPBOARD_SHAPES,
    };
    use mindweave_core::{DocumentSession, RichText, RunStyle};
    use std::collections::HashSet;
    use uuid::Uuid;

    #[test]
    fn selection_is_cleared_or_pruned_after_session_edits() {
        let [page, other_page, layer, other_layer, a, b, c] = [(); 7].map(|_| Uuid::new_v4());
        let scope = (page, layer);
        let active: HashSet<Uuid> = [a, b, c].into();
        let sel = vec![c, a];

        // Unchanged scope keeps the order.
        assert_eq!(pruned(sel.clone(), scope, scope, &active, true), vec![c, a]);
        // Removed ids are dropped.
        let only_c: HashSet<Uuid> = [c].into();
        assert_eq!(pruned(sel.clone(), scope, scope, &only_c, true), vec![c]);
        // Page change, layer change and hidden layer clear it.
        assert!(pruned(sel.clone(), scope, (other_page, layer), &active, true).is_empty());
        assert!(pruned(sel.clone(), scope, (page, other_layer), &active, true).is_empty());
        assert!(pruned(sel, scope, scope, &active, false).is_empty());
    }

    #[test]
    fn copying_more_than_paste_accepts_is_refused() {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        let one = Shape::new(def, "basic-shapes", Frame::default());
        assert_eq!(clipboard_text(&[]), Ok(None));
        let mut shapes = vec![one; MAX_CLIPBOARD_SHAPES];
        let text = clipboard_text(&shapes).unwrap().unwrap();
        assert_eq!(
            from_clipboard(&text).map(|s| s.len()),
            Some(MAX_CLIPBOARD_SHAPES)
        );
        shapes.push(shapes[0].clone());
        assert_eq!(
            clipboard_text(&shapes),
            Err("Copy at most 10,000 shapes at a time.".into())
        );
    }

    #[test]
    fn locked_shapes_drop_out_of_the_selection() {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        let free = Shape::new(def, "basic-shapes", Frame::default());
        let mut locked = Shape::new(def, "basic-shapes", Frame::default());
        locked.locked = true;
        let ids = selectable_ids(&[free.clone(), locked.clone()]);
        let scope = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(
            pruned(vec![locked.id, free.id], scope, scope, &ids, true),
            vec![free.id]
        );
    }

    #[test]
    fn select_all_and_paste_select_unlocked_shapes_on_a_visible_layer() {
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        let [a, mut locked, b] = [(); 3].map(|_| Shape::new(def, "basic-shapes", Frame::default()));
        locked.locked = true;
        let shapes = [a.clone(), locked, b.clone()];
        assert_eq!(selectable_in_order(&shapes, true), vec![a.id, b.id]);
        assert!(selectable_in_order(&shapes, false).is_empty());
        assert!(selectable_in_order(&[], true).is_empty());
    }

    #[test]
    fn edit_menu_items_go_to_a_focused_text_field() {
        use gpui_kit::KeyContext;
        let stack = |names: &[&str]| -> Vec<KeyContext> {
            names
                .iter()
                .map(|n| KeyContext::parse(n).unwrap())
                .collect()
        };
        assert!(in_text_field(&stack(&["Root", "Editor", "Input"])));
        assert!(!in_text_field(&stack(&["Root", "Editor"])));
        assert!(!in_text_field(&[]));
    }

    #[test]
    fn a_colour_change_while_editing_text_stays_in_the_one_session_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("g.mindweave")).unwrap();
        let shape = Shape::new_text_box((0., 0.));
        let id = shape.id;
        s.add_shapes(vec![shape]);
        let set_text = |s: &mut DocumentSession, t: &str| {
            s.update_shapes(&[id], |sh| {
                let label = sh.label.as_mut().unwrap();
                label.text = RichText::new(RunStyle::default(), Default::default());
                label.text.insert(0, t, &RunStyle::default());
            })
        };
        let mut g = SessionGesture::default();
        assert!(g.begin(&mut s, GestureOwner::Text));
        set_text(&mut s, "a");
        // A colour picker used while editing does not own the gesture, so
        // closing it does not end the editing session's step.
        assert!(!g.begin(&mut s, GestureOwner::Color));
        s.update_shapes(&[id], |sh| sh.style.fill = Some("#FF0000".into()));
        assert!(!g.end(&mut s, GestureOwner::Color));
        set_text(&mut s, "ab");
        assert!(g.end(&mut s, GestureOwner::Text));
        // One step back is the state before the session…
        assert!(s.undo());
        let shape = s.find_shape(id).unwrap();
        assert!(shape.label.as_ref().unwrap().text.is_empty());
        assert_eq!(shape.style.fill, None);
        // …and the next one is adding the box.
        assert!(s.undo());
        assert!(s.find_shape(id).is_none());
        // Nothing is left open.
        assert!(!g.end(&mut s, GestureOwner::Text));
        assert!(g.begin(&mut s, GestureOwner::Pointer));
        assert!(g.end(&mut s, GestureOwner::Pointer));
    }

    #[test]
    fn a_colour_drag_after_a_text_session_is_its_own_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("c.mindweave")).unwrap();
        let shape = Shape::new_text_box((0., 0.));
        let id = shape.id;
        s.add_shapes(vec![shape]);
        let fill = |s: &mut DocumentSession, hex: &str| {
            s.update_shapes(&[id], |sh| sh.style.fill = Some(hex.into()))
        };
        let mut g = SessionGesture::default();
        let mut color_edit = false;
        assert!(g.begin(&mut s, GestureOwner::Text));
        // The picker opens while text is being edited: its changes join the
        // editing session, and it does not count as a colour step.
        begin_color_step(&mut g, &mut s, true, &mut color_edit);
        assert!(!color_edit);
        fill(&mut s, "#FF0000");
        // Editing ends with the picker still open.
        assert!(g.end(&mut s, GestureOwner::Text));
        // Further picker changes open a colour step of their own…
        begin_color_step(&mut g, &mut s, true, &mut color_edit);
        assert!(color_edit);
        fill(&mut s, "#00FF00");
        begin_color_step(&mut g, &mut s, true, &mut color_edit);
        fill(&mut s, "#0000FF");
        // …which the picker closing ends, as one step.
        assert!(g.end(&mut s, GestureOwner::Color));
        assert!(s.undo());
        assert_eq!(
            s.find_shape(id).unwrap().style.fill.as_deref(),
            Some("#FF0000")
        );
        // A change from a closed picker is not a colour step.
        let mut closed = false;
        begin_color_step(&mut g, &mut s, false, &mut closed);
        assert!(!closed);
        assert!(g.begin(&mut s, GestureOwner::Pointer));
    }

    #[test]
    fn unreadable_mindweave_payloads_are_not_pasted_as_text() {
        use mindweave_core::geometry::Rect;
        use mindweave_core::MAX_CLIPBOARD_BYTES;
        let view = Rect {
            x: 0.,
            y: 0.,
            w: 200.,
            h: 100.,
        };
        let payload = mindweave_core::to_clipboard(&[Shape::new_text_box((10., 10.))]);
        assert!(matches!(paste_plan(&payload, &view), PastePlan::Shapes(s) if s.len() == 1));
        let newer = payload.replace("\"version\":3", "\"version\":99");
        assert_eq!(paste_plan(&newer, &view), PastePlan::Newer);
        let big = format!(
            "{{\"mindweave\":\"shapes\",\"version\":3,\"pad\":\"{}\"}}",
            "x".repeat(MAX_CLIPBOARD_BYTES)
        );
        assert_eq!(paste_plan(&big, &view), PastePlan::Nothing);
        // Text from another application: a text box centred in view once
        // its text is measured.
        match paste_plan("hello", &view) {
            PastePlan::TextBox(mut text_box) => {
                assert!(text_box.is_text_box());
                assert_eq!(text_box.label.as_ref().unwrap().text.plain(), "hello");
                // "hello" laid out 40 × 16, with 4 units of padding around.
                centre_text_box(&mut text_box, (40., 16.), (100., 50.));
                let f = text_box.frame;
                assert_eq!((f.x, f.y, f.w, f.h), (76., 38., 48., 24.));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(paste_plan("   ", &view), PastePlan::Nothing);
    }

    #[test]
    fn the_root_context_says_when_text_is_being_edited() {
        let editing = key_context(true);
        assert!(editing.contains("Editor") && editing.contains("TextEditing"));
        let not = key_context(false);
        assert!(not.contains("Editor") && !not.contains("TextEditing"));
    }

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
