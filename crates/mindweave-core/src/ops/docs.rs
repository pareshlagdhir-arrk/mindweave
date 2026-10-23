//! Documents, pages and layers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use super::read::kind_of;
use super::{OpError, OpResult, Workspace};
use crate::library::LibraryRegistry;
use crate::session::DocumentSession;

/// Longest page or layer name accepted.
const MAX_NAME_CHARS: usize = 200;

/// A layer as [`Workspace::summary`] reports it.
#[derive(Clone, Debug, Serialize)]
pub struct LayerSummary {
    pub index: usize,
    pub id: Uuid,
    pub name: String,
    pub visible: bool,
    pub objects: usize,
}

/// A page as [`Workspace::summary`] reports it.
#[derive(Clone, Debug, Serialize)]
pub struct PageSummary {
    pub index: usize,
    pub id: Uuid,
    pub name: String,
    /// Top layer first, as the app lists them.
    pub layers: Vec<LayerSummary>,
    pub active_layer: usize,
    /// Objects on the page by kind.
    pub counts: BTreeMap<&'static str, usize>,
    /// Central topic ids of the page's mind maps.
    pub mind_maps: Vec<Uuid>,
    /// Artboard ids on the page.
    pub artboards: Vec<Uuid>,
}

/// What a document holds.
#[derive(Clone, Debug, Serialize)]
pub struct DocumentSummary {
    pub path: String,
    pub format_version: u32,
    pub active_page: usize,
    pub pages: Vec<PageSummary>,
}

/// Checks a page or layer name.
fn check_name(name: &str) -> OpResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(OpError::invalid(format!(
            "A name must have 1 to {MAX_NAME_CHARS} characters."
        )));
    }
    Ok(name.to_string())
}

impl Workspace {
    /// Creates a new document at `path` with one page (named `page_name`
    /// when given) and saves it. An existing file is never replaced.
    pub fn create(
        path: &Path,
        page_name: Option<&str>,
        libs: Arc<LibraryRegistry>,
    ) -> OpResult<Workspace> {
        if path.exists() {
            return Err(OpError::invalid(format!(
                "{} already exists; open it instead, or choose a new file name.",
                path.display()
            )));
        }
        let name = page_name.map(check_name).transpose()?;
        let session = DocumentSession::create(path.to_path_buf())?;
        let mut ws = Workspace::with_session(session, libs);
        if let Some(name) = name {
            ws.session.rename_page(0, &name);
            ws.save()?;
        }
        Ok(ws)
    }

    /// The document's pages, layers and object counts.
    pub fn summary(&self) -> DocumentSummary {
        let doc = self.doc();
        let pages = doc
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let shapes = || page.layers.iter().flat_map(|l| &l.objects);
                let mut counts = BTreeMap::new();
                for s in shapes() {
                    *counts.entry(kind_of(s).name()).or_insert(0) += 1;
                }
                PageSummary {
                    index,
                    id: page.id,
                    name: page.name.clone(),
                    layers: page
                        .layers
                        .iter()
                        .enumerate()
                        .map(|(i, l)| LayerSummary {
                            index: i,
                            id: l.id,
                            name: l.name.clone(),
                            visible: l.visible,
                            objects: l.objects.len(),
                        })
                        .collect(),
                    active_layer: page.active_layer,
                    counts,
                    mind_maps: shapes()
                        .filter(|s| s.topic.as_ref().is_some_and(|t| t.is_central()))
                        .map(|s| s.id)
                        .collect(),
                    artboards: shapes().filter(|s| s.is_artboard()).map(|s| s.id).collect(),
                }
            })
            .collect();
        DocumentSummary {
            path: self.session.path.display().to_string(),
            format_version: doc.version,
            active_page: doc.active_page,
            pages,
        }
    }

    /// Adds a page at the end (named `name`, else "Page N") and makes it
    /// active; returns its index.
    pub fn add_page(&mut self, name: Option<&str>) -> OpResult<usize> {
        let name = name.map(check_name).transpose()?;
        let idx = self.session.add_page();
        if let Some(name) = name {
            self.session.rename_page(idx, &name);
        }
        Ok(idx)
    }

    /// Adds a layer on top of page `page` (named `name`, else "Layer N")
    /// and makes it that page's active layer; returns its index (0, the
    /// top).
    pub fn add_layer(&mut self, page: Option<usize>, name: Option<&str>) -> OpResult<usize> {
        let page = self.page_index(page)?;
        let name = name.map(check_name).transpose()?;
        let old = self.doc().active_page;
        self.session.set_active_page(page);
        let idx = self.session.add_layer();
        if let Some(name) = name {
            self.session.rename_layer(idx, &name);
        }
        self.session.set_active_page(old);
        Ok(idx)
    }

    /// Makes page `page` active and, when given, its layer `layer`: new
    /// objects go on the active page's active layer.
    pub fn set_active(&mut self, page: usize, layer: Option<usize>) -> OpResult<()> {
        let page = self.page_index(Some(page))?;
        if let Some(l) = layer {
            let n = self.doc().pages[page].layers.len();
            if l >= n {
                return Err(OpError::invalid(format!(
                    "Page {page} has no layer {l}; layers are numbered 0 (top) to {}.",
                    n - 1
                )));
            }
        }
        self.session.set_active_page(page);
        if let Some(l) = layer {
            self.session.set_active_layer(l);
        }
        Ok(())
    }

    /// Renames page `page`.
    pub fn rename_page(&mut self, page: usize, name: &str) -> OpResult<()> {
        let page = self.page_index(Some(page))?;
        let name = check_name(name)?;
        self.session.rename_page(page, &name);
        Ok(())
    }
}
