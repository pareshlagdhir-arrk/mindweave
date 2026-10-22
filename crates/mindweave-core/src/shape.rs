//! A shape placed on a layer: a reference to a library shape plus its
//! frame, style and parameter values, and optionally a text label. Unknown
//! fields are kept so files written by newer versions or with missing
//! libraries survive a save.
//!
//! Free text boxes are shapes too, with the reserved library
//! [`TEXT_LIBRARY`] and shape [`TEXT_SHAPE`]: their text is the `label`
//! and their `frame.h` (and `frame.w` while `auto_width`) is derived from
//! the text layout.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::color::normalize_style_color;
use crate::color::AUTO_COLOR;
use crate::geometry::{Frame, Pt};
use crate::image::{ImageData, IMAGE_SHAPE};
use crate::library::ShapeDef;
use crate::line::{ArrowHead, LineDash, LineProps, LINE_LIBRARY, LINE_SHAPE, OLD_ARROW_SHAPE};
use crate::mindmap::{BoundaryProps, TopicProps, BOUNDARY_SHAPE, TOPIC_SHAPE};
use crate::text::{HAlign, RichText, RunStyle, DEFAULT_FONT_SIZE, LABEL_FONT_SIZE};

/// Library id reserved for Mindweave's built-in objects.
pub const TEXT_LIBRARY: &str = "mindweave";
/// Shape id of a free text box in [`TEXT_LIBRARY`].
pub const TEXT_SHAPE: &str = "text";
/// Height of a new, empty text box: one line at the default font size.
const TEXT_BOX_LINE_HEIGHT: f32 = 20.0;

const COORD_MAX: f32 = 1e9;
/// Zero is allowed: horizontal and vertical lines have no height or width.
const SIZE_MIN: f32 = 0.0;
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
    /// How the stroke is broken up.
    #[serde(skip_serializing_if = "is_solid")]
    pub dash: LineDash,
    /// Two parallel strokes instead of one.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub double: bool,
}

fn is_solid(dash: &LineDash) -> bool {
    *dash == LineDash::Solid
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            opacity: 1.0,
            dash: LineDash::Solid,
            double: false,
        }
    }
}

/// Vertical placement of a shape label inside its frame. Ignored for text
/// boxes, whose height fits their text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// Text on a shape, or the text of a text box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub text: RichText,
    #[serde(default)]
    pub valign: VAlign,
}

impl Label {
    /// The empty label a shape gets when its text is first edited: centred,
    /// vertically in the middle, at [`LABEL_FONT_SIZE`].
    pub fn new_shape_label() -> Label {
        Label {
            text: RichText::new(
                RunStyle {
                    size: LABEL_FONT_SIZE,
                    ..RunStyle::default()
                },
                HAlign::Center,
            ),
            valign: VAlign::Middle,
        }
    }
}

/// Properties only text boxes have.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextBoxProps {
    /// The box grows with its text; turned off once the user sets a width.
    pub auto_width: bool,
}

impl Default for TextBoxProps {
    fn default() -> Self {
        TextBoxProps { auto_width: true }
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
    /// The shape's label, or a text box's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Label>,
    /// Set on text boxes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_box: Option<TextBoxProps>,
    /// Set on lines only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<LineProps>,
    /// The picture of an image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageData>,
    /// The groups the shape is in, outermost first (see
    /// [`crate::arrange`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Uuid>,
    /// Set on mind map topics only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<TopicProps>,
    /// Set on mind map boundaries only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<BoundaryProps>,
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
                ..Style::default()
            },
            params: def
                .params
                .iter()
                .map(|p| (p.key.clone(), serde_json::Value::from(p.default)))
                .collect(),
            name: None,
            locked: false,
            label: None,
            text_box: None,
            line: None,
            image: None,
            groups: Vec::new(),
            topic: None,
            boundary: None,
            extra: serde_json::Map::new(),
        }
        .normalized()
    }

    fn normalized(mut self) -> Shape {
        self.normalize();
        self
    }

    /// A new, empty text box with its top-left corner at `at`: one line
    /// high, growing with its text, left aligned in the default style, no
    /// fill or stroke.
    pub fn new_text_box(at: Pt) -> Shape {
        Shape {
            id: Uuid::new_v4(),
            library: TEXT_LIBRARY.to_string(),
            shape: TEXT_SHAPE.to_string(),
            frame: Frame {
                x: at.0,
                y: at.1,
                w: 1.0,
                h: TEXT_BOX_LINE_HEIGHT,
                ..Frame::default()
            },
            style: Style::default(),
            params: BTreeMap::new(),
            name: None,
            locked: false,
            label: Some(Label {
                text: RichText::new(
                    RunStyle {
                        size: DEFAULT_FONT_SIZE,
                        ..RunStyle::default()
                    },
                    HAlign::Left,
                ),
                valign: VAlign::Top,
            }),
            text_box: Some(TextBoxProps::default()),
            line: None,
            image: None,
            groups: Vec::new(),
            topic: None,
            boundary: None,
            extra: serde_json::Map::new(),
        }
    }

    /// A new image showing `data` in `frame`, with no fill or stroke.
    pub fn new_image(data: ImageData, frame: Frame) -> Shape {
        let mut s = Shape::new_text_box((frame.x, frame.y));
        s.shape = IMAGE_SHAPE.to_string();
        s.frame = frame;
        s.label = None;
        s.text_box = None;
        s.image = Some(data);
        s.normalized()
    }

    /// A new central topic of a new mind map in `frame`: "Central Topic",
    /// centred, in theme colours (its size follows its text).
    pub fn new_topic(frame: Frame) -> Shape {
        let mut s = Shape::new_text_box((frame.x, frame.y));
        s.shape = TOPIC_SHAPE.to_string();
        s.frame = frame;
        s.text_box = None;
        s.style = Style {
            fill: Some(AUTO_COLOR.to_string()),
            stroke: Some(AUTO_COLOR.to_string()),
            stroke_width: 2.0,
            ..Style::default()
        };
        let mut label = Label::new_shape_label();
        label
            .text
            .insert(0, "Central Topic", &label.text.style_at(0));
        s.label = Some(label);
        s.topic = Some(TopicProps::central(s.id));
        s.normalized()
    }

    /// Whether this is a mind map topic.
    pub fn is_topic(&self) -> bool {
        self.library == TEXT_LIBRARY && self.shape == TOPIC_SHAPE
    }

    /// A new boundary round topics `members`, tinted and outlined in
    /// `color` (its frame follows them; see [`crate::mindmap::layout_page`]).
    pub fn new_boundary(members: Vec<Uuid>, color: Option<String>) -> Shape {
        let mut s = Shape::new_text_box((0.0, 0.0));
        s.shape = BOUNDARY_SHAPE.to_string();
        s.label = None;
        s.text_box = None;
        s.style = Style {
            fill: color.clone(),
            stroke: color,
            stroke_width: 1.5,
            dash: LineDash::Dashed,
            ..Style::default()
        };
        s.boundary = Some(BoundaryProps {
            members,
            outline: Vec::new(),
        });
        s.normalized()
    }

    /// Whether this is a mind map boundary.
    pub fn is_boundary(&self) -> bool {
        self.library == TEXT_LIBRARY && self.shape == BOUNDARY_SHAPE
    }

    /// The pictures the shape shows: an image's, a topic's.
    pub fn pictures(&self) -> Vec<&ImageData> {
        let mut out: Vec<&ImageData> = self.image.iter().collect();
        if let Some(t) = &self.topic {
            out.extend(t.picture.iter());
        }
        out
    }

    /// [`Self::pictures`], to change.
    pub fn pictures_mut(&mut self) -> Vec<&mut Option<ImageData>> {
        let mut out = vec![&mut self.image];
        if let Some(t) = &mut self.topic {
            out.push(&mut t.picture);
        }
        out
    }

    /// Whether this is an image.
    pub fn is_image(&self) -> bool {
        self.library == TEXT_LIBRARY && self.shape == IMAGE_SHAPE
    }

    pub fn is_text_box(&self) -> bool {
        self.library == TEXT_LIBRARY && self.shape == TEXT_SHAPE
    }

    /// Whether this is a line (the Line tool's shape).
    pub fn is_line(&self) -> bool {
        self.library == LINE_LIBRARY && self.shape == LINE_SHAPE
    }

    /// Whether the label is drawn along the shape as a line label rather
    /// than inside its frame.
    pub fn is_line_like_label(&self) -> bool {
        self.is_line()
    }

    /// Whether the text box grows with its text (`false` for other shapes).
    pub fn auto_width(&self) -> bool {
        self.text_box.as_ref().is_some_and(|t| t.auto_width)
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

    /// Replaces non-finite or out-of-range values with usable ones, drops
    /// invalid colours, normalises the label text and gives a text box its
    /// missing text box properties and label.
    pub fn normalize(&mut self) {
        let f = &mut self.frame;
        for v in [&mut f.w, &mut f.h] {
            *v = if v.is_finite() {
                v.clamp(SIZE_MIN, SIZE_MAX)
            } else {
                0.0
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
            *c = c.as_deref().and_then(normalize_style_color);
        }
        if self.is_text_box() {
            self.text_box.get_or_insert_with(TextBoxProps::default);
            if self.label.is_none() {
                self.label = Shape::new_text_box((0.0, 0.0)).label;
            }
        }
        if let Some(label) = &mut self.label {
            label.text.normalize();
        }
        if self.library == LINE_LIBRARY && self.shape == OLD_ARROW_SHAPE {
            self.shape = LINE_SHAPE.to_string();
            self.params.remove("head_size");
            self.line = Some(LineProps {
                end_head: ArrowHead::Triangle,
                ..LineProps::default()
            });
        }
        if self.is_line() {
            self.line.get_or_insert_with(LineProps::default);
        } else {
            self.line = None;
        }
        if self.is_topic() {
            let id = self.id;
            self.topic.get_or_insert_with(|| TopicProps::central(id));
            if self.label.is_none() {
                self.label = Some(Label::new_shape_label());
            }
        } else {
            self.topic = None;
        }
        if self.is_boundary() {
            self.boundary.get_or_insert_with(BoundaryProps::default);
            self.label = None;
        } else {
            self.boundary = None;
        }
        if self.is_image() {
            // Pictures are drawn upright and unmirrored.
            let f = &mut self.frame;
            (f.rotation, f.flip_x, f.flip_y) = (0.0, false, false);
        } else {
            self.image = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::builtin_libraries;
    use crate::text::{HAlign, DEFAULT_FONT_SIZE, LABEL_FONT_SIZE};

    #[test]
    fn a_new_shape_label_is_empty_centred_and_middle() {
        let label = Label::new_shape_label();
        assert!(label.text.is_empty());
        assert_eq!(label.valign, VAlign::Middle);
        assert_eq!(label.text.paragraphs.len(), 1);
        assert_eq!(label.text.paragraphs[0].align, HAlign::Center);
        assert_eq!(label.text.style_at(0).size, LABEL_FONT_SIZE);
        assert_eq!(
            label.text.style_at(0),
            RunStyle {
                size: LABEL_FONT_SIZE,
                ..RunStyle::default()
            }
        );
    }

    #[test]
    fn images_stay_upright_and_only_images_hold_pictures() {
        let data = ImageData::from_file_bytes(b"GIF89a..".to_vec()).unwrap();
        let mut img = Shape::new_image(
            data.clone(),
            Frame {
                w: 10.0,
                h: 5.0,
                ..Frame::default()
            },
        );
        assert!(img.is_image() && !img.is_text_box());
        assert!(img.label.is_none());
        img.frame.rotation = 30.0;
        img.frame.flip_x = true;
        img.normalize();
        assert_eq!((img.frame.rotation, img.frame.flip_x), (0.0, false));
        // Files hold the picture's id; the bytes are stored beside.
        let back: Shape = serde_json::from_str(&serde_json::to_string(&img).unwrap()).unwrap();
        assert_eq!(back.image.as_ref().map(|d| &d.id), Some(&data.id));
        let mut t = Shape::new_text_box((0.0, 0.0));
        t.image = Some(data);
        t.normalize();
        assert!(t.image.is_none());
    }

    #[test]
    fn old_arrows_open_as_lines_with_an_end_head() {
        let json = r#"{"id":"0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10","library":"basic-shapes",
            "shape":"arrow","x":0,"y":0,"w":30,"h":40,"params":{"head_size":12}}"#;
        let mut s: Shape = serde_json::from_str(json).unwrap();
        s.normalize();
        assert!(s.is_line());
        assert!(s.params.is_empty());
        assert_eq!(s.line.unwrap().end_head, crate::line::ArrowHead::Triangle);
        // Other shapes never carry line properties.
        let mut r = Shape::new_text_box((0.0, 0.0));
        r.line = Some(LineProps::default());
        r.normalize();
        assert!(r.line.is_none());
    }

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
        assert_eq!(s.style.fill.as_deref(), Some("auto"));
        assert_eq!(s.param("corner_radius"), Some(0.0));
        assert_eq!((s.frame.w, s.frame.h), (120.0, 80.0));
    }

    #[test]
    fn text_box_roundtrip_keeps_auto_width() {
        let mut t = Shape::new_text_box((12.0, 34.0));
        assert!(t.is_text_box());
        assert!(!t.is_line_like_label());
        assert_eq!(
            (t.library.as_str(), t.shape.as_str()),
            ("mindweave", "text")
        );
        assert_eq!(
            (t.frame.x, t.frame.y, t.frame.w, t.frame.h),
            (12.0, 34.0, 1.0, 20.0)
        );
        assert_eq!(t.style, Style::default());
        assert_eq!(t.text_box, Some(TextBoxProps { auto_width: true }));
        let label = t.label.as_ref().unwrap();
        assert!(label.text.is_empty());
        assert_eq!(label.text.paragraphs[0].align, HAlign::Left);
        assert_eq!(label.text.style_at(0).size, DEFAULT_FONT_SIZE);

        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["text_box"]["auto_width"], true);
        let back: Shape = serde_json::from_value(json).unwrap();
        assert!(
            back.extra.is_empty(),
            "known fields leaked: {:?}",
            back.extra
        );
        assert_eq!(back, t);

        t.text_box = Some(TextBoxProps { auto_width: false });
        let back: Shape = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back.text_box, Some(TextBoxProps { auto_width: false }));
    }

    #[test]
    fn label_is_a_field_not_extra_and_absent_when_none() {
        let mut s: Shape = serde_json::from_str(
            r#"{"id":"0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10","library":"basic-shapes","shape":"line",
            "x":0,"y":0,"w":1,"h":1,
            "label":{"text":{"paragraphs":[{"runs":[{"text":"hi","style":{"size":0}}]}]},"valign":"bottom"}}"#,
        )
        .unwrap();
        assert!(s.extra.is_empty());
        assert!(s.is_line_like_label());
        assert!(!s.is_text_box());
        let label = s.label.as_ref().unwrap();
        assert_eq!(label.valign, VAlign::Bottom);
        assert_eq!(label.text.plain(), "hi");
        s.label = None;
        let out = serde_json::to_value(&s).unwrap();
        assert!(out.get("label").is_none());
        assert!(out.get("text_box").is_none());
    }

    #[test]
    fn normalize_repairs_text_box_and_label_text() {
        let mut s: Shape = serde_json::from_str(
            r#"{"id":"0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10","library":"mindweave","shape":"text",
            "x":0,"y":0,"w":1,"h":1}"#,
        )
        .unwrap();
        s.normalize();
        assert_eq!(s.text_box, Some(TextBoxProps { auto_width: true }));
        assert!(s.label.as_ref().unwrap().text.is_empty());
        let mut text = s.label.clone().unwrap().text;
        text.paragraphs[0].style.size = f32::NAN;
        s.label = Some(Label {
            text,
            valign: VAlign::Top,
        });
        s.normalize();
        assert_eq!(
            s.label.unwrap().text.paragraphs[0].style.size,
            DEFAULT_FONT_SIZE
        );
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
