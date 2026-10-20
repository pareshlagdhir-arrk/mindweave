//! Application-wide state: settings, the open windows, and the actions that
//! create, open and close them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{TitleBar, WindowExt as _};
use gpui_kit::*;
use mindweave_core::{
    load_settings, save_settings, settings_path, AppSettings, DocumentSession, FormatError,
    WindowSize,
};

use crate::actions::{self, NewFile, OpenFile, Quit, ShowDashboard, ToggleThemePicker};
use crate::dashboard::Dashboard;
use crate::editor::{prompt_to_save, EditorWindow, SaveChoice};
use crate::menu_bar::{native_menus, WindowKind};
use crate::theme_picker::open_theme_picker;
use crate::themes::apply_theme;

const EXTENSION: &str = "mindweave";
const DEFAULT_EDITOR_SIZE: WindowSize = WindowSize {
    width: 1280.,
    height: 820.,
};
const DEFAULT_DASHBOARD_SIZE: WindowSize = WindowSize {
    width: 1200.,
    height: 800.,
};
const MIN_WINDOW_SIZE: WindowSize = WindowSize {
    width: 640.,
    height: 400.,
};

/// An open window and the view it shows.
///
/// Every window's root is gpui-base's `Root` (which hosts gpui-component's
/// dialogs and notifications), so the handle is untyped and the view is kept
/// alongside it.
pub struct AppWindow<V> {
    pub handle: AnyWindowHandle,
    pub view: WeakEntity<V>,
}

impl<V> Clone for AppWindow<V> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle,
            view: self.view.clone(),
        }
    }
}

pub struct AppState {
    pub settings: AppSettings,
    settings_path: PathBuf,
    /// Open editors keyed by canonical document path.
    editors: HashMap<PathBuf, AppWindow<EditorWindow>>,
    dashboard: Option<AppWindow<Dashboard>>,
    /// Set once quitting is confirmed, so closing windows does not reopen
    /// the dashboard.
    quitting: bool,
}

impl Global for AppState {}

impl AppState {
    /// Loads settings, applies the theme, registers actions, key bindings and
    /// the native menu, then opens the dashboard.
    pub fn init(cx: &mut App) {
        let settings_path = settings_path();
        let settings = load_settings(&settings_path);
        apply_theme(&settings.theme, cx);
        cx.set_global(AppState {
            settings,
            settings_path,
            editors: HashMap::new(),
            dashboard: None,
            quitting: false,
        });

        actions::bind_keys(cx);
        cx.on_action(|_: &NewFile, cx| Self::new_file(cx));
        cx.on_action(|_: &OpenFile, cx| Self::open_file_dialog(cx));
        cx.on_action(|_: &Quit, cx| Self::quit(cx));
        // Actions arrive while their window is being updated; defer the ones
        // that update a window.
        cx.on_action(|_: &ShowDashboard, cx| cx.defer(Self::show_dashboard));
        cx.on_action(|_: &ToggleThemePicker, cx| cx.defer(toggle_theme_picker));
        cx.set_menus(native_menus(cx));

        cx.on_window_closed(|cx, id| Self::window_closed(id, cx))
            .detach();
        Self::show_dashboard(cx);
    }

    /// Called when the Dock icon is clicked: with no windows open, brings
    /// the dashboard back.
    pub fn reopened(cx: &mut App) {
        if cx.has_global::<AppState>() && cx.windows().is_empty() {
            Self::show_dashboard(cx);
        }
    }

    /// Changes the settings, saves them, and refreshes the native menu.
    pub fn update_settings(cx: &mut App, f: impl FnOnce(&mut AppSettings)) {
        let state = cx.global_mut::<AppState>();
        f(&mut state.settings);
        if let Err(err) = save_settings(&state.settings, &state.settings_path) {
            eprintln!(
                "Mindweave: could not save settings to {}: {err}",
                state.settings_path.display()
            );
        }
        cx.set_menus(native_menus(cx));
    }

    /// Asks where to create a new document, creates it, and opens it.
    pub fn new_file(cx: &mut App) {
        cx.spawn(async move |cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("New Mindweave File")
                .add_filter("Mindweave", &[EXTENSION])
                .set_file_name(format!("Untitled.{EXTENSION}"))
                .save_file()
                .await;
            if let Some(file) = picked {
                let path = with_mindweave_extension(file.path());
                cx.update(|cx| Self::create_file(path, cx));
            }
        })
        .detach();
    }

    fn create_file(path: PathBuf, cx: &mut App) {
        let path = canonical_new_path(&path);
        // Creating overwrites the file, which would orphan an open editor.
        if let Some(editor) = cx.global::<AppState>().editors.get(&path) {
            activate(editor.handle, cx);
            notify_error(
                "That file is already open. Close it before replacing it.".into(),
                cx,
            );
            return;
        }
        match DocumentSession::create(path) {
            Ok(mut session) => {
                session.path = canonical(&session.path);
                let path = session.path.clone();
                Self::open_editor(session, cx);
                touch_recent(&path, cx);
            }
            Err(err) => notify_error(err.to_string(), cx),
        }
    }

    /// Asks for a document to open and opens it.
    pub fn open_file_dialog(cx: &mut App) {
        cx.spawn(async move |cx| {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Open")
                .add_filter("Mindweave", &[EXTENSION])
                .pick_file()
                .await;
            if let Some(file) = picked {
                let path = file.path().to_path_buf();
                cx.update(|cx| Self::open_path(path, cx));
            }
        })
        .detach();
    }

    /// Opens the document at `path`, or focuses its window when it is
    /// already open. Errors are shown as a notification; a file that no
    /// longer exists is removed from the recent list.
    pub fn open_path(path: PathBuf, cx: &mut App) {
        let path = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(err) => {
                if !path.exists() {
                    forget_recent(&path, cx);
                }
                notify_error(FormatError::Io(err).to_string(), cx);
                return;
            }
        };
        if let Some(editor) = cx.global::<AppState>().editors.get(&path) {
            activate(editor.handle, cx);
            touch_recent(&path, cx);
            return;
        }
        match DocumentSession::open(path.clone()) {
            Ok(session) => {
                Self::open_editor(session, cx);
                touch_recent(&path, cx);
            }
            Err(err) => {
                if !path.exists() {
                    forget_recent(&path, cx);
                }
                notify_error(err.to_string(), cx);
            }
        }
    }

    fn open_editor(session: DocumentSession, cx: &mut App) {
        let path = session.path.clone();
        let title = session.title();
        let size = cx
            .global::<AppState>()
            .settings
            .editor_window
            .unwrap_or(DEFAULT_EDITOR_SIZE);
        let opened = open_app_window(title, size, cx, |window, cx| {
            // The close button asks the editor, which may prompt to save.
            let editor = cx.entity().downgrade();
            window.on_window_should_close(cx, move |window, cx| match editor.upgrade() {
                Some(editor) => {
                    editor.update(cx, |editor: &mut EditorWindow, cx| {
                        editor.request_close(window, cx)
                    });
                    false
                }
                None => true,
            });
            EditorWindow::new(session, window, cx)
        });
        if let Some(editor) = opened {
            cx.global_mut::<AppState>().editors.insert(path, editor);
        }
    }

    /// Forgets the editor for `path`; when none are left, shows the
    /// dashboard (opening it if it was closed).
    pub fn editor_closed(path: &Path, cx: &mut App) {
        let state = cx.global_mut::<AppState>();
        state.editors.remove(path);
        if state.editors.is_empty() && !state.quitting {
            Self::show_dashboard(cx);
        }
    }

    /// Brings the dashboard to the front, opening it if needed.
    pub fn show_dashboard(cx: &mut App) {
        if let Some(dashboard) = &cx.global::<AppState>().dashboard {
            activate(dashboard.handle, cx);
            return;
        }
        let size = cx
            .global::<AppState>()
            .settings
            .dashboard_window
            .unwrap_or(DEFAULT_DASHBOARD_SIZE);
        let opened = open_app_window("Mindweave".into(), size, cx, |window, cx| {
            window.on_window_should_close(cx, |window, cx| {
                Self::remember_window_size(WindowKind::Dashboard, window, cx);
                true
            });
            Dashboard::new(window, cx)
        });
        cx.global_mut::<AppState>().dashboard = opened;
    }

    /// Stores `window`'s size as the size for new windows of `kind`.
    pub fn remember_window_size(kind: WindowKind, window: &Window, cx: &mut App) {
        let size = window.window_bounds().get_bounds().size;
        let size = Some(WindowSize {
            width: size.width.into(),
            height: size.height.into(),
        });
        Self::update_settings(cx, |settings| match kind {
            WindowKind::Dashboard => settings.dashboard_window = size,
            WindowKind::Editor => settings.editor_window = size,
        });
    }

    fn window_closed(id: WindowId, cx: &mut App) {
        let state = cx.global_mut::<AppState>();
        if state.quitting {
            return;
        }
        if state
            .dashboard
            .as_ref()
            .is_some_and(|d| d.handle.window_id() == id)
        {
            state.dashboard = None;
            return;
        }
        let closed = state
            .editors
            .iter()
            .find(|(_, editor)| editor.handle.window_id() == id)
            .map(|(path, _)| path.clone());
        if let Some(path) = closed {
            Self::editor_closed(&path, cx);
        }
    }

    /// Asks about each editor with unsaved changes (Save / Don't Save /
    /// Cancel), then quits. Cancel, or a failed save, stops the quit.
    pub fn quit(cx: &mut App) {
        let editors: Vec<_> = cx.global::<AppState>().editors.values().cloned().collect();
        cx.spawn(async move |cx| {
            for editor in &editors {
                let Some(view) = editor.view.upgrade() else {
                    continue;
                };
                let asked = editor.handle.update(cx, |_, window, cx| {
                    let (dirty, name) = {
                        let editor = view.read(cx);
                        (editor.session().is_dirty(), editor.file_name())
                    };
                    dirty.then(|| {
                        window.activate_window();
                        prompt_to_save(&name, window, cx)
                    })
                });
                let Ok(Some(choice)) = asked else {
                    continue;
                };
                match choice.await {
                    SaveChoice::Save => {
                        let saved = editor
                            .handle
                            .update(cx, |_, window, cx| {
                                view.update(cx, |editor, cx| editor.save(window, cx))
                            })
                            .unwrap_or(false);
                        if !saved {
                            return;
                        }
                    }
                    SaveChoice::DontSave => {}
                    SaveChoice::Cancel => return,
                }
            }
            cx.update(|cx| {
                let state = cx.global::<AppState>();
                let open: Vec<_> = state
                    .dashboard
                    .iter()
                    .map(|d| (WindowKind::Dashboard, d.handle))
                    .chain(
                        state
                            .editors
                            .values()
                            .map(|e| (WindowKind::Editor, e.handle)),
                    )
                    .collect();
                for (kind, handle) in open {
                    handle
                        .update(cx, |_, window, cx| {
                            Self::remember_window_size(kind, window, cx)
                        })
                        .ok();
                }
                cx.global_mut::<AppState>().quitting = true;
                cx.quit();
            });
        })
        .detach();
    }
}

/// Opens a window showing a view built by `build`, sized `size` and centred.
///
/// `gpui_kit::open_window` makes gpui-base's `Root` the window root; that
/// Root carries gpui-component's window state (dialogs, notifications), so
/// the view is not wrapped in a second Root.
fn open_app_window<V: Render>(
    title: String,
    size: WindowSize,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
) -> Option<AppWindow<V>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            gpui_kit::size(px(size.width), px(size.height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..TitleBar::title_bar_options()
        }),
        window_min_size: Some(gpui_kit::size(
            px(MIN_WINDOW_SIZE.width),
            px(MIN_WINDOW_SIZE.height),
        )),
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| build(window, cx))) {
        Ok((handle, view)) => Some(AppWindow {
            handle,
            view: view.downgrade(),
        }),
        Err(err) => {
            // Not a notification: showing one may itself need a new window.
            eprintln!("Mindweave: could not open a window: {err:#}");
            None
        }
    }
}

/// Opens the theme picker in the active window; the chosen theme is saved.
fn toggle_theme_picker(cx: &mut App) {
    let Some(handle) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    handle
        .update(cx, |_, window, cx| {
            if window.has_active_dialog(cx) {
                window.close_dialog(cx);
                return;
            }
            let on_confirm = Rc::new(|name: &str, cx: &mut App| {
                AppState::update_settings(cx, |settings| settings.theme = name.to_string());
            });
            open_theme_picker(on_confirm, window, cx);
        })
        .ok();
}

/// Brings a window to the front once the current update has finished.
fn activate(handle: AnyWindowHandle, cx: &mut App) {
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .ok();
    });
}

/// Shows an error notification in the active window, else the dashboard
/// (opened if needed).
fn notify_error(message: String, cx: &mut App) {
    cx.defer(move |cx| {
        let target = cx.active_window().or_else(|| {
            AppState::show_dashboard(cx);
            cx.global::<AppState>().dashboard.as_ref().map(|d| d.handle)
        });
        let shown = target.and_then(|handle| {
            handle
                .update(cx, |_, window, cx| {
                    window.push_notification(Notification::error(message.clone()), cx)
                })
                .ok()
        });
        if shown.is_none() {
            eprintln!("Mindweave: {message}");
        }
    });
}

fn touch_recent(path: &Path, cx: &mut App) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    AppState::update_settings(cx, |settings| settings.touch_recent(path, now));
}

fn forget_recent(path: &Path, cx: &mut App) {
    AppState::update_settings(cx, |settings| settings.remove_recent(path));
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The canonical form of a path whose file may not exist yet: its parent
/// directory is canonicalised and the file name kept.
fn canonical_new_path(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonical(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// `path` with `.mindweave` appended unless it already ends with it.
fn with_mindweave_extension(path: &Path) -> PathBuf {
    let has_extension = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(EXTENSION));
    if has_extension {
        return path.to_path_buf();
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(EXTENSION);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{canonical, canonical_new_path, with_mindweave_extension};
    use std::path::Path;

    #[test]
    fn mindweave_extension_is_forced() {
        assert_eq!(
            with_mindweave_extension(Path::new("/tmp/Plan")),
            Path::new("/tmp/Plan.mindweave")
        );
        assert_eq!(
            with_mindweave_extension(Path::new("/tmp/Plan.mindweave")),
            Path::new("/tmp/Plan.mindweave")
        );
        assert_eq!(
            with_mindweave_extension(Path::new("/tmp/Plan.MINDWEAVE")),
            Path::new("/tmp/Plan.MINDWEAVE")
        );
        // Another extension is kept as part of the name.
        assert_eq!(
            with_mindweave_extension(Path::new("/tmp/Plan.txt")),
            Path::new("/tmp/Plan.txt.mindweave")
        );
    }

    #[test]
    fn new_path_is_canonical_before_the_file_exists() {
        let dir = std::env::temp_dir().join(format!("mindweave-app-state-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let roundabout = dir.join("sub").join("..").join("New.mindweave");
        let expected = std::fs::canonicalize(&dir).unwrap().join("New.mindweave");
        assert_eq!(canonical_new_path(&roundabout), expected);
        // Once created, plain canonicalisation agrees.
        std::fs::write(&roundabout, "{}").unwrap();
        assert_eq!(canonical(&roundabout), expected);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
