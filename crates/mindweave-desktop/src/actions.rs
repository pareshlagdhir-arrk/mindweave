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

// Keys of the on-canvas text editor, bound only while text is being
// edited (see `TEXT_EDITING_CONTEXT`).
actions!(
    mindweave,
    [
        /// Caret one character left or right, one line up or down.
        TextLeft,
        TextRight,
        TextUp,
        TextDown,
        /// Caret to the previous word's start or the next word's end.
        TextWordLeft,
        TextWordRight,
        /// Caret to the start or end of its line.
        TextLineStart,
        TextLineEnd,
        /// Caret to the start or end of the text.
        TextDocStart,
        TextDocEnd,
        /// The moves above, extending the selection.
        TextSelectLeft,
        TextSelectRight,
        TextSelectUp,
        TextSelectDown,
        TextSelectWordLeft,
        TextSelectWordRight,
        TextSelectLineStart,
        TextSelectLineEnd,
        TextSelectDocStart,
        TextSelectDocEnd,
        /// Deletes the selection, else the character (or with Alt the
        /// word) before or after the caret.
        TextBackspace,
        TextDelete,
        TextBackspaceWord,
        TextDeleteWord,
        /// Return: a new paragraph; Shift+Return: a line break in it.
        TextNewParagraph,
        TextLineBreak,
        TextSelectAll,
        /// Style toggles: the selection, or what typing uses next.
        TextBold,
        TextItalic,
        TextUnderline,
        TextStrike,
        TextCopy,
        TextCut,
        TextPaste,
        /// Undo and redo typing, in word-sized steps.
        TextUndo,
        TextRedo,
        /// Escape: stops editing.
        TextExit,
    ]
);

/// The key context of the editor window's root.
pub const EDITOR_CONTEXT: &str = "Editor";
/// Added to the editor root's key context while text on the canvas is
/// being edited.
pub const TEXT_EDITING_CONTEXT: &str = "TextEditing";
/// Where tool keys apply: in the editor, but not while a text field (gpui's
/// `Input` context) has focus or text on the canvas is being edited, so
/// typing is not taken as a tool or shape key.
const EDITOR_NOT_TYPING: &str = "Editor && !Input && !TextEditing";
/// Everywhere but while text on the canvas is being edited, for global
/// keys that the text editor uses.
const NOT_EDITING_TEXT: &str = "!TextEditing";
/// Where the canvas text editor's keys apply: while editing, but not in a
/// text field that has focus meanwhile.
const TEXT_EDITING: &str = "TextEditing && !Input";

/// The default keymap. Most bindings have no key context, so they apply in
/// every window; the tool and edit keys apply only in the editor.
pub fn key_bindings() -> Vec<KeyBinding> {
    let editor = Some(EDITOR_NOT_TYPING);
    let mut bindings = vec![
        KeyBinding::new("cmd-n", NewFile, None),
        KeyBinding::new("cmd-o", OpenFile, None),
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-k cmd-t", ToggleThemePicker, None),
        // Cmd+B is bold while editing text.
        KeyBinding::new("cmd-b", ToggleLayers, Some(NOT_EDITING_TEXT)),
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
    ];
    bindings.extend(text_key_bindings());
    bindings
}

/// The canvas text editor's keys.
fn text_key_bindings() -> Vec<KeyBinding> {
    let text = Some(TEXT_EDITING);
    vec![
        KeyBinding::new("left", TextLeft, text),
        KeyBinding::new("right", TextRight, text),
        KeyBinding::new("up", TextUp, text),
        KeyBinding::new("down", TextDown, text),
        KeyBinding::new("alt-left", TextWordLeft, text),
        KeyBinding::new("alt-right", TextWordRight, text),
        KeyBinding::new("cmd-left", TextLineStart, text),
        KeyBinding::new("cmd-right", TextLineEnd, text),
        KeyBinding::new("cmd-up", TextDocStart, text),
        KeyBinding::new("cmd-down", TextDocEnd, text),
        KeyBinding::new("shift-left", TextSelectLeft, text),
        KeyBinding::new("shift-right", TextSelectRight, text),
        KeyBinding::new("shift-up", TextSelectUp, text),
        KeyBinding::new("shift-down", TextSelectDown, text),
        KeyBinding::new("alt-shift-left", TextSelectWordLeft, text),
        KeyBinding::new("alt-shift-right", TextSelectWordRight, text),
        KeyBinding::new("cmd-shift-left", TextSelectLineStart, text),
        KeyBinding::new("cmd-shift-right", TextSelectLineEnd, text),
        KeyBinding::new("cmd-shift-up", TextSelectDocStart, text),
        KeyBinding::new("cmd-shift-down", TextSelectDocEnd, text),
        KeyBinding::new("backspace", TextBackspace, text),
        KeyBinding::new("shift-backspace", TextBackspace, text),
        KeyBinding::new("delete", TextDelete, text),
        KeyBinding::new("alt-backspace", TextBackspaceWord, text),
        KeyBinding::new("alt-delete", TextDeleteWord, text),
        KeyBinding::new("enter", TextNewParagraph, text),
        KeyBinding::new("shift-enter", TextLineBreak, text),
        KeyBinding::new("cmd-a", TextSelectAll, text),
        KeyBinding::new("cmd-b", TextBold, text),
        KeyBinding::new("cmd-i", TextItalic, text),
        KeyBinding::new("cmd-u", TextUnderline, text),
        KeyBinding::new("cmd-shift-x", TextStrike, text),
        KeyBinding::new("cmd-c", TextCopy, text),
        KeyBinding::new("cmd-x", TextCut, text),
        KeyBinding::new("cmd-v", TextPaste, text),
        KeyBinding::new("cmd-z", TextUndo, text),
        KeyBinding::new("cmd-shift-z", TextRedo, text),
        KeyBinding::new("escape", TextExit, text),
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
            .filter(|b| !b.action().name().starts_with("mindweave::Text"))
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
            assert!(!predicate.eval(&context(&["Root", "Editor TextEditing"])));
            assert!(predicate
                .depth_of(&context(&["Root", EDITOR_CONTEXT, "Input"]))
                .is_none());
            assert!(!predicate.eval(&context(&["Root"])));
        }
    }

    fn names(bindings: &[KeyBinding]) -> Vec<&'static str> {
        bindings.iter().map(|b| b.action().name()).collect()
    }

    #[test]
    fn text_keys_win_while_editing_and_other_keys_type() {
        use gpui_kit::{KeyContext, Keymap, Keystroke};
        let stack = |names: &[&str]| -> Vec<KeyContext> {
            names
                .iter()
                .map(|n| KeyContext::parse(n).unwrap())
                .collect()
        };
        let key = |s: &str| [Keystroke::parse(s).unwrap()];
        let keymap = Keymap::new(key_bindings());
        let editing = stack(&["Root", "Editor TextEditing"]);
        let text = text_key_bindings();
        assert_eq!(text.len(), 38);
        for binding in &text {
            let (found, pending) = keymap.bindings_for_input(binding.keystrokes(), &editing);
            assert!(!pending);
            // The first binding is the one dispatched.
            assert_eq!(
                names(&found).first(),
                Some(&binding.action().name()),
                "{:?}",
                binding.keystrokes()
            );
        }
        // Tool keys and other letters are typed, not taken as commands.
        for k in ["v", "h", "r", "o", "l", "a", "space", "shift-a", "cmd-d"] {
            let (found, _) = keymap.bindings_for_input(&key(k), &editing);
            assert!(found.is_empty(), "{k}: {:?}", names(&found));
        }
        // Not editing: the shape keys.
        let shapes = stack(&["Root", "Editor"]);
        let first = |k: &str, ctx: &[KeyContext]| {
            names(&keymap.bindings_for_input(&key(k), ctx).0)
                .first()
                .copied()
        };
        assert_eq!(first("cmd-z", &shapes), Some("mindweave::Undo"));
        assert_eq!(first("left", &shapes), Some("mindweave::NudgeLeft"));
        assert_eq!(first("cmd-b", &shapes), Some("mindweave::ToggleLayers"));
        // A text field focused while editing keeps its keys.
        let field = stack(&["Root", "Editor TextEditing", "Input"]);
        assert_eq!(first("left", &field), None);
        // Cmd+B is bold while editing, with no other binding competing
        // (not even in a field focused meanwhile); elsewhere it toggles the
        // Layers panel, in every window.
        let (found, _) = keymap.bindings_for_input(&key("cmd-b"), &editing);
        assert_eq!(names(&found), ["mindweave::TextBold"]);
        assert_eq!(first("cmd-b", &field), None);
        assert_eq!(
            first("cmd-b", &stack(&["Root"])),
            Some("mindweave::ToggleLayers")
        );
        // No binding without a context shares a key with a text key.
        let text_keys: Vec<String> = text
            .iter()
            .map(|b| format!("{:?}", b.keystrokes()))
            .collect();
        for b in key_bindings() {
            if b.predicate().is_none() {
                assert!(
                    !text_keys.contains(&format!("{:?}", b.keystrokes())),
                    "{}",
                    b.action().name()
                );
            }
        }
    }
}
