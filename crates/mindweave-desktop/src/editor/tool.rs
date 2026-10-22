//! The editor's active tool and the ribbon tab being shown.

/// The ribbon tab holding Pointer, Hand and Text.
pub const HOME_TAB: &str = "home";
/// The ribbon tab holding the mind map tools.
pub const MINDMAP_TAB: &str = "mindmap";
/// The built-in library whose tools have their own keys.
pub const BASIC_SHAPES: &str = "basic-shapes";
/// Basic Shapes tools with a key of their own: shape id and key.
pub const SHAPE_KEYS: [(&str, &str); 3] = [("rectangle", "R"), ("ellipse", "O"), ("line", "L")];

/// What a left drag on the canvas does.
#[derive(Clone, Debug, PartialEq)]
pub enum Tool {
    Pointer,
    Hand,
    /// A click adds a text box there and starts editing it.
    Text,
    /// Draws a mind map relationship: a curved, dashed arrow between two
    /// topics.
    Relationship,
    /// Places shape `shape` of library `library`.
    Shape {
        library: String,
        shape: String,
    },
}

impl Tool {
    /// The tool placing `shape` of `library`.
    pub fn shape(library: &str, shape: &str) -> Tool {
        Tool::Shape {
            library: library.to_string(),
            shape: shape.to_string(),
        }
    }

    /// The ribbon tab showing this tool's button.
    pub fn tab(&self) -> &str {
        match self {
            Tool::Pointer | Tool::Hand | Tool::Text => HOME_TAB,
            Tool::Relationship => MINDMAP_TAB,
            Tool::Shape { library, .. } => library,
        }
    }
}

/// The key that selects shape `shape` of `library`, if it has one.
pub fn shape_key(library: &str, shape: &str) -> Option<&'static str> {
    if library != BASIC_SHAPES {
        return None;
    }
    SHAPE_KEYS
        .iter()
        .find(|(id, _)| *id == shape)
        .map(|(_, key)| *key)
}

/// The active tool, whether it stays active after placing a shape, and the
/// ribbon tab being shown.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolState {
    pub tool: Tool,
    /// Set by double-clicking a tool: it then stays active after placing a
    /// shape, until Escape or another tool is chosen.
    pub locked: bool,
    /// [`HOME_TAB`] or a library id.
    pub tab: String,
}

impl Default for ToolState {
    fn default() -> Self {
        ToolState {
            tool: Tool::Pointer,
            locked: false,
            tab: HOME_TAB.to_string(),
        }
    }
}

impl ToolState {
    /// Makes `tool` active, locked when `lock` is set, and shows its tab.
    pub fn select(&mut self, tool: Tool, lock: bool) {
        self.tab = tool.tab().to_string();
        self.tool = tool;
        self.locked = lock;
    }

    /// Called after a shape or text box is placed: back to Pointer unless
    /// locked. The tab stays.
    pub fn after_place(&mut self) {
        if !self.locked {
            self.tool = Tool::Pointer;
        }
    }

    /// Escape: back to Pointer, unlocked. The tab stays.
    pub fn cancel(&mut self) {
        self.tool = Tool::Pointer;
        self.locked = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle() -> Tool {
        Tool::shape(BASIC_SHAPES, "rectangle")
    }

    #[test]
    fn after_place_returns_to_pointer_unless_locked() {
        let mut state = ToolState::default();
        state.select(rectangle(), false);
        state.after_place();
        assert_eq!(state.tool, Tool::Pointer);

        state.select(rectangle(), true);
        state.after_place();
        assert_eq!(state.tool, rectangle());
        assert!(state.locked);
        // Placing does not change the tab.
        assert_eq!(state.tab, BASIC_SHAPES);
    }

    #[test]
    fn text_tool_returns_to_pointer_unless_locked() {
        let mut state = ToolState::default();
        state.select(Tool::Text, false);
        assert_eq!(state.tab, HOME_TAB);
        state.after_place();
        assert_eq!(state.tool, Tool::Pointer);

        state.select(Tool::Text, true);
        state.after_place();
        assert_eq!(state.tool, Tool::Text);
        assert!(state.locked);
        state.cancel();
        assert_eq!(state.tool, Tool::Pointer);
    }

    #[test]
    fn cancel_unlocks() {
        let mut state = ToolState::default();
        state.select(rectangle(), true);
        state.cancel();
        assert_eq!(state.tool, Tool::Pointer);
        assert!(!state.locked);
        assert_eq!(state.tab, BASIC_SHAPES);
        // A later placement with a fresh tool is not locked.
        state.select(rectangle(), false);
        state.after_place();
        assert_eq!(state.tool, Tool::Pointer);
    }

    #[test]
    fn selecting_shape_switches_tab() {
        let mut state = ToolState::default();
        assert_eq!(state.tab, HOME_TAB);
        state.select(Tool::shape("flowchart", "decision"), false);
        assert_eq!(state.tab, "flowchart");
        state.select(rectangle(), false);
        assert_eq!(state.tab, BASIC_SHAPES);
        state.select(Tool::Hand, false);
        assert_eq!(state.tab, HOME_TAB);
    }

    #[test]
    fn choosing_another_tool_unlocks() {
        let mut state = ToolState::default();
        state.select(rectangle(), true);
        state.select(Tool::shape(BASIC_SHAPES, "ellipse"), false);
        assert!(!state.locked);
    }

    #[test]
    fn only_basic_shapes_tools_have_keys() {
        assert_eq!(shape_key(BASIC_SHAPES, "rectangle"), Some("R"));
        assert_eq!(shape_key(BASIC_SHAPES, "ellipse"), Some("O"));
        assert_eq!(shape_key(BASIC_SHAPES, "line"), Some("L"));
        assert_eq!(shape_key(BASIC_SHAPES, "arrow"), None);
        assert_eq!(shape_key(BASIC_SHAPES, "star"), None);
        assert_eq!(shape_key("flowchart", "rectangle"), None);
    }
}
