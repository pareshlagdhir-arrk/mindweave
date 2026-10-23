//! The editor's active tool and the ribbon tab being shown.

use mindweave_core::{Icon, Library, LibraryItem};

/// The ribbon tab holding Pointer, Hand and Text.
pub const HOME_TAB: &str = "home";
/// The ribbon tab holding the mind map tools.
pub const MINDMAP_TAB: &str = "mindmap";
/// The ribbon tab holding the artboard tool and presets.
pub const ARTBOARD_TAB: &str = "artboard";
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
    /// Draws an artboard of any size (Custom Artboard); a click places
    /// one of the default size.
    Artboard,
    /// Places shape `shape` of library `library`.
    Shape {
        library: String,
        shape: String,
    },
    /// Places template `template` of library `library`: its parts, as one
    /// group.
    Template {
        library: String,
        template: String,
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

    /// The tool placing template `template` of `library`.
    pub fn template(library: &str, template: &str) -> Tool {
        Tool::Template {
            library: library.to_string(),
            template: template.to_string(),
        }
    }

    /// The ribbon tab showing this tool's button.
    pub fn tab(&self) -> &str {
        match self {
            Tool::Pointer | Tool::Hand | Tool::Text => HOME_TAB,
            Tool::Relationship => MINDMAP_TAB,
            Tool::Artboard => ARTBOARD_TAB,
            Tool::Shape { library, .. } | Tool::Template { library, .. } => library,
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

/// One button of a library's ribbon: a shape or a template.
pub struct RibbonItem<'a> {
    /// The tool the button selects.
    pub tool: Tool,
    /// The shape or template id.
    pub id: &'a str,
    pub name: &'a str,
    pub icon: &'a Icon,
}

/// The buttons of `library`'s ribbon, shapes and templates in file order.
pub fn ribbon_items(library: &Library) -> Vec<RibbonItem<'_>> {
    library
        .items
        .iter()
        .filter_map(|item| match item {
            LibraryItem::Shape(ix) => {
                let shape = library.shapes.get(*ix)?;
                Some(RibbonItem {
                    tool: Tool::shape(&library.id, &shape.id),
                    id: &shape.id,
                    name: &shape.name,
                    icon: &shape.icon,
                })
            }
            LibraryItem::Template(t) => Some(RibbonItem {
                tool: Tool::template(&library.id, &t.id),
                id: &t.id,
                name: &t.name,
                icon: &t.icon,
            }),
        })
        .collect()
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
    fn the_artboard_tool_shows_its_tab_and_returns_to_pointer() {
        let mut state = ToolState::default();
        state.select(Tool::Artboard, false);
        assert_eq!(state.tab, ARTBOARD_TAB);
        state.after_place();
        assert_eq!(state.tool, Tool::Pointer);
        state.select(Tool::Artboard, true);
        state.after_place();
        assert_eq!(state.tool, Tool::Artboard);
    }

    #[test]
    fn choosing_another_tool_unlocks() {
        let mut state = ToolState::default();
        state.select(rectangle(), true);
        state.select(Tool::shape(BASIC_SHAPES, "ellipse"), false);
        assert!(!state.locked);
    }

    #[test]
    fn template_tool_round_trips_tab() {
        let class = Tool::template("uml-test", "class");
        assert_eq!(class.tab(), "uml-test");
        let mut state = ToolState::default();
        state.select(class.clone(), false);
        assert_eq!(state.tab, "uml-test");
        state.after_place();
        assert_eq!(state.tool, Tool::Pointer);
        assert_eq!(state.tab, "uml-test");
        // Locked, it stays for the next placement.
        state.select(class.clone(), true);
        state.after_place();
        assert_eq!(state.tool, class);
        // A shape of the same id is another tool.
        assert_ne!(class, Tool::shape("uml-test", "class"));
    }

    #[test]
    fn templates_appear_in_ribbon_order() {
        let library = crate::libraries::template_test_library();
        let tools: Vec<Tool> = ribbon_items(&library).into_iter().map(|i| i.tool).collect();
        assert_eq!(
            tools,
            [
                Tool::shape("uml-test", "note"),
                Tool::template("uml-test", "class"),
                Tool::shape("uml-test", "after"),
            ]
        );
        let names: Vec<&str> = ribbon_items(&library).iter().map(|i| i.name).collect();
        assert_eq!(names, ["Note", "Class", "After"]);
        // A library without templates lists its shapes, as before.
        let basic = &mindweave_core::builtin_libraries()[0];
        let ids: Vec<&str> = ribbon_items(basic).iter().map(|i| i.id).collect();
        let shapes: Vec<&str> = basic.shapes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, shapes);
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
