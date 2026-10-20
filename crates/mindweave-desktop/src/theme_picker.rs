//! A compact, filterable theme picker shown as an overlay near the top of a
//! window. Moving the highlight previews the theme; Enter confirms it and
//! Escape (or a click outside) restores the theme that was active on open.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, IconName, IndexPath, Sizable as _, WindowExt as _};
use gpui_kit::*;

use crate::themes::{apply_theme, theme_names};

/// Called with the confirmed theme name.
pub type OnConfirm = Rc<dyn Fn(&str, &mut App)>;

const PICKER_WIDTH: f32 = 420.;
const PICKER_TOP: f32 = 72.;
const ROW_HEIGHT: f32 = 26.;
const SEARCH_HEIGHT: f32 = 36.;
const LIST_PADDING_Y: f32 = 4.;
const MAX_VISIBLE_ROWS: usize = 12;

/// Shows the theme picker as an overlay in `window`.
///
/// `on_confirm` runs with the chosen theme name when the user presses Enter
/// or clicks a row; the theme itself is already applied by then.
pub fn open_theme_picker(on_confirm: OnConfirm, window: &mut Window, cx: &mut App) {
    let picker = cx.new(|cx| ThemePicker::new(on_confirm, window, cx));
    // The same name drives the check mark and the Escape restore.
    let original = picker.read(cx).original(cx);
    let focus = picker.focus_handle(cx);
    let initial_focus = Cell::new(true);

    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            focus.focus(window, cx);
        }
        let original = original.clone();
        dialog
            .w(px(PICKER_WIDTH))
            .margin_top(px(PICKER_TOP))
            .p_0()
            .close_button(false)
            .on_cancel(move |_, _, cx| {
                apply_theme(&original, cx);
                true
            })
            .child(picker.clone())
    });
}

/// The picker view: a search input above the list of theme names.
pub struct ThemePicker {
    list: Entity<ListState<ThemeListDelegate>>,
}

impl ThemePicker {
    pub fn new(on_confirm: OnConfirm, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = ThemeListDelegate::new(on_confirm, cx);
        let current = delegate.current_row();
        let list = cx.new(|cx| {
            let mut state = ListState::new(delegate, window, cx).searchable(true);
            if let Some(row) = current {
                state.set_selected_index(Some(IndexPath::new(row)), window, cx);
                state.scroll_to_selected_item(window, cx);
            }
            state
        });
        Self { list }
    }

    /// The theme that was active when the picker opened.
    pub fn original(&self, cx: &App) -> SharedString {
        self.list.read(cx).delegate().original.clone()
    }
}

impl Focusable for ThemePicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.list.focus_handle(cx)
    }
}

impl Render for ThemePicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .list
            .read(cx)
            .delegate()
            .matches
            .len()
            .clamp(1, MAX_VISIBLE_ROWS);
        let height = SEARCH_HEIGHT + LIST_PADDING_Y * 2. + rows as f32 * ROW_HEIGHT;

        div().w_full().h(px(height)).text_size(px(13.)).child(
            List::new(&self.list)
                .small()
                .py(px(LIST_PADDING_Y))
                .search_placeholder("Select theme...")
                .size_full(),
        )
    }
}

/// Supplies theme names to the list and previews the highlighted theme.
pub struct ThemeListDelegate {
    on_confirm: OnConfirm,
    /// The theme active when the picker opened; marked with a check.
    original: SharedString,
    all: Vec<SharedString>,
    matches: Vec<SharedString>,
    selected: Option<usize>,
}

impl ThemeListDelegate {
    fn new(on_confirm: OnConfirm, cx: &App) -> Self {
        let all = theme_names(cx);
        Self {
            on_confirm,
            original: cx.theme().theme_name().clone(),
            matches: all.clone(),
            all,
            selected: None,
        }
    }

    fn current_row(&self) -> Option<usize> {
        self.matches.iter().position(|name| *name == self.original)
    }

    fn selected_name(&self) -> Option<SharedString> {
        self.selected.and_then(|ix| self.matches.get(ix)).cloned()
    }
}

impl ListDelegate for ThemeListDelegate {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        let query = query.to_lowercase();
        self.matches = self
            .all
            .iter()
            .filter(|name| name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.matches.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let name = self.matches.get(ix.row)?.clone();
        Some(
            ListItem::new(ix.row)
                .h(px(ROW_HEIGHT))
                .px_2()
                .py_0()
                .text_size(px(13.))
                .check_icon(IconName::Check)
                .confirmed(name == self.original)
                .child(name),
        )
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .h(px(ROW_HEIGHT))
            .px_2()
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(cx.theme().muted_foreground)
            .child("No matching themes")
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix.map(|ix| ix.row);
        if let Some(name) = self.selected_name() {
            if name != *cx.theme().theme_name() {
                apply_theme(&name, cx);
            }
        }
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        let Some(name) = self.selected_name() else {
            return;
        };
        apply_theme(&name, cx);
        (self.on_confirm)(&name, cx);
        // Close after this update finishes: the dialog owns the picker entity.
        window.defer(cx, |window, cx| window.close_dialog(cx));
    }
}
