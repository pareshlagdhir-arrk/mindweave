//! The Topic section of the Properties sidebar, shown when every selected
//! shape is a mind map topic: the map's structure, the branch line style,
//! the topic's picture and link, and re-arranging the map. Each change is
//! one undo step.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mindweave_core::{BranchStyle, Shape, Structure};
use uuid::Uuid;

use super::properties_panel::field_row;
use super::shape_properties::{common, Value, SECTION_GAP};
use super::{section_label, EditorWindow};

/// Width of a dropdown or the link field.
const CONTROL_WIDTH: f32 = 120.;
/// The branch choice that follows the parent.
const SAME_AS_PARENT: &str = "Same as Parent";

/// The link field, and the link it shows.
pub struct TopicFields {
    pub link: Entity<InputState>,
    /// The topics and link shown, so a new selection shows its own.
    shows: Option<(Vec<Uuid>, Option<String>)>,
}

impl TopicFields {
    pub fn new(window: &mut Window, cx: &mut Context<EditorWindow>) -> (Self, Subscription) {
        let link = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        let subscription = cx.subscribe_in(&link, window, |this, input, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let value = input.read(cx).value().to_string();
                let shown = this
                    .topic_fields
                    .shows
                    .as_ref()
                    .and_then(|(_, l)| l.clone());
                if value.trim() != shown.as_deref().unwrap_or("") {
                    this.set_topic_link(&value, cx);
                }
            }
        });
        (TopicFields { link, shows: None }, subscription)
    }
}

impl EditorWindow {
    /// Shows the selected topics' link in the link field when the
    /// selection or the link changed (not while it is being typed in).
    pub(super) fn sync_topic_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let topics = self.selected_topics();
        let links: Vec<Option<String>> = topics
            .iter()
            .filter_map(|id| self.session.find_shape(*id))
            .filter_map(|s| s.topic.as_ref().map(|t| t.link.clone()))
            .collect();
        let link = match common(links) {
            Some(Value::Same(link)) => link,
            _ => None,
        };
        let wanted = (topics, link.clone());
        if self.topic_fields.shows.as_ref() == Some(&wanted) {
            return;
        }
        let input = self.topic_fields.link.clone();
        if input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        input.update(cx, |state, cx| {
            state.set_value(link.unwrap_or_default(), window, cx)
        });
        self.topic_fields.shows = Some(wanted);
    }

    /// Sets the structure of the maps the selected topics belong to.
    fn set_structure(&mut self, structure: Structure, cx: &mut Context<Self>) {
        let maps: Vec<Uuid> = self
            .selected_topics()
            .iter()
            .filter_map(|id| Some(self.session.find_shape(*id)?.topic.as_ref()?.map))
            .collect();
        self.update_session(
            |s| {
                s.update_shapes(&maps, |shape| {
                    if let Some(t) = shape.topic.as_mut() {
                        t.structure = structure;
                    }
                })
            },
            cx,
        );
    }

    /// Sets the branch style of the selected topics (`None`: follow the
    /// parent; a central topic always has one).
    fn set_branch(&mut self, style: Option<BranchStyle>, cx: &mut Context<Self>) {
        let ids = self.selected_topics();
        self.update_session(
            |s| {
                s.update_shapes(&ids, |shape| {
                    if let Some(t) = shape.topic.as_mut() {
                        t.branch = if t.is_central() {
                            Some(style.unwrap_or_default())
                        } else {
                            style
                        };
                    }
                })
            },
            cx,
        );
    }
}

/// Whether something is selected and all of it is topics.
fn all_topics(shapes: &[&Shape]) -> bool {
    !shapes.is_empty() && shapes.iter().all(|s| s.topic.is_some())
}

/// A dropdown button showing `current` (or "Mixed") and offering
/// `choices`; a pick runs `pick` with the choice.
fn dropdown(
    id: &'static str,
    current: Option<String>,
    choices: Vec<String>,
    pick: impl Fn(&mut EditorWindow, &str, &mut Context<EditorWindow>) + 'static,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    let this = cx.entity().downgrade();
    let pick = std::rc::Rc::new(pick);
    Button::new(id)
        .outline()
        .small()
        .w(px(CONTROL_WIDTH))
        .label(current.clone().unwrap_or_else(|| "Mixed".into()))
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for choice in &choices {
                let (this, pick, choice) = (this.clone(), pick.clone(), choice.clone());
                let checked = current.as_deref() == Some(choice.as_str());
                menu = menu.item(
                    PopupMenuItem::new(choice.clone())
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            this.update(cx, |this, cx| pick(this, &choice, cx)).ok();
                        }),
                );
            }
            menu
        })
}

/// A small ghost button running `run`.
fn small_button(
    id: &'static str,
    label: &'static str,
    run: impl Fn(&mut EditorWindow, &mut Window, &mut Context<EditorWindow>) + 'static,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .label(label)
        .on_click(cx.listener(move |this, _, window, cx| run(this, window, cx)))
}

/// The Topic section, when every selected shape is a topic.
pub(super) fn render(editor: &EditorWindow, cx: &mut Context<EditorWindow>) -> Option<AnyElement> {
    let shapes = editor.shapes_among(&editor.selection);
    if !all_topics(&shapes) {
        return None;
    }
    let topics: Vec<_> = shapes.iter().filter_map(|s| s.topic.clone()).collect();
    let any_central = topics.iter().any(|t| t.is_central());
    let has_picture = topics.iter().any(|t| t.picture.is_some());
    let link = match common(topics.iter().map(|t| t.link.clone())) {
        Some(Value::Same(link)) => link,
        _ => None,
    };

    // The structure of the maps the topics are in, read from their
    // central topics.
    let page = editor.session().page();
    let structure_of = |map: Uuid| {
        page.layers
            .iter()
            .flat_map(|l| &l.objects)
            .find(|s| s.id == map)
            .and_then(|s| s.topic.as_ref())
            .map(|t| t.structure)
    };
    let structure = match common(topics.iter().map(|t| structure_of(t.map))) {
        Some(Value::Same(Some(s))) => Some(s.name().to_string()),
        _ => None,
    };
    let branch = match common(topics.iter().map(|t| t.branch)) {
        Some(Value::Same(Some(b))) => Some(b.name().to_string()),
        Some(Value::Same(None)) => Some(SAME_AS_PARENT.to_string()),
        _ => None,
    };
    let mut branch_choices: Vec<String> = Vec::new();
    if !any_central {
        branch_choices.push(SAME_AS_PARENT.into());
    }
    branch_choices.extend(BranchStyle::ALL.map(|b| b.name().to_string()));

    let structure_row = field_row(
        "Structure",
        None,
        dropdown(
            "topic-structure",
            structure,
            Structure::ALL.map(|s| s.name().to_string()).to_vec(),
            |this, choice, cx| {
                if let Some(s) = Structure::ALL.into_iter().find(|s| s.name() == choice) {
                    this.set_structure(s, cx);
                }
            },
            cx,
        ),
    );
    let branch_row = field_row(
        "Branch",
        Some("The line to the topic, and to its subtopics unless they set their own"),
        dropdown(
            "topic-branch",
            branch,
            branch_choices,
            |this, choice, cx| {
                let style = BranchStyle::ALL.into_iter().find(|b| b.name() == choice);
                this.set_branch(style, cx);
            },
            cx,
        ),
    );
    let picture_row = field_row(
        "Picture",
        Some("An icon or image above the text: choose an image, or drag an icon from the Icons pane (⌘⇧I) onto the topic. Drag the picture's corner handle to resize it."),
        h_flex()
            .gap(px(2.))
            .child(small_button(
                "topic-picture-choose",
                "Choose…",
                |this, window, cx| this.choose_topic_image(window, cx),
                cx,
            ))
            .when(has_picture, |row| {
                row.child(small_button(
                    "topic-picture-remove",
                    "Remove",
                    |this, _, cx| this.clear_topic_picture(cx),
                    cx,
                ))
            }),
    );
    // Auto layout is the map's: on when every selected topic's map has it.
    let auto = editor
        .selected_maps()
        .iter()
        .all(|m| editor.session().auto_layout(*m));
    let this = cx.entity().downgrade();
    let layout_row = field_row(
        "Auto layout",
        Some("On: the map keeps its topics arranged, and turning it on puts dragged topics back. Off: topics stay where you drop them."),
        Switch::new("topic-auto-layout")
            .checked(auto)
            .small()
            .on_click(move |on, window, cx| {
                this.update(cx, |this, cx| this.set_map_auto_layout(*on, window, cx))
                    .ok();
            }),
    );
    let open = link.clone().map(|link| {
        Button::new("topic-link-open")
            .ghost()
            .xsmall()
            .icon(IconName::ExternalLink)
            .tooltip("Open Link")
            .on_click(move |_, _, cx| cx.open_url(&link))
    });
    let link_row = field_row(
        "Link",
        Some("A web address or file; click the topic's link badge to open it"),
        h_flex()
            .gap(px(2.))
            .child(
                div()
                    .w(px(CONTROL_WIDTH - 24.))
                    .child(Input::new(&editor.topic_fields.link).small()),
            )
            .children(open),
    );

    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(section_label("Topic", cx).mt(px(SECTION_GAP)))
            .child(structure_row)
            .child(branch_row)
            .child(layout_row)
            .child(picture_row)
            .child(link_row)
            .into_any_element(),
    )
}
