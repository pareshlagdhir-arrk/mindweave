//! The dashboard window: a file browser shown on launch, with a New file
//! tile and one tile per recently opened document.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::*;

use crate::actions::{CloseWindow, ToggleThemePicker};
use crate::app_state::AppState;
use crate::menu_bar::{menu_bar, WindowKind};

const NAV_WIDTH: f32 = 200.;
const TILE_WIDTH: f32 = 180.;
const TILE_HEIGHT: f32 = 152.;
const THUMBNAIL_HEIGHT: f32 = 100.;
const DOT_SPACING: f32 = 12.;
const DOT_SIZE: f32 = 2.;
const GRID_GAP: f32 = 14.;
const GRID_PADDING: f32 = 20.;
const STATUS_BAR_HEIGHT: f32 = 24.;
const TEXT_SIZE: f32 = 13.;
const SMALL_TEXT_SIZE: f32 = 12.;

pub struct Dashboard {
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Dashboard {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let _subscriptions = vec![
            // Settings changes (a file opened, a missing file forgotten,
            // a theme picked) update the tiles and status bar.
            cx.observe_global::<AppState>(|_, cx| cx.notify()),
            // Files deleted while the app was in the background drop out
            // of the grid when the dashboard is next activated.
            cx.observe_window_activation(window, |_, _, cx| cx.notify()),
        ];
        Self {
            focus_handle,
            _subscriptions,
        }
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
            .text_size(px(TEXT_SIZE))
            .child(menu_bar(WindowKind::Dashboard, None, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(nav(cx))
                    .child(file_grid(cx)),
            )
            .child(status_bar(cx))
    }
}

/// The left navigation: Recent (the only section) and an Open… button.
fn nav(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .w(px(NAV_WIDTH))
        .flex_none()
        .p(px(8.))
        .gap(px(2.))
        .bg(theme.sidebar)
        .border_r_1()
        .border_color(theme.sidebar_border)
        .child(
            h_flex()
                .h(px(26.))
                .px(px(8.))
                .rounded(theme.radius)
                .bg(theme.list_active)
                .child("Recent"),
        )
        .child(
            Button::new("open-file")
                .ghost()
                .small()
                .w_full()
                .justify_start()
                .mt(px(6.))
                .icon(IconName::FolderOpen)
                .label("Open…")
                .on_click(|_, _, cx| AppState::open_file_dialog(cx)),
        )
}

/// The New file tile followed by one tile per recent file that still exists.
fn file_grid(cx: &App) -> impl IntoElement {
    let recent: Vec<(PathBuf, u64)> = cx
        .global::<AppState>()
        .settings
        .recent_existing()
        .into_iter()
        .map(|r| {
            let modified = std::fs::metadata(&r.path).and_then(|m| m.modified()).ok();
            (r.path.clone(), last_edited(modified, r.opened_at))
        })
        .collect();
    let now = unix_now();

    let mut grid = div()
        .flex()
        .flex_wrap()
        .items_start()
        .gap(px(GRID_GAP))
        .child(new_file_tile(cx));
    if recent.is_empty() {
        grid = grid.child(
            div()
                .h(px(TILE_HEIGHT))
                .flex()
                .items_center()
                .text_size(px(SMALL_TEXT_SIZE))
                .text_color(cx.theme().muted_foreground)
                .child("No recent files"),
        );
    } else {
        grid = grid.children(
            recent
                .into_iter()
                .enumerate()
                .map(|(ix, (path, edited_at))| file_tile(ix, path, edited_at, now, cx)),
        );
    }

    div()
        .id("dashboard-files")
        .flex_1()
        .min_w_0()
        .overflow_y_scroll()
        .p(px(GRID_PADDING))
        .child(grid)
}

fn new_file_tile(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .id("new-file")
        .w(px(TILE_WIDTH))
        .h(px(TILE_HEIGHT))
        .flex_none()
        .justify_center()
        .gap(px(6.))
        .rounded(theme.radius_lg)
        .border_1()
        .border_dashed()
        .border_color(theme.muted_foreground.opacity(0.5))
        .text_color(theme.primary)
        .cursor_pointer()
        .hover(|style| style.bg(theme.list_hover))
        .child(Icon::new(IconName::Plus).small())
        .child("New file")
        .on_click(|_, _, cx| AppState::new_file(cx))
}

fn file_tile(ix: usize, path: PathBuf, edited_at: u64, now: u64, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let tooltip: SharedString = path.display().to_string().into();
    v_flex()
        .id(("recent-file", ix))
        .w(px(TILE_WIDTH))
        .h(px(TILE_HEIGHT))
        .flex_none()
        .overflow_hidden()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.colors.list)
        .cursor_pointer()
        .hover(|style| style.bg(theme.list_hover))
        .child(
            div()
                .h(px(THUMBNAIL_HEIGHT))
                .flex_none()
                .bg(theme.muted)
                .border_b_1()
                .border_color(theme.border)
                .child(dot_pattern(theme.muted_foreground.opacity(0.4))),
        )
        .child(
            v_flex()
                .flex_1()
                .justify_center()
                .gap(px(2.))
                .px(px(10.))
                .child(div().truncate().child(file_stem(&path)))
                .child(
                    div()
                        .text_size(px(SMALL_TEXT_SIZE))
                        .text_color(theme.muted_foreground)
                        .child(format_relative(edited_at, now)),
                ),
        )
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            let path = path.clone();
            // Opening a window is deferred until this window's update ends.
            cx.defer(move |cx| AppState::open_path(path, cx));
        })
}

/// A placeholder thumbnail: a regular grid of small dots.
fn dot_pattern(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let step = px(DOT_SPACING);
            let dot = gpui_kit::size(px(DOT_SIZE), px(DOT_SIZE));
            let mut y = bounds.top() + step / 2.;
            while y < bounds.bottom() {
                let mut x = bounds.left() + step / 2.;
                while x < bounds.right() {
                    window.paint_quad(
                        fill(Bounds::new(point(x, y), dot), color).corner_radii(px(DOT_SIZE / 2.)),
                    );
                    x += step;
                }
                y += step;
            }
        },
    )
    .size_full()
}

/// Left: "Ready". Right: the theme name, which opens the theme picker.
fn status_bar(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let foreground = theme.foreground;
    h_flex()
        .h(px(STATUS_BAR_HEIGHT))
        .flex_none()
        .px(px(12.))
        .bg(theme.status_bar)
        .border_t_1()
        .border_color(theme.status_bar_border)
        .text_size(px(SMALL_TEXT_SIZE))
        .text_color(theme.muted_foreground)
        .child("Ready")
        .child(div().flex_1())
        .child(
            div()
                .id("theme-name")
                .cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .child(theme.theme_name().clone())
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleThemePicker), cx)),
        )
}

/// The file name without its extension, as shown on a tile.
fn file_stem(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// When a recent file was last edited (Unix seconds): its modification
/// time, or `opened_at` when that is unknown.
fn last_edited(modified: Option<SystemTime>, opened_at: u64) -> u64 {
    modified
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(opened_at, |d| d.as_secs())
}

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// How long ago `time` was, relative to `now` (both Unix seconds):
/// "Just now", "N min ago", "N h ago", "Yesterday", "N days ago" (2–6
/// days), then the UTC date as "D Mon YYYY". Times in the future count as
/// "Just now".
pub fn format_relative(time: u64, now: u64) -> String {
    let elapsed = now.saturating_sub(time);
    if elapsed < MINUTE {
        "Just now".to_string()
    } else if elapsed < HOUR {
        format!("{} min ago", elapsed / MINUTE)
    } else if elapsed < DAY {
        format!("{} h ago", elapsed / HOUR)
    } else if elapsed < 2 * DAY {
        "Yesterday".to_string()
    } else if elapsed < 7 * DAY {
        format!("{} days ago", elapsed / DAY)
    } else {
        let (year, month, day) = civil_from_days(time / DAY);
        format!("{day} {} {year}", MONTHS[(month - 1) as usize])
    }
}

/// The proleptic Gregorian (year, month 1–12, day 1–31) of `days` since
/// 1970-01-01, after Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{civil_from_days, file_stem, format_relative, last_edited};
    use std::path::Path;
    use std::time::{Duration, UNIX_EPOCH};

    const NOW: u64 = 1_790_000_000;

    fn ago(seconds: u64) -> String {
        format_relative(NOW - seconds, NOW)
    }

    #[test]
    fn format_relative_buckets() {
        assert_eq!(ago(0), "Just now");
        assert_eq!(ago(120), "2 min ago");
        assert_eq!(ago(7200), "2 h ago");
        assert_eq!(ago(86_400 + 10), "Yesterday");
        assert_eq!(ago(3 * 86_400), "3 days ago");
    }

    #[test]
    fn format_relative_bucket_edges() {
        assert_eq!(ago(59), "Just now");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(3_599), "59 min ago");
        assert_eq!(ago(3_600), "1 h ago");
        assert_eq!(ago(86_399), "23 h ago");
        assert_eq!(ago(86_400), "Yesterday");
        assert_eq!(ago(2 * 86_400), "2 days ago");
        assert_eq!(ago(7 * 86_400 - 1), "6 days ago");
        // A clock that moved backwards does not underflow.
        assert_eq!(format_relative(NOW + 500, NOW), "Just now");
    }

    #[test]
    fn format_relative_shows_a_date_after_a_week() {
        // 1_790_000_000 is 2026-09-21 14:13:20 UTC.
        assert_eq!(format_relative(NOW, NOW + 30 * 86_400), "21 Sep 2026");
        assert_eq!(format_relative(NOW, NOW + 7 * 86_400), "21 Sep 2026");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 951_782_400 is 2000-02-29 00:00:00 UTC.
        assert_eq!(civil_from_days(951_782_400 / 86_400), (2000, 2, 29));
        assert_eq!(civil_from_days(1_790_000_000 / 86_400), (2026, 9, 21));
        // 1_798_675_200 is 2026-12-31 00:00:00 UTC.
        assert_eq!(civil_from_days(1_798_675_200 / 86_400), (2026, 12, 31));
    }

    #[test]
    fn tiles_show_the_file_stem() {
        assert_eq!(
            file_stem(Path::new("/tmp/Q4 roadmap.mindweave")),
            "Q4 roadmap"
        );
        assert_eq!(file_stem(Path::new("/tmp/notes")), "notes");
    }

    #[test]
    fn tiles_show_the_modified_time_else_the_opened_time() {
        let modified = UNIX_EPOCH + Duration::from_secs(NOW - 300);
        assert_eq!(last_edited(Some(modified), NOW - 9_000), NOW - 300);
        assert_eq!(last_edited(None, NOW - 9_000), NOW - 9_000);
        let before_epoch = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(last_edited(Some(before_epoch), NOW - 9_000), NOW - 9_000);
    }
}
