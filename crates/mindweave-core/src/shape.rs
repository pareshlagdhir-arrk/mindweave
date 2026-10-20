//! A shape placed on a layer: a reference to a library shape plus its
//! frame, style and parameter values. Unknown fields are kept so files
//! written by newer versions or with missing libraries survive a save.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::color::normalize_hex_color;
use crate::geometry::Frame;
use crate::library::ShapeDef;

const COORD_MAX: f32 = 1e9;
const SIZE_MIN: f32 = 1.0;
const SIZE_MAX: f32 = 1e6;
const STROKE_WIDTH_MAX: f32 = 100.0;

fn one() -> f32 {
    1.0
}

/// Colours are uppercase `#RRGGBB`; `None` means no fill or no stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub fill: Option<String>,
    pub stroke: Option<String>,
    #[serde(default = "one")]
    pub stroke_width: f32,
    /// From 0 to 1.
    #[serde(default = "one")]
    pub opacity: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: Uuid,
    /// Id of the library the shape comes from; it may not be installed.
    pub library: String,
    pub shape: String,
    #[serde(flatten)]
    pub frame: Frame,
    #[serde(default)]
    pub style: Style,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub locked: bool,
    /// Fields this version does not know, written back unchanged.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Shape {
    /// A new shape of `def` (from library `library`) with its default
    /// style and parameter values.
    pub fn new(def: &ShapeDef, library: &str, frame: Frame) -> Shape {
        Shape {
            id: Uuid::new_v4(),
            library: library.to_string(),
            shape: def.id.clone(),
            frame,
            style: Style {
                fill: def.style.fill.clone(),
                stroke: def.style.stroke.clone(),
                stroke_width: def.style.stroke_width,
                opacity: def.style.opacity,
            },
            params: def
                .params
                .iter()
                .map(|p| (p.key.clone(), serde_json::Value::from(p.default)))
                .collect(),
            name: None,
            locked: false,
            extra: serde_json::Map::new(),
        }
    }

    /// The numeric value of parameter `key`, if set to a number.
    pub fn param(&self, key: &str) -> Option<f32> {
        self.params.get(key)?.as_f64().map(|v| v as f32)
    }

    /// Sets parameter `key`. Non-finite values are ignored.
    pub fn set_param(&mut self, key: &str, v: f32) {
        if v.is_finite() {
            self.params
                .insert(key.to_string(), serde_json::Value::from(v));
        }
    }

    /// Replaces non-finite or out-of-range values with usable ones and
    /// drops invalid colours.
    pub fn normalize(&mut self) {
        let f = &mut self.frame;
        for (v, fallback) in [(&mut f.w, SIZE_MIN), (&mut f.h, SIZE_MIN)] {
            *v = if v.is_finite() {
                v.clamp(SIZE_MIN, SIZE_MAX)
            } else {
                fallback
            };
        }
        for v in [&mut f.x, &mut f.y] {
            *v = if v.is_finite() {
                v.clamp(-COORD_MAX, COORD_MAX)
            } else {
                0.0
            };
        }
        f.rotation = if f.rotation.is_finite() {
            let r = f.rotation.clamp(-COORD_MAX, COORD_MAX).rem_euclid(360.0);
            if r >= 360.0 {
                0.0
            } else {
                r
            }
        } else {
            0.0
        };
        let s = &mut self.style;
        s.opacity = if s.opacity.is_finite() {
            s.opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        s.stroke_width = if s.stroke_width.is_finite() {
            s.stroke_width.clamp(0.0, STROKE_WIDTH_MAX)
        } else {
            1.0
        };
        for c in [&mut s.fill, &mut s.stroke] {
            *c = c.as_deref().and_then(normalize_hex_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::builtin_libraries;

    #[test]
    fn flatten_keeps_frame_out_of_extra_and_accepts_integers() {
        let json = r#"{"id":"0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10","library":"l","shape":"s",
            "x":10,"y":-2,"w":30,"h":40,"flip_x":true,"style":{},"future":1}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(
            (s.frame.x, s.frame.y, s.frame.w, s.frame.h),
            (10.0, -2.0, 30.0, 40.0)
        );
        assert!(s.frame.flip_x);
        assert_eq!(s.extra.len(), 1);
        assert!(s.extra.contains_key("future"));
        let out = serde_json::to_value(&s).unwrap();
        assert_eq!(
            out.as_object()
                .unwrap()
                .keys()
                .filter(|k| *k == "x")
                .count(),
            1
        );
        assert_eq!(out["future"], 1);
    }

    #[test]
    fn normalize_wraps_rotation_and_drops_bad_colours() {
        let mut s: Shape = serde_json::from_str(
            r#"{"id":"0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10","library":"l","shape":"s",
            "x":0,"y":0,"w":1,"h":1,"rotation":-90,
            "style":{"fill":"ff0000","stroke":"red","stroke_width":500}}"#,
        )
        .unwrap();
        s.normalize();
        assert_eq!(s.frame.rotation, 270.0);
        assert_eq!(s.style.fill.as_deref(), Some("#FF0000"));
        assert_eq!(s.style.stroke, None);
        assert_eq!(s.style.stroke_width, 100.0);
    }

    #[test]
    fn shape_new_takes_library_defaults() {
        let libs = builtin_libraries();
        let lib = libs.iter().find(|l| l.id == "basic-shapes").unwrap();
        let def = lib.shape("rectangle").unwrap();
        let (w, h) = def.default_size;
        let s = Shape::new(
            def,
            &lib.id,
            Frame {
                w,
                h,
                ..Frame::default()
            },
        );
        assert_eq!(s.library, "basic-shapes");
        assert_eq!(s.shape, "rectangle");
        assert_eq!(s.style.fill.as_deref(), Some("#F5F5F5"));
        assert_eq!(s.param("corner_radius"), Some(0.0));
        assert_eq!((s.frame.w, s.frame.h), (120.0, 80.0));
    }

    #[test]
    fn set_param_round_trips() {
        let mut s: Shape = serde_json::from_str(
            r#"{"id":"0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10","library":"l","shape":"s","x":0,"y":0,"w":1,"h":1}"#,
        )
        .unwrap();
        assert_eq!(s.param("a"), None);
        s.set_param("a", 2.5);
        s.set_param("b", f32::NAN);
        assert_eq!(s.param("a"), Some(2.5));
        assert_eq!(s.param("b"), None);
    }
}
