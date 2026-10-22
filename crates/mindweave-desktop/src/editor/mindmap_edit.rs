//! Mind map commands in the editor: adding central topics, subtopics,
//! siblings and summaries, boundaries, moving a topic by dragging it onto
//! another, and a topic's icon, image and link.

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use mindweave_core::geometry::Pt;
use mindweave_core::{fit_topic_picture, ImageData, TOPIC_IMAGE_SIZE};
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
        match self.selection.as_slice() {
            [id] => self
                .session
                .find_shape(*id)
                .filter(|s| s.topic.is_some())
                .map(|s| s.id),
            _ => None,
        }
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
        self.update_session(|s| id = Some(s.add_central_topic(at)), cx);
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
        self.update_session(|s| added = s.add_summary(&topics), cx);
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
        self.session
            .shapes_on_active_layer()
            .iter()
            .rev()
            .find_map(|s| {
                let t = s.topic.as_ref()?;
                let link = t.link.clone()?;
                let badge = t.parts(s.frame.w, s.frame.h).badge?;
                let (x, y) = (at.0 - s.frame.x, at.1 - s.frame.y);
                // A little slack round the small badge.
                let slack = 3.;
                (x >= badge.x - slack
                    && x <= badge.x + badge.w + slack
                    && y >= badge.y - slack
                    && y <= badge.y + badge.h + slack)
                    .then_some(link)
            })
    }
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
    use super::normalized_link;

    #[test]
    fn links_without_a_scheme_are_web_addresses() {
        assert_eq!(normalized_link("example.com"), "https://example.com");
        assert_eq!(normalized_link("http://x.org"), "http://x.org");
        assert_eq!(normalized_link("mailto:a@b.c"), "mailto:a@b.c");
        assert_eq!(normalized_link("/Users/me/notes.md"), "/Users/me/notes.md");
    }
}
