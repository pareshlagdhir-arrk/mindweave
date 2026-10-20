//! The editor window for one document: toolbar, Layers panel, canvas, page
//! tabs, Properties panel and status bar.

mod canvas;
mod interaction;
mod layers_panel;
mod page_tabs;
mod properties_panel;
mod shape_paint;
mod shape_properties;
mod status_bar;
pub mod tool;
mod toolbar;

use std::collections::HashSet;
use std::future::Future;

use gpui_kit::component::input::{self, Escape, Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::{v_flex, ActiveTheme as _, Sizable as _, WindowExt as _};
use gpui_kit::*;
use mindweave_core::{
    duplicate, format_hex_color, from_clipboard, parse_hex_color, prepare_paste, to_clipboard,
    DocumentSession, Shape, Viewport,
};

// `Copy` is used by path: importing it would hide the derivable trait.
use crate::actions::{
    self, Cancel, CloseWindow, Cut, Delete, Duplicate, NudgeDown, NudgeDownFar, NudgeLeft,
    NudgeLeftFar, NudgeRight, NudgeRightFar, NudgeUp, NudgeUpFar, Paste, Redo, Save, SelectAll,
    ToggleLayers, ToggleProperties, ToolArrow, ToolEllipse, ToolHand, ToolLine, ToolPointer,
    ToolRectangle, Undo, EDITOR_CONTEXT,
};
use crate::app_state::AppState;
use crate::menu_bar::{menu_bar, WindowKind};
use interaction::{Gesture, Press, NUDGE_FAR_STEP, NUDGE_STEP};
use shape_paint::GeometryCache;
use tool::{Tool, ToolState, BASIC_SHAPES};
use uuid::Uuid;

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
    /// The active tool and the toolbar tab being shown.
    pub tools: ToolState,
    /// The pointer gesture in progress on the canvas.
    pub gesture: Gesture,
    /// The left press behind a Pointer-tool gesture.
    press: Press,
    /// Selected shapes, all on the active layer.
    pub selection: Vec<Uuid>,
    /// Canvas outlines of the shapes painted last frame.
    geometry_cache: GeometryCache,
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
            tools: ToolState::default(),
            gesture: Gesture::Idle,
            press: Press::default(),
            selection: Vec::new(),
            geometry_cache: GeometryCache::default(),
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
    /// or layer changes, and otherwise loses shapes no longer there.
    pub fn update_session(&mut self, f: impl FnOnce(&mut DocumentSession), cx: &mut Context<Self>) {
        let scope = self.selection_scope();
        f(&mut self.session);
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

    /// Ends a colour picker's undo step, then applies edits that otherwise
    /// wait for Enter or blur: an open rename and a Properties number being
    /// typed (each its own undo step). Undo, Redo, Save, Close, Quit, the
    /// edit actions and page and layer switches call this first, so
    /// keyboard shortcuts and clicks do not skip or misplace them.
    pub fn commit_pending_edits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Makes `tool` the active (unlocked) tool, from its key.
    fn select_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tools.select(tool, false);
        cx.notify();
    }

    fn on_tool_pointer(&mut self, _: &ToolPointer, _: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::Pointer, cx);
    }

    fn on_tool_hand(&mut self, _: &ToolHand, _: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::Hand, cx);
    }

    fn on_tool_rectangle(&mut self, _: &ToolRectangle, _: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::shape(BASIC_SHAPES, "rectangle"), cx);
    }

    fn on_tool_ellipse(&mut self, _: &ToolEllipse, _: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::shape(BASIC_SHAPES, "ellipse"), cx);
    }

    fn on_tool_line(&mut self, _: &ToolLine, _: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::shape(BASIC_SHAPES, "line"), cx);
    }

    fn on_tool_arrow(&mut self, _: &ToolArrow, _: &mut Window, cx: &mut Context<Self>) {
        self.select_tool(Tool::shape(BASIC_SHAPES, "arrow"), cx);
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

    fn on_undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, window, cx);
    }

    fn on_redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, window, cx);
    }

    /// The selected shapes, back to front.
    fn selected_shapes(&self) -> Vec<Shape> {
        self.shapes_among(&self.selection)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Puts the selected shapes on the clipboard; returns whether there
    /// were any (with none the clipboard is left alone).
    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let shapes = self.selected_shapes();
        if shapes.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(to_clipboard(&shapes)));
        true
    }

    /// Removes the selected shapes as one undo step.
    fn delete_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        let ids = self.selection.clone();
        self.update_session(
            |s| {
                s.remove_shapes(&ids);
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

    fn on_copy(&mut self, _: &actions::Copy, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Copy, window, cx) {
            return;
        }
        self.begin_edit_action(window, cx);
        self.copy_selection(cx);
    }

    fn on_cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Cut, window, cx) {
            return;
        }
        self.begin_edit_action(window, cx);
        if self.copy_selection(cx) {
            self.delete_selection(cx);
        }
    }

    /// Pastes shapes copied from any Mindweave window onto the active
    /// layer: in place when that is in view, else centred in view.
    /// Clipboard text that is not a shapes payload is ignored.
    fn on_paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Paste, window, cx) {
            return;
        }
        self.begin_edit_action(window, cx);
        let Some(shapes) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .and_then(|text| from_clipboard(&text))
        else {
            return;
        };
        let shapes = prepare_paste(shapes, &self.visible_canvas());
        self.add_and_select(shapes, window, cx);
    }

    fn on_duplicate(&mut self, _: &Duplicate, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let copies = duplicate(&self.selected_shapes());
        if !copies.is_empty() {
            self.add_and_select(copies, window, cx);
        }
    }

    fn on_delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::Delete, window, cx) {
            return;
        }
        self.begin_edit_action(window, cx);
        self.delete_selection(cx);
    }

    /// Selects every unlocked shape of the active layer (none when it is
    /// hidden).
    fn on_select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.forward_to_text_field(input::SelectAll, window, cx) {
            return;
        }
        self.begin_edit_action(window, cx);
        self.selection = selectable_in_order(
            self.session.shapes_on_active_layer(),
            self.session.active_layer_selectable(),
        );
        cx.notify();
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

    /// The canvas area.
    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .child(canvas::canvas(cx.entity(), self.session.page()))
    }
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

/// Ids of the unlocked shapes in `shapes`, in order; none when their
/// layer is not visible. What Select All, Paste and Duplicate select.
fn selectable_in_order(shapes: &[Shape], layer_visible: bool) -> Vec<Uuid> {
    if !layer_visible {
        return Vec::new();
    }
    shapes.iter().filter(|s| !s.locked).map(|s| s.id).collect()
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
            .key_context(EDITOR_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_toggle_layers))
            .on_action(cx.listener(Self::on_toggle_properties))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(Self::on_tool_pointer))
            .on_action(cx.listener(Self::on_tool_hand))
            .on_action(cx.listener(Self::on_tool_rectangle))
            .on_action(cx.listener(Self::on_tool_ellipse))
            .on_action(cx.listener(Self::on_tool_line))
            .on_action(cx.listener(Self::on_tool_arrow))
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
    use super::{
        hex_to_hsla, hsla_to_hex, in_text_field, is_plain_space, letter_spaced, panel_group_id,
        parse_number, pruned, row_click, selectable_ids, selectable_in_order, RowClick,
    };
    use gpui_kit::Keystroke;
    use mindweave_core::geometry::Frame;
    use mindweave_core::{builtin_libraries, LibraryRegistry, Shape};
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
