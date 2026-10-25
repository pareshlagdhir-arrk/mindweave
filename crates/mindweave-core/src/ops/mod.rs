//! The command layer: high-level, validated edits of a `.mindweave` file
//! for agents, tools and scripts (the MCP server calls these).
//!
//! A [`Workspace`] is an open document with the libraries its shapes come
//! from. Every command checks its input and returns an [`OpError`] whose
//! message says what to do instead; commands that create objects return
//! their ids. Edits go through [`DocumentSession`], so they behave as in
//! the app: mind maps are laid out again, lines follow the shapes they are
//! attached to, and new objects placed inside an artboard join it.
//!
//! Without the app's fonts, text boxes and topics are sized with an
//! estimate (see [`crate::text_measure`]); the app refits them when it
//! opens the file.

mod artboards;
mod diagrams;
mod docs;
mod error;
mod mindmaps;
mod notes;
mod pictures;
mod read;
mod validate;

use std::path::Path;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::document::Document;
use crate::library::LibraryRegistry;
use crate::mindmap::TOPIC_TEXT_MAX;
use crate::session::DocumentSession;
use crate::shape::Shape;
use crate::text_measure::measure;

pub use artboards::{artboard_presets, ArtboardPresetInfo, ArtboardSize};
pub use diagrams::{
    list_libraries, ArrangeHow, ConnectOptions, LibraryInfo, LibraryItemInfo, StyleInput, TextInput,
};
pub use docs::{DocumentSummary, LayerSummary, PageSummary};
pub use error::{OpError, OpResult};
pub use mindmaps::{TaskEdit, TopicEdit, TopicNode, TopicPosition};
pub use notes::{rich_text_from_markdown, rich_text_plain};
pub use pictures::Picture;
pub use read::{
    FrameInfo, LineInfo, ObjectFilter, ObjectInfo, ObjectKind, StyleInfo, TaskInfo, TextStyleInfo,
    TopicInfo,
};
pub use validate::{validate, Problem, Severity};

/// Padding between a text box's edge and its text (as the app draws it).
pub const TEXT_BOX_PADDING: f32 = 4.0;
/// Most characters accepted for one text, label or note.
pub const MAX_TEXT_CHARS: usize = 10_000;
/// Most objects one command may create or change.
pub const MAX_BATCH: usize = 500;
/// Largest coordinate accepted.
pub const MAX_COORD: f32 = 1e6;

/// Measures a topic equation (LaTeX source, font size) as `(w, h)` in
/// canvas units; `None` when it does not render.
pub type EquationMeasure = Box<dyn Fn(&str, f32) -> Option<(f32, f32)> + Send + Sync>;

/// An open document with its libraries.
pub struct Workspace {
    session: DocumentSession,
    libs: Arc<LibraryRegistry>,
    equation: Option<EquationMeasure>,
}

impl Workspace {
    /// Wraps `session`, whose shapes come from `libs`.
    fn with_session(mut session: DocumentSession, libs: Arc<LibraryRegistry>) -> Workspace {
        session.set_libraries(libs.clone());
        Workspace {
            session,
            libs,
            equation: None,
        }
    }

    /// Opens the document at `path`.
    pub fn open(path: &Path, libs: Arc<LibraryRegistry>) -> OpResult<Workspace> {
        let session = DocumentSession::open(path.to_path_buf())?;
        Ok(Workspace::with_session(session, libs))
    }

    /// Sets how topic equations are measured, so topics make room for
    /// them; without it an equation adds no size until the app opens the
    /// file.
    pub fn set_equation_measure(&mut self, measure: EquationMeasure) {
        self.equation = Some(measure);
    }

    /// Saves the document to its file (written to a temporary file, then
    /// renamed over it).
    pub fn save(&mut self) -> OpResult<()> {
        Ok(self.session.save()?)
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.session.is_dirty()
    }

    /// The document.
    pub fn doc(&self) -> &Document {
        self.session.doc()
    }

    /// The problems of the document (see [`validate`]).
    pub fn validate(&self) -> Vec<Problem> {
        validate(self.doc(), Some(&self.libs))
    }

    /// The libraries shapes come from.
    pub fn libraries(&self) -> &LibraryRegistry {
        &self.libs
    }

    /// The page, layer and shape with `id`.
    fn locate(&self, id: Uuid) -> OpResult<(usize, usize, &Shape)> {
        self.doc()
            .pages
            .iter()
            .enumerate()
            .find_map(|(p, page)| {
                page.layers.iter().enumerate().find_map(|(l, layer)| {
                    layer.objects.iter().find(|s| s.id == id).map(|s| (p, l, s))
                })
            })
            .ok_or(OpError::NotFound(id))
    }

    /// The shape with `id`.
    fn shape(&self, id: Uuid) -> OpResult<&Shape> {
        self.locate(id).map(|(_, _, s)| s)
    }

    /// The page and layer all of `ids` are on: they must share one.
    fn common_layer(&self, ids: &[Uuid]) -> OpResult<(usize, usize)> {
        let mut at = None;
        for id in ids {
            let (p, l, _) = self.locate(*id)?;
            match at {
                None => at = Some((p, l)),
                Some(prev) if prev != (p, l) => {
                    return Err(OpError::invalid(
                        "These objects are on different pages or layers; pass objects of one layer at a time.",
                    ))
                }
                Some(_) => {}
            }
        }
        at.ok_or_else(|| OpError::invalid("Pass at least one object id."))
    }

    /// The index of `page`, or the active page; errors on an unknown index.
    fn page_index(&self, page: Option<usize>) -> OpResult<usize> {
        let n = self.doc().pages.len();
        match page {
            None => Ok(self.doc().active_page),
            Some(p) if p < n => Ok(p),
            Some(p) => Err(OpError::invalid(format!(
                "There is no page {p}; pages are numbered 0 to {}.",
                n - 1
            ))),
        }
    }

    /// Runs `f` with page `page` and its layer `layer` (its active layer
    /// when `None`) active, then makes the previously active page and its
    /// layer active again: commands leave the view state as it was.
    fn on_layer<R>(
        &mut self,
        page: usize,
        layer: Option<usize>,
        f: impl FnOnce(&mut DocumentSession) -> R,
    ) -> R {
        let doc = self.session.doc();
        let (old_page, old_layer) = (doc.active_page, doc.pages[page].active_layer);
        self.session.set_active_page(page);
        if let Some(l) = layer {
            self.session.set_active_layer(l);
        }
        let out = f(&mut self.session);
        self.session.set_active_page(page);
        self.session.set_active_layer(old_layer);
        self.session.set_active_page(old_page);
        out
    }

    /// Sizes text boxes and topics among `ids` (on `page`) to their text
    /// with the estimate of [`crate::text_measure`], measuring equations
    /// first when a measure is set; maps are laid out again.
    fn fit(&mut self, page: usize, ids: &[Uuid]) {
        let wanted: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
        let shapes: Vec<Shape> = self.doc().pages[page]
            .layers
            .iter()
            .flat_map(|l| &l.objects)
            .filter(|s| wanted.contains(&s.id))
            .cloned()
            .collect();
        let equation = self.equation.as_ref();
        let mut eq_sizes = Vec::new();
        let mut boxes = Vec::new();
        let mut topics = Vec::new();
        for s in &shapes {
            let Some(label) = &s.label else { continue };
            if s.is_text_box() {
                let wrap = (!s.auto_width()).then(|| (s.frame.w - 2.0 * TEXT_BOX_PADDING).max(1.0));
                let (w, h) = measure(&label.text, wrap);
                boxes.push((s.id, w + 2.0 * TEXT_BOX_PADDING, h + 2.0 * TEXT_BOX_PADDING));
            } else if let Some(t) = &s.topic {
                let mut t = t.clone();
                if let (Some(eq), Some(m)) = (t.equation.as_mut(), equation) {
                    let size = m(&eq.latex, max_font_size(&label.text)).unwrap_or((0.0, 0.0));
                    eq.size = size;
                    eq_sizes.push((s.id, size));
                }
                let (mut tw, mut th) = measure(&label.text, None);
                if tw > TOPIC_TEXT_MAX {
                    (tw, th) = measure(&label.text, Some(TOPIC_TEXT_MAX));
                }
                let (w, h) = t.size_for_text((tw, th));
                topics.push((s.id, w, h));
            }
        }
        self.on_layer(page, None, |s| {
            for (id, size) in eq_sizes {
                s.set_topic_equation_size(id, size);
            }
            for (id, w, h) in boxes {
                s.set_text_box_layout(id, w, h);
            }
            for (id, w, h) in topics {
                s.set_topic_size(id, w, h);
            }
        });
    }
}

/// The largest font size in `text` (the default size when empty).
fn max_font_size(text: &crate::text::RichText) -> f32 {
    text.paragraphs
        .iter()
        .flat_map(|p| p.runs.iter().map(|r| r.style.size).chain([p.style.size]))
        .fold(0.0_f32, f32::max)
        .max(1.0)
}

/// Parses `value` as one of the names of `T` (its serde names), for
/// `what`; the error lists `allowed`.
fn parse_name<T: DeserializeOwned>(what: &str, value: &str, allowed: &[&str]) -> OpResult<T> {
    serde_json::from_value(serde_json::Value::String(value.trim().to_lowercase())).map_err(|_| {
        OpError::invalid(format!(
            "Unknown {what} `{value}`; use one of: {}.",
            allowed.join(", ")
        ))
    })
}

/// Checks a text argument's length.
fn check_text(what: &str, text: &str) -> OpResult<()> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(OpError::invalid(format!(
            "The {what} is longer than {MAX_TEXT_CHARS} characters; shorten it."
        )));
    }
    Ok(())
}

/// Checks a coordinate or size argument.
fn check_number(what: &str, v: f32) -> OpResult<f32> {
    if v.is_finite() && v.abs() <= MAX_COORD {
        Ok(v)
    } else {
        Err(OpError::invalid(format!(
            "{what} must be a number between -{MAX_COORD} and {MAX_COORD}."
        )))
    }
}

/// Checks a batch size.
fn check_batch(what: &str, n: usize) -> OpResult<()> {
    if n == 0 {
        return Err(OpError::invalid(format!("Pass at least one {what}.")));
    }
    if n > MAX_BATCH {
        return Err(OpError::invalid(format!(
            "At most {MAX_BATCH} {what}s per call; split the work into several calls."
        )));
    }
    Ok(())
}

/// The shapes among `ids` (of a layer's `shapes`) whose artboard
/// membership placing them decides: not artboards, nor members of an
/// artboard among `ids` (they travel with it).
fn membership_targets(shapes: &[Shape], ids: &[Uuid]) -> Vec<Uuid> {
    let picked: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
    let boards: std::collections::HashSet<Uuid> = shapes
        .iter()
        .filter(|s| s.is_artboard() && picked.contains(&s.id))
        .map(|s| s.id)
        .collect();
    shapes
        .iter()
        .filter(|s| picked.contains(&s.id) && !s.is_artboard())
        .filter(|s| !s.artboard.is_some_and(|a| boards.contains(&a)))
        .map(|s| s.id)
        .collect()
}

/// Adds `shapes` to the active layer and gives them their artboard
/// membership in the same step (as the app places shapes).
fn add_with_membership(session: &mut DocumentSession, shapes: Vec<Shape>) {
    let ids: Vec<Uuid> = shapes.iter().map(|s| s.id).collect();
    let targets = membership_targets(&shapes, &ids);
    session.add_shapes(shapes);
    if !targets.is_empty() {
        session.assign_membership(&targets);
    }
}

/// Gives the shapes `ids` on the active layer their membership after a
/// move or resize; call inside the gesture of that edit.
fn settle_membership(session: &mut DocumentSession, ids: &[Uuid]) {
    let targets = membership_targets(session.shapes_on_active_layer(), ids);
    if !targets.is_empty() {
        session.assign_membership(&targets);
    }
}

#[cfg(test)]
mod tests;
