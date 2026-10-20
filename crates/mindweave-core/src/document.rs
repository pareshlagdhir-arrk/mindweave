//! Document model: pages, layers and per-page canvas settings.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::format::FORMAT_VERSION;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundPattern {
    Lines,
    #[default]
    Dots,
    None,
}

/// Side of a dot in the Dots pattern, in pixels, unless a page sets its own.
pub const DEFAULT_DOT_SIZE: f32 = 1.5;

fn default_dot_size() -> f32 {
    DEFAULT_DOT_SIZE
}

/// A page's background. Colours are uppercase `#RRGGBB` strings; `None`
/// means the theme's colour. Fields added after the first release default
/// when missing, so older files still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasSettings {
    pub pattern: BackgroundPattern,
    /// Line spacing for Lines, dot spacing for Dots, in canvas pixels.
    pub grid_size: f32,
    /// Canvas fill; `None` is the theme background.
    #[serde(default)]
    pub background: Option<String>,
    /// Lines colour; `None` is the theme border colour.
    #[serde(default)]
    pub line_color: Option<String>,
    /// Every Nth grid line is drawn stronger; 0 turns this off.
    #[serde(default)]
    pub major_every: u32,
    #[serde(default = "default_dot_size")]
    pub dot_size: f32,
    /// Dots colour; `None` is the theme border colour.
    #[serde(default)]
    pub dot_color: Option<String>,
}

impl Default for CanvasSettings {
    fn default() -> Self {
        Self {
            pattern: BackgroundPattern::Dots,
            grid_size: 20.0,
            background: None,
            line_color: None,
            major_every: 0,
            dot_size: DEFAULT_DOT_SIZE,
            dot_color: None,
        }
    }
}

/// `x`/`y` is the canvas point shown at the top-left of the canvas area.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: Uuid,
    pub name: String,
    pub visible: bool,
    /// Always empty for now; drawable objects are out of scope.
    #[serde(default)]
    pub objects: Vec<serde_json::Value>,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            visible: true,
            objects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: Uuid,
    pub name: String,
    pub canvas: CanvasSettings,
    pub viewport: Viewport,
    pub layers: Vec<Layer>,
    pub active_layer: usize,
}

impl Page {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            canvas: CanvasSettings::default(),
            viewport: Viewport::default(),
            layers: vec![Layer::new("Layer 1")],
            active_layer: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    pub pages: Vec<Page>,
    pub active_page: usize,
}

impl Document {
    pub fn new() -> Self {
        Self {
            version: FORMAT_VERSION,
            pages: vec![Page::new("Page 1")],
            active_page: 0,
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}
