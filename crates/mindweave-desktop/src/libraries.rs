//! The shape libraries: the built-in ones plus those in the user's library
//! folder, loaded once at startup.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use gpui_kit::{App, Global, SharedString};
use mindweave_core::{builtin_libraries, user_libraries_dir, Icon, LibraryRegistry};

/// The icon shown for a Lucide name that is not bundled.
pub const FALLBACK_ICON: &str = "shapes";

/// The installed libraries, in toolbar order. Shared, so painting can hold
/// them while it borrows the app mutably.
pub struct Libraries(pub Arc<LibraryRegistry>);

impl Global for Libraries {}

/// Loads the libraries into the [`Libraries`] global. Returns the notices
/// for the user (skipped libraries, libraries with warnings, icons that
/// are not bundled); ignored fields are only logged, once per launch.
pub fn init(cx: &mut App) -> Vec<String> {
    let user_dir = user_libraries_dir();
    let (registry, report) = LibraryRegistry::new(builtin_libraries(), Some(&user_dir));
    for line in &report.log {
        eprintln!("Mindweave: shape library: {line}");
    }
    let mut notices = report.notices;
    notices.extend(unbundled_icon_notices(&registry, bundled_icons(cx)));
    cx.set_global(Libraries(Arc::new(registry)));
    notices
}

/// The asset paths of the bundled icons.
fn bundled_icons(cx: &App) -> &'static HashSet<SharedString> {
    static BUNDLED: OnceLock<HashSet<SharedString>> = OnceLock::new();
    BUNDLED.get_or_init(|| {
        cx.asset_source()
            .list("icons/")
            .unwrap_or_default()
            .into_iter()
            .collect()
    })
}

/// The asset path of the bundled Lucide icon `name`, or of
/// [`FALLBACK_ICON`] when there is no such icon.
pub fn lucide_icon_path(name: &str, cx: &App) -> SharedString {
    icon_path_in(name, bundled_icons(cx))
}

/// One notice per library that names Lucide icons this build does not
/// bundle (they show [`FALLBACK_ICON`]).
fn unbundled_icon_notices(
    registry: &LibraryRegistry,
    bundled: &HashSet<SharedString>,
) -> Vec<String> {
    let missing = |icon: &Icon| match icon {
        Icon::Lucide(name)
            if !bundled.contains(&SharedString::from(format!("icons/{name}.svg"))) =>
        {
            Some(name.clone())
        }
        _ => None,
    };
    registry
        .libraries()
        .iter()
        .filter_map(|library| {
            let names: Vec<String> = std::iter::once(&library.icon)
                .chain(library.shapes.iter().map(|s| &s.icon))
                .filter_map(missing)
                .map(|name| format!("`{name}`"))
                .collect();
            (!names.is_empty()).then(|| {
                format!(
                    "{}: {} not bundled with Mindweave; the default icon is used",
                    library.name,
                    match names.as_slice() {
                        [one] => format!("icon {one} is"),
                        many => format!("icons {} are", many.join(", ")),
                    }
                )
            })
        })
        .collect()
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
    fn unbundled_lucide_icons_get_a_notice() {
        let registry = LibraryRegistry::new(builtin_libraries(), None).0;
        let all: HashSet<SharedString> = registry.libraries()[0]
            .shapes
            .iter()
            .filter_map(|s| match &s.icon {
                Icon::Lucide(name) => Some(format!("icons/{name}.svg").into()),
                Icon::Outline(_) => None,
            })
            .chain([SharedString::from("icons/shapes.svg")])
            .collect();
        assert!(unbundled_icon_notices(&registry, &all).is_empty());

        let mut fewer = all.clone();
        fewer.remove("icons/star.svg");
        assert_eq!(
            unbundled_icon_notices(&registry, &fewer),
            ["Basic Shapes: icon `star` is not bundled with Mindweave; the default icon is used"]
        );
        fewer.remove("icons/shapes.svg");
        assert_eq!(
            unbundled_icon_notices(&registry, &fewer),
            [
                "Basic Shapes: icons `shapes`, `star` are not bundled with Mindweave; \
              the default icon is used"
            ]
        );
    }

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
