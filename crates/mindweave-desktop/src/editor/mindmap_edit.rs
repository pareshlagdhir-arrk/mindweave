//! Mind map commands in the editor: adding central topics, subtopics,
//! siblings and summaries, boundaries, moving a topic by dragging it onto
//! another, and a topic's icon, image and link.

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::{Pt, Rect};
use mindweave_core::mindmap::TopicParts;
use mindweave_core::{fit_topic_picture, DocumentSession, ImageData, Shape, TOPIC_IMAGE_SIZE};
use uuid::Uuid;

use super::EditorWindow;

/// Where a new topic goes, relative to the selected one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTopic {
    /// The last child of the selected topic.
    Sub,
    /// Right after it among its siblings (a subtopic of a central topic).
    After,
    /// Right before it.
    Before,
}

impl EditorWindow {
    /// The one selected topic.
    pub(super) fn selected_topic(&self) -> Option<Uuid> {
        one_topic(&self.session, &self.selection)
    }

    /// The selected topics, in selection order.
    pub(super) fn selected_topics(&self) -> Vec<Uuid> {
        self.selection
            .iter()
            .copied()
            .filter(|id| {
                self.session
                    .find_shape(*id)
                    .is_some_and(|s| s.topic.is_some())
            })
            .collect()
    }

    /// Selects topic `id` and starts editing its text, all of it
    /// selected, so typing names it.
    fn select_and_name(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.selection = vec![id];
        self.edit_selected_text(window, cx);
        cx.notify();
    }

    /// Adds a new mind map in the middle of the view and names its
    /// central topic.
    pub(super) fn add_central_topic(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        if !self.session.active_layer_selectable() {
            window.push_notification(Notification::warning("Show the layer to draw on it."), cx);
            return;
        }
        let view = self.visible_canvas();
        let at = (view.x + view.w / 2., view.y + view.h / 2.);
        let mut id = None;
        self.update_session(
            |s| {
                let central = s.add_central_topic(at);
                // Added inside an artboard, the map joins it.
                s.assign_membership(&[central]);
                id = Some(central);
            },
            cx,
        );
        if let Some(id) = id {
            self.select_and_name(id, window, cx);
        }
    }

    /// Adds a topic `kind` of the selected one and names it. Without a
    /// selected topic, says what to select.
    pub(super) fn add_topic(
        &mut self,
        kind: NewTopic,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_edit_action(window, cx);
        let Some(topic) = self.selected_topic() else {
            window.push_notification(
                Notification::info("Select one topic of a mind map first."),
                cx,
            );
            return;
        };
        let mut added = None;
        self.update_session(
            |s| {
                added = match kind {
                    NewTopic::Sub => s.add_subtopic(topic),
                    NewTopic::After => s.add_sibling(topic, false),
                    NewTopic::Before => s.add_sibling(topic, true),
                };
                // The new topic shares its map's artboard.
                if let Some(id) = added {
                    s.assign_membership(&[id]);
                }
            },
            cx,
        );
        match added {
            Some(id) => self.select_and_name(id, window, cx),
            None => window.push_notification(
                Notification::info("A summary topic cannot have subtopics."),
                cx,
            ),
        }
    }

    /// Adds a summary of the selected sibling topics and names it.
    pub(super) fn add_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let topics = self.selected_topics();
        let mut added = None;
        self.update_session(
            |s| {
                added = s.add_summary(&topics);
                // The summary shares its map's artboard.
                if let Some(id) = added {
                    s.assign_membership(&[id]);
                }
            },
            cx,
        );
        match added {
            Some(id) => self.select_and_name(id, window, cx),
            None => window.push_notification(
                Notification::info(
                    "Select one or more topics under the same parent (not the central topic).",
                ),
                cx,
            ),
        }
    }

    /// Draws or removes a boundary round the selected topics.
    pub(super) fn toggle_boundary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit_action(window, cx);
        let topics = self.selected_topics();
        if topics.is_empty() {
            window.push_notification(Notification::info("Select the topics to enclose."), cx);
            return;
        }
        self.update_session(
            |s| {
                s.toggle_boundary(&topics);
            },
            cx,
        );
    }

    /// The topic at canvas point `at` a dragged topic `dragged` could move
    /// under: in the same map, not in the dragged topic's own subtree.
    pub(super) fn topic_drop_target(
        &mut self,
        at: Pt,
        dragged: Uuid,
        cx: &mut Context<Self>,
    ) -> Option<Uuid> {
        let page = self.session.page();
        let own = mindweave_core::mindmap::with_descendants(page, &[dragged]);
        // The dragged branch is under the pointer: look beneath it.
        let target = self.shape_at_except(at, &own, cx)?;
        let map_of = |id: Uuid| {
            self.session
                .find_shape(id)
                .and_then(|s| s.topic.as_ref())
                .map(|t| t.map)
        };
        (map_of(target)? == map_of(dragged)?).then_some(target)
    }

    /// Shows the picture in file `path` in topic `id`, its longest side
    /// `longest` (the picture's own size at most).
    pub(super) fn set_topic_picture_file(
        &mut self,
        ids: &[Uuid],
        path: &std::path::Path,
        longest: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let data = std::fs::read(path)
            .ok()
            .and_then(ImageData::from_file_bytes);
        let size = data
            .as_ref()
            .and_then(|d| self.image_cache.natural_size(d, cx));
        let (Some(data), Some(size)) = (data, size) else {
            window.push_notification(
                Notification::warning(
                    "That file is not a picture Mindweave can open, or it is larger than 20 MB.",
                ),
                cx,
            );
            return;
        };
        let shown = fit_topic_picture(size, longest);
        self.update_session(
            |s| {
                s.update_shapes(ids, |shape| {
                    if let Some(t) = shape.topic.as_mut() {
                        t.picture = Some(data.clone());
                        t.picture_size = Some(shown);
                    }
                })
            },
            cx,
        );
    }

    /// Removes the selected topics' picture.
    pub(super) fn clear_topic_picture(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_topics();
        self.update_session(
            |s| {
                s.update_shapes(&ids, |shape| {
                    if let Some(t) = shape.topic.as_mut() {
                        t.picture = None;
                        t.picture_size = None;
                    }
                })
            },
            cx,
        );
    }

    /// Asks for a picture and shows it in the selected topics.
    pub(super) fn choose_topic_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.selected_topics();
        if ids.is_empty() {
            return;
        }
        let picked = rfd::AsyncFileDialog::new()
            .set_title("Topic Image")
            .add_filter("Images", &mindweave_core::ImageFormat::EXTENSIONS)
            .pick_file();
        cx.spawn_in(window, async move |this, cx| {
            let Some(file) = picked.await else {
                return;
            };
            let path = file.path().to_path_buf();
            this.update_in(cx, |this, window, cx| {
                this.set_topic_picture_file(&ids, &path, TOPIC_IMAGE_SIZE, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// The maps (central topic ids) of the selected topics.
    pub(super) fn selected_maps(&self) -> Vec<Uuid> {
        let mut maps: Vec<Uuid> = self
            .selected_topics()
            .iter()
            .filter_map(|id| Some(self.session.find_shape(*id)?.topic.as_ref()?.map))
            .collect();
        maps.sort();
        maps.dedup();
        maps
    }

    /// Turns automatic layout of the selected topics' maps on or off.
    pub(super) fn set_map_auto_layout(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_edit_action(window, cx);
        let maps = self.selected_maps();
        self.update_session(|s| s.set_auto_layout(&maps, on), cx);
    }

    /// Sets the selected topics' link to `link` (cleared when blank).
    pub(super) fn set_topic_link(&mut self, link: &str, cx: &mut Context<Self>) {
        let ids = self.selected_topics();
        let link = link.trim();
        let link = (!link.is_empty()).then(|| normalized_link(link));
        self.update_session(
            |s| {
                s.update_shapes(&ids, |shape| {
                    if let Some(t) = shape.topic.as_mut() {
                        t.link = link.clone();
                    }
                })
            },
            cx,
        );
    }

    /// The link of the topic whose link badge is at canvas point `at`.
    pub(super) fn link_badge_at(&self, at: Pt) -> Option<String> {
        link_badge_at(&self.session, at)
    }

    /// The topic whose note badge is at canvas point `at`.
    pub(super) fn note_badge_at(&self, at: Pt) -> Option<Uuid> {
        note_badge_at(&self.session, at)
    }

    /// The topic whose equation is at canvas point `at`.
    pub(super) fn equation_at(&self, at: Pt) -> Option<Uuid> {
        equation_at(&self.session, at)
    }
}

/// The topic `selection` is, when it is exactly one topic: what the Note
/// and Equation commands work on.
pub(super) fn one_topic(session: &DocumentSession, selection: &[Uuid]) -> Option<Uuid> {
    match selection {
        [id] => session
            .find_shape(*id)
            .filter(|s| s.topic.is_some())
            .map(|s| s.id),
        _ => None,
    }
}

/// A little slack round the small badges, in canvas units.
const BADGE_SLACK: f32 = 3.;

/// The topmost topic on the active layer whose `part` (topic-local),
/// grown by `slack` on every side, holds canvas point `at`.
fn topic_part_at(
    session: &DocumentSession,
    at: Pt,
    part: impl Fn(&TopicParts) -> Option<Rect>,
    slack: f32,
) -> Option<&Shape> {
    session.shapes_on_active_layer().iter().rev().find(|s| {
        let Some(t) = s.topic.as_ref() else {
            return false;
        };
        let Some(r) = part(&t.parts(s.frame.w, s.frame.h)) else {
            return false;
        };
        let (x, y) = (at.0 - s.frame.x, at.1 - s.frame.y);
        x >= r.x - slack && x <= r.x + r.w + slack && y >= r.y - slack && y <= r.y + r.h + slack
    })
}

/// The link of the topic whose link badge is at canvas point `at`.
fn link_badge_at(session: &DocumentSession, at: Pt) -> Option<String> {
    topic_part_at(session, at, |p| p.badge, BADGE_SLACK)?
        .topic
        .as_ref()?
        .link
        .clone()
}

/// The topic whose note badge is at canvas point `at`.
fn note_badge_at(session: &DocumentSession, at: Pt) -> Option<Uuid> {
    topic_part_at(session, at, |p| p.note, BADGE_SLACK).map(|s| s.id)
}

/// The topic whose equation is at canvas point `at`.
fn equation_at(session: &DocumentSession, at: Pt) -> Option<Uuid> {
    topic_part_at(session, at, |p| p.equation, 0.).map(|s| s.id)
}

/// `link` as a URL to open: as typed when it names a scheme, else a web
/// address.
pub fn normalized_link(link: &str) -> String {
    if link.contains("://") || link.starts_with("mailto:") || link.starts_with('/') {
        link.to_string()
    } else {
        format!("https://{link}")
    }
}

#[cfg(test)]
mod tests {
    use super::{equation_at, link_badge_at, normalized_link, note_badge_at, one_topic};
    use mindweave_core::geometry::Rect;
    use mindweave_core::mindmap::Equation;
    use mindweave_core::{DocumentSession, HAlign, RichText, Run, RunStyle, Shape};

    #[test]
    fn note_and_equation_need_exactly_one_topic() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("o.mindweave")).unwrap();
        let a = s.add_central_topic((0., 0.));
        let b = s.add_central_topic((400., 0.));
        let text_box = Shape::new_text_box((0., 300.));
        let other = text_box.id;
        s.add_shapes(vec![text_box]);
        assert_eq!(one_topic(&s, &[a]), Some(a));
        assert_eq!(one_topic(&s, &[]), None);
        assert_eq!(one_topic(&s, &[a, b]), None);
        assert_eq!(one_topic(&s, &[other]), None);
        assert_eq!(one_topic(&s, &[uuid::Uuid::new_v4()]), None);
    }

    #[test]
    fn links_without_a_scheme_are_web_addresses() {
        assert_eq!(normalized_link("example.com"), "https://example.com");
        assert_eq!(normalized_link("http://x.org"), "http://x.org");
        assert_eq!(normalized_link("mailto:a@b.c"), "mailto:a@b.c");
        assert_eq!(normalized_link("/Users/me/notes.md"), "/Users/me/notes.md");
    }

    /// The centre of topic-local `r` on a topic at `(x, y)`.
    fn centre((x, y): (f32, f32), r: Rect) -> (f32, f32) {
        (x + r.x + r.w / 2., y + r.y + r.h / 2.)
    }

    #[test]
    fn note_badge_and_equation_are_hit() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DocumentSession::create(dir.path().join("n.mindweave")).unwrap();
        let id = s.add_central_topic((0., 0.));
        let mut note = RichText::new(RunStyle::default(), HAlign::default());
        note.paragraphs[0].runs.push(Run {
            text: "A note".into(),
            style: RunStyle::default(),
        });
        s.update_shapes(&[id], |shape| {
            let t = shape.topic.as_mut().unwrap();
            t.note = Some(note.clone());
            t.link = Some("https://example.com".into());
            t.equation = Some(Equation {
                latex: "x^2".into(),
                size: (40., 20.),
            });
        });
        let shape = s.find_shape(id).unwrap();
        let t = shape.topic.as_ref().unwrap();
        let (w, h) = t.size_for_text((80., 20.));
        s.set_topic_size(id, w, h);
        let shape = s.find_shape(id).unwrap();
        let at = (shape.frame.x, shape.frame.y);
        let parts = shape.topic.as_ref().unwrap().parts(w, h);

        let note = centre(at, parts.note.unwrap());
        assert_eq!(note_badge_at(&s, note), Some(id));
        assert_eq!(link_badge_at(&s, note), None);
        let link = centre(at, parts.badge.unwrap());
        assert_eq!(
            link_badge_at(&s, link).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(note_badge_at(&s, link), None);
        let equation = centre(at, parts.equation.unwrap());
        assert_eq!(equation_at(&s, equation), Some(id));
        assert_eq!(note_badge_at(&s, equation), None);

        let text = (
            at.0 + parts.text.x + 10.,
            at.1 + parts.text.y + parts.text.h / 2.,
        );
        assert_eq!(note_badge_at(&s, text), None);
        assert_eq!(link_badge_at(&s, text), None);
        assert_eq!(equation_at(&s, text), None);
    }
}
