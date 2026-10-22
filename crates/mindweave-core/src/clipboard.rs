//! Clipboard payloads for shapes, plain-text paste, and duplicating shapes.
//!
//! Shapes (with their labels, and text boxes) are copied as JSON text:
//! `{"mindweave":"shapes","version":3,"shapes":[...],"images":{...}}`,
//! each picture stored once in `images` (see [`crate::image`]). Version 1
//! payloads (shapes without text) and 2 (without images) are still read. Clipboard text comes from any
//! application, so reading it never panics and rejects anything that is
//! not such a payload.

use serde::Serialize;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use crate::geometry::{Frame, Pt, Rect};
use crate::image::{collect_images, resolve_images, ImageStore};
use crate::shape::Shape;
use crate::text::RunStyle;
use crate::transform::{selection_bounds, DUPLICATE_OFFSET};

/// Value of the `"mindweave"` key that marks a shapes payload.
pub const CLIPBOARD_MARKER: &str = "shapes";
/// Payload version this build writes; it reads this and every older one.
pub const CLIPBOARD_VERSION: u64 = 3;
/// Oldest payload version this build reads.
const OLDEST_CLIPBOARD_VERSION: u64 = 1;
/// Most characters (graphemes) of plain text pasted into a new text box.
pub const MAX_PASTE_CHARS: usize = 100_000;
/// Most shapes accepted from one payload.
pub const MAX_CLIPBOARD_SHAPES: usize = 10_000;
/// Longest clipboard text parsed, in bytes.
pub const MAX_CLIPBOARD_BYTES: usize = 64 * 1024 * 1024;
/// The `"mindweave"` key must start within this many bytes of the opening
/// brace, so other JSON is rejected without being parsed.
const MARKER_WINDOW: usize = 64;

/// `shapes` as clipboard text.
///
/// Shapes are normalised first: JSON has no NaN or infinity, so a
/// non-finite value would otherwise make the payload unreadable.
pub fn to_clipboard(shapes: &[Shape]) -> String {
    #[derive(Serialize)]
    struct Payload<'a> {
        mindweave: &'a str,
        version: u64,
        shapes: &'a [Shape],
        #[serde(skip_serializing_if = "ImageStore::is_empty")]
        images: ImageStore,
    }
    let shapes: Vec<Shape> = shapes
        .iter()
        .map(|s| {
            let mut s = s.clone();
            s.normalize();
            s
        })
        .collect();
    let payload = Payload {
        mindweave: CLIPBOARD_MARKER,
        version: CLIPBOARD_VERSION,
        images: collect_images(&shapes),
        shapes: &shapes,
    };
    // Serialising plain data to a string does not fail; an empty string
    // would be rejected by `from_clipboard` rather than panic.
    serde_json::to_string(&payload).unwrap_or_default()
}

/// The shapes in clipboard `text`, normalised, or `None` when `text` is
/// not a shapes payload of a version this build reads, is too large, holds more than
/// [`MAX_CLIPBOARD_SHAPES`] shapes, or a shape does not parse.
pub fn from_clipboard(text: &str) -> Option<Vec<Shape>> {
    if text.len() > MAX_CLIPBOARD_BYTES || !looks_like_payload(text) {
        return None;
    }
    // serde_json limits nesting depth, so hostile input cannot overflow
    // the stack.
    let mut payload: Value = serde_json::from_str(text).ok()?;
    let obj = payload.as_object_mut()?;
    if obj.get("mindweave")?.as_str()? != CLIPBOARD_MARKER
        || !(OLDEST_CLIPBOARD_VERSION..=CLIPBOARD_VERSION).contains(&obj.get("version")?.as_u64()?)
    {
        return None;
    }
    let items = match obj.remove("shapes")? {
        Value::Array(items) if items.len() <= MAX_CLIPBOARD_SHAPES => items,
        _ => return None,
    };
    let images: ImageStore = match obj.remove("images") {
        Some(images) => serde_json::from_value(images).ok()?,
        None => ImageStore::new(),
    };
    let mut shapes: Vec<Shape> = items
        .into_iter()
        .map(|item| serde_json::from_value(item).ok())
        .collect::<Option<_>>()?;
    resolve_images(&mut shapes, &images);
    shapes.iter_mut().for_each(Shape::normalize);
    Some(shapes)
}

/// A new text box at `at` holding plain `text` (from another application)
/// in the default style, cut to its first [`MAX_PASTE_CHARS`] graphemes;
/// `None` when `text` is blank.
pub fn plain_text_box(text: &str, at: Pt) -> Option<Shape> {
    if text.trim().is_empty() {
        return None;
    }
    let end = text
        .grapheme_indices(true)
        .nth(MAX_PASTE_CHARS)
        .map_or(text.len(), |(i, _)| i);
    let mut shape = Shape::new_text_box(at);
    if let Some(label) = &mut shape.label {
        label.text.insert(0, &text[..end], &RunStyle::default());
    }
    Some(shape)
}

/// Whether `text` is Mindweave clipboard JSON (a JSON object with the
/// `"mindweave"` key near its start), readable by this build or not. Text
/// that is but that [`from_clipboard`] rejects (newer version, too large,
/// malformed) must not be pasted as plain text.
pub fn is_mindweave_payload(text: &str) -> bool {
    looks_like_payload(text)
}

/// Whether `text` is a shapes payload of a version newer than this build
/// reads. Text too large to read is not checked.
pub fn is_newer_payload(text: &str) -> bool {
    if text.len() > MAX_CLIPBOARD_BYTES || !looks_like_payload(text) {
        return false;
    }
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    obj.get("mindweave").and_then(Value::as_str) == Some(CLIPBOARD_MARKER)
        && obj
            .get("version")
            .and_then(Value::as_u64)
            .is_some_and(|v| v > CLIPBOARD_VERSION)
}

/// Whether `text` starts like a shapes payload: a JSON object with the
/// `"mindweave"` key near its start. Cheap, so foreign text is not parsed.
fn looks_like_payload(text: &str) -> bool {
    const KEY: &[u8] = b"\"mindweave\"";
    let text = text.trim_start().as_bytes();
    let start = &text[..text.len().min(MARKER_WINDOW + KEY.len())];
    text.first() == Some(&b'{') && start.windows(KEY.len()).any(|w| w == KEY)
}

/// `shapes` ready to paste into `view` (the visible canvas area): fresh
/// ids, unlocked (nothing can unlock a shape yet), kept in place if their bounds intersect `view`, otherwise moved
/// so their bounds are centred in `view`.
pub fn prepare_paste(mut shapes: Vec<Shape>, view: &Rect) -> Vec<Shape> {
    let frames: Vec<Frame> = shapes.iter().map(|s| s.frame).collect();
    let (dx, dy) = match selection_bounds(&frames) {
        Some(b) if !intersects(&b, view) => {
            let (bx, by) = b.center();
            let (vx, vy) = view.center();
            (vx - bx, vy - by)
        }
        _ => (0.0, 0.0),
    };
    for s in &mut shapes {
        s.locked = false;
        s.frame.x += dx;
        s.frame.y += dy;
    }
    renew_ids(&mut shapes);
    shapes
}

/// Gives `shapes` fresh ids, and their groups too, so copies form groups
/// of their own. Line ends attached to one of `shapes` follow it to its
/// new id; ends attached to anything else are let go.
fn renew_ids(shapes: &mut [Shape]) {
    let ids: std::collections::HashMap<Uuid, Uuid> =
        shapes.iter().map(|s| (s.id, Uuid::new_v4())).collect();
    let mut groups: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
    for s in shapes.iter_mut() {
        s.id = ids[&s.id];
        for g in &mut s.groups {
            *g = *groups.entry(*g).or_insert_with(Uuid::new_v4);
        }
        if let Some(line) = &mut s.line {
            line.start = line.start.and_then(|id| ids.get(&id).copied());
            line.end = line.end.and_then(|id| ids.get(&id).copied());
        }
        // Topics keep their tree among the copies; a topic whose parent
        // was not copied becomes the central topic of a map of its own.
        if let Some(t) = &mut s.topic {
            match t.parent.and_then(|p| ids.get(&p).copied()) {
                Some(parent) => {
                    t.parent = Some(parent);
                    t.map = ids.get(&t.map).copied().unwrap_or(t.map);
                }
                None => {
                    t.parent = None;
                    t.map = s.id;
                    t.summary = None;
                    t.branch.get_or_insert_with(Default::default);
                }
            }
            t.summary = t
                .summary
                .and_then(|(a, b)| Some((*ids.get(&a)?, *ids.get(&b)?)));
        }
    }
}

/// Whether `a` and `b` overlap or touch.
fn intersects(a: &Rect, b: &Rect) -> bool {
    a.x <= b.x + b.w && b.x <= a.x + a.w && a.y <= b.y + b.h && b.y <= a.y + a.h
}

/// Copies of `shapes` with fresh ids, offset by
/// [`DUPLICATE_OFFSET`] on both axes.
pub fn duplicate(shapes: &[Shape]) -> Vec<Shape> {
    let mut copies: Vec<Shape> = shapes
        .iter()
        .map(|s| {
            let mut d = s.clone();
            d.frame.x += DUPLICATE_OFFSET;
            d.frame.y += DUPLICATE_OFFSET;
            d
        })
        .collect();
    renew_ids(&mut copies);
    copies
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::RunStyle;

    #[test]
    fn copies_of_a_group_form_a_new_group() {
        let (mut a, mut b) = (shape(0., 0., 10., 10.), shape(20., 0., 10., 10.));
        let g = Uuid::new_v4();
        a.groups = vec![g];
        b.groups = vec![g];
        let copies = duplicate(&[a, b]);
        assert_ne!(copies[0].groups, vec![g]);
        assert_eq!(copies[0].groups, copies[1].groups);
        assert_eq!(copies[0].groups.len(), 1);
    }

    #[test]
    fn copied_images_carry_each_picture_once() {
        let pic = crate::image::ImageData::from_file_bytes(b"GIF89a-copied".to_vec()).unwrap();
        let shapes: Vec<Shape> = (0..3)
            .map(|_| Shape::new_image(pic.clone(), Frame::default()))
            .collect();
        let text = to_clipboard(&shapes);
        assert_eq!(text.matches("R0lGODlhLWNvcGllZA==").count(), 1);
        let back = from_clipboard(&text).unwrap();
        assert_eq!(back, shapes);
    }

    #[test]
    fn copied_lines_stay_attached_only_to_copied_shapes() {
        let (a, b) = (shape(0., 0., 10., 10.), shape(50., 0., 10., 10.));
        let mut line = shape(10., 5., 40., 0.);
        line.line = Some(crate::line::LineProps {
            start: Some(a.id),
            end: Some(b.id),
            ..Default::default()
        });
        for copies in [
            duplicate(&[a.clone(), line.clone()]),
            prepare_paste(vec![a.clone(), line.clone()], &view()),
        ] {
            let props = copies[1].line.clone().unwrap();
            assert_eq!(props.start, Some(copies[0].id));
            assert_ne!(copies[0].id, a.id);
            assert_eq!(props.end, None);
        }
    }

    #[test]
    fn mindweave_payloads_are_recognised_even_when_unreadable() {
        let current = to_clipboard(&[Shape::new_text_box((0., 0.))]);
        assert!(is_mindweave_payload(&current));
        assert!(!is_newer_payload(&current));
        let newer = current.replace("\"version\":3", "\"version\":99");
        assert!(from_clipboard(&newer).is_none());
        assert!(is_mindweave_payload(&newer));
        assert!(is_newer_payload(&newer));
        // Too large to read: still a payload, not known to be newer.
        let big = format!(
            "{{\"mindweave\":\"shapes\",\"version\":3,\"pad\":\"{}\"}}",
            "x".repeat(MAX_CLIPBOARD_BYTES)
        );
        assert!(from_clipboard(&big).is_none());
        assert!(is_mindweave_payload(&big));
        assert!(!is_newer_payload(&big));
        // Other text is not.
        assert!(!is_mindweave_payload("hello"));
        assert!(!is_mindweave_payload("{\"other\":1}"));
        assert!(!is_newer_payload("hello"));
    }

    fn shape(x: f32, y: f32, w: f32, h: f32) -> Shape {
        let def = crate::library::builtin_libraries()[0].shapes[0].clone();
        let frame = Frame {
            x,
            y,
            w,
            h,
            ..Default::default()
        };
        Shape::new(&def, "basic-shapes", frame)
    }

    fn view() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        }
    }

    #[test]
    fn clipboard_roundtrip() {
        let mut a = shape(10.0, 20.0, 30.0, 40.0);
        a.frame.rotation = 45.0;
        a.name = Some("A".into());
        a.extra.insert("future".into(), 1.into());
        let shapes = vec![a, shape(-5.0, 0.0, 1.0, 0.0)];
        let text = to_clipboard(&shapes);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mindweave"], "shapes");
        assert_eq!(v["version"], 3);
        assert_eq!(v["shapes"].as_array().unwrap().len(), 2);
        assert_eq!(from_clipboard(&text), Some(shapes));
        assert_eq!(from_clipboard(&to_clipboard(&[])), Some(vec![]));
    }

    #[test]
    fn from_clipboard_rejects_foreign_text() {
        let good = to_clipboard(&[shape(0.0, 0.0, 10.0, 10.0)]);
        let truncated = &good[..good.len() / 2];
        let version_3 = good.replace("\"version\":3", "\"version\":4");
        assert_ne!(version_3, good);
        let version_0 = good.replace("\"version\":3", "\"version\":0");
        let other_marker = good.replace("\"mindweave\":\"shapes\"", "\"mindweave\":\"pages\"");
        assert_ne!(other_marker, good);
        for text in [
            "",
            "hello world",
            "{}",
            "[1,2,3]",
            r#"{"foo":"bar"}"#,
            r#"{"mindweave":"shapes","version":1}"#,
            r#"{"mindweave":"shapes","version":1,"shapes":{}}"#,
            r#"{"mindweave":"shapes","version":"1","shapes":[]}"#,
            r#"{"mindweave":"shapes","version":1,"shapes":[{"id":"nope"}]}"#,
            truncated,
            &version_3,
            &version_0,
            &other_marker,
        ] {
            assert_eq!(from_clipboard(text), None, "accepted {text:?}");
        }
    }

    #[test]
    fn clipboard_v2_roundtrip_with_labels() {
        use crate::shape::{Label, VAlign};
        use crate::text::{HAlign, RichText, RunStyle};
        let mut text = RichText::new(RunStyle::default(), HAlign::Right);
        let red = RunStyle {
            color: Some("#FF0000".into()),
            italic: true,
            ..RunStyle::default()
        };
        text.insert(0, "one\ntwo", &red);
        let mut a = shape(0.0, 0.0, 10.0, 10.0);
        a.label = Some(Label {
            text: text.clone(),
            valign: VAlign::Top,
        });
        let mut tb = Shape::new_text_box((3.0, 4.0));
        tb.label.as_mut().unwrap().text = text;
        tb.text_box.as_mut().unwrap().auto_width = false;
        let shapes = vec![a, tb];
        let payload = to_clipboard(&shapes);
        assert!(payload.starts_with(r#"{"mindweave":"shapes","version":3,"#));
        let back = from_clipboard(&payload).unwrap();
        assert_eq!(back, shapes);
        assert!(back.iter().all(|s| s.extra.is_empty()));
        assert!(back[1].is_text_box());
    }

    #[test]
    fn clipboard_v1_still_accepted() {
        let a = shape(1.0, 2.0, 3.0, 4.0);
        let one = serde_json::to_string(&a).unwrap();
        let v1 = format!(r#"{{"mindweave":"shapes","version":1,"shapes":[{one}]}}"#);
        assert_eq!(from_clipboard(&v1), Some(vec![a]));
    }

    #[test]
    fn plain_paste_is_capped() {
        let at = (7.0, 8.0);
        assert_eq!(plain_text_box("", at), None);
        assert_eq!(plain_text_box(" \n\t \r\n", at), None);
        let tb = plain_text_box("Hello\r\nworld", at).unwrap();
        assert!(tb.is_text_box());
        assert_eq!((tb.frame.x, tb.frame.y), at);
        let label = tb.label.as_ref().unwrap();
        assert_eq!(label.text.plain(), "Hello\nworld");
        assert_eq!(label.text.style_at(0), RunStyle::default());
        assert!(tb.text_box.unwrap().auto_width);

        // Capped in graphemes: a letter with a combining accent counts
        // once and is never split.
        let unit = "e\u{301}";
        let long = unit.repeat(MAX_PASTE_CHARS + 5);
        let tb = plain_text_box(&long, at).unwrap();
        let text = tb.label.unwrap().text.plain();
        assert_eq!(text, unit.repeat(MAX_PASTE_CHARS));
        assert_eq!(MAX_PASTE_CHARS, 100_000);
    }

    #[test]
    fn from_clipboard_enforces_limits() {
        let one = serde_json::to_string(&shape(0.0, 0.0, 10.0, 10.0)).unwrap();
        let payload = |n: usize| {
            format!(
                r#"{{"mindweave":"shapes","version":2,"shapes":[{}]}}"#,
                vec![one.as_str(); n].join(",")
            )
        };
        assert_eq!(
            from_clipboard(&payload(MAX_CLIPBOARD_SHAPES)).map(|s| s.len()),
            Some(MAX_CLIPBOARD_SHAPES)
        );
        assert_eq!(from_clipboard(&payload(MAX_CLIPBOARD_SHAPES + 1)), None);
        let mut huge = to_clipboard(&[]);
        huge.push_str(&" ".repeat(MAX_CLIPBOARD_BYTES));
        assert_eq!(from_clipboard(&huge), None);
        // Room for a few embedded images.
        assert_eq!(MAX_CLIPBOARD_BYTES, 64 * 1024 * 1024);
    }

    #[test]
    fn from_clipboard_checks_the_start_before_parsing() {
        // Whitespace around a payload is fine; the marker may follow a
        // little later than in our own payloads.
        let good = to_clipboard(&[shape(0.0, 0.0, 10.0, 10.0)]);
        assert!(from_clipboard(&format!("\n  {good}  \n")).is_some());
        let reordered = good.replacen(
            r#"{"mindweave":"shapes","version":3,"#,
            r#"{"version":2,"mindweave":"shapes","#,
            1,
        );
        assert_ne!(reordered, good);
        assert!(from_clipboard(&reordered).is_some());
        // Large JSON that does not start like a payload is never parsed:
        // here it would be accepted if it were.
        let late = format!(r#"{{"padding":"{}",{}"#, "x".repeat(64), &good[1..]);
        assert_eq!(from_clipboard(&late), None);
        assert_eq!(from_clipboard(&format!("[{good}]")), None);
    }

    #[test]
    fn from_clipboard_normalises_shapes() {
        let mut s = shape(0.0, 0.0, 10.0, 10.0);
        s.frame.w = -5.0;
        s.frame.rotation = -90.0;
        s.style.opacity = 7.0;
        s.style.fill = Some("ff0000".into());
        let got = from_clipboard(&to_clipboard(&[s])).unwrap();
        let f = got[0].frame;
        assert_eq!((f.w, f.rotation), (0.0, 270.0));
        assert_eq!(got[0].style.opacity, 1.0);
        assert_eq!(got[0].style.fill.as_deref(), Some("#FF0000"));
    }

    #[test]
    fn paste_keeps_position_when_visible() {
        // Partly visible: overlaps the view's right edge.
        let shapes = vec![
            shape(790.0, 100.0, 50.0, 50.0),
            shape(900.0, 100.0, 5.0, 5.0),
        ];
        let pasted = prepare_paste(shapes.clone(), &view());
        let frames = |s: &[Shape]| s.iter().map(|s| s.frame).collect::<Vec<_>>();
        assert_eq!(frames(&pasted), frames(&shapes));
    }

    #[test]
    fn paste_centres_when_offscreen() {
        let shapes = vec![
            shape(2000.0, 3000.0, 100.0, 50.0),
            shape(2200.0, 3100.0, 100.0, 50.0),
        ];
        let pasted = prepare_paste(shapes, &view());
        let frames: Vec<Frame> = pasted.iter().map(|s| s.frame).collect();
        let b = selection_bounds(&frames).unwrap();
        assert_eq!(b.center(), view().center());
        // Relative layout is kept.
        assert_eq!(frames[1].x - frames[0].x, 200.0);
        assert_eq!(frames[1].y - frames[0].y, 100.0);
        assert_eq!((frames[0].x, frames[0].y), (250.0, 225.0));
        assert!(prepare_paste(vec![], &view()).is_empty());
    }

    #[test]
    fn paste_and_duplicate_assign_new_ids() {
        let shapes = vec![shape(0.0, 0.0, 10.0, 10.0), shape(5.0, 5.0, 10.0, 10.0)];
        let old: Vec<_> = shapes.iter().map(|s| s.id).collect();
        let pasted = prepare_paste(shapes.clone(), &view());
        let duped = duplicate(&shapes);
        for out in [&pasted, &duped] {
            assert_eq!(out.len(), 2);
            assert!(out.iter().all(|s| !old.contains(&s.id)));
            assert_ne!(out[0].id, out[1].id);
        }
        assert!(pasted.iter().all(|p| duped.iter().all(|d| d.id != p.id)));
        // Everything but the id is kept by paste.
        let mut p = pasted[0].clone();
        p.id = shapes[0].id;
        assert_eq!(p, shapes[0]);
    }

    #[test]
    fn pasted_shapes_are_unlocked() {
        // There is no way to unlock a shape yet, so a pasted locked shape
        // could never be selected.
        let mut a = shape(0.0, 0.0, 10.0, 10.0);
        a.locked = true;
        let pasted = prepare_paste(vec![a], &view());
        assert!(!pasted[0].locked);
    }

    #[test]
    fn duplicate_offsets_by_10() {
        let mut a = shape(10.0, 20.0, 30.0, 40.0);
        a.frame.rotation = 30.0;
        a.frame.flip_x = true;
        let d = duplicate(std::slice::from_ref(&a));
        assert_eq!(
            d[0].frame,
            Frame {
                x: 20.0,
                y: 30.0,
                ..a.frame
            }
        );
        let mut back = d[0].clone();
        back.id = a.id;
        back.frame = a.frame;
        assert_eq!(back, a);
        assert!(duplicate(&[]).is_empty());
    }
}
