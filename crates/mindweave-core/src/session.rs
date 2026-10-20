//! An open document: edit operations plus dirty tracking.

use std::path::PathBuf;

use crate::document::{BackgroundPattern, Document, Layer, Page, Viewport};
use crate::format::{self, FormatError};
pub use crate::viewport::{ZOOM_MAX, ZOOM_MIN};

pub const GRID_MIN: f32 = 4.0;
pub const GRID_MAX: f32 = 200.0;

const DEFAULT_GRID: f32 = 20.0;

/// A document bound to a file path, tracking unsaved content edits.
///
/// View state (viewport, active page/layer) is saved with the file but
/// never marks the session dirty.
#[derive(Debug)]
pub struct DocumentSession {
    pub doc: Document,
    pub path: PathBuf,
    dirty: bool,
}

/// Replace out-of-range or non-finite values with usable ones.
fn normalize_page(page: &mut Page) {
    let g = page.canvas.grid_size;
    page.canvas.grid_size = if g.is_finite() {
        g.clamp(GRID_MIN, GRID_MAX)
    } else {
        DEFAULT_GRID
    };
    let v = &mut page.viewport;
    v.zoom = if v.zoom.is_finite() {
        v.zoom.clamp(ZOOM_MIN, ZOOM_MAX)
    } else {
        1.0
    };
    if !v.x.is_finite() {
        v.x = 0.0;
    }
    if !v.y.is_finite() {
        v.y = 0.0;
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
        Ok(Self {
            doc,
            path,
            dirty: false,
        })
    }

    pub fn open(path: PathBuf) -> Result<Self, FormatError> {
        let mut doc = format::load(&path)?;
        doc.pages.iter_mut().for_each(normalize_page);
        Ok(Self {
            doc,
            path,
            dirty: false,
        })
    }

    pub fn save(&mut self) -> Result<(), FormatError> {
        format::save(&self.doc, &self.path)?;
        self.dirty = false;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// File stem, followed by `" •"` when there are unsaved changes.
    pub fn title(&self) -> String {
        let stem = self
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        if self.dirty {
            format!("{stem} •")
        } else {
            stem
        }
    }

    /// The active page.
    pub fn page(&self) -> &Page {
        &self.doc.pages[self.doc.active_page]
    }

    fn page_mut(&mut self) -> &mut Page {
        &mut self.doc.pages[self.doc.active_page]
    }

    // Content edits (mark dirty).

    pub fn add_page(&mut self) -> usize {
        let name = next_name("Page", self.doc.pages.iter().map(|p| p.name.as_str()));
        self.doc.pages.push(Page::new(name));
        self.doc.active_page = self.doc.pages.len() - 1;
        self.dirty = true;
        self.doc.active_page
    }

    pub fn rename_page(&mut self, idx: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(p) = self.doc.pages.get_mut(idx) {
            if p.name != name {
                p.name = name.to_string();
                self.dirty = true;
            }
        }
    }

    /// Insert a new layer at the top of the active page and make it active.
    pub fn add_layer(&mut self) -> usize {
        let page = self.page_mut();
        let name = next_name("Layer", page.layers.iter().map(|l| l.name.as_str()));
        page.layers.insert(0, Layer::new(name));
        page.active_layer = 0;
        self.dirty = true;
        0
    }

    pub fn rename_layer(&mut self, idx: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(l) = self.page_mut().layers.get_mut(idx) {
            if l.name != name {
                l.name = name.to_string();
                self.dirty = true;
            }
        }
    }

    pub fn toggle_layer_visibility(&mut self, idx: usize) {
        if let Some(l) = self.page_mut().layers.get_mut(idx) {
            l.visible = !l.visible;
            self.dirty = true;
        }
    }

    pub fn set_pattern(&mut self, p: BackgroundPattern) {
        let canvas = &mut self.page_mut().canvas;
        if canvas.pattern != p {
            canvas.pattern = p;
            self.dirty = true;
        }
    }

    /// Round to whole pixels and clamp to `GRID_MIN..=GRID_MAX`; NaN is ignored.
    pub fn set_grid_size(&mut self, px: f32) {
        if px.is_nan() {
            return;
        }
        let px = px.round().clamp(GRID_MIN, GRID_MAX);
        let canvas = &mut self.page_mut().canvas;
        if canvas.grid_size != px {
            canvas.grid_size = px;
            self.dirty = true;
        }
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

    pub fn set_viewport(&mut self, v: Viewport) {
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
        normalize_page(&mut p);
        assert_eq!(p.canvas.grid_size, 20.0);
        assert_eq!(
            (p.viewport.x, p.viewport.y, p.viewport.zoom),
            (0.0, 0.0, 1.0)
        );
    }
}
