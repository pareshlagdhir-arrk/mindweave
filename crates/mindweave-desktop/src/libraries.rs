//! The shape libraries: the built-in ones plus those in the user's library
//! folder, loaded once at startup.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use gpui_kit::{App, Global, SharedString};
use mindweave_core::{builtin_libraries, user_libraries_dir, LibraryRegistry};

/// The icon shown for a Lucide name that is not bundled.
pub const FALLBACK_ICON: &str = "shapes";

/// The installed libraries, in toolbar order. Shared, so painting can hold
/// them while it borrows the app mutably.
pub struct Libraries(pub Arc<LibraryRegistry>);

impl Global for Libraries {}

/// Loads the libraries into the [`Libraries`] global. Returns the notices
/// for the user (skipped libraries, libraries with warnings).
pub fn init(cx: &mut App) -> Vec<String> {
    let user_dir = user_libraries_dir();
    let (registry, notices) = LibraryRegistry::new(builtin_libraries(), Some(&user_dir));
    cx.set_global(Libraries(Arc::new(registry)));
    notices
}

/// The asset path of the bundled Lucide icon `name`, or of
/// [`FALLBACK_ICON`] when there is no such icon.
pub fn lucide_icon_path(name: &str, cx: &App) -> SharedString {
    static BUNDLED: OnceLock<HashSet<SharedString>> = OnceLock::new();
    let bundled = BUNDLED.get_or_init(|| {
        cx.asset_source()
            .list("icons/")
            .unwrap_or_default()
            .into_iter()
            .collect()
    });
    icon_path_in(name, bundled)
}

fn icon_path_in(name: &str, bundled: &HashSet<SharedString>) -> SharedString {
    // Names come from untrusted library files; keep them to Lucide's form.
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let path: SharedString = format!("icons/{name}.svg").into();
    if valid && bundled.contains(&path) {
        path
    } else {
        format!("icons/{FALLBACK_ICON}.svg").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_malformed_icon_names_fall_back() {
        let bundled: HashSet<SharedString> = ["icons/square.svg", "icons/shapes.svg"]
            .into_iter()
            .map(SharedString::from)
            .collect();
        assert_eq!(icon_path_in("square", &bundled), "icons/square.svg");
        for name in ["no-such-icon", "", "../square", "Square", "square.svg"] {
            assert_eq!(icon_path_in(name, &bundled), "icons/shapes.svg", "{name:?}");
        }
    }
}
