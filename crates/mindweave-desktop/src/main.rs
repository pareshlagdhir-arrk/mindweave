use gpui_kit::component::{ActiveTheme as _, Root};
use gpui_kit::*;

// Opened by the `ToggleThemePicker` action, which Task 7 adds.
#[allow(dead_code)]
mod theme_picker;
mod themes;

struct MindweaveView;

impl Render for MindweaveView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(cx.theme().background)
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            themes::init_themes(cx);
            themes::apply_theme(themes::DEFAULT_THEME, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Mindweave".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|_| MindweaveView);
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        });
}
