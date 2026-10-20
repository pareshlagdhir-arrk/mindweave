//! The editor window for one document.
//!
//! Placeholder until the editor shell lands: it shows the document name under
//! the menu bar, saves, and asks before closing with unsaved changes.

use std::future::Future;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::*;
use mindweave_core::DocumentSession;

use crate::actions::{CloseWindow, Save};
use crate::app_state::AppState;
use crate::menu_bar::{menu_bar, WindowKind};

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

pub struct EditorWindow {
    session: DocumentSession,
    focus_handle: FocusHandle,
}

impl EditorWindow {
    pub fn new(session: DocumentSession, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            session,
            focus_handle,
        }
    }

    pub fn session(&self) -> &DocumentSession {
        &self.session
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
        let saved = match self.session.save() {
            Ok(()) => true,
            Err(err) => {
                window.push_notification(Notification::error(err.to_string()), cx);
                false
            }
        };
        window.set_window_title(&self.session.title());
        cx.notify();
        saved
    }

    /// Closes the window, first asking to save when there are unsaved
    /// changes. Used by Close Window and the window's close button.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        self.save(window, cx);
    }

    fn on_close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close(window, cx);
    }
}

fn close(window: &mut Window, cx: &mut App) {
    AppState::remember_window_size(WindowKind::Editor, window, cx);
    window.remove_window();
}

impl Focusable for EditorWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_close_window))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(13.))
            .child(menu_bar(WindowKind::Editor, cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Editor — {}", self.session.title())),
            )
    }
}
