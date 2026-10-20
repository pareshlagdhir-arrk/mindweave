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
        ToolPointer,
        ToolHand,
        ToolRectangle,
        ToolEllipse,
        ToolLine,
        ToolArrow,
        /// Escape in the editor: back to the Pointer tool.
        Cancel,
        /// Stands in for menu items that are not built yet. Nothing handles
        /// it, so the native menu always shows such items disabled.
        NotImplemented,
    ]
);

/// The key context of the editor window's root.
pub const EDITOR_CONTEXT: &str = "Editor";
/// Where tool keys apply: in the editor, but not while a text field (gpui's
/// `Input` context) has focus, so typing a name or number is not taken as a
/// tool key.
const EDITOR_NOT_TYPING: &str = "Editor && !Input";

/// The default keymap. Most bindings have no key context, so they apply in
/// every window; the tool keys apply only in the editor.
pub fn key_bindings() -> Vec<KeyBinding> {
    let editor = Some(EDITOR_NOT_TYPING);
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
        KeyBinding::new("v", ToolPointer, editor),
        KeyBinding::new("h", ToolHand, editor),
        KeyBinding::new("r", ToolRectangle, editor),
        KeyBinding::new("o", ToolEllipse, editor),
        KeyBinding::new("l", ToolLine, editor),
        KeyBinding::new("a", ToolArrow, editor),
        KeyBinding::new("escape", Cancel, editor),
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
            ("v", "mindweave::ToolPointer"),
            ("h", "mindweave::ToolHand"),
            ("r", "mindweave::ToolRectangle"),
            ("o", "mindweave::ToolEllipse"),
            ("l", "mindweave::ToolLine"),
            ("a", "mindweave::ToolArrow"),
            ("escape", "mindweave::Cancel"),
        ];
        let expected: Vec<(String, &str)> =
            expected.iter().map(|(k, a)| (k.to_string(), *a)).collect();
        assert_eq!(bindings, expected);
        assert_eq!(ShowDashboard.name(), "mindweave::ShowDashboard");
    }

    #[test]
    fn tool_keys_apply_in_the_editor_but_not_in_text_fields() {
        use gpui_kit::KeyContext;
        let context = |names: &[&str]| -> Vec<KeyContext> {
            names
                .iter()
                .map(|n| KeyContext::parse(n).unwrap())
                .collect()
        };
        let tool_bindings: Vec<KeyBinding> = key_bindings()
            .into_iter()
            .filter(|b| b.action().name().contains("Tool") || b.action().name().ends_with("Cancel"))
            .collect();
        assert_eq!(tool_bindings.len(), 7);
        for binding in &tool_bindings {
            let predicate = binding.predicate().expect("tool keys have a context");
            assert!(predicate.eval(&context(&["Root", EDITOR_CONTEXT])));
            assert!(!predicate.eval(&context(&["Root", EDITOR_CONTEXT, "Input"])));
            assert!(predicate
                .depth_of(&context(&["Root", EDITOR_CONTEXT, "Input"]))
                .is_none());
            assert!(!predicate.eval(&context(&["Root"])));
        }
    }
}
