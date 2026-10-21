//! The imported icon sets, shared by every window: loaded from
//! [`user_icons_dir`] at start, reloaded after an import or removal.
//! Imports run in the background and report through a notification in
//! the window that started them.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::{
    import_icon_folder, import_icon_zip, load_icon_sets, user_icons_dir, IconSet,
};

/// The icon sets, sorted by name.
#[derive(Default)]
pub struct IconLibrary(pub Arc<Vec<IconSet>>);

impl Global for IconLibrary {}

/// Loads the icon sets into the [`IconLibrary`] global.
pub fn init(cx: &mut App) {
    reload(cx);
}

/// Reads the icon sets again.
pub fn reload(cx: &mut App) {
    cx.set_global(IconLibrary(Arc::new(load_icon_sets(&user_icons_dir()))));
    cx.refresh_windows();
}

/// What to import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSource {
    Folder,
    Zip,
}

/// Asks for a folder or `.zip` of icons and imports it as a new set.
pub fn import(source: ImportSource, window: &mut Window, cx: &mut App) {
    let dialog = rfd::AsyncFileDialog::new().set_title("Import Icons");
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let picked = match source {
            ImportSource::Folder => dialog.pick_folder().await,
            ImportSource::Zip => dialog.add_filter("Zip archive", &["zip"]).pick_file().await,
        };
        let Some(picked) = picked else {
            return;
        };
        let path: PathBuf = picked.path().to_path_buf();
        let root = user_icons_dir();
        let result = cx
            .background_spawn(async move {
                match source {
                    ImportSource::Folder => import_icon_folder(&path, &root),
                    ImportSource::Zip => import_icon_zip(&path, &root),
                }
            })
            .await;
        cx.update(|cx| {
            reload(cx);
            let note = match result {
                Ok(set) => Notification::success(format!(
                    "Imported {} icons as \"{}\".",
                    set.icons.len(),
                    set.name
                )),
                Err(err) => Notification::error(err.to_string()),
            };
            handle
                .update(cx, |_, window, cx| window.push_notification(note, cx))
                .ok();
        });
    })
    .detach();
}

/// Moves icon set `name` to the Trash and reloads the sets.
pub fn remove(name: &str, window: &mut Window, cx: &mut App) {
    let Some(set) = cx.global::<IconLibrary>().0.iter().find(|s| s.name == name) else {
        return;
    };
    match trash::delete(&set.dir) {
        Ok(()) => {
            reload(cx);
            window.push_notification(
                Notification::info(format!("Moved the \"{name}\" icons to the Trash.")),
                cx,
            );
        }
        Err(err) => window.push_notification(
            Notification::error(format!("Could not remove the \"{name}\" icons: {err}")),
            cx,
        ),
    }
}
