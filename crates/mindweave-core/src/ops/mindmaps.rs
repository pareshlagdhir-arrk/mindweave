//! Mind maps: maps, topics, notes, equations, links, tasks, summaries,
//! boundaries and relationships.

use serde::Deserialize;
use uuid::Uuid;

use super::read::{kind_of, ObjectKind};
use super::{
    check_batch, check_number, check_text, notes, parse_name, OpError, OpResult, Workspace,
};
use crate::color::AUTO_COLOR;
use crate::geometry::Frame;
use crate::line::{
    line_frame, ArrowHead, LineDash, LineProps, LineRoute, LINE_LIBRARY, LINE_SHAPE,
};
use crate::mindmap::{BranchStyle, Structure};
use crate::session::DocumentSession;
use crate::shape::{Label, Shape};
use crate::task::{is_date, Task, TaskStatus};
use crate::text::{RichText, RunStyle};
use crate::transform::frame_aabb;

/// Space left between a new mind map and what is already on the page.
const NEW_MAP_GAP: f32 = 200.0;
/// Most topics one tree may hold.
const MAX_TREE: usize = 500;
/// Deepest tree accepted.
const MAX_DEPTH: usize = 32;

/// Where [`Workspace::add_topics`] puts new topics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TopicPosition {
    /// Under the topic, after its last child.
    #[default]
    Child,
    /// Beside the topic, after it (in the order given).
    After,
    /// Beside the topic, before it (in the order given).
    Before,
}

/// A topic with its subtree, for building a map in one call.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TopicNode {
    pub text: String,
    /// A note, in simple Markdown (see [`notes`]).
    pub note: Option<String>,
    /// LaTeX shown below the text.
    pub equation: Option<String>,
    pub link: Option<String>,
    /// Makes the topic a task.
    pub task: Option<TaskEdit>,
    pub children: Vec<TopicNode>,
}

/// Changes to a topic's task; `None` leaves a value as it is. Any change
/// makes a plain topic a task; `remove` makes it a plain topic again.
/// An empty description or date clears it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TaskEdit {
    pub remove: bool,
    /// Simple Markdown, as a note takes.
    pub description: Option<String>,
    /// `YYYY-MM-DD`.
    pub start: Option<String>,
    /// `YYYY-MM-DD`.
    pub end: Option<String>,
    /// 0 to 100; ignored by parent tasks, which roll up their subtasks'.
    pub progress: Option<u8>,
    /// `not-started`, `in-progress`, `blocked` or `done`.
    pub status: Option<String>,
}

impl TaskEdit {
    /// `task` (a new one when `None`) with this edit made, or `None` to
    /// remove it; refuses bad dates, progress or status.
    fn apply(&self, task: Option<Task>) -> OpResult<Option<Task>> {
        if self.remove {
            return Ok(None);
        }
        let mut task = task.unwrap_or_default();
        if let Some(d) = &self.description {
            check_text("task description", d)?;
            task.description = Some(notes::rich_text_from_markdown(d, &RunStyle::default()))
                .filter(|t: &RichText| !t.plain().trim().is_empty());
        }
        let date = |what: &str, d: &Option<String>, now: Option<String>| match d.as_deref() {
            None => Ok(now),
            Some(d) if d.trim().is_empty() => Ok(None),
            Some(d) if is_date(d.trim()) => Ok(Some(d.trim().to_string())),
            Some(d) => Err(OpError::invalid(format!(
                "The task's {what} date \"{d}\" is not a date written YYYY-MM-DD."
            ))),
        };
        task.start = date("start", &self.start, task.start.take())?;
        task.end = date("end", &self.end, task.end.take())?;
        if let (Some(s), Some(e)) = (&task.start, &task.end) {
            if s > e {
                return Err(OpError::invalid(
                    "The task's start date is after its end date.",
                ));
            }
        }
        if let Some(p) = self.progress {
            if p > 100 {
                return Err(OpError::invalid("A task's progress is 0 to 100."));
            }
            task.set_progress(p);
        }
        if let Some(s) = &self.status {
            let status = TaskStatus::from_key(s).ok_or_else(|| {
                OpError::invalid(format!(
                    "Unknown task status \"{s}\"; use not-started, in-progress, blocked or done."
                ))
            })?;
            match self.progress {
                // Both given: keep the progress unless the status fixes it.
                Some(_) => task.status = status,
                None => task.set_status(status),
            }
        }
        Ok(Some(task.normalized()))
    }
}

/// Changes to a topic; `None` leaves a property as it is, and an empty
/// note, equation or link removes it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TopicEdit {
    pub text: Option<String>,
    /// Simple Markdown: lines, `- ` bullets, `1. ` items, `**bold**`,
    /// `*italic*`.
    pub note: Option<String>,
    pub equation: Option<String>,
    pub link: Option<String>,
    pub task: Option<TaskEdit>,
}

/// A web or file link as the app stores it: without a scheme it is a
/// web address.
fn normalized_link(link: &str) -> String {
    if link.contains("://") || link.starts_with("mailto:") || link.starts_with('/') {
        link.to_string()
    } else {
        format!("https://{link}")
    }
}

/// Replaces the text of `shape`'s label with `text`, keeping its style
/// and alignment.
fn replace_label_text(shape: &mut Shape, text: &str) {
    let label = shape.label.get_or_insert_with(Label::new_shape_label);
    let style = label.text.style_at(0);
    let align = label
        .text
        .paragraphs
        .first()
        .map(|p| p.align)
        .unwrap_or_default();
    label.text = notes::rich_text_plain(text, &style, align);
}

impl Workspace {
    /// The topic `id` with its page and layer; `what` names the command.
    pub(super) fn topic_at(&self, id: Uuid, what: &str) -> OpResult<(usize, usize)> {
        let (p, l, s) = self.locate(id)?;
        if s.topic.is_none() {
            return Err(OpError::WrongKind {
                id,
                kind: kind_of(s).name(),
                expected: format!("{what} needs a mind map topic"),
            });
        }
        Ok((p, l))
    }

    /// Where a new map goes on page `page`: right of everything there,
    /// level with its top, else at the origin.
    fn new_map_point(&self, page: usize) -> (f32, f32) {
        let frames: Vec<Frame> = self.doc().pages[page]
            .layers
            .iter()
            .flat_map(|l| &l.objects)
            .map(|s| s.frame)
            .collect();
        match crate::transform::selection_bounds(&frames) {
            Some(r) => (r.x + r.w + NEW_MAP_GAP + 100.0, r.y + 100.0),
            None => (0.0, 0.0),
        }
    }

    /// Adds a mind map whose central topic says `central_text`, centred
    /// on `at` (else right of what is on the page) of page `page`, in
    /// `structure` (`map`, `logic` or `org`). Returns the central topic's
    /// id, which is also the map's id.
    pub fn add_mind_map(
        &mut self,
        page: Option<usize>,
        central_text: &str,
        at: Option<(f32, f32)>,
        structure: Option<&str>,
    ) -> OpResult<Uuid> {
        check_text("central topic text", central_text)?;
        let page = self.page_index(page)?;
        let structure: Option<Structure> = structure
            .map(|s| parse_name("structure", s, &["map", "logic", "org"]))
            .transpose()?;
        let at = match at {
            Some((x, y)) => (check_number("x", x)?, check_number("y", y)?),
            None => self.new_map_point(page),
        };
        let id = self.on_layer(page, None, |s| {
            s.begin_gesture();
            let id = s.add_central_topic(at);
            s.assign_membership(&[id]);
            s.update_shapes(&[id], |shape| {
                replace_label_text(shape, central_text);
                if let (Some(t), Some(st)) = (shape.topic.as_mut(), structure) {
                    t.structure = st;
                }
            });
            s.end_gesture();
            id
        });
        self.fit(page, &[id]);
        Ok(id)
    }

    /// Adds topics saying `texts` at `position` relative to topic
    /// `topic`, in the order given; returns their ids. Beside the central
    /// topic (which has no siblings) they become its children.
    pub fn add_topics(
        &mut self,
        topic: Uuid,
        texts: &[String],
        position: TopicPosition,
    ) -> OpResult<Vec<Uuid>> {
        check_batch("topic", texts.len())?;
        for t in texts {
            check_text("topic text", t)?;
        }
        let (page, layer) = self.topic_at(topic, "add_topics")?;
        let summary = self.shape(topic)?.topic.as_ref().and_then(|t| t.summary);
        if summary.is_some() && position == TopicPosition::Child {
            return Err(OpError::invalid(
                "A summary topic cannot have subtopics; add them to a regular topic.",
            ));
        }
        let ids = self.on_layer(page, Some(layer), |s| {
            s.begin_gesture();
            let mut ids = Vec::new();
            let mut prev = topic;
            for text in texts {
                let id = match position {
                    TopicPosition::Child => s.add_subtopic(topic),
                    TopicPosition::After => s.add_sibling(prev, false),
                    TopicPosition::Before => s.add_sibling(topic, true),
                };
                let Some(id) = id else { break };
                s.assign_membership(&[id]);
                s.update_shapes(&[id], |shape| replace_label_text(shape, text));
                ids.push(id);
                prev = id;
            }
            s.end_gesture();
            ids
        });
        if ids.len() != texts.len() {
            return Err(OpError::invalid(
                "Some topics could not be added: the topic must be a regular topic of a map.",
            ));
        }
        self.fit(page, &ids);
        Ok(ids)
    }

    /// Adds `nodes` (each with its subtree, notes, equations and links)
    /// under topic `parent`; returns every new topic's id, depth first in
    /// the order given.
    pub fn add_tree(&mut self, parent: Uuid, nodes: &[TopicNode]) -> OpResult<Vec<Uuid>> {
        let count = count_nodes(nodes, 0)?;
        if count > MAX_TREE {
            return Err(OpError::invalid(format!(
                "At most {MAX_TREE} topics per call; build the map in several calls."
            )));
        }
        check_batch("topic", count)?;
        let mut out = Vec::new();
        self.add_subtree(parent, nodes, &mut out)?;
        Ok(out)
    }

    /// Adds `nodes` under `parent`, recursively, collecting new ids.
    fn add_subtree(
        &mut self,
        parent: Uuid,
        nodes: &[TopicNode],
        out: &mut Vec<Uuid>,
    ) -> OpResult<()> {
        let texts: Vec<String> = nodes.iter().map(|n| n.text.clone()).collect();
        let ids = self.add_topics(parent, &texts, TopicPosition::Child)?;
        for (id, node) in ids.into_iter().zip(nodes) {
            out.push(id);
            if node.note.is_some()
                || node.equation.is_some()
                || node.link.is_some()
                || node.task.is_some()
            {
                self.set_topic(
                    id,
                    &TopicEdit {
                        text: None,
                        note: node.note.clone(),
                        equation: node.equation.clone(),
                        link: node.link.clone(),
                        task: node.task.clone(),
                    },
                )?;
            }
            if !node.children.is_empty() {
                self.add_subtree(id, &node.children, out)?;
            }
        }
        Ok(())
    }

    /// Changes topic `id`'s text, note, equation, link or task (see
    /// [`TopicEdit`]) as one step.
    pub fn set_topic(&mut self, id: Uuid, edit: &TopicEdit) -> OpResult<()> {
        let (page, layer) = self.topic_at(id, "set_topic")?;
        for (what, v) in [
            ("text", &edit.text),
            ("note", &edit.note),
            ("equation", &edit.equation),
            ("link", &edit.link),
        ] {
            if let Some(v) = v {
                check_text(what, v)?;
            }
        }
        let note = edit.note.as_ref().map(|n| {
            Some(notes::rich_text_from_markdown(n, &RunStyle::default()))
                .filter(|t: &RichText| !t.plain().trim().is_empty())
        });
        let link = edit.link.as_ref().map(|l| {
            let l = l.trim();
            (!l.is_empty()).then(|| normalized_link(l))
        });
        let task = match &edit.task {
            Some(t) => {
                let now = self.shape(id)?.topic.as_ref().and_then(|t| t.task.clone());
                Some(t.apply(now)?)
            }
            None => None,
        };
        self.on_layer(page, Some(layer), |s| {
            s.begin_gesture();
            if let Some(text) = &edit.text {
                s.update_shapes(&[id], |shape| replace_label_text(shape, text));
            }
            if let Some(note) = note {
                s.set_topic_note(id, note);
            }
            if let Some(eq) = &edit.equation {
                s.set_topic_equation(id, Some(eq.clone()));
            }
            if let Some(link) = link {
                s.update_shapes(&[id], |shape| {
                    if let Some(t) = shape.topic.as_mut() {
                        t.link = link.clone();
                    }
                });
            }
            if let Some(task) = task {
                s.update_shapes(&[id], |shape| {
                    if let Some(t) = shape.topic.as_mut() {
                        t.task = task.clone();
                    }
                });
            }
            s.end_gesture();
        });
        self.fit(page, &[id]);
        Ok(())
    }

    /// Sets map `map`'s structure (`map`, `logic` or `org`) and, when
    /// given, its branch style (`tapered`, `curve`, `elbow`, `straight`).
    pub fn set_structure(
        &mut self,
        map: Uuid,
        structure: Option<&str>,
        branch: Option<&str>,
    ) -> OpResult<()> {
        let (page, layer) = self.topic_at(map, "set_structure")?;
        let central = self
            .shape(map)?
            .topic
            .as_ref()
            .is_some_and(|t| t.is_central());
        if !central {
            return Err(OpError::invalid(
                "set_structure needs the map's central topic id (topic.map in get_object).",
            ));
        }
        let structure: Option<Structure> = structure
            .map(|s| parse_name("structure", s, &["map", "logic", "org"]))
            .transpose()?;
        let branch: Option<BranchStyle> = branch
            .map(|b| {
                parse_name(
                    "branch style",
                    b,
                    &["tapered", "curve", "elbow", "straight"],
                )
            })
            .transpose()?;
        self.on_layer(page, Some(layer), |s| {
            s.update_shapes(&[map], |shape| {
                if let Some(t) = shape.topic.as_mut() {
                    if let Some(st) = structure {
                        t.structure = st;
                    }
                    if let Some(b) = branch {
                        t.branch = Some(b);
                    }
                }
            })
        });
        Ok(())
    }

    /// Adds a summary of sibling topics `topics` saying `text`; returns
    /// its id.
    pub fn add_summary(&mut self, topics: &[Uuid], text: Option<&str>) -> OpResult<Uuid> {
        check_batch("topic", topics.len())?;
        if let Some(t) = text {
            check_text("summary text", t)?;
        }
        let (page, layer) = self.common_layer(topics)?;
        for t in topics {
            self.topic_at(*t, "add_summary")?;
        }
        let id = self.on_layer(page, Some(layer), |s| {
            s.begin_gesture();
            let id = s.add_summary(topics);
            if let Some(id) = id {
                s.assign_membership(&[id]);
                if let Some(text) = text {
                    s.update_shapes(&[id], |shape| replace_label_text(shape, text));
                }
            }
            s.end_gesture();
            id
        });
        let id = id.ok_or_else(|| {
            OpError::invalid(
                "A summary needs topics with the same parent, none of them a central or summary topic.",
            )
        })?;
        self.fit(page, &[id]);
        Ok(id)
    }

    /// Draws a boundary round topics `topics` (with their subtrees);
    /// returns its id. A boundary round exactly these topics is returned
    /// as it is.
    pub fn add_boundary(&mut self, topics: &[Uuid]) -> OpResult<Uuid> {
        check_batch("topic", topics.len())?;
        let (page, layer) = self.common_layer(topics)?;
        for t in topics {
            self.topic_at(*t, "add_boundary")?;
        }
        let wanted: std::collections::HashSet<Uuid> = topics.iter().copied().collect();
        let existing = self.doc().pages[page].layers[layer]
            .objects
            .iter()
            .find(|s| {
                s.boundary.as_ref().is_some_and(|b| {
                    b.members
                        .iter()
                        .copied()
                        .collect::<std::collections::HashSet<_>>()
                        == wanted
                })
            })
            .map(|s| s.id);
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = self.on_layer(page, Some(layer), |s| {
            s.begin_gesture();
            let id = s.toggle_boundary(topics);
            if let Some(id) = id {
                s.assign_membership(&[id]);
            }
            s.end_gesture();
            id
        });
        id.ok_or_else(|| OpError::invalid("The boundary could not be drawn round these topics."))
    }

    /// Draws a relationship (a curved, dashed arrow) from topic `from` to
    /// topic `to`, labelled `text`; returns its id.
    pub fn add_relationship(&mut self, from: Uuid, to: Uuid, text: Option<&str>) -> OpResult<Uuid> {
        if from == to {
            return Err(OpError::invalid(
                "A relationship joins two different topics.",
            ));
        }
        if let Some(t) = text {
            check_text("relationship text", t)?;
        }
        self.topic_at(from, "add_relationship")?;
        self.topic_at(to, "add_relationship")?;
        let (page, layer) = self.common_layer(&[from, to])?;
        let centre =
            |id: Uuid| -> OpResult<(f32, f32)> { Ok(frame_aabb(&self.shape(id)?.frame).center()) };
        let (a, b) = (centre(from)?, centre(to)?);
        let shape = relationship_shape(self, a, b, from, to, text)?;
        let id = shape.id;
        self.on_layer(page, Some(layer), |s| add_line(s, shape));
        Ok(id)
    }
}

/// A new relationship line from `a` to `b`, attached to `from` and `to`.
fn relationship_shape(
    ws: &Workspace,
    a: (f32, f32),
    b: (f32, f32),
    from: Uuid,
    to: Uuid,
    text: Option<&str>,
) -> OpResult<Shape> {
    let def = ws
        .libraries()
        .shape(LINE_LIBRARY, LINE_SHAPE)
        .ok_or_else(|| OpError::invalid("The Basic Shapes library is missing."))?;
    let mut shape = Shape::new(def, LINE_LIBRARY, line_frame(a, b));
    shape.style.stroke = Some(AUTO_COLOR.to_string());
    shape.style.stroke_width = 1.5;
    shape.style.dash = LineDash::Dashed;
    shape.line = Some(LineProps {
        route: LineRoute::Curved,
        end_head: ArrowHead::Arrow,
        relationship: true,
        start: Some(from),
        end: Some(to),
        ..LineProps::default()
    });
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        replace_label_text(&mut shape, text);
    }
    Ok(shape)
}

/// Adds line `shape` with its membership, in one step.
pub(super) fn add_line(s: &mut DocumentSession, shape: Shape) {
    super::add_with_membership(s, vec![shape]);
}

/// The number of topics in `nodes`, checking depth.
fn count_nodes(nodes: &[TopicNode], depth: usize) -> OpResult<usize> {
    if depth > MAX_DEPTH {
        return Err(OpError::invalid(format!(
            "The tree is deeper than {MAX_DEPTH} levels."
        )));
    }
    let mut n = nodes.len();
    for node in nodes {
        n += count_nodes(&node.children, depth + 1)?;
    }
    Ok(n)
}

/// Whether `kind` is a topic kind.
pub(super) fn is_topic_kind(kind: ObjectKind) -> bool {
    matches!(kind, ObjectKind::Topic | ObjectKind::Summary)
}
