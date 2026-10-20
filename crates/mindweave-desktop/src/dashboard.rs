//! The dashboard window.
//!
//! Placeholder until the dashboard lands: it shows its name under the menu
//! bar and closes on Close Window.

use gpui_kit::component::{v_flex, ActiveTheme as _};
use gpui_kit::*;

use crate::actions::CloseWindow;
use crate::app_state::AppState;
use crate::menu_bar::{menu_bar, WindowKind};

pub struct Dashboard {
    focus_handle: FocusHandle,
}

impl Dashboard {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self { focus_handle }
    }

    fn on_close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        AppState::remember_window_size(WindowKind::Dashboard, window, cx);
        window.remove_window();
    }
}

impl Focusable for Dashboard {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close_window))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(13.))
            .child(menu_bar(WindowKind::Dashboard, cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(cx.theme().muted_foreground)
                    .child("Dashboard"),
            )
    }
}
