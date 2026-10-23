//! `.mindweave` file format: pretty-printed JSON with a top-level
//! `version`. Version 4 adds `images`: each picture the document shows,
//! stored once by id (see [`crate::image`]). Version 5 adds topic notes,
//! equations, list items and links. Version 6 adds artboards and the
//! artboard each shape belongs to (see [`crate::artboard`]). Version 7
//! adds the `"ink"` style colour and hollow arrowheads, which older builds
//! would silently drop.

use std::path::Path;

use thiserror::Error;

use crate::document::Document;
use crate::image::{collect_images, resolve_images, ImageStore};

pub const FORMAT_VERSION: u32 = 7;
/// Top-level key of the stored pictures.
const IMAGES_KEY: &str = "images";

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("Could not read or write the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("This file is not a valid Mindweave document: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("This file was made by a newer version of Mindweave (format {0}).")]
    UnsupportedVersion(u32),
    #[error("This file contains no pages or a page without layers.")]
    Empty,
}

pub fn load(path: &Path) -> Result<Document, FormatError> {
    let text = std::fs::read_to_string(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&text)?;
    // Check the version before the full parse so newer formats report clearly.
    if let Some(v) = value.get("version").and_then(|v| v.as_u64()) {
        if v > u64::from(FORMAT_VERSION) {
            return Err(FormatError::UnsupportedVersion(
                u32::try_from(v).unwrap_or(u32::MAX),
            ));
        }
    }
    let images: ImageStore = match value.as_object_mut().and_then(|o| o.remove(IMAGES_KEY)) {
        Some(images) => serde_json::from_value(images)?,
        None => ImageStore::new(),
    };
    let mut doc: Document = serde_json::from_value(value)?;
    let shapes = doc
        .pages
        .iter_mut()
        .flat_map(|p| p.layers.iter_mut())
        .flat_map(|l| l.objects.iter_mut());
    resolve_images(shapes, &images);
    if doc.pages.is_empty() || doc.pages.iter().any(|p| p.layers.is_empty()) {
        return Err(FormatError::Empty);
    }
    doc.version = FORMAT_VERSION;
    if doc.active_page >= doc.pages.len() {
        doc.active_page = 0;
    }
    for page in &mut doc.pages {
        if page.active_layer >= page.layers.len() {
            page.active_layer = 0;
        }
    }
    Ok(doc)
}

pub fn save(doc: &Document, path: &Path) -> Result<(), FormatError> {
    let mut value = serde_json::to_value(doc)?;
    let images = collect_images(
        doc.pages
            .iter()
            .flat_map(|p| &p.layers)
            .flat_map(|l| &l.objects),
    );
    if let (Some(obj), false) = (value.as_object_mut(), images.is_empty()) {
        obj.insert(IMAGES_KEY.to_string(), serde_json::to_value(&images)?);
    }
    let json = serde_json::to_string_pretty(&value)?;
    let tmp = path.with_extension("mindweave.tmp");
    std::fs::write(&tmp, json)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::*;
    use std::fs;

    #[test]
    fn new_document_has_one_page_one_layer() {
        let d = Document::new();
        assert_eq!(d.version, 7);
        assert_eq!(d.pages.len(), 1);
        assert_eq!(d.pages[0].name, "Page 1");
        assert_eq!(d.pages[0].layers[0].name, "Layer 1");
        assert_eq!(d.pages[0].canvas.pattern, BackgroundPattern::Dots);
        assert_eq!(d.pages[0].canvas.grid_size, 20.0);
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let mut d = Document::new();
        let mut p2 = d.pages[0].clone();
        p2.id = uuid::Uuid::new_v4();
        p2.name = "Page 2".into();
        p2.canvas.pattern = BackgroundPattern::Lines;
        p2.viewport = Viewport {
            x: 5.0,
            y: -3.0,
            zoom: 2.0,
        };
        d.pages.push(p2);
        d.active_page = 1;
        save(&d, &path).unwrap();
        assert!(!dir.path().join("a.mindweave.tmp").exists());
        assert_eq!(load(&path).unwrap(), d);
    }

    #[test]
    fn each_picture_is_saved_once_and_shared_on_load() {
        use crate::image::ImageData;
        use crate::shape::Shape;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let pic = ImageData::from_file_bytes(b"GIF89a-picture-bytes".to_vec()).unwrap();
        let mut d = Document::new();
        let frame = crate::geometry::Frame::default();
        d.pages[0].layers[0].objects = (0..10)
            .map(|_| Shape::new_image(pic.clone(), frame))
            .collect();
        save(&d, &path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        // The bytes appear once, however many images show them.
        assert_eq!(text.matches("R0lGODlhLXBpY3R1cmUtYnl0ZXM=").count(), 1);
        let back = load(&path).unwrap();
        assert_eq!(back, d);
        let objects = &back.pages[0].layers[0].objects;
        let bytes = |i: usize| objects[i].image.as_ref().unwrap().bytes.clone();
        assert!(std::sync::Arc::ptr_eq(&bytes(0), &bytes(9)));
        // A document without images has no images key.
        save(&Document::new(), &path).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("\"images\""));
    }

    #[test]
    fn saved_json_is_pretty_and_lowercase_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        save(&Document::new(), &path).unwrap();
        let s = fs::read_to_string(&path).unwrap();
        assert!(s.contains("\n  "));
        assert!(s.contains("\"pattern\": \"dots\""));
        assert!(s.contains("\"version\": 7"));
    }

    #[test]
    fn load_rejects_newer_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        fs::write(&path, r#"{"version":8,"pages":[],"active_page":0}"#).unwrap();
        assert!(matches!(
            load(&path),
            Err(FormatError::UnsupportedVersion(8))
        ));
    }

    #[test]
    fn load_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        fs::write(&path, "not json").unwrap();
        assert!(matches!(load(&path), Err(FormatError::Parse(_))));
    }

    #[test]
    fn load_rejects_empty_pages_and_layers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let mut d = Document::new();
        d.pages.clear();
        fs::write(&path, serde_json::to_string(&d).unwrap()).unwrap();
        assert!(matches!(load(&path), Err(FormatError::Empty)));
        let mut d = Document::new();
        d.pages[0].layers.clear();
        fs::write(&path, serde_json::to_string(&d).unwrap()).unwrap();
        assert!(matches!(load(&path), Err(FormatError::Empty)));
    }

    #[test]
    fn load_missing_file_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            load(&dir.path().join("nope.mindweave")),
            Err(FormatError::Io(_))
        ));
    }

    #[test]
    fn load_fills_in_pattern_settings_missing_from_older_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.mindweave");
        // A page as saved before the per-pattern settings existed.
        let old = r#"{
          "version": 1,
          "pages": [{
            "id": "6f1c1f5e-2f0a-4f43-9d5c-6a0f4c1d2e3f",
            "name": "Page 1",
            "canvas": { "pattern": "lines", "grid_size": 24.0 },
            "viewport": { "x": 0.0, "y": 0.0, "zoom": 1.0 },
            "layers": [{
              "id": "0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10",
              "name": "Layer 1",
              "visible": true
            }],
            "active_layer": 0
          }],
          "active_page": 0
        }"#;
        fs::write(&path, old).unwrap();
        let c = &load(&path).unwrap().pages[0].canvas;
        assert_eq!(c.pattern, BackgroundPattern::Lines);
        assert_eq!(c.grid_size, 24.0);
        assert_eq!(c.background, None);
        assert_eq!(c.line_color, None);
        assert_eq!(c.major_every, 0);
        assert_eq!(c.dot_size, 1.5);
        assert_eq!(c.dot_color, None);
    }

    #[test]
    fn load_clamps_bad_active_indices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let mut d = Document::new();
        d.active_page = 9;
        d.pages[0].active_layer = 7;
        fs::write(&path, serde_json::to_string(&d).unwrap()).unwrap();
        let l = load(&path).unwrap();
        assert_eq!(l.active_page, 0);
        assert_eq!(l.pages[0].active_layer, 0);
    }

    const SHAPE_DOC: &str = r##"{
      "version": 2,
      "pages": [{
        "id": "6f1c1f5e-2f0a-4f43-9d5c-6a0f4c1d2e3f",
        "name": "Page 1",
        "canvas": { "pattern": "dots", "grid_size": 20.0 },
        "viewport": { "x": 0.0, "y": 0.0, "zoom": 1.0 },
        "layers": [{
          "id": "0b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a10",
          "name": "Layer 1",
          "visible": true,
          "objects": [{
            "id": "1b6a2c1d-6c55-4e0e-8a83-0b3e8f1f9a11",
            "library": "aws",
            "shape": "ec2",
            "x": 10.0, "y": 20.5, "w": 120.0, "h": 80.0,
            "rotation": 0.0, "flip_x": false, "flip_y": false,
            "style": { "fill": "#112233", "stroke": null, "stroke_width": 2.0, "opacity": 1.0 },
            "params": { "glow": 3, "corner_radius": 4.5 },
            "name": null,
            "locked": false,
            "future": { "a": [1, 2, { "b": null }] }
          }]
        }],
        "active_layer": 0
      }],
      "active_page": 0
    }"##;

    fn objects_json(path: &std::path::Path) -> serde_json::Value {
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        v["pages"][0]["layers"][0]["objects"][0].clone()
    }

    #[test]
    fn shape_roundtrip_preserves_unknown_fields_and_params() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (
            dir.path().join("a.mindweave"),
            dir.path().join("b.mindweave"),
        );
        fs::write(&a, SHAPE_DOC).unwrap();
        let d = load(&a).unwrap();
        let s = &d.pages[0].layers[0].objects[0];
        assert_eq!(s.param("glow"), Some(3.0));
        assert_eq!(s.frame.x, 10.0);
        save(&d, &b).unwrap();
        let out = objects_json(&b);
        assert_eq!(out["future"], serde_json::json!({"a": [1, 2, {"b": null}]}));
        assert_eq!(out["params"]["glow"], 3);
        assert_eq!(out["params"]["corner_radius"], 4.5);
    }

    #[test]
    fn missing_library_shape_survives_save() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (
            dir.path().join("a.mindweave"),
            dir.path().join("b.mindweave"),
        );
        fs::write(&a, SHAPE_DOC).unwrap();
        save(&load(&a).unwrap(), &b).unwrap();
        let before: serde_json::Value = serde_json::from_str(SHAPE_DOC).unwrap();
        assert_eq!(
            objects_json(&b),
            before["pages"][0]["layers"][0]["objects"][0]
        );
    }

    #[test]
    fn v1_file_loads_and_saves_as_current() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (
            dir.path().join("a.mindweave"),
            dir.path().join("b.mindweave"),
        );
        let mut v: serde_json::Value = serde_json::from_str(SHAPE_DOC).unwrap();
        v["version"] = 1.into();
        v["pages"][0]["layers"][0]["objects"] = serde_json::json!([]);
        fs::write(&a, v.to_string()).unwrap();
        let d = load(&a).unwrap();
        assert_eq!(d.version, 7);
        save(&d, &b).unwrap();
        assert!(fs::read_to_string(&b).unwrap().contains("\"version\": 7"));
    }

    #[test]
    fn label_roundtrip_and_v2_file_loads() {
        use crate::shape::{Label, Shape, VAlign};
        use crate::text::{HAlign, RichText, RunStyle};
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (
            dir.path().join("a.mindweave"),
            dir.path().join("b.mindweave"),
        );
        // A version 2 file (no labels or text boxes) loads unchanged.
        fs::write(&a, SHAPE_DOC).unwrap();
        let mut d = load(&a).unwrap();
        assert_eq!(d.version, 7);
        let objects = &mut d.pages[0].layers[0].objects;
        assert_eq!(objects[0].label, None);
        assert_eq!(objects[0].text_box, None);
        assert_eq!(objects[0].extra.len(), 1);

        let bold = RunStyle {
            bold: true,
            font: Some("No Such Font".into()),
            color: Some("#FF0000".into()),
            ..RunStyle::default()
        };
        let mut text = RichText::new(RunStyle::default(), HAlign::Center);
        text.insert(0, "Hello\nworld", &RunStyle::default());
        text.insert(5, "!", &bold);
        objects[0].label = Some(Label {
            text: text.clone(),
            valign: VAlign::Bottom,
        });
        let mut tb = Shape::new_text_box((5.0, 6.0));
        tb.label.as_mut().unwrap().text = text;
        tb.text_box.as_mut().unwrap().auto_width = false;
        objects.push(tb);

        save(&d, &b).unwrap();
        let saved = fs::read_to_string(&b).unwrap();
        assert!(saved.contains("\"version\": 7"));
        let back = load(&b).unwrap();
        assert_eq!(back, d);
        let shapes = &back.pages[0].layers[0].objects;
        assert_eq!(shapes[0].extra.len(), 1, "label must not land in extra");
        assert!(shapes[1].extra.is_empty());
        assert_eq!(
            shapes[0].label.as_ref().unwrap().text.plain(),
            "Hello!\nworld"
        );
    }

    /// A document with one central topic, as the notes tests start from.
    fn doc_with_topic() -> Document {
        use crate::mindmap::TopicProps;
        use crate::shape::Shape;
        let mut d = Document::new();
        let mut shape = Shape::new_topic(crate::geometry::Frame {
            x: 0.0,
            y: 0.0,
            w: 140.0,
            h: 48.0,
            rotation: 0.0,
            ..Default::default()
        });
        let id = shape.id;
        shape.topic = Some(TopicProps::central(id));
        d.pages[0].layers[0].objects.push(shape);
        d
    }

    #[test]
    fn notes_and_equations_round_trip() {
        use crate::mindmap::Equation;
        use crate::text::{HAlign, RichText, RunStyle};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let mut d = doc_with_topic();
        let mut note = RichText::new(RunStyle::default(), HAlign::Left);
        note.insert(0, "Remember", &RunStyle::default());
        let topic = d.pages[0].layers[0].objects[0].topic.as_mut().unwrap();
        topic.note = Some(note);
        topic.equation = Some(Equation {
            latex: r"\ce{H2O}".into(),
            size: (40.0, 18.0),
        });
        save(&d, &path).unwrap();
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("\"version\": 7"));
        assert_eq!(load(&path).unwrap(), d);
    }

    #[test]
    fn version_4_files_load_without_notes() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (
            dir.path().join("a.mindweave"),
            dir.path().join("b.mindweave"),
        );
        let mut v = serde_json::to_value(doc_with_topic()).unwrap();
        v["version"] = 4.into();
        fs::write(&a, v.to_string()).unwrap();
        let d = load(&a).unwrap();
        let topic = d.pages[0].layers[0].objects[0].topic.as_ref().unwrap();
        assert_eq!((topic.note.as_ref(), topic.equation.as_ref()), (None, None));
        save(&d, &b).unwrap();
        assert!(fs::read_to_string(&b).unwrap().contains("\"version\": 7"));
    }

    #[test]
    fn v5_files_load_and_save_as_v7() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (
            dir.path().join("a.mindweave"),
            dir.path().join("b.mindweave"),
        );
        let mut v = serde_json::to_value(doc_with_topic()).unwrap();
        v["version"] = 5.into();
        fs::write(&a, v.to_string()).unwrap();
        let d = load(&a).unwrap();
        assert_eq!(d.version, 7);
        let shape = &d.pages[0].layers[0].objects[0];
        assert_eq!(
            (shape.artboard, shape.artboard_props.as_ref()),
            (None, None)
        );
        save(&d, &b).unwrap();
        let saved = fs::read_to_string(&b).unwrap();
        assert!(saved.contains("\"version\": 7"));
        assert!(!saved.contains("artboard"));
    }

    #[test]
    fn v6_files_load_and_save_as_v7() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (
            dir.path().join("a.mindweave"),
            dir.path().join("b.mindweave"),
        );
        let mut v = serde_json::to_value(doc_with_topic()).unwrap();
        v["version"] = 6.into();
        fs::write(&a, v.to_string()).unwrap();
        let mut d = load(&a).unwrap();
        assert_eq!(d.version, 7);
        save(&d, &b).unwrap();
        assert!(fs::read_to_string(&b).unwrap().contains("\"version\": 7"));
        // Ink and hollow heads survive a save and load.
        let shape = &mut d.pages[0].layers[0].objects[0];
        shape.style.fill = Some(crate::INK_COLOR.to_string());
        shape.style.stroke = Some(crate::INK_COLOR.to_string());
        shape.line = Some(crate::LineProps {
            start_head: crate::ArrowHead::HollowDiamond,
            end_head: crate::ArrowHead::HollowTriangle,
            ..crate::LineProps::default()
        });
        save(&d, &b).unwrap();
        let back = load(&b).unwrap();
        let shape = &back.pages[0].layers[0].objects[0];
        assert_eq!(shape.style.fill.as_deref(), Some("ink"));
        assert_eq!(shape.style.stroke.as_deref(), Some("ink"));
        let props = shape.line.as_ref().unwrap();
        assert_eq!(
            (props.start_head, props.end_head),
            (
                crate::ArrowHead::HollowDiamond,
                crate::ArrowHead::HollowTriangle
            )
        );
    }

    #[test]
    fn artboard_round_trips() {
        use crate::artboard::ArtboardProps;
        use crate::shape::Shape;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mindweave");
        let mut d = doc_with_topic();
        let mut board = Shape::new_artboard(
            crate::geometry::Frame {
                x: -10.0,
                y: 20.0,
                w: 393.0,
                h: 852.0,
                ..Default::default()
            },
            "iPhone 16",
        );
        board.artboard_props = Some(ArtboardProps { clip: false });
        let objects = &mut d.pages[0].layers[0].objects;
        objects[0].artboard = Some(board.id);
        objects.insert(0, board.clone());
        save(&d, &path).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back, d);
        let shapes = &back.pages[0].layers[0].objects;
        assert!(shapes[0].is_artboard());
        assert_eq!(shapes[0].name.as_deref(), Some("iPhone 16"));
        assert_eq!(shapes[1].artboard, Some(board.id));
        assert!(shapes.iter().all(|s| s.extra.is_empty()));
    }
}
