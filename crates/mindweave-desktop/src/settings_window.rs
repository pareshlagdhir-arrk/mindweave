//! The Settings window (Mindweave > Settings…): one tab per area of
//! preferences. Libraries installs, uninstalls, enables, disables and
//! orders the shape libraries; Icon Sets imports, uninstalls, shows and
//! hides the icon sets. Each tab is a searchable list beside the details
//! of the selected row. Changes apply at once in every window.

use std::path::Path;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::library::quote;
use mindweave_core::{ordered_libraries, IconSet, Library, LibraryRegistry};

use crate::actions::CloseWindow;
use crate::app_state::AppState;
use crate::editor::tool::BASIC_SHAPES;
use crate::icon_library::{self, IconLibrary, ImportSource};
use crate::libraries::{self, Libraries};
use crate::menu_bar::{menu_bar, WindowKind};

const TEXT_SIZE: f32 = 13.;
const SMALL_TEXT_SIZE: f32 = 12.;
const TITLE_SIZE: f32 = 16.;
const LIST_WIDTH: f32 = 380.;
const ROW_HEIGHT: f32 = 40.;
const PADDING: f32 = 12.;
/// The label of the button showing a folder in the file manager.
const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else {
    "Show in Folder"
};

/// A tab of the window. A General tab would join [`SettingsTab::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Libraries,
    IconSets,
}

impl SettingsTab {
    /// Every tab, in the order shown.
    const ALL: [SettingsTab; 2] = [SettingsTab::Libraries, SettingsTab::IconSets];

    /// The tab's title.
    fn label(self) -> &'static str {
        match self {
            SettingsTab::Libraries => "Libraries",
            SettingsTab::IconSets => "Icon Sets",
        }
    }
}

/// The Settings window's view.
pub struct SettingsWindow {
    focus_handle: FocusHandle,
    tab: SettingsTab,
    library_search: Entity<InputState>,
    icon_search: Entity<InputState>,
    /// The library whose details are shown, by id.
    selected_library: Option<String>,
    /// The icon set whose details are shown, by name.
    selected_icon_set: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    /// The window's view, on the Libraries tab.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let library_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search libraries"));
        let icon_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search icon sets"));
        // The lists filter as a query changes.
        let redraw_on_change = |_: &mut Self, _, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        };
        let _subscriptions = vec![
            // Enabled, hidden and ordered lists live in the settings.
            cx.observe_global::<AppState>(|_, cx| cx.notify()),
            cx.subscribe(&library_search, redraw_on_change),
            cx.subscribe(&icon_search, redraw_on_change),
        ];
        Self {
            focus_handle,
            tab: SettingsTab::Libraries,
            library_search,
            icon_search,
            selected_library: None,
            selected_icon_set: None,
            _subscriptions,
        }
    }

    fn on_close_window(&mut self, _: &CloseWindow, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = SettingsTab::ALL
            .iter()
            .position(|t| *t == self.tab)
            .unwrap_or(0);
        let tabs = TabBar::new("settings-tabs")
            .underline()
            .px(px(PADDING))
            .selected_index(selected)
            .children(SettingsTab::ALL.map(|t| Tab::new().label(t.label())))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.tab = SettingsTab::ALL[*ix];
                cx.notify();
            }));
        let body = match self.tab {
            SettingsTab::Libraries => libraries_tab(self, cx).into_any_element(),
            SettingsTab::IconSets => icon_sets_tab(self, cx).into_any_element(),
        };
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close_window))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(TEXT_SIZE))
            .child(menu_bar(WindowKind::Dashboard, Some("Settings".into()), cx))
            .child(tabs)
            .child(h_flex().flex_1().min_h_0().items_stretch().child(body))
    }
}

/// Whether `name` holds every word of `query`, ignoring case.
fn matches_query(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    query
        .split_whitespace()
        .all(|word| name.contains(&word.to_lowercase()))
}

/// The libraries as Settings lists them: in ribbon order, so Basic Shapes
/// (which stays after Home) first, then the others in the user's `order`.
fn listed_libraries<'a>(registry: &'a LibraryRegistry, order: &[String]) -> Vec<&'a Library> {
    let (basic, rest): (Vec<_>, Vec<_>) = ordered_libraries(registry.libraries(), order)
        .into_iter()
        .partition(|l| l.id == BASIC_SHAPES);
    basic.into_iter().chain(rest).collect()
}

/// The ids of the libraries the user can reorder, in their order.
fn movable_ids(listed: &[&Library]) -> Vec<String> {
    listed
        .iter()
        .filter(|l| l.id != BASIC_SHAPES)
        .map(|l| l.id.clone())
        .collect()
}

/// The list beside the details: a search field and an action button over
/// the rows.
fn list_pane(
    search: &Entity<InputState>,
    action: impl IntoElement,
    rows: Vec<AnyElement>,
    empty: &'static str,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    v_flex()
        .w(px(LIST_WIDTH))
        .flex_none()
        .border_r_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .p(px(PADDING))
                .gap(px(8.))
                .child(
                    div().flex_1().child(
                        Input::new(search)
                            .small()
                            .prefix(Icon::new(IconName::Search).small().text_color(muted)),
                    ),
                )
                .child(action),
        )
        .child(
            div()
                .id("settings-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(PADDING / 2.))
                .when(rows.is_empty(), |list| {
                    list.p(px(PADDING)).text_color(muted).child(empty)
                })
                .children(rows),
        )
}

/// A row of a list: `title` over `subtitle`, then `trailing` controls;
/// clicking it runs `select`.
fn list_row(
    id: impl Into<ElementId>,
    title: SharedString,
    subtitle: SharedString,
    selected: bool,
    trailing: Vec<AnyElement>,
    select: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let hover = theme.list_hover;
    h_flex()
        .id(id)
        .h(px(ROW_HEIGHT))
        .px(px(8.))
        .gap(px(6.))
        .rounded(theme.radius)
        .when(selected, |row| row.bg(theme.list_active))
        .when(!selected, |row| row.hover(move |s| s.bg(hover)))
        .on_click(move |_, window, cx| select(window, cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(title))
                .child(
                    div()
                        .truncate()
                        .text_size(px(SMALL_TEXT_SIZE))
                        .text_color(theme.muted_foreground)
                        .child(subtitle),
                ),
        )
        .children(trailing)
        .into_any_element()
}

/// The details pane: a title, then label and value lines, then buttons.
fn details_pane(
    title: Option<SharedString>,
    lines: Vec<(&'static str, SharedString)>,
    buttons: Vec<AnyElement>,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let Some(title) = title else {
        return v_flex()
            .flex_1()
            .p(px(PADDING * 2.))
            .text_color(muted)
            .child("Select a row to see its details.")
            .into_any_element();
    };
    v_flex()
        .id("settings-details")
        .flex_1()
        .min_w_0()
        .overflow_y_scroll()
        .p(px(PADDING * 2.))
        .gap(px(10.))
        .child(div().text_size(px(TITLE_SIZE)).child(title))
        .children(lines.into_iter().map(|(label, value)| {
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(SMALL_TEXT_SIZE))
                        .text_color(muted)
                        .child(label),
                )
                .child(div().child(value))
        }))
        .child(h_flex().pt(px(6.)).gap(px(8.)).children(buttons))
        .into_any_element()
}

/// The button showing `path` in the file manager.
fn reveal_button(path: &Path) -> AnyElement {
    let path = path.to_path_buf();
    Button::new("reveal")
        .small()
        .icon(IconName::FolderOpen)
        .label(REVEAL_LABEL)
        .on_click(move |_, _, cx| cx.reveal_path(&path))
        .into_any_element()
}

/// An Install or Import button with a menu of Folder… and .zip File….
fn import_button(
    id: &'static str,
    label: &'static str,
    run: fn(ImportSource, &mut Window, &mut App),
) -> impl IntoElement {
    Button::new(id)
        .small()
        .icon(IconName::Plus)
        .label(label)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            menu.item(
                PopupMenuItem::new("Folder…")
                    .on_click(move |_, window, cx| run(ImportSource::Folder, window, cx)),
            )
            .item(
                PopupMenuItem::new(".zip File…")
                    .on_click(move |_, window, cx| run(ImportSource::Zip, window, cx)),
            )
        })
}

/// The Libraries tab.
fn libraries_tab(view: &SettingsWindow, cx: &mut Context<SettingsWindow>) -> impl IntoElement {
    let registry = cx.global::<Libraries>().0.clone();
    let settings = &cx.global::<AppState>().settings;
    let listed = listed_libraries(&registry, &settings.library_order);
    let movable = movable_ids(&listed);
    let query = view.library_search.read(cx).value().to_string();
    let this = cx.entity().downgrade();

    let rows: Vec<AnyElement> = listed
        .iter()
        .filter(|l| matches_query(&l.name, &query))
        .map(|library| {
            let id = library.id.clone();
            let enabled = settings.library_enabled(&id);
            let position = movable.iter().position(|m| *m == id);
            let mover = |up: bool| {
                let (id, movable) = (id.clone(), movable.clone());
                let can =
                    position.is_some_and(|ix| if up { ix > 0 } else { ix + 1 < movable.len() });
                Button::new(SharedString::from(format!(
                    "{}-{id}",
                    if up { "up" } else { "down" }
                )))
                .ghost()
                .xsmall()
                .icon(if up {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                })
                .tooltip(if up { "Move Up" } else { "Move Down" })
                .disabled(!can)
                .on_click(move |_, _, cx| {
                    AppState::update_settings(cx, |s| s.move_library(&movable, &id, up));
                })
                .into_any_element()
            };
            let toggle = {
                let id = id.clone();
                Switch::new(SharedString::from(format!("enabled-{id}")))
                    .small()
                    .checked(enabled)
                    .tooltip(if enabled { "Disable" } else { "Enable" })
                    .accessibility_label(format!("Show {} in the ribbon", library.name))
                    .on_click(move |on, _, cx| {
                        AppState::update_settings(cx, |s| s.set_library_enabled(&id, *on));
                    })
                    .into_any_element()
            };
            let mut trailing = Vec::new();
            if position.is_some() {
                trailing.extend([mover(true), mover(false)]);
            }
            trailing.push(toggle);
            let subtitle = match (library.builtin, enabled) {
                (true, true) => "Built-in",
                (true, false) => "Built-in · Disabled",
                (false, true) => "Installed",
                (false, false) => "Installed · Disabled",
            };
            let this = this.clone();
            let select = id.clone();
            list_row(
                SharedString::from(format!("library-{id}")),
                library.name.clone().into(),
                subtitle.into(),
                view.selected_library.as_deref() == Some(&id),
                trailing,
                move |_, cx| {
                    let id = select.clone();
                    this.update(cx, |this, cx| {
                        this.selected_library = Some(id);
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            )
        })
        .collect();

    let details = library_details(view.selected_library.as_deref(), &registry, cx);
    h_flex()
        .flex_1()
        .min_h_0()
        .items_stretch()
        .child(list_pane(
            &view.library_search,
            import_button("install-library", "Install", libraries::install),
            rows,
            "No libraries match.",
            cx,
        ))
        .child(details)
}

/// The details of library `id`, if it is installed.
fn library_details(id: Option<&str>, registry: &LibraryRegistry, cx: &App) -> impl IntoElement {
    let Some(library) = id.and_then(|id| registry.library(id)) else {
        return details_pane(None, Vec::new(), Vec::new(), cx);
    };
    let mut lines: Vec<(&'static str, SharedString)> = vec![
        ("Id", library.id.clone().into()),
        ("Version", library.version.clone().into()),
    ];
    if let Some(author) = &library.author {
        lines.push(("Author", author.clone().into()));
    }
    if let Some(description) = &library.description {
        lines.push(("Description", description.clone().into()));
    }
    lines.push((
        "Contents",
        contents_summary(library.shapes.len(), library.templates().count()).into(),
    ));
    let folder = registry.folder(&library.id);
    lines.push((
        "Location",
        match folder {
            Some(folder) => folder.display().to_string().into(),
            None => "Built in to Mindweave".into(),
        },
    ));
    let mut buttons = Vec::new();
    if let Some(folder) = folder {
        buttons.push(reveal_button(folder));
        let id = library.id.clone();
        buttons.push(
            Button::new("uninstall-library")
                .small()
                .danger()
                .label("Uninstall…")
                .on_click(move |_, window, cx| libraries::confirm_uninstall(&id, window, cx))
                .into_any_element(),
        );
    }
    details_pane(Some(quote(&library.name).into()), lines, buttons, cx)
}

/// "3 shapes, 1 template" and the like.
fn contents_summary(shapes: usize, templates: usize) -> String {
    let count = |n: usize, one: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    };
    format!(
        "{}, {}",
        count(shapes, "shape"),
        count(templates, "template")
    )
}

/// The Icon Sets tab.
fn icon_sets_tab(view: &SettingsWindow, cx: &mut Context<SettingsWindow>) -> impl IntoElement {
    let sets = cx.global::<IconLibrary>().all.clone();
    let settings = &cx.global::<AppState>().settings;
    let query = view.icon_search.read(cx).value().to_string();
    let this = cx.entity().downgrade();

    let rows: Vec<AnyElement> = sets
        .iter()
        .filter(|s| matches_query(&s.name, &query))
        .map(|set| {
            let name = set.name.clone();
            let shown = settings.icon_set_shown(&name);
            let toggle = {
                let name = name.clone();
                Switch::new(SharedString::from(format!("shown-{name}")))
                    .small()
                    .checked(shown)
                    .tooltip(if shown { "Disable" } else { "Enable" })
                    .accessibility_label(format!("Show {name} in the Icons pane"))
                    .on_click(move |on, _, cx| icon_library::set_shown(&name, *on, cx))
                    .into_any_element()
            };
            let mut subtitle = icon_count(set.icons.len());
            if !shown {
                subtitle.push_str(" · Disabled");
            }
            let this = this.clone();
            let select = name.clone();
            list_row(
                SharedString::from(format!("icon-set-{name}")),
                name.clone().into(),
                subtitle.into(),
                view.selected_icon_set.as_deref() == Some(&name),
                vec![toggle],
                move |_, cx| {
                    let name = select.clone();
                    this.update(cx, |this, cx| {
                        this.selected_icon_set = Some(name);
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            )
        })
        .collect();

    let selected = view
        .selected_icon_set
        .as_deref()
        .and_then(|name| sets.iter().find(|s| s.name == name));
    let empty = if sets.is_empty() {
        "No icon sets yet. Import a folder or .zip of SVG or PNG icons."
    } else {
        "No icon sets match."
    };
    h_flex()
        .flex_1()
        .min_h_0()
        .items_stretch()
        .child(list_pane(
            &view.icon_search,
            import_button("import-icons", "Import", icon_library::import),
            rows,
            empty,
            cx,
        ))
        .child(icon_set_details(selected, cx))
}

/// "1 icon", "12 icons".
fn icon_count(n: usize) -> String {
    match n {
        1 => "1 icon".into(),
        n => format!("{n} icons"),
    }
}

/// The details of icon set `set`.
fn icon_set_details(set: Option<&IconSet>, cx: &App) -> impl IntoElement {
    let Some(set) = set else {
        return details_pane(None, Vec::new(), Vec::new(), cx);
    };
    let lines = vec![
        ("Icons", icon_count(set.icons.len()).into()),
        ("Location", set.dir.display().to_string().into()),
    ];
    let name: SharedString = set.name.clone().into();
    let buttons = vec![
        reveal_button(&set.dir),
        Button::new("uninstall-icon-set")
            .small()
            .danger()
            .label("Uninstall…")
            .on_click(move |_, window, cx| icon_library::confirm_remove(name.clone(), window, cx))
            .into_any_element(),
    ];
    details_pane(Some(set.name.clone().into()), lines, buttons, cx)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{contents_summary, listed_libraries, matches_query, movable_ids};
    use mindweave_core::{builtin_libraries, LibraryRegistry};

    #[test]
    fn search_matches_every_word_ignoring_case() {
        assert!(matches_query("Basic Shapes", ""));
        assert!(matches_query("Basic Shapes", "shap"));
        assert!(matches_query("Basic Shapes", "shapes BASIC"));
        assert!(!matches_query("Basic Shapes", "flow"));
    }

    #[test]
    fn basic_shapes_leads_the_list_and_others_follow_the_users_order() {
        // Three user libraries next to the built-in Basic Shapes.
        let user = tempfile::tempdir().unwrap();
        for id in ["alpha", "beta", "gamma"] {
            let dir = user.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("library.toml"),
                format!("format = 1\nid = \"{id}\"\nname = \"{id}\"\nversion = \"1\"\n"),
            )
            .unwrap();
            std::fs::write(
                dir.join("shapes.toml"),
                "[[shape]]\nid = \"r\"\nname = \"R\"\nprimitive = \"rect\"\n",
            )
            .unwrap();
        }
        let registry = LibraryRegistry::new(builtin_libraries(), Some(user.path())).0;
        let order = vec!["gamma".to_string(), "basic-shapes".to_string()];
        let listed = listed_libraries(&registry, &order);
        let ids: Vec<&str> = listed.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["basic-shapes", "gamma", "alpha", "beta"]);
        assert_eq!(movable_ids(&listed), ["gamma", "alpha", "beta"]);
    }

    #[test]
    fn contents_count_shapes_and_templates() {
        assert_eq!(contents_summary(1, 0), "1 shape, 0 templates");
        assert_eq!(contents_summary(6, 1), "6 shapes, 1 template");
    }
}
