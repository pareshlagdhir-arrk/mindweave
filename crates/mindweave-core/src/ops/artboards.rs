//! Artboards: named frames that hold the objects placed inside them.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{check_number, check_text, OpError, OpResult, Workspace, MAX_COORD};
use crate::artboard::{artboard_placement, ARTBOARD_PRESETS};
use crate::geometry::Frame;
use crate::shape::Shape;

/// Name of an artboard of a custom size without a name of its own.
const CUSTOM_NAME: &str = "Artboard";

/// A ready-made artboard size.
#[derive(Clone, Debug, Serialize)]
pub struct ArtboardPresetInfo {
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
    /// `device`, `desktop`, `paper` or `social`.
    pub category: &'static str,
}

/// The size of a new artboard: a preset's name, or width and height.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ArtboardSize {
    Preset(String),
    Size { width: f32, height: f32 },
}

/// The built-in artboard presets.
pub fn artboard_presets() -> Vec<ArtboardPresetInfo> {
    use crate::artboard::PresetCategory::*;
    ARTBOARD_PRESETS
        .iter()
        .map(|p| ArtboardPresetInfo {
            name: p.name,
            width: p.size.0,
            height: p.size.1,
            category: match p.category {
                Device => "device",
                Desktop => "desktop",
                Paper => "paper",
                Social => "social",
            },
        })
        .collect()
}

impl Workspace {
    /// The built-in artboard presets.
    pub fn list_artboard_presets(&self) -> Vec<ArtboardPresetInfo> {
        artboard_presets()
    }

    /// Adds an artboard of `size` named `name` (else the preset's name)
    /// on page `page`, its top-left corner at `at`, else beside the
    /// rightmost artboard (as the ribbon places presets) or at the origin.
    /// Objects added later with their centre inside it join it. Returns
    /// its id.
    pub fn add_artboard(
        &mut self,
        page: Option<usize>,
        size: &ArtboardSize,
        name: Option<&str>,
        at: Option<(f32, f32)>,
    ) -> OpResult<Uuid> {
        let page = self.page_index(page)?;
        let (w, h, default_name) = match size {
            ArtboardSize::Preset(p) => {
                let preset = ARTBOARD_PRESETS
                    .iter()
                    .find(|x| x.name.eq_ignore_ascii_case(p.trim()))
                    .ok_or_else(|| {
                        let names: Vec<&str> = ARTBOARD_PRESETS.iter().map(|x| x.name).collect();
                        OpError::invalid(format!(
                            "No artboard preset `{p}`; use one of: {}, or give width and height.",
                            names.join(", ")
                        ))
                    })?;
                (preset.size.0, preset.size.1, preset.name)
            }
            ArtboardSize::Size { width, height } => (*width, *height, CUSTOM_NAME),
        };
        if !(w.is_finite()
            && h.is_finite()
            && (1.0..=MAX_COORD).contains(&w)
            && (1.0..=MAX_COORD).contains(&h))
        {
            return Err(OpError::invalid(format!(
                "Artboard width and height must be from 1 to {MAX_COORD}."
            )));
        }
        let name = match name {
            Some(n) => {
                check_text("name", n)?;
                n.trim().to_string()
            }
            None => default_name.to_string(),
        };
        let (x, y) = match at {
            Some((x, y)) => (check_number("x", x)?, check_number("y", y)?),
            None => {
                let boards: Vec<Shape> = self.doc().pages[page]
                    .layers
                    .iter()
                    .flat_map(|l| l.objects.iter().filter(|s| s.is_artboard()).cloned())
                    .collect();
                artboard_placement(&boards, (w, h), (w / 2.0, h / 2.0))
            }
        };
        let board = Shape::new_artboard(
            Frame {
                x,
                y,
                w,
                h,
                ..Frame::default()
            },
            &name,
        );
        let id = board.id;
        self.on_layer(page, None, |s| s.add_shapes(vec![board]));
        Ok(id)
    }
}
