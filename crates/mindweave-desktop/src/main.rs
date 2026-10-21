mod actions;
mod app_state;
mod dashboard;
mod editor;
mod icon_library;
mod libraries;
mod menu_bar;
mod theme_picker;
mod themes;

use app_state::AppState;

fn main() {
    // The full icon set: user libraries may name any Lucide icon.
    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.on_reopen(AppState::reopened);
    app.run(|cx| {
        gpui_kit::init(cx);
        themes::init_themes(cx);
        AppState::init(cx);
        cx.activate(true);
    });
}
