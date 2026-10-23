//! The Icons pane, below Layers in the left sidebar: the imported icon
//! sets, each under a header that folds it away and holds its Uninstall
//! button, filtered by a search. A click places an icon in the middle of
//! the view; dragging one onto the canvas places it there. Placed icons
//! are images, drawn in their own colours.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::geometry::{Frame, Pt};
use mindweave_core::{search_icons, IconSet, ImageData, Shape};

use super::images::{contrast_plate, file_luminance, hsla_luminance};
use super::{section_label, EditorWindow, SMALL_TEXT_SIZE};
use crate::icon_library::{self, IconLibrary, ImportSource};
use crate::libraries::lucide_icon_path;

/// Icons per row of the grid.
const COLUMNS: usize = 5;
/// Height of every row (set headers included, as the list needs one
/// height), side of a grid cell, and side of the icon inside it, in
/// pixels.
const ROW: f32 = 40.;
const CELL_ICON: f32 = 28.;
const HEADER_HEIGHT: f32 = 32.;
/// Longest side of a newly placed icon, in canvas units.
pub const PLACED_ICON_SIZE: f32 = 64.;

/// One icon in the grid: its name and file.
#[derive(Clone, Debug, PartialEq)]
struct Found {
    name: SharedString,
    path: PathBuf,
}

/// A row of the pane's list.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// A set's header: its name, how many icons it shows, and whether
    /// they are shown.
    Header {
        set: SharedString,
        count: usize,
        open: bool,
    },
    /// A group's header within a set: the group, the key folding it away,
    /// how many icons it shows, and whether they are shown.
    Group {
        group: SharedString,
        key: String,
        count: usize,
        open: bool,
    },
    /// Up to [`COLUMNS`] icons of the set or group above.
    Icons(Vec<Found>),
}

/// The key folding group `group` of set `set` away.
fn group_key(set: &str, group: &str) -> String {
    format!("{set}\u{1f}{group}")
}

/// The pane's rows for `sets`: each set's header, then, unless the set is
/// folded away (`collapsed`), its icons matching `query` in rows of
/// [`COLUMNS`]. While searching, sets with no match are left out and the
/// others are shown open.
fn rows(sets: &[IconSet], query: &str, collapsed: &BTreeSet<String>) -> Vec<Row> {
    let searching = !query.trim().is_empty();
    let mut out = Vec::new();
    for set in sets {
        let found: Vec<(&str, Found)> = search_icons(std::slice::from_ref(set), None, query)
            .into_iter()
            .map(|(_, icon)| {
                (
                    icon.group.as_str(),
                    Found {
                        name: icon.name.clone().into(),
                        path: icon.path.clone(),
                    },
                )
            })
            .collect();
        if searching && found.is_empty() {
            continue;
        }
        let open = searching || !collapsed.contains(&set.name);
        out.push(Row::Header {
            set: set.name.clone().into(),
            count: found.len(),
            open,
        });
        if !open {
            continue;
        }
        // Icons come sorted by group; each named group gets a header.
        let mut rest = found.as_slice();
        while let Some((group, _)) = rest.first() {
            let group = *group;
            let len = rest.iter().take_while(|(g, _)| *g == group).count();
            let (members, tail) = rest.split_at(len);
            rest = tail;
            let icons: Vec<Found> = members.iter().map(|(_, f)| f.clone()).collect();
            let shown = if group.is_empty() {
                true
            } else {
                let key = group_key(&set.name, group);
                let open = searching || !collapsed.contains(&key);
                out.push(Row::Group {
                    group: group.to_string().into(),
                    key,
                    count: icons.len(),
                    open,
                });
                open
            };
            if shown {
                out.extend(icons.chunks(COLUMNS).map(|c| Row::Icons(c.to_vec())));
            }
        }
    }
    out
}

/// The pane's search, folded sets, and the rows they gave last.
pub struct IconsPane {
    pub search: Entity<InputState>,
    query: String,
    /// Sets folded away, by name.
    collapsed: BTreeSet<String>,
    /// Rows for a query, folded sets and library, reused while they stay.
    rows: RefCell<Option<RowsCache>>,
    /// Each shown icon's tone, worked out once.
    tones: Tones,
}

/// Icon tones by file (see [`file_luminance`]).
type Tones = std::rc::Rc<RefCell<std::collections::HashMap<PathBuf, Option<f32>>>>;

/// The query, folded sets and library (by address) rows were made for,
/// and the rows.
type RowsCache = (String, BTreeSet<String>, usize, Arc<Vec<Row>>);

impl IconsPane {
    pub fn new(window: &mut Window, cx: &mut Context<EditorWindow>) -> (Self, Subscription) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search icons"));
        let subscription = cx.subscribe_in(&search, window, |this, input, event, _, cx| {
            if let InputEvent::Change = event {
                this.icons.query = input.read(cx).value().to_string();
                cx.notify();
            }
        });
        (
            IconsPane {
                search,
                query: String::new(),
                collapsed: BTreeSet::new(),
                rows: RefCell::new(None),
                tones: Tones::default(),
            },
            subscription,
        )
    }

    /// The rows for `sets` as the pane stands.
    fn rows(&self, sets: &Arc<Vec<IconSet>>) -> Arc<Vec<Row>> {
        let library = Arc::as_ptr(sets) as usize;
        let mut cached = self.rows.borrow_mut();
        if let Some((q, c, l, rows)) = cached.as_ref() {
            if *q == self.query && *c == self.collapsed && *l == library {
                return rows.clone();
            }
        }
        let made = Arc::new(rows(sets, &self.query, &self.collapsed));
        *cached = Some((
            self.query.clone(),
            self.collapsed.clone(),
            library,
            made.clone(),
        ));
        made
    }
}

/// An icon being dragged from the pane.
#[derive(Clone, Debug)]
pub struct IconDrag {
    pub path: PathBuf,
}

/// What follows the pointer while an icon is dragged.
struct DragPreview(PathBuf);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        img(self.0.clone()).size(px(CELL_ICON)).opacity(0.8)
    }
}

impl EditorWindow {
    /// Places the icon in file `path`, its longest side
    /// [`PLACED_ICON_SIZE`], centred on canvas point `at` or on the middle
    /// of the view, and selects it.
    pub(super) fn place_icon(
        &mut self,
        path: &std::path::Path,
        at: Option<Pt>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let data = std::fs::read(path)
            .ok()
            .and_then(ImageData::from_file_bytes);
        let natural = data
            .as_ref()
            .and_then(|d| self.image_cache.natural_size(d, cx));
        let (Some(data), Some((w, h))) = (data, natural) else {
            self.place_pictures(vec![Vec::new()], window, cx);
            return;
        };
        let scale = PLACED_ICON_SIZE / w.max(h).max(1.);
        let (w, h) = (w * scale, h * scale);
        let centre = at.unwrap_or_else(|| {
            let view = self.visible_canvas();
            (view.x + view.w / 2., view.y + view.h / 2.)
        });
        let frame = Frame {
            x: centre.0 - w / 2.,
            y: centre.1 - h / 2.,
            w,
            h,
            ..Frame::default()
        };
        self.add_and_select(vec![Shape::new_image(data, frame)], window, cx);
        self.focus_handle.focus(window, cx);
    }

    /// Folds set `set` (or a group, by [`group_key`]) away, or opens it
    /// again.
    fn toggle_icon_set(&mut self, set: &str, cx: &mut Context<Self>) {
        if !self.icons.collapsed.remove(set) {
            self.icons.collapsed.insert(set.to_string());
        }
        cx.notify();
    }
}

/// A set's header: a chevron and its name, which fold it away or open
/// it, its icon count, and its Uninstall button.
fn header_row(
    ix: usize,
    set: SharedString,
    count: usize,
    open: bool,
    editor: WeakEntity<EditorWindow>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let chevron = if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let toggle_set = set.clone();
    let remove_set = set.clone();
    h_flex()
        .id(("icon-set-header", ix))
        // Full width, so the Uninstall button sits at the right end
        // however long the name is.
        .w_full()
        .min_w_0()
        .h(px(ROW))
        .px(px(6.))
        .gap(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme.list_hover))
        .child(
            Icon::new(chevron)
                .size(px(14.))
                .text_color(theme.muted_foreground),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(SMALL_TEXT_SIZE))
                .child(set.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(SMALL_TEXT_SIZE))
                .text_color(theme.muted_foreground)
                .child(count.to_string()),
        )
        .child(
            Button::new(("uninstall-icon-set", ix))
                .ghost()
                .xsmall()
                .icon(Icon::empty().path(lucide_icon_path("trash", cx)))
                .tooltip("Uninstall Icon Set…")
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    icon_library::confirm_remove(remove_set.clone(), window, cx);
                }),
        )
        .on_click(move |_, _, cx| {
            editor
                .update(cx, |this, cx| this.toggle_icon_set(&toggle_set, cx))
                .ok();
        })
        .into_any_element()
}

/// A group's header within a set: a chevron, the group's name and its
/// icon count, indented under the set; a click folds it away or opens it.
fn group_row(
    ix: usize,
    group: SharedString,
    key: String,
    count: usize,
    open: bool,
    editor: WeakEntity<EditorWindow>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let chevron = if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    h_flex()
        .id(("icon-group-header", ix))
        .w_full()
        .min_w_0()
        .h(px(ROW))
        .pl(px(18.))
        .pr(px(10.))
        .gap(px(4.))
        .cursor_pointer()
        .text_size(px(SMALL_TEXT_SIZE))
        .text_color(theme.muted_foreground)
        .hover(|s| s.bg(theme.list_hover))
        .child(Icon::new(chevron).size(px(12.)))
        .child(div().flex_1().min_w_0().truncate().child(group))
        .child(div().flex_none().child(count.to_string()))
        .on_click(move |_, _, cx| {
            editor
                .update(cx, |this, cx| this.toggle_icon_set(&key, cx))
                .ok();
        })
        .into_any_element()
}

/// A row of icons; `first` numbers the first for element ids.
fn icons_row(
    icons: &[Found],
    first: usize,
    editor: WeakEntity<EditorWindow>,
    tones: &Tones,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let (hover, radius) = (theme.list_hover, theme.radius);
    let background = hsla_luminance(theme.sidebar);
    let cells = icons.iter().enumerate().map(|(i, icon)| {
        let editor = editor.clone();
        let path = icon.path.clone();
        let name = icon.name.clone();
        // An icon as dark (or light) as the pane sits on a contrasting
        // plate, as on the canvas.
        let tone = *tones
            .borrow_mut()
            .entry(icon.path.clone())
            .or_insert_with(|| file_luminance(&icon.path, cx));
        let plate = tone.and_then(|t| contrast_plate(t, background));
        let picture = div()
            .size(px(CELL_ICON + 4.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when_some(plate, |d, color| d.bg(color))
            .child(img(icon.path.clone()).size(px(CELL_ICON)));
        div()
            .id(("icon", first + i))
            .size(px(ROW))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(radius)
            .hover(move |s| s.bg(hover))
            .child(picture)
            .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
            .on_drag(IconDrag { path: path.clone() }, |drag, _, _, cx| {
                cx.new(|_| DragPreview(drag.path.clone()))
            })
            .on_click(move |_, window, cx| {
                editor
                    .update(cx, |this, cx| this.place_icon(&path, None, window, cx))
                    .ok();
            })
    });
    h_flex()
        .h(px(ROW))
        .px(px(6.))
        .children(cells)
        .into_any_element()
}

/// The pane.
pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let sets = cx.global::<IconLibrary>().shown.clone();
    let muted = cx.theme().muted_foreground;

    let import_menu = Button::new("import-icons")
        .ghost()
        .small()
        .icon(IconName::Plus)
        .tooltip("Import Icons")
        .dropdown_menu_with_anchor(Anchor::TopRight, |menu, _, _| {
            menu.item(
                PopupMenuItem::new("Import Folder…").on_click(|_, window, cx| {
                    icon_library::import(ImportSource::Folder, window, cx)
                }),
            )
            .item(
                PopupMenuItem::new(".zip File…")
                    .on_click(|_, window, cx| icon_library::import(ImportSource::Zip, window, cx)),
            )
        });
    let header = h_flex()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .pl(px(12.))
        .pr(px(6.))
        .justify_between()
        .child(section_label("Icons", cx))
        .child(import_menu);
    let note = |text: &'static str| {
        div()
            .px(px(12.))
            .text_size(px(SMALL_TEXT_SIZE))
            .text_color(muted)
            .child(text)
    };

    if sets.is_empty() {
        let text = if cx.global::<IconLibrary>().all.is_empty() {
            "No icons yet. Use + to import a folder or .zip of SVG or PNG icons, \
             such as the AWS, Google Cloud or Material icon packs."
        } else {
            "Every icon set is hidden. Show them again in Settings."
        };
        return v_flex()
            .size_full()
            .child(header)
            .child(note(text))
            .into_any_element();
    }

    let rows = editor.icons.rows(&sets);
    let empty = rows.is_empty();
    // Icon element ids: the row's index times the columns, plus the cell.
    let handle = cx.entity().downgrade();
    let tones = editor.icons.tones.clone();
    let list = uniform_list("icon-rows", rows.len(), move |range, _, cx| {
        range
            .map(|ix| match &rows[ix] {
                Row::Header { set, count, open } => {
                    header_row(ix, set.clone(), *count, *open, handle.clone(), cx)
                }
                Row::Group {
                    group,
                    key,
                    count,
                    open,
                } => group_row(
                    ix,
                    group.clone(),
                    key.clone(),
                    *count,
                    *open,
                    handle.clone(),
                    cx,
                ),
                Row::Icons(icons) => icons_row(icons, ix * COLUMNS, handle.clone(), &tones, cx),
            })
            .collect()
    })
    .flex_1();

    let mut pane = v_flex().size_full().child(header).child(
        div()
            .px(px(12.))
            .pb(px(6.))
            .child(Input::new(&editor.icons.search).small().cleanable(true)),
    );
    if empty {
        pane = pane.child(note("No icons match."));
    }
    pane.child(list).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{rows, Row, COLUMNS};
    use mindweave_core::load_icon_sets;
    use std::collections::BTreeSet;

    fn sets() -> (tempfile::TempDir, Vec<mindweave_core::IconSet>) {
        let root = tempfile::tempdir().unwrap();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"/>"#;
        for (set, n) in [("AWS", 7), ("GCP", 2)] {
            let dir = root.path().join(set);
            std::fs::create_dir_all(&dir).unwrap();
            for i in 0..n {
                std::fs::write(dir.join(format!("{set}-icon-{i}.svg")), svg).unwrap();
            }
        }
        let sets = load_icon_sets(root.path());
        (root, sets)
    }

    fn shape(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Header { set, count, open } => format!("{set} {count} {open}"),
                Row::Group {
                    group, count, open, ..
                } => format!("- {group} {count} {open}"),
                Row::Icons(icons) => icons.len().to_string(),
            })
            .collect()
    }

    #[test]
    fn groups_get_their_own_headers_and_fold_away() {
        let root = tempfile::tempdir().unwrap();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"/>"#;
        for (group, n) in [("Compute", 6), ("Storage", 1)] {
            let dir = root.path().join("AWS").join(group);
            std::fs::create_dir_all(&dir).unwrap();
            for i in 0..n {
                std::fs::write(dir.join(format!("{group}-{i}.svg")), svg).unwrap();
            }
        }
        let sets = load_icon_sets(root.path());
        let none = BTreeSet::new();
        assert_eq!(
            shape(&rows(&sets, "", &none)),
            [
                "AWS 7 true",
                "- Compute 6 true",
                "5",
                "1",
                "- Storage 1 true",
                "1"
            ]
        );
        let folded: BTreeSet<String> = [super::group_key("AWS", "Compute")].into();
        assert_eq!(
            shape(&rows(&sets, "", &folded)),
            ["AWS 7 true", "- Compute 6 false", "- Storage 1 true", "1"]
        );
    }

    #[test]
    fn each_set_has_a_header_and_folds_away() {
        let (_root, sets) = sets();
        assert_eq!(COLUMNS, 5);
        let none = BTreeSet::new();
        assert_eq!(
            shape(&rows(&sets, "", &none)),
            ["AWS 7 true", "5", "2", "GCP 2 true", "2"]
        );
        let folded: BTreeSet<String> = ["AWS".to_string()].into();
        assert_eq!(
            shape(&rows(&sets, "", &folded)),
            ["AWS 7 false", "GCP 2 true", "2"]
        );
        // A search opens folded sets and leaves out sets without a match.
        assert_eq!(
            shape(&rows(&sets, "aws icon 3", &folded)),
            ["AWS 1 true", "1"]
        );
        assert!(rows(&sets, "nothing", &none).is_empty());
    }
}
