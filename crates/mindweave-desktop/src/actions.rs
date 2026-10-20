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
        /// Escape in the editor: aborts a gesture, else clears the
        /// selection and goes back to the Pointer tool.
        Cancel,
        /// Arrow keys: move the selection 1 unit; with Shift, 10.
        NudgeLeft,
        NudgeRight,
        NudgeUp,
        NudgeDown,
        NudgeLeftFar,
        NudgeRightFar,
        NudgeUpFar,
        NudgeDownFar,
        /// The Edit menu. In the editor these act on the shapes; text
        /// fields keep their own keys for them.
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        /// Copies the selection, offset by `DUPLICATE_OFFSET`.
        Duplicate,
        Delete,
        SelectAll,
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
/// every window; the tool and edit keys apply only in the editor.
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
        KeyBinding::new("left", NudgeLeft, editor),
        KeyBinding::new("right", NudgeRight, editor),
        KeyBinding::new("up", NudgeUp, editor),
        KeyBinding::new("down", NudgeDown, editor),
        KeyBinding::new("shift-left", NudgeLeftFar, editor),
        KeyBinding::new("shift-right", NudgeRightFar, editor),
        KeyBinding::new("shift-up", NudgeUpFar, editor),
        KeyBinding::new("shift-down", NudgeDownFar, editor),
        KeyBinding::new("cmd-z", Undo, editor),
        KeyBinding::new("cmd-shift-z", Redo, editor),
        KeyBinding::new("cmd-x", Cut, editor),
        KeyBinding::new("cmd-c", Copy, editor),
        KeyBinding::new("cmd-v", Paste, editor),
        KeyBinding::new("cmd-d", Duplicate, editor),
        KeyBinding::new("backspace", Delete, editor),
        KeyBinding::new("delete", Delete, editor),
        KeyBinding::new("cmd-a", SelectAll, editor),
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
            ("left", "mindweave::NudgeLeft"),
            ("right", "mindweave::NudgeRight"),
            ("up", "mindweave::NudgeUp"),
            ("down", "mindweave::NudgeDown"),
            ("shift-left", "mindweave::NudgeLeftFar"),
            ("shift-right", "mindweave::NudgeRightFar"),
            ("shift-up", "mindweave::NudgeUpFar"),
            ("shift-down", "mindweave::NudgeDownFar"),
            ("cmd-z", "mindweave::Undo"),
            ("cmd-shift-z", "mindweave::Redo"),
            ("cmd-x", "mindweave::Cut"),
            ("cmd-c", "mindweave::Copy"),
            ("cmd-v", "mindweave::Paste"),
            ("cmd-d", "mindweave::Duplicate"),
            ("backspace", "mindweave::Delete"),
            ("delete", "mindweave::Delete"),
            ("cmd-a", "mindweave::SelectAll"),
        ];
        let expected: Vec<(String, &str)> =
            expected.iter().map(|(k, a)| (k.to_string(), *a)).collect();
        assert_eq!(bindings, expected);
        assert_eq!(ShowDashboard.name(), "mindweave::ShowDashboard");
    }

    /// Actions bound only in the editor, outside text fields, which keep
    /// their own copy, paste, undo and select-all keys.
    const EDIT_ACTIONS: [&str; 9] = [
        "::Cancel",
        "::Undo",
        "::Redo",
        "::Cut",
        "::Copy",
        "::Paste",
        "::Duplicate",
        "::Delete",
        "::SelectAll",
    ];

    #[test]
    fn editor_keys_apply_in_the_editor_but_not_in_text_fields() {
        use gpui_kit::KeyContext;
        let context = |names: &[&str]| -> Vec<KeyContext> {
            names
                .iter()
                .map(|n| KeyContext::parse(n).unwrap())
                .collect()
        };
        let tool_bindings: Vec<KeyBinding> = key_bindings()
            .into_iter()
            .filter(|b| {
                let name = b.action().name();
                name.contains("Tool")
                    || name.contains("Nudge")
                    || EDIT_ACTIONS.iter().any(|a| name.ends_with(a))
            })
            .collect();
        assert_eq!(tool_bindings.len(), 15 + 9);
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
