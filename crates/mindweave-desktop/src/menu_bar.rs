//! The application menus: one description drives both the compact in-window
//! menu bar and the native macOS menu.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{h_flex, ActiveTheme as _, Sizable as _, TitleBar};
use gpui_kit::*;

use crate::actions::{
    self, BringForward, BringToFront, CloseWindow, Cut, Delete, Duplicate, Group, NewFile,
    NotImplemented, OpenFile, Paste, Quit, Redo, Save, SelectAll, SendBackward, SendToBack,
    ShowDashboard, ToggleLayers, ToggleProperties, ToggleThemePicker, Undo, Ungroup, ZoomIn,
    ZoomOut, ZoomReset,
};
use crate::app_state::AppState;

/// Height of the in-window menu bar. It doubles as the title bar, so the
/// traffic lights (placed 9px from the top, 12px tall) sit centred in it.
pub const MENU_BAR_HEIGHT: f32 = 30.;
const MENU_LABEL_SIZE: f32 = 12.;
const MENU_MIN_WIDTH: f32 = 200.;
/// The left padding `TitleBar` keeps for the traffic lights on macOS.
const TRAFFIC_LIGHTS_WIDTH: f32 = 80.;

/// Which kind of window a menu bar is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Dashboard,
    Editor,
}

/// One row of a menu.
pub(crate) enum MenuEntry {
    Item {
        label: &'static str,
        action: Box<dyn Action>,
        enabled: bool,
        checked: bool,
    },
    Separator,
}

/// A top-level menu: its title and rows.
pub(crate) struct MenuSpec {
    pub name: &'static str,
    pub entries: Vec<MenuEntry>,
}

/// The sidebar state shown as check marks in the View menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PanelState {
    pub show_layers: bool,
    pub show_properties: bool,
}

impl PanelState {
    fn current(cx: &App) -> Self {
        match cx.try_global::<AppState>() {
            Some(state) => Self {
                show_layers: state.settings.show_layers,
                show_properties: state.settings.show_properties,
            },
            None => Self {
                show_layers: true,
                show_properties: true,
            },
        }
    }
}

fn item(label: &'static str, action: impl Action, enabled: bool) -> MenuEntry {
    MenuEntry::Item {
        label,
        action: Box::new(action),
        enabled,
        checked: false,
    }
}

fn toggle(label: &'static str, action: impl Action, enabled: bool, checked: bool) -> MenuEntry {
    MenuEntry::Item {
        label,
        action: Box::new(action),
        enabled,
        checked,
    }
}

/// A present-but-disabled row for a feature that is not built yet.
fn later(label: &'static str) -> MenuEntry {
    item(label, NotImplemented, false)
}

/// The File, Edit, View, Arrange and Help menus for `kind`.
///
/// `include_quit` adds Quit to the end of File; the native menu leaves it out
/// because macOS puts Quit in the application menu.
pub(crate) fn menus(kind: WindowKind, panels: PanelState, include_quit: bool) -> Vec<MenuSpec> {
    use MenuEntry::Separator;
    let editor = kind == WindowKind::Editor;

    let mut file = vec![
        item("New File…", NewFile, true),
        item("Open…", OpenFile, true),
        Separator,
        item("Save", Save, editor),
        item("Close Window", CloseWindow, true),
        Separator,
        item("Show Dashboard", ShowDashboard, true),
    ];
    if include_quit {
        file.extend([Separator, item("Quit Mindweave", Quit, true)]);
    }

    vec![
        MenuSpec {
            name: "File",
            entries: file,
        },
        MenuSpec {
            name: "Edit",
            // Undo and Redo stay enabled whatever the history holds: the
            // native menu cannot update an item's state while it is open,
            // so with nothing to undo they do nothing.
            entries: vec![
                item("Undo", Undo, editor),
                item("Redo", Redo, editor),
                Separator,
                item("Cut", Cut, editor),
                item("Copy", actions::Copy, editor),
                item("Paste", Paste, editor),
                item("Duplicate", Duplicate, editor),
                item("Delete", Delete, editor),
                Separator,
                item("Select All", SelectAll, editor),
            ],
        },
        MenuSpec {
            name: "View",
            entries: vec![
                item("Zoom In", ZoomIn, editor),
                item("Zoom Out", ZoomOut, editor),
                item("Actual Size", ZoomReset, editor),
                Separator,
                toggle("Layers", ToggleLayers, editor, panels.show_layers),
                toggle(
                    "Properties",
                    ToggleProperties,
                    editor,
                    panels.show_properties,
                ),
                Separator,
                item("Theme…", ToggleThemePicker, true),
            ],
        },
        MenuSpec {
            name: "Arrange",
            entries: vec![
                item("Bring to Front", BringToFront, editor),
                item("Bring Forward", BringForward, editor),
                item("Send Backward", SendBackward, editor),
                item("Send to Back", SendToBack, editor),
                Separator,
                item("Group", Group, editor),
                item("Ungroup", Ungroup, editor),
            ],
        },
        MenuSpec {
            name: "Help",
            entries: vec![later("Mindweave Help")],
        },
    ]
}

/// The native macOS menu: the application menu followed by the same menus as
/// the in-window bar. Items are listed as in an editor window; macOS greys
/// out the ones the active window does not handle (for example Zoom in the
/// dashboard).
pub fn native_menus(cx: &App) -> Vec<Menu> {
    native_menus_for(PanelState::current(cx))
}

fn native_menus_for(panels: PanelState) -> Vec<Menu> {
    let mut native = vec![Menu::new("Mindweave").items([MenuItem::action("Quit Mindweave", Quit)])];
    native.extend(
        menus(WindowKind::Editor, panels, false)
            .into_iter()
            .map(|spec| {
                Menu::new(spec.name).items(spec.entries.into_iter().map(|entry| match entry {
                    MenuEntry::Separator => MenuItem::separator(),
                    MenuEntry::Item {
                        label,
                        action,
                        enabled,
                        checked,
                    } => MenuItem::Action {
                        name: label.into(),
                        action,
                        os_action: None,
                        checked,
                        disabled: !enabled,
                    },
                }))
            }),
    );
    native
}

/// Whether the menus are drawn inside the window. On macOS they live only in
/// the system menu bar.
const IN_WINDOW_MENUS: bool = !cfg!(target_os = "macos");

/// The window's title bar: it leaves room for the traffic lights, drags the
/// window, and shows `title` (when given) centred in the window. Off macOS it
/// also carries the compact menu bar.
pub fn menu_bar(
    window_kind: WindowKind,
    title: Option<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let menus = IN_WINDOW_MENUS.then(|| {
        h_flex().h_full().items_center().gap(px(2.)).children(
            menus(window_kind, PanelState::current(cx), true)
                .into_iter()
                .map(menu_button),
        )
    });
    // Equal flexible sides keep the title centred; the right side also
    // matches the traffic-light padding so the centre is the window's. When
    // the menus need more room the title moves right instead of overlapping.
    let row = h_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(h_flex().flex_1().h_full().children(menus))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(MENU_LABEL_SIZE))
                .text_color(theme.muted_foreground)
                .children(title),
        )
        .child(div().flex_1().pr(px(TRAFFIC_LIGHTS_WIDTH)));
    TitleBar::new()
        .h(px(MENU_BAR_HEIGHT))
        .bg(theme.title_bar)
        .border_color(theme.title_bar_border)
        .child(row)
}

fn menu_button(spec: MenuSpec) -> impl IntoElement {
    let entries = Rc::new(spec.entries);
    Button::new(spec.name)
        .ghost()
        .small()
        .text_size(px(MENU_LABEL_SIZE))
        .label(spec.name)
        .dropdown_menu(move |mut menu, window, cx| {
            // Dispatch from the element that had focus before the menu opened,
            // so the window's own action handlers (Save, Close…) receive it.
            if let Some(focus) = window.focused(cx) {
                menu = menu.action_context(focus);
            }
            for entry in entries.iter() {
                menu = match entry {
                    MenuEntry::Separator => menu.separator(),
                    MenuEntry::Item {
                        label,
                        action,
                        enabled,
                        checked,
                    } => menu.menu_with_check_and_disabled(
                        *label,
                        *checked,
                        action.boxed_clone(),
                        !*enabled,
                    ),
                };
            }
            menu.min_w(px(MENU_MIN_WIDTH))
        })
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{menus, native_menus_for, MenuEntry, PanelState, WindowKind};
    use gpui_kit::MenuItem;

    const PANELS: PanelState = PanelState {
        show_layers: true,
        show_properties: false,
    };

    /// `(menu, label, enabled, checked)` for every item.
    fn items(
        kind: WindowKind,
        include_quit: bool,
    ) -> Vec<(&'static str, &'static str, bool, bool)> {
        menus(kind, PANELS, include_quit)
            .into_iter()
            .flat_map(|spec| {
                let name = spec.name;
                spec.entries
                    .into_iter()
                    .filter_map(move |entry| match entry {
                        MenuEntry::Item {
                            label,
                            enabled,
                            checked,
                            ..
                        } => Some((name, label, enabled, checked)),
                        MenuEntry::Separator => None,
                    })
            })
            .collect()
    }

    fn enabled(kind: WindowKind, label: &str) -> bool {
        items(kind, true)
            .into_iter()
            .find(|(_, l, _, _)| *l == label)
            .unwrap_or_else(|| panic!("no menu item {label}"))
            .2
    }

    #[test]
    fn menu_titles_in_order() {
        let names: Vec<_> = menus(WindowKind::Editor, PANELS, true)
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["File", "Edit", "View", "Arrange", "Help"]);
    }

    #[test]
    fn unbuilt_menus_are_present_but_disabled() {
        for kind in [WindowKind::Dashboard, WindowKind::Editor] {
            let unbuilt: Vec<_> = items(kind, true)
                .into_iter()
                .filter(|(menu, ..)| *menu == "Help")
                .collect();
            assert!(!unbuilt.is_empty());
            assert!(
                unbuilt.iter().all(|(_, _, enabled, _)| !enabled),
                "{unbuilt:?}"
            );
        }
    }

    #[test]
    fn arrange_menu_works_in_the_editor_only() {
        for (kind, enabled) in [(WindowKind::Dashboard, false), (WindowKind::Editor, true)] {
            let arrange: Vec<_> = items(kind, true)
                .into_iter()
                .filter(|(menu, ..)| *menu == "Arrange")
                .collect();
            assert_eq!(arrange.len(), 6);
            assert!(
                arrange.iter().all(|(_, _, e, _)| *e == enabled),
                "{arrange:?}"
            );
        }
    }

    /// The Edit menu's rows, `None` for a separator.
    fn edit_rows(kind: WindowKind) -> Vec<Option<(&'static str, bool)>> {
        menus(kind, PANELS, true)
            .into_iter()
            .find(|m| m.name == "Edit")
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Item { label, enabled, .. } => Some((label, enabled)),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn edit_menu_works_in_the_editor_only() {
        let labels = [
            Some("Undo"),
            Some("Redo"),
            None,
            Some("Cut"),
            Some("Copy"),
            Some("Paste"),
            Some("Duplicate"),
            Some("Delete"),
            None,
            Some("Select All"),
        ];
        for (kind, enabled) in [(WindowKind::Editor, true), (WindowKind::Dashboard, false)] {
            let rows = edit_rows(kind);
            let shown: Vec<_> = rows.iter().map(|r| r.map(|(l, _)| l)).collect();
            assert_eq!(shown, labels);
            assert!(
                rows.iter().flatten().all(|(_, e)| *e == enabled),
                "{kind:?}: {rows:?}"
            );
        }
    }

    #[test]
    fn edit_items_dispatch_the_edit_actions() {
        let edit = menus(WindowKind::Editor, PANELS, true)
            .into_iter()
            .find(|m| m.name == "Edit")
            .unwrap();
        let actions: Vec<_> = edit
            .entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item { action, .. } => Some(action.name()),
                MenuEntry::Separator => None,
            })
            .collect();
        assert_eq!(
            actions,
            [
                "mindweave::Undo",
                "mindweave::Redo",
                "mindweave::Cut",
                "mindweave::Copy",
                "mindweave::Paste",
                "mindweave::Duplicate",
                "mindweave::Delete",
                "mindweave::SelectAll",
            ]
        );
    }

    #[test]
    fn dashboard_disables_document_items() {
        for label in [
            "Save",
            "Zoom In",
            "Zoom Out",
            "Actual Size",
            "Layers",
            "Properties",
        ] {
            assert!(!enabled(WindowKind::Dashboard, label), "{label}");
            assert!(enabled(WindowKind::Editor, label), "{label}");
        }
        for label in [
            "New File…",
            "Open…",
            "Close Window",
            "Show Dashboard",
            "Quit Mindweave",
            "Theme…",
        ] {
            assert!(enabled(WindowKind::Dashboard, label), "{label}");
            assert!(enabled(WindowKind::Editor, label), "{label}");
        }
    }

    #[test]
    fn sidebar_items_show_their_state() {
        let all = items(WindowKind::Editor, true);
        let checked = |label: &str| all.iter().find(|(_, l, ..)| *l == label).unwrap().3;
        assert!(checked("Layers"));
        assert!(!checked("Properties"));
    }

    #[test]
    fn native_menu_puts_quit_in_the_app_menu_only() {
        let native = native_menus_for(PANELS);
        let names: Vec<_> = native.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(
            names,
            ["Mindweave", "File", "Edit", "View", "Arrange", "Help"]
        );
        let quits: Vec<_> = native
            .iter()
            .flat_map(|m| m.items.iter().map(move |i| (m.name.clone(), i)))
            .filter(|(_, i)| matches!(i, MenuItem::Action { name, .. } if name.contains("Quit")))
            .map(|(menu, _)| menu)
            .collect();
        assert_eq!(quits, ["Mindweave"]);
    }

    #[test]
    fn quit_is_only_added_on_request() {
        let has_quit = |include| {
            items(WindowKind::Editor, include)
                .iter()
                .any(|(_, label, ..)| *label == "Quit Mindweave")
        };
        assert!(has_quit(true));
        assert!(!has_quit(false));
    }
}
