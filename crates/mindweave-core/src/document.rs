//! Document model: pages, layers and per-page canvas settings.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::format::FORMAT_VERSION;
use crate::shape::Shape;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundPattern {
    Lines,
    #[default]
    Dots,
    None,
}

/// A page's colour theme: the app's, or always light or always dark. The
/// shapes' Auto colours (fills, outlines, text, ink) and the grid follow
/// it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CanvasTheme {
    #[default]
    App,
    Light,
    Dark,
}

impl CanvasTheme {
    pub const ALL: [CanvasTheme; 3] = [CanvasTheme::App, CanvasTheme::Light, CanvasTheme::Dark];

    pub fn name(self) -> &'static str {
        match self {
            CanvasTheme::App => "Same as App",
            CanvasTheme::Light => "Light",
            CanvasTheme::Dark => "Dark",
        }
    }
}

/// The light canvas: background, ink (Auto outlines and text) and grid.
pub const LIGHT_CANVAS: [&str; 3] = ["#FFFFFF", "#1F2328", "#D0D7DE"];
/// The dark canvas: background, ink and grid.
pub const DARK_CANVAS: [&str; 3] = ["#1E2227", "#E6E6E6", "#3E4451"];

/// A canvas's colours as `#RRGGBB`; `None` where the app theme's colour
/// applies (see [`CanvasSettings::palette`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasPalette {
    pub background: Option<(u8, u8, u8)>,
    /// What Auto outlines and text, and `"ink"`, are drawn in.
    pub ink: Option<(u8, u8, u8)>,
    /// The Lines and Dots colour unless the page sets its own.
    pub grid: Option<(u8, u8, u8)>,
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
    /// Light, dark, or the app's theme.
    #[serde(default, skip_serializing_if = "is_app_theme")]
    pub theme: CanvasTheme,
}

fn is_app_theme(t: &CanvasTheme) -> bool {
    *t == CanvasTheme::App
}

impl CanvasSettings {
    /// The page's colours. A light or dark page has that theme's colours;
    /// a background colour of its own wins, and then ink and grid are
    /// chosen to stand out against it (dark on a light background, light
    /// on a dark one). With neither, the app theme's colours apply.
    pub fn palette(&self) -> CanvasPalette {
        use crate::color::{parse_hex_color, relative_luminance};
        let rgb = |c: &str| parse_hex_color(c);
        let fixed = match self.theme {
            CanvasTheme::App => None,
            CanvasTheme::Light => Some(LIGHT_CANVAS),
            CanvasTheme::Dark => Some(DARK_CANVAS),
        };
        let own = self.background.as_deref().and_then(rgb);
        match (own, fixed) {
            (Some(bg), _) => {
                let [_, ink, grid] = if relative_luminance(bg) > 0.4 {
                    LIGHT_CANVAS
                } else {
                    DARK_CANVAS
                };
                CanvasPalette {
                    background: Some(bg),
                    ink: rgb(ink),
                    grid: rgb(grid),
                }
            }
            (None, Some([bg, ink, grid])) => CanvasPalette {
                background: rgb(bg),
                ink: rgb(ink),
                grid: rgb(grid),
            },
            (None, None) => CanvasPalette {
                background: None,
                ink: None,
                grid: None,
            },
        }
    }
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
            theme: CanvasTheme::App,
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
    /// Shapes, back to front.
    #[serde(default)]
    pub objects: Vec<Shape>,
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
