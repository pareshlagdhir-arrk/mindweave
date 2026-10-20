mod actions;
mod app_state;
mod dashboard;
mod editor;
mod menu_bar;
mod theme_picker;
mod themes;

use app_state::AppState;

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.on_reopen(AppState::reopened);
    app.run(|cx| {
        gpui_kit::init(cx);
        themes::init_themes(cx);
        AppState::init(cx);
        cx.activate(true);
    });
}
