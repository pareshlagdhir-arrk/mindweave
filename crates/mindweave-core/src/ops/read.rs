//! Reading a document: objects with their kind, frame, text, style and
//! links to other objects.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{OpResult, Workspace};
use crate::geometry::line_endpoints;
use crate::line::LineDash;
use crate::mindmap::Structure;
use crate::shape::Shape;
use crate::task::{task_views, TaskStatus};
use crate::text::{HAlign, RichText};

/// Longest text shown per object in a listing; [`Workspace::get_object`]
/// shows it whole.
const LIST_TEXT_CHARS: usize = 200;

/// What an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    /// A library shape (or a template part).
    Shape,
    /// A free text box.
    Text,
    Line,
    /// A mind map relationship line.
    Relationship,
    /// A mind map topic (the central topic too).
    Topic,
    /// A mind map summary topic.
    Summary,
    /// A mind map boundary.
    Boundary,
    Artboard,
    Image,
    /// A shape with its own outline (an imported SVG path).
    Path,
}

impl ObjectKind {
    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            ObjectKind::Shape => "shape",
            ObjectKind::Text => "text",
            ObjectKind::Line => "line",
            ObjectKind::Relationship => "relationship",
            ObjectKind::Topic => "topic",
            ObjectKind::Summary => "summary",
            ObjectKind::Boundary => "boundary",
            ObjectKind::Artboard => "artboard",
            ObjectKind::Image => "image",
            ObjectKind::Path => "path",
        }
    }
}

/// The kind of `shape`.
pub fn kind_of(shape: &Shape) -> ObjectKind {
    if let Some(t) = &shape.topic {
        return if t.summary.is_some() {
            ObjectKind::Summary
        } else {
            ObjectKind::Topic
        };
    }
    if let Some(l) = &shape.line {
        return if l.relationship {
            ObjectKind::Relationship
        } else {
            ObjectKind::Line
        };
    }
    if shape.is_boundary() {
        ObjectKind::Boundary
    } else if shape.is_artboard() {
        ObjectKind::Artboard
    } else if shape.is_text_box() {
        ObjectKind::Text
    } else if shape.is_image() {
        ObjectKind::Image
    } else if shape.is_path() {
        ObjectKind::Path
    } else {
        ObjectKind::Shape
    }
}

/// Where an object is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FrameInfo {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(skip_serializing_if = "is_zero")]
    pub rotation: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// How an object is painted.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StyleInfo {
    /// `#RRGGBB`, `auto`, `ink`, or `none`.
    pub fill: String,
    pub stroke: String,
    pub stroke_width: f32,
    pub opacity: f32,
    pub dash: LineDash,
}

/// How an object's text looks (from its first character).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextStyleInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    /// `#RRGGBB`, or `auto` (contrasting with what is behind).
    pub color: String,
    pub align: HAlign,
}

/// What makes an object a topic.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TopicInfo {
    /// The map's central topic.
    pub map: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Uuid>,
    /// Child topics in order (summaries not included).
    pub children: Vec<Uuid>,
    /// Summary topics of this topic's children.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub summaries: Vec<Uuid>,
    /// For a summary: the first and last topic it sums up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_of: Option<[Uuid; 2]>,
    /// On a central topic: the map's structure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<Structure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskInfo>,
}

/// A topic's task. For a parent task (one with subtasks) progress,
/// status and dates are rolled up from its subtasks.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskInfo {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    pub progress: u8,
    pub status: TaskStatus,
    /// How many subtasks it has.
    #[serde(skip_serializing_if = "no_subtasks")]
    pub subtasks: usize,
}

fn no_subtasks(n: &usize) -> bool {
    *n == 0
}

/// What makes an object a line.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LineInfo {
    /// Where it starts and ends.
    pub from: (f32, f32),
    pub to: (f32, f32),
    /// The objects its ends are attached to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Uuid>,
    pub start_head: crate::line::ArrowHead,
    pub end_head: crate::line::ArrowHead,
    pub route: crate::line::LineRoute,
}

/// One object of a document.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ObjectInfo {
    pub id: Uuid,
    pub kind: ObjectKind,
    pub page: usize,
    pub layer: usize,
    /// Library and shape id, for library shapes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    pub frame: FrameInfo,
    /// The text, plain (paragraphs separated by `\n`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// An artboard's name, or a shape's own name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub style: StyleInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_style: Option<TextStyleInfo>,
    /// Groups, outermost first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Uuid>,
    /// The artboard the object belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artboard: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<TopicInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<LineInfo>,
    /// A boundary's topics, or an artboard's members.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<Uuid>>,
    /// Shape parameters (such as `corner_radius`), in full detail only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Which objects [`Workspace::list_objects`] returns; every given
/// condition must hold.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ObjectFilter {
    /// Only this kind.
    pub kind: Option<ObjectKind>,
    /// Only objects whose text or name contains this (any case).
    pub text_contains: Option<String>,
    /// Only members of this artboard.
    pub artboard: Option<Uuid>,
    /// Only objects of this mind map (central topic id): topics,
    /// boundaries and relationships.
    pub mind_map: Option<Uuid>,
}

/// A style colour as reported: `none` for no colour.
fn color_name(c: &Option<String>) -> String {
    c.clone().unwrap_or_else(|| "none".to_string())
}

/// `s` cut to `max` characters, with an ellipsis when cut.
fn shorten(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// The text style of `text` at its start.
fn text_style(text: &RichText) -> TextStyleInfo {
    let s = text.style_at(0);
    TextStyleInfo {
        font: s.font,
        size: s.size,
        bold: s.bold,
        italic: s.italic,
        color: s.color.unwrap_or_else(|| "auto".to_string()),
        align: text.paragraphs.first().map(|p| p.align).unwrap_or_default(),
    }
}

/// The map a shape belongs to, by `maps` (topic id → map id).
fn map_of(shape: &Shape, maps: &HashMap<Uuid, Uuid>) -> Option<Uuid> {
    if let Some(t) = &shape.topic {
        return Some(t.map);
    }
    if let Some(b) = &shape.boundary {
        return b.members.iter().find_map(|m| maps.get(m).copied());
    }
    let l = shape.line.as_ref().filter(|l| l.relationship)?;
    [l.start, l.end]
        .into_iter()
        .flatten()
        .find_map(|e| maps.get(&e).copied())
}

impl Workspace {
    /// The objects of page `page` (the active page when `None`), bottom
    /// layer first and back to front, that pass `filter`. Texts and notes
    /// are shortened unless `full`.
    pub fn list_objects(
        &self,
        page: Option<usize>,
        filter: &ObjectFilter,
        full: bool,
    ) -> OpResult<Vec<ObjectInfo>> {
        let page = self.page_index(page)?;
        let p = &self.doc().pages[page];
        let maps: HashMap<Uuid, Uuid> = p
            .layers
            .iter()
            .flat_map(|l| &l.objects)
            .filter_map(|s| Some((s.id, s.topic.as_ref()?.map)))
            .collect();
        let needle = filter.text_contains.as_ref().map(|t| t.to_lowercase());
        let mut out = Vec::new();
        for (li, layer) in p.layers.iter().enumerate().rev() {
            for s in &layer.objects {
                if filter.kind.is_some_and(|k| k != kind_of(s))
                    || filter.artboard.is_some_and(|a| s.artboard != Some(a))
                    || filter.mind_map.is_some_and(|m| map_of(s, &maps) != Some(m))
                {
                    continue;
                }
                if let Some(needle) = &needle {
                    let text = s.label.as_ref().map(|l| l.text.plain()).unwrap_or_default();
                    let name = s.name.clone().unwrap_or_default();
                    if !text.to_lowercase().contains(needle)
                        && !name.to_lowercase().contains(needle)
                    {
                        continue;
                    }
                }
                out.push(self.info(page, li, s, full));
            }
        }
        Ok(out)
    }

    /// Everything about object `id`.
    pub fn get_object(&self, id: Uuid) -> OpResult<ObjectInfo> {
        let (page, layer, s) = self.locate(id)?;
        Ok(self.info(page, layer, s, true))
    }

    /// The report on shape `s` (on `page`, layer `layer`).
    fn info(&self, page: usize, layer: usize, s: &Shape, full: bool) -> ObjectInfo {
        let kind = kind_of(s);
        let max = if full { usize::MAX } else { LIST_TEXT_CHARS };
        let shapes = || {
            self.doc().pages[page]
                .layers
                .iter()
                .flat_map(|l| &l.objects)
        };
        let library = (kind == ObjectKind::Shape).then(|| s.library.clone());
        let topic = s.topic.as_ref().map(|t| {
            let mut kids: Vec<(f64, Uuid, bool)> = shapes()
                .filter_map(|c| {
                    let ct = c.topic.as_ref()?;
                    (ct.parent == Some(s.id)).then_some((ct.order, c.id, ct.summary.is_some()))
                })
                .collect();
            kids.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            TopicInfo {
                map: t.map,
                parent: t.parent,
                children: kids.iter().filter(|k| !k.2).map(|k| k.1).collect(),
                summaries: kids.iter().filter(|k| k.2).map(|k| k.1).collect(),
                summary_of: t.summary.map(|(a, b)| [a, b]),
                structure: t.is_central().then_some(t.structure),
                note: t.note.as_ref().map(|n| shorten(n.plain(), max)),
                equation: t.equation.as_ref().map(|e| e.latex.clone()),
                link: t.link.clone(),
                task: t.task.as_ref().and_then(|task| {
                    let view = task_views(&self.doc().pages[page]).remove(&s.id)?;
                    Some(TaskInfo {
                        description: shorten(task.description.clone(), max),
                        start: view.start,
                        end: view.end,
                        progress: view.progress,
                        status: view.status,
                        subtasks: view.subtasks,
                    })
                }),
            }
        });
        let line = s.line.as_ref().map(|l| {
            let (from, to) = line_endpoints(&s.frame);
            LineInfo {
                from,
                to,
                start: l.start,
                end: l.end,
                start_head: l.start_head,
                end_head: l.end_head,
                route: l.route,
            }
        });
        let members = if let Some(b) = &s.boundary {
            Some(b.members.clone())
        } else if s.is_artboard() {
            Some(
                shapes()
                    .filter(|m| m.artboard == Some(s.id))
                    .map(|m| m.id)
                    .collect(),
            )
        } else {
            None
        };
        let f = &s.frame;
        ObjectInfo {
            id: s.id,
            kind,
            page,
            layer,
            shape: library.as_ref().map(|_| s.shape.clone()),
            library,
            frame: FrameInfo {
                x: f.x,
                y: f.y,
                w: f.w,
                h: f.h,
                rotation: f.rotation,
            },
            text: s
                .label
                .as_ref()
                .map(|l| shorten(l.text.plain(), max))
                .filter(|t| !t.is_empty()),
            name: s.name.clone(),
            style: StyleInfo {
                fill: color_name(&s.style.fill),
                stroke: color_name(&s.style.stroke),
                stroke_width: s.style.stroke_width,
                opacity: s.style.opacity,
                dash: s.style.dash,
            },
            text_style: s
                .label
                .as_ref()
                .filter(|l| full || !l.text.is_empty())
                .map(|l| text_style(&l.text)),
            groups: s.groups.clone(),
            artboard: s.artboard,
            topic,
            line,
            members,
            params: (full && !s.params.is_empty()).then(|| {
                s.params
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            }),
        }
    }
}
