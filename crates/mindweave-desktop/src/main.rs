mod actions;
mod app_icon;
mod app_state;
mod assets;
mod dashboard;
mod editor;
mod equation;
mod icon_library;
mod libraries;
mod menu_bar;
mod settings_window;
mod theme_picker;
mod themes;

use app_state::AppState;

fn main() {
    // The full icon set: user libraries may name any Lucide icon.
    let app = gpui_kit::application().with_assets(assets::Assets);
    app.on_reopen(AppState::reopened);
    app.run(|cx| {
        gpui_kit::init(cx);
        app_icon::set_dock_icon();
        themes::init_themes(cx);
        AppState::init(cx);
        cx.activate(true);
    });
}
