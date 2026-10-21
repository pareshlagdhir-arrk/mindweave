//! An open document: edit operations, undo history and dirty tracking.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;

use uuid::Uuid;

use crate::color::normalize_hex_color;
use crate::document::{
    BackgroundPattern, CanvasSettings, Document, Layer, Page, Viewport, DEFAULT_DOT_SIZE,
};
use crate::format::{self, FormatError};
use crate::geometry::Frame;
use crate::history::{History, HISTORY_CAP};
use crate::shape::{Label, Shape};
pub use crate::viewport::{ZOOM_MAX, ZOOM_MIN};

pub const GRID_MIN: f32 = 4.0;
pub const GRID_MAX: f32 = 200.0;
pub const MAJOR_EVERY_MAX: u32 = 20;
pub const DOT_SIZE_MIN: f32 = 1.0;
pub const DOT_SIZE_MAX: f32 = 6.0;
/// Largest viewport offset kept on open, in canvas pixels; beyond it grid
/// indices and f32 precision break down.
pub const VIEWPORT_COORD_MAX: f32 = 1e9;

const DEFAULT_GRID: f32 = 20.0;

/// A document bound to a file path, with undo history and unsaved-change
/// tracking.
///
/// View state (each page's viewport and active layer, and the document's
/// active page) is saved with the file but is neither in the undo history
/// nor counted as an unsaved change.
///
/// Each content state carries a revision number so `is_dirty` is cheap:
/// the revision at the last save means clean without comparing pages.
#[derive(Debug)]
pub struct DocumentSession {
    doc: Document,
    pub path: PathBuf,
    /// Page content at the last save or open.
    saved: Vec<Page>,
    /// Revision of the current content; undo and redo restore it.
    revision: u64,
    /// Revision of the content at the last save or open.
    saved_revision: u64,
    /// Last revision handed out; new content never reuses a number.
    last_revision: u64,
    /// `(revision, dirty)` from the last page comparison.
    dirty_cache: Cell<Option<(u64, bool)>>,
    history: History<(u64, Vec<Page>)>,
    /// Revision and page content when the open gesture began.
    gesture: Option<(u64, Vec<Page>)>,
    /// Bumped whenever a text box's laid-out size changes.
    layout_generation: u64,
}

/// Whether `a` and `b` have the same content, ignoring view state.
fn content_eq(a: &[Page], b: &[Page]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| page_content_eq(a, b))
}

fn page_content_eq(a: &Page, b: &Page) -> bool {
    // Destructured so a new `Page` field must be classified here.
    let Page {
        id,
        name,
        canvas,
        viewport: _,
        layers,
        active_layer: _,
    } = a;
    *id == b.id
        && *name == b.name
        && *canvas == b.canvas
        && layers.len() == b.layers.len()
        && layers
            .iter()
            .zip(&b.layers)
            .all(|(a, b)| layer_content_eq(a, b))
}

fn layer_content_eq(a: &Layer, b: &Layer) -> bool {
    // Destructured so a new `Layer` field must be classified here.
    let Layer {
        id,
        name,
        visible,
        objects,
    } = a;
    *id == b.id
        && *name == b.name
        && *visible == b.visible
        && objects.len() == b.objects.len()
        && objects
            .iter()
            .zip(&b.objects)
            .all(|(a, b)| shape_content_eq(a, b))
}

/// Moves `f`, resized from `before` about its centre, so its unflipped
/// local top-left corner stays where it was on the canvas: a rotated text
/// box keeps its left edge in place while its size follows its text.
fn keep_top_left(before: &Frame, f: &mut Frame) {
    // Unrotated, or the same size: x and y already hold, exactly.
    if f.rotation == 0.0 || (f.w == before.w && f.h == before.h) {
        return;
    }
    let corner = top_left(before);
    let offset = corner_offset(f);
    f.x = corner.0 - offset.0;
    f.y = corner.1 - offset.1;
}

/// Canvas position of `f`'s unflipped local top-left corner.
fn top_left(f: &Frame) -> (f32, f32) {
    let offset = corner_offset(f);
    (f.x + offset.0, f.y + offset.1)
}

/// From `(f.x, f.y)` to the canvas position of `f`'s unflipped local
/// top-left corner (rotation is about the frame's centre).
fn corner_offset(f: &Frame) -> (f32, f32) {
    let (sin, cos) = f.rotation.to_radians().sin_cos();
    let (dx, dy) = (-f.w / 2.0, -f.h / 2.0);
    (
        f.w / 2.0 + dx * cos - dy * sin,
        f.h / 2.0 + dx * sin + dy * cos,
    )
}

/// Whether two text-box frames place the box at the same spot: the same
/// `x`/`y` unrotated; rotated, the same top-left corner (which layout
/// keeps fixed) up to float rounding.
fn same_text_box_position(a: &Frame, b: &Frame) -> bool {
    if a.rotation != b.rotation {
        return false;
    }
    if a.rotation == 0.0 {
        return (a.x, a.y) == (b.x, b.y);
    }
    let (pa, pb) = (top_left(a), top_left(b));
    let near = |u: f32, v: f32| (u - v).abs() <= 1e-4 * u.abs().max(v.abs()).max(1.0);
    near(pa.0, pb.0) && near(pa.1, pb.1)
}

/// Shape equality ignoring the sizes a text box derives from its text
/// layout: `frame.h` always, `frame.w` while it grows with its text, and
/// the `x`/`y` shift that keeps a rotated box's top-left corner in place.
fn shape_content_eq(a: &Shape, b: &Shape) -> bool {
    if !(a.is_text_box() && b.is_text_box()) || a.auto_width() != b.auto_width() {
        return a == b;
    }
    // Destructured so a new `Shape` field must be classified here.
    let Shape {
        id,
        library,
        shape,
        frame,
        style,
        params,
        name,
        locked,
        label,
        text_box,
        extra,
    } = a;
    let auto = a.auto_width();
    let content = |f: &Frame| Frame {
        x: 0.0,
        y: 0.0,
        h: 0.0,
        w: if auto { 0.0 } else { f.w },
        ..*f
    };
    *id == b.id
        && content(frame) == content(&b.frame)
        && same_text_box_position(frame, &b.frame)
        && *library == b.library
        && *shape == b.shape
        && *style == b.style
        && *params == b.params
        && *name == b.name
        && *locked == b.locked
        && *label == b.label
        && *text_box == b.text_box
        && *extra == b.extra
}

/// Replace out-of-range or non-finite values with usable ones.
fn normalize_page(page: &mut Page) {
    let g = page.canvas.grid_size;
    page.canvas.grid_size = if g.is_finite() {
        g.clamp(GRID_MIN, GRID_MAX)
    } else {
        DEFAULT_GRID
    };
    let c = &mut page.canvas;
    c.major_every = c.major_every.min(MAJOR_EVERY_MAX);
    c.dot_size = if c.dot_size.is_finite() {
        clamp_dot_size(c.dot_size)
    } else {
        DEFAULT_DOT_SIZE
    };
    for color in [&mut c.background, &mut c.line_color, &mut c.dot_color] {
        *color = color.as_deref().and_then(normalize_hex_color);
    }
    page.layers
        .iter_mut()
        .flat_map(|l| l.objects.iter_mut())
        .for_each(|s| s.normalize());
    let v = &mut page.viewport;
    v.zoom = if v.zoom.is_finite() {
        v.zoom.clamp(ZOOM_MIN, ZOOM_MAX)
    } else {
        1.0
    };
    for c in [&mut v.x, &mut v.y] {
        *c = if c.is_finite() {
            c.clamp(-VIEWPORT_COORD_MAX, VIEWPORT_COORD_MAX)
        } else {
            0.0
        };
    }
}

/// Rounds to the nearest half pixel and clamps to
/// `DOT_SIZE_MIN..=DOT_SIZE_MAX`.
fn clamp_dot_size(px: f32) -> f32 {
    ((px * 2.0).round() / 2.0).clamp(DOT_SIZE_MIN, DOT_SIZE_MAX)
}

/// `value` normalised (`None` is the theme colour), or `None` when it is
/// unparsable.
fn color_value(value: Option<&str>) -> Option<Option<String>> {
    match value {
        Some(text) => normalize_hex_color(text).map(Some),
        None => Some(None),
    }
}

/// Smallest `N >= 1` such that `"{prefix} N"` is not among `names`.
fn next_name<'a>(prefix: &str, names: impl Iterator<Item = &'a str>) -> String {
    let used: Vec<&str> = names.collect();
    let n = (1..)
        .find(|n| !used.contains(&format!("{prefix} {n}").as_str()))
        .unwrap_or(1);
    format!("{prefix} {n}")
}

impl DocumentSession {
    /// Create a new document at `path`, saved immediately and clean.
    pub fn create(path: PathBuf) -> Result<Self, FormatError> {
        let doc = Document::new();
        format::save(&doc, &path)?;
        Ok(Self::with_doc(doc, path))
    }

    pub fn open(path: PathBuf) -> Result<Self, FormatError> {
        let mut doc = format::load(&path)?;
        doc.pages.iter_mut().for_each(normalize_page);
        Ok(Self::with_doc(doc, path))
    }

    fn with_doc(doc: Document, path: PathBuf) -> Self {
        Self {
            saved: doc.pages.clone(),
            doc,
            path,
            revision: 0,
            saved_revision: 0,
            last_revision: 0,
            dirty_cache: Cell::new(None),
            history: History::new(HISTORY_CAP),
            gesture: None,
            layout_generation: 0,
        }
    }

    pub fn save(&mut self) -> Result<(), FormatError> {
        format::save(&self.doc, &self.path)?;
        self.saved = self.doc.pages.clone();
        self.saved_revision = self.revision;
        self.dirty_cache.set(None);
        Ok(())
    }

    /// Whether the content differs from the last save, ignoring view state.
    ///
    /// Free at the saved revision; otherwise the pages are compared at
    /// most once per revision (an edit can restore the saved content).
    pub fn is_dirty(&self) -> bool {
        if self.revision == self.saved_revision {
            return false;
        }
        if let Some((rev, dirty)) = self.dirty_cache.get() {
            if rev == self.revision {
                return dirty;
            }
        }
        let dirty = !content_eq(&self.doc.pages, &self.saved);
        self.dirty_cache.set(Some((self.revision, dirty)));
        dirty
    }

    /// File stem, followed by `" •"` when there are unsaved changes.
    pub fn title(&self) -> String {
        let stem = self
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        if self.is_dirty() {
            format!("{stem} •")
        } else {
            stem
        }
    }

    /// A number naming the current content: it changes with every content
    /// edit, undo and redo, never with view changes. Equal revisions mean
    /// equal content apart from text-box layout sizes (see
    /// [`Self::layout_generation`]). Cheap to read, so callers can skip
    /// work when nothing changed.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// A number that changes whenever [`Self::set_text_box_layout`] changes
    /// a text box's size, which [`Self::revision`] does not track. Callers
    /// showing sizes compare both.
    pub fn layout_generation(&self) -> u64 {
        self.layout_generation
    }

    /// The document. Edits go through the session so they are recorded
    /// for undo and dirty tracking.
    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// The active page.
    pub fn page(&self) -> &Page {
        &self.doc.pages[self.doc.active_page]
    }

    fn page_mut(&mut self) -> &mut Page {
        &mut self.doc.pages[self.doc.active_page]
    }

    // Undo history.

    /// Records the current content as an undo step before a content edit
    /// and gives the edited content a new revision. Callers check first
    /// that the edit changes something. Inside a gesture nothing is
    /// recorded until `end_gesture`.
    fn checkpoint(&mut self) {
        if self.gesture.is_none() {
            self.history.record((self.revision, self.doc.pages.clone()));
        }
        self.last_revision += 1;
        self.revision = self.last_revision;
    }

    /// Starts grouping content edits into one undo step. A nested begin is
    /// a no-op.
    pub fn begin_gesture(&mut self) {
        if self.gesture.is_none() {
            self.gesture = Some((self.revision, self.doc.pages.clone()));
        }
    }

    /// Ends the open gesture, recording one undo step if the content
    /// changed. Without an open gesture this does nothing.
    pub fn end_gesture(&mut self) {
        if let Some((rev, before)) = self.gesture.take() {
            if content_eq(&before, &self.doc.pages) {
                self.revision = rev;
            } else {
                self.history.record((rev, before));
            }
        }
    }

    /// Undoes the last content step (ending any open gesture first).
    /// Returns whether anything was undone.
    pub fn undo(&mut self) -> bool {
        self.end_gesture();
        if !self.history.can_undo() {
            return false;
        }
        let current = (self.revision, self.doc.pages.clone());
        match self.history.undo(current) {
            Some((rev, pages)) => {
                self.revision = rev;
                self.restore(pages);
                true
            }
            None => false,
        }
    }

    /// Redoes the last undone step (ending any open gesture first).
    /// Returns whether anything was redone.
    pub fn redo(&mut self) -> bool {
        self.end_gesture();
        if !self.history.can_redo() {
            return false;
        }
        let current = (self.revision, self.doc.pages.clone());
        match self.history.redo(current) {
            Some((rev, pages)) => {
                self.revision = rev;
                self.restore(pages);
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo() || self.gesture_changed()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo() && !self.gesture_changed()
    }

    fn gesture_changed(&self) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|(_, before)| !content_eq(before, &self.doc.pages))
    }

    /// Replaces the page content with `pages`, keeping the current view
    /// state of pages that still exist (matched by id): the viewport, and
    /// the active layer if it still exists (else its index, clamped). The
    /// active page is kept by id too, else its index is clamped.
    fn restore(&mut self, mut pages: Vec<Page>) {
        let active_id = self.page().id;
        for page in &mut pages {
            if let Some(cur) = self.doc.pages.iter().find(|p| p.id == page.id) {
                page.viewport = cur.viewport;
                let layer_id = cur.layers.get(cur.active_layer).map(|l| l.id);
                page.active_layer = page
                    .layers
                    .iter()
                    .position(|l| Some(l.id) == layer_id)
                    .unwrap_or(cur.active_layer);
            }
            page.active_layer = page.active_layer.min(page.layers.len().saturating_sub(1));
        }
        self.doc.active_page = pages
            .iter()
            .position(|p| p.id == active_id)
            .unwrap_or(self.doc.active_page)
            .min(pages.len().saturating_sub(1));
        self.doc.pages = pages;
    }

    // Content edits (recorded in history only when they change something).

    pub fn add_page(&mut self) -> usize {
        self.checkpoint();
        let name = next_name("Page", self.doc.pages.iter().map(|p| p.name.as_str()));
        self.doc.pages.push(Page::new(name));
        self.doc.active_page = self.doc.pages.len() - 1;
        self.doc.active_page
    }

    pub fn rename_page(&mut self, idx: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.doc.pages.get(idx).is_none_or(|p| p.name == name) {
            return;
        }
        self.checkpoint();
        self.doc.pages[idx].name = name.to_string();
    }

    /// Insert a new layer at the top of the active page and make it active.
    pub fn add_layer(&mut self) -> usize {
        self.checkpoint();
        let page = self.page_mut();
        let name = next_name("Layer", page.layers.iter().map(|l| l.name.as_str()));
        page.layers.insert(0, Layer::new(name));
        page.active_layer = 0;
        0
    }

    pub fn rename_layer(&mut self, idx: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.page().layers.get(idx).is_none_or(|l| l.name == name) {
            return;
        }
        self.checkpoint();
        self.page_mut().layers[idx].name = name.to_string();
    }

    pub fn toggle_layer_visibility(&mut self, idx: usize) {
        if idx >= self.page().layers.len() {
            return;
        }
        self.checkpoint();
        let layer = &mut self.page_mut().layers[idx];
        layer.visible = !layer.visible;
    }

    /// Sets one canvas setting of the active page, recording a step only
    /// when the value changes.
    fn set_canvas<T: PartialEq>(&mut self, value: T, field: fn(&mut CanvasSettings) -> &mut T) {
        if *field(&mut self.page_mut().canvas) == value {
            return;
        }
        self.checkpoint();
        *field(&mut self.page_mut().canvas) = value;
    }

    pub fn set_pattern(&mut self, p: BackgroundPattern) {
        self.set_canvas(p, |c| &mut c.pattern);
    }

    /// Round to whole pixels and clamp to `GRID_MIN..=GRID_MAX`; NaN is ignored.
    pub fn set_grid_size(&mut self, px: f32) {
        if px.is_nan() {
            return;
        }
        self.set_canvas(px.round().clamp(GRID_MIN, GRID_MAX), |c| &mut c.grid_size);
    }

    /// Canvas fill colour (`#RRGGBB`); `None` uses the theme background.
    /// An unparsable colour is ignored.
    pub fn set_background(&mut self, color: Option<&str>) {
        if let Some(v) = color_value(color) {
            self.set_canvas(v, |c| &mut c.background);
        }
    }

    /// Lines colour (`#RRGGBB`); `None` uses the theme border colour.
    /// An unparsable colour is ignored.
    pub fn set_line_color(&mut self, color: Option<&str>) {
        if let Some(v) = color_value(color) {
            self.set_canvas(v, |c| &mut c.line_color);
        }
    }

    /// Dots colour (`#RRGGBB`); `None` uses the theme border colour.
    /// An unparsable colour is ignored.
    pub fn set_dot_color(&mut self, color: Option<&str>) {
        if let Some(v) = color_value(color) {
            self.set_canvas(v, |c| &mut c.dot_color);
        }
    }

    /// Every `n`th grid line is drawn stronger (0 = off), clamped to
    /// `MAJOR_EVERY_MAX`.
    pub fn set_major_every(&mut self, n: u32) {
        self.set_canvas(n.min(MAJOR_EVERY_MAX), |c| &mut c.major_every);
    }

    /// Round to half pixels and clamp to `DOT_SIZE_MIN..=DOT_SIZE_MAX`; NaN
    /// is ignored.
    pub fn set_dot_size(&mut self, px: f32) {
        if px.is_nan() {
            return;
        }
        self.set_canvas(clamp_dot_size(px), |c| &mut c.dot_size);
    }

    // Shapes.

    fn active_layer(&self) -> &Layer {
        let page = self.page();
        &page.layers[page.active_layer]
    }

    fn active_layer_mut(&mut self) -> &mut Layer {
        let page = self.page_mut();
        let idx = page.active_layer;
        &mut page.layers[idx]
    }

    /// Shapes of the active layer, back to front.
    pub fn shapes_on_active_layer(&self) -> &[Shape] {
        &self.active_layer().objects
    }

    /// The shape `id` on the active layer.
    pub fn find_shape(&self, id: Uuid) -> Option<&Shape> {
        self.shapes_on_active_layer().iter().find(|s| s.id == id)
    }

    /// Whether shapes on the active layer can be selected (it is visible).
    pub fn active_layer_selectable(&self) -> bool {
        self.active_layer().visible
    }

    /// Appends `shapes`, normalised, to the top of the active layer.
    pub fn add_shapes(&mut self, mut shapes: Vec<Shape>) {
        if shapes.is_empty() {
            return;
        }
        shapes.iter_mut().for_each(Shape::normalize);
        self.checkpoint();
        self.active_layer_mut().objects.extend(shapes);
    }

    /// Removes the shapes with `ids` from every layer of the active page
    /// and returns them, back to front.
    pub fn remove_shapes(&mut self, ids: &[Uuid]) -> Vec<Shape> {
        let ids: HashSet<Uuid> = ids.iter().copied().collect();
        let found = self
            .page()
            .layers
            .iter()
            .any(|l| l.objects.iter().any(|s| ids.contains(&s.id)));
        if !found {
            return Vec::new();
        }
        self.checkpoint();
        let mut removed = Vec::new();
        // Layers are stored top first; walk bottom first.
        for layer in self.page_mut().layers.iter_mut().rev() {
            let (out, keep) = std::mem::take(&mut layer.objects)
                .into_iter()
                .partition(|s| ids.contains(&s.id));
            layer.objects = keep;
            removed.extend::<Vec<Shape>>(out);
        }
        removed
    }

    /// Applies `f` to each shape with one of `ids` on the active layer and
    /// normalises the result, recording a step only if some shape changed.
    pub fn update_shapes(&mut self, ids: &[Uuid], mut f: impl FnMut(&mut Shape)) {
        let ids: HashSet<Uuid> = ids.iter().copied().collect();
        let changed: Vec<(usize, Shape)> = self
            .shapes_on_active_layer()
            .iter()
            .enumerate()
            .filter(|(_, s)| ids.contains(&s.id))
            .filter_map(|(i, s)| {
                let mut new = s.clone();
                f(&mut new);
                new.normalize();
                (new != *s).then_some((i, new))
            })
            .collect();
        if changed.is_empty() {
            return;
        }
        self.checkpoint();
        let objects = &mut self.active_layer_mut().objects;
        for (i, shape) in changed {
            objects[i] = shape;
        }
    }

    /// Sets or clears the label of shape `id` on the active layer as one
    /// undo step; nothing is recorded when the label is unchanged.
    pub fn set_label(&mut self, id: Uuid, label: Option<Label>) {
        self.update_shapes(&[id], |s| s.label = label.clone());
    }

    /// Stores the size text box `id` (on the active page) derives from its
    /// text layout: the height, and the width while it grows with its
    /// text. This is not an edit: no undo step, no new revision, and the
    /// dirty comparison ignores these sizes. Non-finite values, unknown
    /// ids and shapes that are not text boxes are ignored.
    pub fn set_text_box_layout(&mut self, id: Uuid, w: f32, h: f32) {
        if !(w.is_finite() && h.is_finite()) {
            return;
        }
        let shape = self
            .page_mut()
            .layers
            .iter_mut()
            .flat_map(|l| l.objects.iter_mut())
            .find(|s| s.id == id && s.is_text_box());
        if let Some(shape) = shape {
            let before = shape.frame;
            if shape.auto_width() {
                shape.frame.w = w;
            }
            shape.frame.h = h;
            shape.normalize();
            keep_top_left(&before, &mut shape.frame);
            if shape.frame != before {
                self.layout_generation += 1;
            }
        }
    }

    /// Removes shape `id` from the active page without an undo step of its
    /// own: inside an open gesture it joins the gesture, so removing an
    /// empty text box added in the same gesture leaves no step and the
    /// gesture's starting revision. Without a gesture it is an ordinary,
    /// recorded removal.
    pub fn remove_shape_silently(&mut self, id: Uuid) {
        self.remove_shapes(&[id]);
    }

    // View edits (never dirty).

    pub fn set_active_page(&mut self, idx: usize) {
        if idx < self.doc.pages.len() {
            self.doc.active_page = idx;
        }
    }

    pub fn set_active_layer(&mut self, idx: usize) {
        let page = self.page_mut();
        if idx < page.layers.len() {
            page.active_layer = idx;
        }
    }

    /// Sets the active page's viewport. A viewport with a non-finite value
    /// is ignored: it would not survive a save and reload.
    pub fn set_viewport(&mut self, v: Viewport) {
        if !(v.x.is_finite() && v.y.is_finite() && v.zoom.is_finite()) {
            return;
        }
        self.page_mut().viewport = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{BackgroundPattern, Viewport};

    fn new_session(name: &str) -> (tempfile::TempDir, DocumentSession) {
        let dir = tempfile::tempdir().unwrap();
        let s = DocumentSession::create(dir.path().join(name)).unwrap();
        (dir, s)
    }

    #[test]
    fn a_gesture_put_back_as_it_was_leaves_no_undo_step() {
        use crate::geometry::Frame;
        use crate::library::{builtin_libraries, LibraryRegistry};
        let (_dir, mut s) = new_session("cancel.mindweave");
        let libs = LibraryRegistry::new(builtin_libraries(), None).0;
        let def = libs.shape("basic-shapes", "rectangle").unwrap();
        let frame = Frame {
            w: 10.,
            h: 10.,
            ..Frame::default()
        };
        let shape = Shape::new(def, "basic-shapes", frame);
        let id = shape.id;
        s.add_shapes(vec![shape]);
        s.save().unwrap();
        let can_undo = s.can_undo();
        let original = s.find_shape(id).unwrap().frame;

        s.begin_gesture();
        s.update_shapes(&[id], |sh| sh.frame.x += 25.);
        assert!(s.is_dirty());
        s.update_shapes(&[id], |sh| sh.frame = original);
        s.end_gesture();

        assert_eq!(s.can_undo(), can_undo);
        assert!(!s.is_dirty());
        assert_eq!(s.find_shape(id).unwrap().frame, original);
    }

    #[test]
    fn create_writes_file_and_is_clean() {
        let (_d, s) = new_session("a.mindweave");
        assert!(s.path.exists());
        assert!(!s.is_dirty());
    }

    #[test]
    fn add_page_names_and_activates() {
        let (_d, mut s) = new_session("a.mindweave");
        assert_eq!(s.add_page(), 1);
        assert_eq!(s.doc.pages[1].name, "Page 2");
        assert_eq!(s.doc.active_page, 1);
        assert!(s.is_dirty());
        s.rename_page(1, "Ideas");
        assert_eq!(s.add_page(), 2);
        assert_eq!(s.doc.pages[2].name, "Page 2");
    }

    #[test]
    fn add_layer_goes_on_top_and_activates() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_active_layer(0);
        assert_eq!(s.add_layer(), 0);
        assert_eq!(s.page().layers[0].name, "Layer 2");
        assert_eq!(s.page().layers[1].name, "Layer 1");
        assert_eq!(s.page().active_layer, 0);
        assert!(s.is_dirty());
    }

    #[test]
    fn rename_to_blank_is_ignored() {
        let (_d, mut s) = new_session("a.mindweave");
        s.rename_page(0, "   ");
        s.rename_layer(0, "");
        assert_eq!(s.page().name, "Page 1");
        assert_eq!(s.page().layers[0].name, "Layer 1");
        assert!(!s.is_dirty());
        s.rename_page(0, "  Plan  ");
        assert_eq!(s.page().name, "Plan");
        assert!(s.is_dirty());
    }

    #[test]
    fn out_of_range_indices_are_noops() {
        let (_d, mut s) = new_session("a.mindweave");
        s.rename_page(5, "x");
        s.rename_layer(5, "x");
        s.toggle_layer_visibility(5);
        s.set_active_page(5);
        s.set_active_layer(5);
        assert!(!s.is_dirty());
        assert_eq!(s.doc.active_page, 0);
    }

    #[test]
    fn toggle_visibility_marks_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        s.toggle_layer_visibility(0);
        assert!(!s.page().layers[0].visible);
        assert!(s.is_dirty());
    }

    #[test]
    fn pattern_and_same_value_dirty_rules() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_pattern(BackgroundPattern::Dots);
        assert!(!s.is_dirty());
        s.set_pattern(BackgroundPattern::Lines);
        assert_eq!(s.page().canvas.pattern, BackgroundPattern::Lines);
        assert!(s.is_dirty());
    }

    #[test]
    fn grid_size_is_clamped() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_grid_size(0.0);
        assert_eq!(s.page().canvas.grid_size, 4.0);
        s.set_grid_size(-5.0);
        assert_eq!(s.page().canvas.grid_size, 4.0);
        s.set_grid_size(999.0);
        assert_eq!(s.page().canvas.grid_size, 200.0);
        s.set_grid_size(12.6);
        assert_eq!(s.page().canvas.grid_size, 13.0);
        s.set_grid_size(f32::NAN);
        assert_eq!(s.page().canvas.grid_size, 13.0);
    }

    #[test]
    fn same_grid_size_is_not_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_grid_size(20.0);
        assert!(!s.is_dirty());
        s.set_grid_size(f32::NAN);
        assert!(!s.is_dirty());
    }

    #[test]
    fn new_pattern_settings_have_theme_defaults() {
        let c = crate::document::CanvasSettings::default();
        assert_eq!(c.background, None);
        assert_eq!(c.line_color, None);
        assert_eq!(c.major_every, 0);
        assert_eq!(c.dot_size, 1.5);
        assert_eq!(c.dot_color, None);
    }

    #[test]
    fn colours_are_normalized_and_mark_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_background(Some("ff8000"));
        s.set_line_color(Some("#abcdef"));
        s.set_dot_color(Some("#123456"));
        let c = &s.page().canvas;
        assert_eq!(c.background.as_deref(), Some("#FF8000"));
        assert_eq!(c.line_color.as_deref(), Some("#ABCDEF"));
        assert_eq!(c.dot_color.as_deref(), Some("#123456"));
        assert!(s.is_dirty());
    }

    #[test]
    fn invalid_or_same_colour_is_ignored_and_not_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_background(Some("nope"));
        s.set_line_color(Some("#12345"));
        s.set_dot_color(Some(""));
        s.set_background(None);
        assert_eq!(s.page().canvas, crate::document::CanvasSettings::default());
        assert!(!s.is_dirty());
        s.set_background(Some("#FF8000"));
        s.save().unwrap();
        // Same colour in another spelling.
        s.set_background(Some("ff8000"));
        s.set_background(Some("garbage"));
        assert_eq!(s.page().canvas.background.as_deref(), Some("#FF8000"));
        assert!(!s.is_dirty());
    }

    #[test]
    fn resetting_a_colour_to_theme_marks_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_dot_color(Some("#123456"));
        s.save().unwrap();
        s.set_dot_color(None);
        assert_eq!(s.page().canvas.dot_color, None);
        assert!(s.is_dirty());
    }

    #[test]
    fn major_every_is_clamped_and_dirty_only_on_change() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_major_every(0);
        assert!(!s.is_dirty());
        s.set_major_every(5);
        assert_eq!(s.page().canvas.major_every, 5);
        assert!(s.is_dirty());
        s.set_major_every(99);
        assert_eq!(s.page().canvas.major_every, MAJOR_EVERY_MAX);
        s.save().unwrap();
        s.set_major_every(21);
        assert!(!s.is_dirty());
    }

    #[test]
    fn dot_size_is_rounded_to_halves_and_clamped() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_dot_size(1.5);
        assert!(!s.is_dirty());
        s.set_dot_size(2.3);
        assert_eq!(s.page().canvas.dot_size, 2.5);
        assert!(s.is_dirty());
        s.set_dot_size(0.0);
        assert_eq!(s.page().canvas.dot_size, DOT_SIZE_MIN);
        s.set_dot_size(100.0);
        assert_eq!(s.page().canvas.dot_size, DOT_SIZE_MAX);
        s.set_dot_size(f32::INFINITY);
        assert_eq!(s.page().canvas.dot_size, DOT_SIZE_MAX);
        s.save().unwrap();
        s.set_dot_size(f32::NAN);
        assert_eq!(s.page().canvas.dot_size, DOT_SIZE_MAX);
        assert!(!s.is_dirty());
    }

    #[test]
    fn pattern_settings_round_trip_through_save_and_open() {
        let (_d, mut s) = new_session("a.mindweave");
        s.set_background(Some("#101010"));
        s.set_line_color(Some("#202020"));
        s.set_major_every(4);
        s.set_dot_size(3.0);
        s.set_dot_color(Some("#303030"));
        s.save().unwrap();
        let r = DocumentSession::open(s.path.clone()).unwrap();
        assert_eq!(r.page().canvas, s.page().canvas);
        assert!(!r.is_dirty());
    }

    #[test]
    fn open_clamps_huge_viewport_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("far.mindweave");
        let mut d = Document::new();
        d.pages[0].viewport = Viewport {
            x: 1e30,
            y: -1e20,
            zoom: 1.0,
        };
        crate::format::save(&d, &path).unwrap();
        let s = DocumentSession::open(path).unwrap();
        let v = s.page().viewport;
        assert_eq!((v.x, v.y), (VIEWPORT_COORD_MAX, -VIEWPORT_COORD_MAX));
        assert!(!s.is_dirty());
    }

    #[test]
    fn open_normalizes_pattern_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.mindweave");
        let mut d = Document::new();
        let c = &mut d.pages[0].canvas;
        c.background = Some("ff8000".into());
        c.line_color = Some("not a colour".into());
        c.dot_color = Some("#12".into());
        c.major_every = 500;
        c.dot_size = 2.2;
        let mut p2 = Page::new("Two");
        p2.canvas.dot_size = 0.1;
        d.pages.push(p2);
        crate::format::save(&d, &path).unwrap();
        let s = DocumentSession::open(path).unwrap();
        let c = &s.doc.pages[0].canvas;
        assert_eq!(c.background.as_deref(), Some("#FF8000"));
        assert_eq!(c.line_color, None);
        assert_eq!(c.dot_color, None);
        assert_eq!(c.major_every, MAJOR_EVERY_MAX);
        assert_eq!(c.dot_size, 2.0);
        assert_eq!(s.doc.pages[1].canvas.dot_size, DOT_SIZE_MIN);
        assert!(!s.is_dirty());
    }

    #[test]
    fn view_changes_do_not_mark_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_page();
        s.save().unwrap();
        let v = Viewport {
            x: 10.0,
            y: 20.0,
            zoom: 2.0,
        };
        s.set_viewport(v);
        s.set_active_page(0);
        s.set_active_layer(0);
        assert!(!s.is_dirty());
        s.set_active_page(1);
        s.set_viewport(v);
        s.save().unwrap();
        let r = DocumentSession::open(s.path.clone()).unwrap();
        assert_eq!(r.page().viewport, v);
        assert_eq!(r.doc.active_page, 1);
    }

    #[test]
    fn set_viewport_ignores_non_finite_values() {
        let (_d, mut s) = new_session("a.mindweave");
        let good = Viewport {
            x: 10.0,
            y: 20.0,
            zoom: 2.0,
        };
        s.set_viewport(good);
        for bad in [
            Viewport {
                x: f32::NAN,
                ..good
            },
            Viewport {
                y: f32::INFINITY,
                ..good
            },
            Viewport {
                zoom: f32::NAN,
                ..good
            },
            Viewport {
                zoom: f32::NEG_INFINITY,
                ..good
            },
        ] {
            s.set_viewport(bad);
            assert_eq!(s.page().viewport, good);
        }
    }

    #[test]
    fn save_clears_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_page();
        assert!(s.is_dirty());
        s.save().unwrap();
        assert!(!s.is_dirty());
    }

    #[test]
    fn title_shows_dirty_marker() {
        let (_d, mut s) = new_session("arch.mindweave");
        assert_eq!(s.title(), "arch");
        s.add_page();
        assert_eq!(s.title(), "arch •");
    }

    #[test]
    fn open_normalizes_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.mindweave");
        let mut d = Document::new();
        d.pages[0].canvas.grid_size = 0.0;
        d.pages[0].viewport.zoom = -3.0;
        let mut p2 = Page::new("Two");
        p2.canvas.grid_size = 5000.0;
        p2.viewport.zoom = 0.0;
        d.pages.push(p2);
        crate::format::save(&d, &path).unwrap();
        let s = DocumentSession::open(path).unwrap();
        assert_eq!(s.doc.pages[0].canvas.grid_size, GRID_MIN);
        assert_eq!(s.doc.pages[0].viewport.zoom, ZOOM_MIN);
        assert_eq!(s.doc.pages[1].canvas.grid_size, GRID_MAX);
        assert_eq!(s.doc.pages[1].viewport.zoom, ZOOM_MIN);
        assert!(!s.is_dirty());
    }

    #[test]
    fn normalize_replaces_non_finite_values() {
        let mut p = Page::new("p");
        p.canvas.grid_size = f32::NAN;
        p.viewport = Viewport {
            x: f32::NAN,
            y: f32::INFINITY,
            zoom: f32::NAN,
        };
        p.canvas.dot_size = f32::NAN;
        normalize_page(&mut p);
        assert_eq!(p.canvas.grid_size, 20.0);
        assert_eq!(p.canvas.dot_size, 1.5);
        assert_eq!(
            (p.viewport.x, p.viewport.y, p.viewport.zoom),
            (0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn open_normalises_bad_shape_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let mut d = Document::new();
        let def = crate::library::builtin_libraries()[0].shapes[0].clone();
        let mut s = crate::shape::Shape::new(&def, "basic-shapes", Default::default());
        s.frame.x = f32::NAN;
        s.frame.w = -5.0;
        s.frame.h = 10.0;
        s.style.opacity = 7.0;
        d.pages[0].layers[0].objects.push(s);
        // JSON cannot hold NaN, so write through serde with it replaced by null-free value.
        let mut v = serde_json::to_value(&d).unwrap();
        v["pages"][0]["layers"][0]["objects"][0]["x"] = 5.into();
        std::fs::write(&path, v.to_string()).unwrap();
        let session = DocumentSession::open(path).unwrap();
        let s = &session.page().layers[0].objects[0];
        assert_eq!(s.frame.w, 0.0);
        assert_eq!(s.style.opacity, 1.0);
        assert!(!session.is_dirty());
        let mut s = s.clone();
        s.frame.x = f32::NAN;
        s.frame.y = f32::INFINITY;
        s.normalize();
        assert_eq!((s.frame.x, s.frame.y), (0.0, 0.0));
    }

    #[test]
    fn zero_height_line_survives_open() {
        let (_d, mut s) = new_session("a.mindweave");
        let mut line = rect(0.0);
        line.frame.h = 0.0;
        let id = line.id;
        s.add_shapes(vec![line]);
        s.save().unwrap();
        let r = DocumentSession::open(s.path.clone()).unwrap();
        assert_eq!(r.find_shape(id).unwrap().frame.h, 0.0);
        assert_eq!(r.find_shape(id).unwrap().frame.w, 10.0);
        assert!(!r.is_dirty());
    }

    fn rect(x: f32) -> Shape {
        let def = crate::library::builtin_libraries()[0].shapes[0].clone();
        let frame = crate::geometry::Frame {
            x,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            ..Default::default()
        };
        Shape::new(&def, "basic-shapes", frame)
    }

    fn label(text: &str) -> crate::shape::Label {
        use crate::text::{HAlign, RichText, RunStyle};
        let mut t = RichText::new(RunStyle::default(), HAlign::Center);
        t.insert(0, text, &RunStyle::default());
        crate::shape::Label {
            text: t,
            valign: crate::shape::VAlign::Middle,
        }
    }

    #[test]
    fn layout_size_does_not_dirty() {
        let (_d, mut s) = new_session("a.mindweave");
        let tb = Shape::new_text_box((0.0, 0.0));
        let id = tb.id;
        s.add_shapes(vec![tb]);
        s.save().unwrap();
        let rev = s.revision();
        let can_undo = s.can_undo();

        // Auto width: both derived sizes change, nothing is recorded.
        s.set_text_box_layout(id, 80.0, 40.0);
        let f = s.find_shape(id).unwrap().frame;
        assert_eq!((f.w, f.h), (80.0, 40.0));
        assert!(!s.is_dirty());
        assert_eq!(s.revision(), rev);
        assert_eq!(s.can_undo(), can_undo);

        // Fixed width: the width is content, only the height is derived.
        s.update_shapes(&[id], |sh| {
            sh.text_box.as_mut().unwrap().auto_width = false;
            sh.frame.w = 100.0;
        });
        s.save().unwrap();
        s.set_text_box_layout(id, 55.0, 60.0);
        let f = s.find_shape(id).unwrap().frame;
        assert_eq!((f.w, f.h), (100.0, 60.0));
        assert!(!s.is_dirty());

        // The comparison after an edit ignores the derived height too.
        s.update_shapes(&[id], |sh| sh.frame.x += 5.0);
        s.set_text_box_layout(id, 0.0, 99.0);
        s.update_shapes(&[id], |sh| sh.frame.x -= 5.0);
        assert!(!s.is_dirty());
        // A real width change on a fixed-width box is dirty.
        s.update_shapes(&[id], |sh| sh.frame.w = 120.0);
        assert!(s.is_dirty());

        // Non-text-box shapes and bad values are ignored.
        let r = rect(0.0);
        let rid = r.id;
        s.add_shapes(vec![r]);
        let rev = s.revision();
        s.set_text_box_layout(rid, 50.0, 50.0);
        assert_eq!(s.find_shape(rid).unwrap().frame.w, 10.0);
        s.set_text_box_layout(id, f32::NAN, f32::INFINITY);
        assert_eq!(s.find_shape(id).unwrap().frame.h, 99.0);
        assert_eq!(s.revision(), rev);
    }

    #[test]
    fn layout_generation_counts_layout_size_changes() {
        let (_d, mut s) = new_session("a.mindweave");
        let tb = Shape::new_text_box((0.0, 0.0));
        let id = tb.id;
        s.add_shapes(vec![tb]);
        let gen = s.layout_generation();
        let rev = s.revision();

        s.set_text_box_layout(id, 80.0, 40.0);
        let gen2 = s.layout_generation();
        assert_ne!(gen2, gen);
        assert_eq!(s.revision(), rev);
        // The same size again changes nothing.
        s.set_text_box_layout(id, 80.0, 40.0);
        assert_eq!(s.layout_generation(), gen2);
        // Ignored calls change nothing either.
        s.set_text_box_layout(id, f32::NAN, 1.0);
        s.set_text_box_layout(uuid::Uuid::new_v4(), 5.0, 5.0);
        assert_eq!(s.layout_generation(), gen2);
        s.set_text_box_layout(id, 80.0, 41.0);
        assert_ne!(s.layout_generation(), gen2);
    }

    #[test]
    fn layout_keeps_a_rotated_text_box_left_edge_in_place() {
        let (_d, mut s) = new_session("a.mindweave");
        let mut tb = Shape::new_text_box((100.0, 50.0));
        tb.frame.w = 40.0;
        tb.frame.h = 20.0;
        tb.frame.rotation = 30.0;
        let id = tb.id;
        s.add_shapes(vec![tb]);
        let corners = |f: &Frame| {
            let (sin, cos) = f.rotation.to_radians().sin_cos();
            let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
            let at = |lx: f32, ly: f32| {
                let (dx, dy) = (lx - f.w / 2.0, ly - f.h / 2.0);
                (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
            };
            (at(0.0, 0.0), at(0.0, f.h))
        };
        let before = corners(&s.find_shape(id).unwrap().frame);

        s.set_text_box_layout(id, 120.0, 60.0);
        let f = s.find_shape(id).unwrap().frame;
        assert_eq!((f.w, f.h, f.rotation), (120.0, 60.0, 30.0));
        let (top_left, bottom_left) = corners(&f);
        let close =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(close(top_left, before.0), "{top_left:?} vs {:?}", before.0);
        // The left edge keeps its direction; it grew downwards.
        let dir = |a: (f32, f32), b: (f32, f32)| ((b.0 - a.0) / 60.0, (b.1 - a.1) / 60.0);
        let (sin, cos) = 30f32.to_radians().sin_cos();
        assert!(close(dir(top_left, bottom_left), (-sin, cos)));

        // Moving x and y with the layout is not an unsaved change.
        s.save().unwrap();
        s.update_shapes(&[id], |sh| sh.frame.x += 5.0);
        s.set_text_box_layout(id, 70.0, 90.0);
        s.update_shapes(&[id], |sh| sh.frame.x -= 5.0);
        assert!(!s.is_dirty());
        s.update_shapes(&[id], |sh| sh.frame.y += 1.0);
        assert!(s.is_dirty());

        // Unrotated boxes keep x and y.
        let tb = Shape::new_text_box((7.0, 9.0));
        let id = tb.id;
        s.add_shapes(vec![tb]);
        s.set_text_box_layout(id, 50.0, 30.0);
        let f = s.find_shape(id).unwrap().frame;
        assert_eq!((f.x, f.y, f.w, f.h), (7.0, 9.0, 50.0, 30.0));
    }

    #[test]
    fn set_label_is_one_undo_step() {
        let (_d, mut s) = new_session("a.mindweave");
        let r = rect(0.0);
        let id = r.id;
        s.add_shapes(vec![r]);
        s.save().unwrap();
        let rev = s.revision();

        s.set_label(id, Some(label("Hi")));
        assert_eq!(s.find_shape(id).unwrap().label, Some(label("Hi")));
        assert!(s.is_dirty());
        let rev2 = s.revision();
        assert_ne!(rev2, rev);
        // Same label again: no step.
        s.set_label(id, Some(label("Hi")));
        assert_eq!(s.revision(), rev2);

        assert!(s.undo());
        assert_eq!(s.find_shape(id).unwrap().label, None);
        assert!(!s.is_dirty());
        assert!(s.redo());
        assert_eq!(s.find_shape(id).unwrap().label, Some(label("Hi")));
        s.set_label(id, None);
        assert_eq!(s.find_shape(id).unwrap().label, None);
        assert!(s.undo());
        assert_eq!(s.find_shape(id).unwrap().label, Some(label("Hi")));
        // Unknown id: nothing happens.
        let rev = s.revision();
        s.set_label(uuid::Uuid::new_v4(), Some(label("x")));
        assert_eq!(s.revision(), rev);
    }

    #[test]
    fn empty_box_removed_inside_gesture_leaves_no_step() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_shapes(vec![rect(0.0)]);
        s.save().unwrap();
        let rev = s.revision();
        let can_undo = s.can_undo();

        s.begin_gesture();
        let tb = Shape::new_text_box((10.0, 10.0));
        let id = tb.id;
        s.add_shapes(vec![tb]);
        s.set_text_box_layout(id, 30.0, 20.0);
        s.update_shapes(&[id], |sh| {
            sh.label.as_mut().unwrap().text = label("typed").text;
        });
        s.update_shapes(&[id], |sh| {
            sh.label.as_mut().unwrap().text = Shape::new_text_box((0.0, 0.0)).label.unwrap().text;
        });
        s.remove_shape_silently(id);
        assert!(s.find_shape(id).is_none());
        s.end_gesture();

        assert_eq!(s.revision(), rev);
        assert_eq!(s.can_undo(), can_undo);
        assert!(!s.is_dirty());
        assert_eq!(s.shapes_on_active_layer().len(), 1);
    }

    #[test]
    fn open_normalises_text_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let mut d = Document::new();
        let mut r = rect(0.0);
        r.label = Some(label("abc"));
        d.pages[0].layers[0].objects.push(r);
        let mut v = serde_json::to_value(&d).unwrap();
        let runs = serde_json::json!([
            {"text": "a", "style": {"size": 0, "color": "nope", "font": "Missing Font"}},
            {"text": "b", "style": {"size": 1e9, "color": "#00ff00"}},
            {"text": "c", "style": {"size": 12}}
        ]);
        v["pages"][0]["layers"][0]["objects"][0]["label"]["text"]["paragraphs"][0]["runs"] = runs;
        std::fs::write(&path, v.to_string()).unwrap();

        let session = DocumentSession::open(path).unwrap();
        let shape = &session.page().layers[0].objects[0];
        let runs = &shape.label.as_ref().unwrap().text.paragraphs[0].runs;
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].style.size, 1.0);
        assert_eq!(runs[0].style.color, None);
        assert_eq!(runs[0].style.font.as_deref(), Some("Missing Font"));
        assert_eq!(runs[1].style.size, 1000.0);
        assert_eq!(runs[1].style.color.as_deref(), Some("#00FF00"));
        assert_eq!(runs[2].style.size, 12.0);
        assert!(!session.is_dirty());

        // JSON cannot hold NaN; a NaN that reaches the model is repaired.
        let (_d, mut s) = new_session("b.mindweave");
        let mut r = rect(0.0);
        let mut l = label("x");
        l.text.paragraphs[0].runs[0].style.size = f32::NAN;
        r.label = Some(l);
        let id = r.id;
        s.add_shapes(vec![r]);
        let l = s.find_shape(id).unwrap().label.clone().unwrap();
        assert_eq!(l.text.paragraphs[0].runs[0].style.size, 16.0);
        let mut l = label("y");
        l.text.paragraphs[0].runs[0].style.size = f32::NAN;
        s.set_label(id, Some(l));
        let l = s.find_shape(id).unwrap().label.clone().unwrap();
        assert_eq!(l.text.paragraphs[0].runs[0].style.size, 16.0);
    }

    #[test]
    fn undo_to_saved_state_is_clean() {
        let (_d, mut s) = new_session("a.mindweave");
        s.save().unwrap();
        assert!(!s.can_undo());
        s.add_shapes(vec![rect(0.0)]);
        assert!(s.is_dirty());
        assert!(s.can_undo());
        assert!(s.undo());
        assert!(s.shapes_on_active_layer().is_empty());
        assert!(!s.is_dirty());
        assert!(s.can_redo());
        assert!(s.redo());
        assert_eq!(s.shapes_on_active_layer().len(), 1);
        assert!(s.is_dirty());
        assert!(!s.redo());
    }

    #[test]
    fn gesture_is_one_step() {
        let (_d, mut s) = new_session("a.mindweave");
        let shape = rect(0.0);
        let id = shape.id;
        s.add_shapes(vec![shape.clone()]);
        s.begin_gesture();
        s.begin_gesture(); // nested: no-op
        for _ in 0..3 {
            s.update_shapes(&[id], |sh| sh.frame.x += 5.0);
        }
        s.end_gesture();
        assert_eq!(s.find_shape(id).unwrap().frame.x, 15.0);
        assert!(s.undo());
        assert_eq!(s.find_shape(id), Some(&shape));
        assert!(s.undo());
        assert!(s.find_shape(id).is_none());
        assert!(!s.can_undo());
    }

    #[test]
    fn empty_gesture_records_nothing() {
        let (_d, mut s) = new_session("a.mindweave");
        s.begin_gesture();
        s.set_pattern(BackgroundPattern::Dots); // same value
        s.set_viewport(Viewport {
            x: 5.0,
            y: 5.0,
            zoom: 1.0,
        });
        s.end_gesture();
        assert!(!s.can_undo());
        assert!(!s.is_dirty());
        // A gesture that changes and restores content records nothing.
        s.begin_gesture();
        s.toggle_layer_visibility(0);
        s.toggle_layer_visibility(0);
        s.end_gesture();
        assert!(!s.can_undo());
        s.end_gesture(); // unmatched end: no-op
        assert!(!s.can_undo());
    }

    #[test]
    fn undo_keeps_viewport() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_shapes(vec![rect(0.0)]);
        let v = Viewport {
            x: 40.0,
            y: -10.0,
            zoom: 2.0,
        };
        s.set_viewport(v);
        assert!(s.undo());
        assert_eq!(s.page().viewport, v);
        assert!(s.redo());
        assert_eq!(s.page().viewport, v);
    }

    #[test]
    fn undo_restores_page_add_while_on_other_page() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_page();
        assert_eq!(s.doc.active_page, 1);
        assert!(s.undo());
        assert_eq!(s.doc.pages.len(), 1);
        assert_eq!(s.doc.active_page, 0);
        assert!(!s.is_dirty());
        assert!(s.redo());
        assert_eq!(s.doc.pages.len(), 2);
        // Undo the add while viewing the first page.
        s.set_active_page(0);
        s.add_layer();
        s.set_active_layer(1);
        s.set_active_page(1);
        assert!(s.undo()); // removes the layer on page 0
        assert_eq!(s.doc.active_page, 1);
        assert_eq!(s.doc.pages[0].layers.len(), 1);
        assert_eq!(s.doc.pages[0].active_layer, 0);
        assert!(s.undo()); // removes page 2 while it is active
        assert_eq!(s.doc.pages.len(), 1);
        assert_eq!(s.doc.active_page, 0);
    }

    #[test]
    fn existing_edits_are_undoable() {
        let (_d, mut s) = new_session("a.mindweave");
        s.rename_layer(0, "Back");
        s.set_pattern(BackgroundPattern::Lines);
        s.set_background(Some("#101010"));
        s.set_grid_size(30.0);
        s.rename_page(0, "Plan");
        assert!(s.undo());
        assert_eq!(s.page().name, "Page 1");
        assert!(s.undo());
        assert_eq!(s.page().canvas.grid_size, 20.0);
        assert!(s.undo());
        assert_eq!(s.page().canvas.background, None);
        assert!(s.undo());
        assert_eq!(s.page().canvas.pattern, BackgroundPattern::Dots);
        assert!(s.undo());
        assert_eq!(s.page().layers[0].name, "Layer 1");
        assert!(!s.undo());
        assert!(!s.is_dirty());
        // No-op edits record nothing.
        s.set_pattern(BackgroundPattern::Dots);
        s.set_background(Some("garbage"));
        s.rename_layer(0, "Layer 1");
        s.set_dot_size(1.5);
        s.set_major_every(0);
        assert!(!s.can_undo());
        // A new edit clears redo.
        assert!(s.can_redo());
        s.toggle_layer_visibility(0);
        assert!(!s.can_redo());
    }

    #[test]
    fn remove_shapes_returns_removed() {
        let (_d, mut s) = new_session("a.mindweave");
        let (a, b, c) = (rect(0.0), rect(1.0), rect(2.0));
        s.add_shapes(vec![a.clone(), b.clone()]);
        s.add_layer();
        s.add_shapes(vec![c.clone()]);
        assert_eq!(s.shapes_on_active_layer(), std::slice::from_ref(&c));
        s.save().unwrap();
        // Searches every layer of the active page.
        let removed = s.remove_shapes(&[a.id, c.id]);
        assert_eq!(removed, vec![a.clone(), c.clone()]);
        assert!(s.shapes_on_active_layer().is_empty());
        assert_eq!(s.page().layers[1].objects, vec![b.clone()]);
        assert!(s.is_dirty());
        assert!(s.undo());
        assert!(!s.is_dirty());
        assert_eq!(s.page().layers[1].objects, vec![a, b]);
        // Unknown ids remove nothing and record nothing.
        s.redo();
        s.save().unwrap();
        assert!(s.remove_shapes(&[uuid::Uuid::new_v4()]).is_empty());
        assert!(!s.is_dirty());
        assert!(!s.can_redo());
        assert!(s.can_undo());
        assert!(s.undo());
        assert_eq!(s.page().layers[0].objects, vec![c]);
    }

    fn many_rects(n: usize) -> (Vec<Shape>, Vec<Uuid>) {
        let shapes: Vec<Shape> = (0..n).map(|i| rect(i as f32)).collect();
        let ids = shapes.iter().map(|s| s.id).collect();
        (shapes, ids)
    }

    #[test]
    fn update_shapes_changes_every_one_of_many_ids() {
        let (_d, mut s) = new_session("a.mindweave");
        let (shapes, ids) = many_rects(2_000);
        s.add_shapes(shapes);
        s.update_shapes(&ids, |sh| sh.frame.y = 5.0);
        assert!(s
            .shapes_on_active_layer()
            .iter()
            .all(|sh| sh.frame.y == 5.0));
        // One undo step for the whole batch.
        assert!(s.undo());
        assert!(s
            .shapes_on_active_layer()
            .iter()
            .all(|sh| sh.frame.y == 0.0));
        // A subset by id leaves the others alone.
        s.redo();
        s.update_shapes(&ids[..1_000], |sh| sh.frame.y = 9.0);
        let layer = s.shapes_on_active_layer();
        assert!(layer[..1_000].iter().all(|sh| sh.frame.y == 9.0));
        assert!(layer[1_000..].iter().all(|sh| sh.frame.y == 5.0));
        assert_eq!(s.remove_shapes(&ids[1_000..]).len(), 1_000);
        assert_eq!(s.shapes_on_active_layer().len(), 1_000);
    }

    /// Coarse guard that id lookup is not O(selected × shapes): a drag of
    /// 2,000 selected shapes updates them once per pointer move.
    /// Run with `cargo test --release -p mindweave-core -- --ignored`.
    #[test]
    #[ignore = "timing guard; release builds only"]
    fn update_shapes_of_two_thousand_is_fast() {
        let (_d, mut s) = new_session("a.mindweave");
        let (shapes, ids) = many_rects(2_000);
        s.add_shapes(shapes);
        s.begin_gesture();
        let start = std::time::Instant::now();
        for i in 0..60 {
            s.update_shapes(&ids, |sh| sh.frame.y = i as f32);
        }
        let per_move = start.elapsed() / 60;
        assert!(
            per_move < std::time::Duration::from_millis(1),
            "update_shapes took {per_move:?} per call"
        );
    }

    #[test]
    fn update_shapes_marks_dirty_only_on_change() {
        let (_d, mut s) = new_session("a.mindweave");
        let shape = rect(0.0);
        let id = shape.id;
        s.add_shapes(vec![shape]);
        s.save().unwrap();
        s.update_shapes(&[id], |sh| sh.frame.x = 0.0);
        s.update_shapes(&[uuid::Uuid::new_v4()], |sh| sh.frame.x = 99.0);
        assert!(!s.is_dirty());
        assert!(s.undo()); // only the add is in history
        assert!(s.find_shape(id).is_none());
        assert!(s.redo());
        s.update_shapes(&[id], |sh| sh.frame.x = 7.0);
        assert!(s.is_dirty());
        assert_eq!(s.find_shape(id).unwrap().frame.x, 7.0);
        s.update_shapes(&[id], |sh| sh.frame.x = 0.0);
        assert!(!s.is_dirty());
    }

    #[test]
    fn undo_keeps_active_layer_by_id() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_layer();
        let (a, b) = (s.page().layers[0].id, s.page().layers[1].id);
        s.add_layer(); // [N, A, B]
        s.set_active_layer(1);
        assert_eq!(s.page().layers[1].id, a);
        assert!(s.undo()); // [A, B]
        assert_eq!(s.page().layers.len(), 2);
        assert_eq!(s.page().layers[s.page().active_layer].id, a);
        assert!(s.redo()); // [N, A, B]: A stays active
        assert_eq!(s.page().layers[s.page().active_layer].id, a);
        // A missing layer falls back to the clamped index.
        s.set_active_layer(0);
        assert!(s.undo());
        assert_eq!(s.page().active_layer, 0);
        assert!(s.undo()); // [B]
        assert_eq!(s.page().layers[s.page().active_layer].id, b);
    }

    #[test]
    fn edit_after_undo_gets_new_revision() {
        let (_d, mut s) = new_session("a.mindweave");
        s.save().unwrap();
        s.rename_page(0, "One");
        assert!(s.is_dirty());
        assert!(s.undo());
        assert!(!s.is_dirty());
        s.rename_page(0, "Two");
        assert!(s.is_dirty());
        assert!(s.undo());
        assert!(!s.is_dirty());
        assert!(s.redo());
        assert!(s.is_dirty());
        assert_eq!(s.page().name, "Two");
        // Saving after an undo: redo is dirty, undoing back is clean.
        assert!(s.undo());
        s.rename_page(0, "Three");
        s.save().unwrap();
        assert!(s.undo());
        assert!(s.is_dirty());
        assert!(s.redo());
        assert!(!s.is_dirty());
    }

    #[test]
    fn revision_names_the_content() {
        let (_d, mut s) = new_session("a.mindweave");
        let start = s.revision();
        s.set_viewport(Viewport {
            x: 5.0,
            y: 5.0,
            zoom: 2.0,
        });
        s.add_page();
        let one_page_more = s.revision();
        assert_ne!(one_page_more, start);
        // View changes keep it.
        s.set_active_page(0);
        assert_eq!(s.revision(), one_page_more);
        assert!(s.undo());
        assert_eq!(s.revision(), start);
        assert!(s.redo());
        assert_eq!(s.revision(), one_page_more);
        // A gesture that changes nothing leaves it as it was.
        s.begin_gesture();
        s.toggle_layer_visibility(0);
        s.toggle_layer_visibility(0);
        s.end_gesture();
        assert_eq!(s.revision(), one_page_more);
    }

    #[test]
    fn gesture_dirty_tracks_content() {
        let (_d, mut s) = new_session("a.mindweave");
        s.begin_gesture();
        s.toggle_layer_visibility(0);
        assert!(s.is_dirty());
        s.toggle_layer_visibility(0);
        s.end_gesture();
        assert!(!s.is_dirty());
        assert!(!s.can_undo());
    }

    #[test]
    fn update_with_nan_is_normalised() {
        let (_d, mut s) = new_session("a.mindweave");
        let mut shape = rect(3.0);
        shape.frame.y = f32::NAN;
        shape.style.opacity = 9.0;
        let id = shape.id;
        s.add_shapes(vec![shape]);
        let added = s.find_shape(id).unwrap();
        assert_eq!((added.frame.y, added.style.opacity), (0.0, 1.0));
        s.update_shapes(&[id], |sh| sh.frame.x = f32::NAN);
        assert!(s.find_shape(id).unwrap().frame.x.is_finite());
        s.save().unwrap();
        s.update_shapes(&[id], |sh| sh.frame.x = f32::NAN);
        assert!(!s.is_dirty());
        assert!(s.undo()); // the first update
        assert_eq!(s.find_shape(id).unwrap().frame.x, 3.0);
        assert!(s.undo()); // the add
        assert!(!s.can_undo());
    }

    #[test]
    fn doc_accessor_reads_document() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_page();
        assert_eq!(s.doc().pages.len(), 2);
        assert_eq!(s.doc().active_page, 1);
    }

    #[test]
    fn active_layer_selectable_follows_visibility() {
        let (_d, mut s) = new_session("a.mindweave");
        assert!(s.active_layer_selectable());
        s.toggle_layer_visibility(0);
        assert!(!s.active_layer_selectable());
    }

    #[test]
    fn dirty_ignores_view_state() {
        let (_d, mut s) = new_session("a.mindweave");
        s.add_page();
        s.add_layer();
        s.save().unwrap();
        s.set_active_layer(1);
        s.set_active_page(0);
        s.set_viewport(Viewport {
            x: 3.0,
            y: 4.0,
            zoom: 1.5,
        });
        assert!(!s.is_dirty());
        assert!(!s.title().ends_with('•'));
    }
}
