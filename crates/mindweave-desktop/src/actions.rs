//! Application actions and their default key bindings.

use gpui_kit::{actions, App, KeyBinding};

actions!(
    mindweave,
    [
        NewFile,
        OpenFile,
        Save,
        CloseWindow,
        Quit,
        ToggleThemePicker,
        ToggleLayers,
        ToggleProperties,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ShowDashboard,
        /// Stands in for menu items that are not built yet. Nothing handles
        /// it, so the native menu always shows such items disabled.
        NotImplemented,
    ]
);

/// The default keymap. Bindings have no key context, so they apply in every
/// window.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-n", NewFile, None),
        KeyBinding::new("cmd-o", OpenFile, None),
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-k cmd-t", ToggleThemePicker, None),
        KeyBinding::new("cmd-b", ToggleLayers, None),
        KeyBinding::new("cmd-alt-b", ToggleProperties, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ZoomReset, None),
    ]
}

/// Registers [`key_bindings`] with the application.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(key_bindings());
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Action as _;

    #[test]
    fn keymap_matches_the_spec() {
        let bindings: Vec<(String, &str)> = key_bindings()
            .iter()
            .map(|b| {
                let keys: Vec<String> = b.keystrokes().iter().map(|k| k.unparse()).collect();
                (keys.join(" "), b.action().name())
            })
            .collect();
        let expected = [
            ("cmd-n", "mindweave::NewFile"),
            ("cmd-o", "mindweave::OpenFile"),
            ("cmd-s", "mindweave::Save"),
            ("cmd-w", "mindweave::CloseWindow"),
            ("cmd-q", "mindweave::Quit"),
            ("cmd-k cmd-t", "mindweave::ToggleThemePicker"),
            ("cmd-b", "mindweave::ToggleLayers"),
            // `unparse` writes modifiers in gpui order.
            ("alt-cmd-b", "mindweave::ToggleProperties"),
            ("cmd-=", "mindweave::ZoomIn"),
            ("cmd--", "mindweave::ZoomOut"),
            ("cmd-0", "mindweave::ZoomReset"),
        ];
        let expected: Vec<(String, &str)> =
            expected.iter().map(|(k, a)| (k.to_string(), *a)).collect();
        assert_eq!(bindings, expected);
        assert_eq!(ShowDashboard.name(), "mindweave::ShowDashboard");
    }
}
