//! Clipboard payloads for shapes, and duplicating shapes.
//!
//! Shapes are copied as JSON text:
//! `{"mindweave":"shapes","version":1,"shapes":[...]}`. Clipboard text
//! comes from any application, so reading it never panics and rejects
//! anything that is not such a payload.

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::geometry::{Frame, Rect};
use crate::shape::Shape;
use crate::transform::{selection_bounds, DUPLICATE_OFFSET};

/// Value of the `"mindweave"` key that marks a shapes payload.
pub const CLIPBOARD_MARKER: &str = "shapes";
/// Payload version this build reads and writes.
pub const CLIPBOARD_VERSION: u64 = 1;
/// Most shapes accepted from one payload.
pub const MAX_CLIPBOARD_SHAPES: usize = 10_000;
/// Longest clipboard text parsed, in bytes.
pub const MAX_CLIPBOARD_BYTES: usize = 50 * 1024 * 1024;

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
        shapes: &shapes,
    };
    // Serialising plain data to a string does not fail; an empty string
    // would be rejected by `from_clipboard` rather than panic.
    serde_json::to_string(&payload).unwrap_or_default()
}

/// The shapes in clipboard `text`, normalised, or `None` when `text` is
/// not a version 1 shapes payload, is too large, holds more than
/// [`MAX_CLIPBOARD_SHAPES`] shapes, or a shape does not parse.
pub fn from_clipboard(text: &str) -> Option<Vec<Shape>> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return None;
    }
    // serde_json limits nesting depth, so hostile input cannot overflow
    // the stack.
    let mut payload: Value = serde_json::from_str(text).ok()?;
    let obj = payload.as_object_mut()?;
    if obj.get("mindweave")?.as_str()? != CLIPBOARD_MARKER
        || obj.get("version")?.as_u64()? != CLIPBOARD_VERSION
    {
        return None;
    }
    let items = match obj.remove("shapes")? {
        Value::Array(items) if items.len() <= MAX_CLIPBOARD_SHAPES => items,
        _ => return None,
    };
    items
        .into_iter()
        .map(|item| {
            let mut s: Shape = serde_json::from_value(item).ok()?;
            s.normalize();
            Some(s)
        })
        .collect()
}

/// `shapes` ready to paste into `view` (the visible canvas area): fresh
/// ids, kept in place if their bounds intersect `view`, otherwise moved
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
        s.id = Uuid::new_v4();
        s.frame.x += dx;
        s.frame.y += dy;
    }
    shapes
}

/// Whether `a` and `b` overlap or touch.
fn intersects(a: &Rect, b: &Rect) -> bool {
    a.x <= b.x + b.w && b.x <= a.x + a.w && a.y <= b.y + b.h && b.y <= a.y + a.h
}

/// Copies of `shapes` with fresh ids, offset by
/// [`DUPLICATE_OFFSET`] on both axes.
pub fn duplicate(shapes: &[Shape]) -> Vec<Shape> {
    shapes
        .iter()
        .map(|s| {
            let mut d = s.clone();
            d.id = Uuid::new_v4();
            d.frame.x += DUPLICATE_OFFSET;
            d.frame.y += DUPLICATE_OFFSET;
            d
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(v["version"], 1);
        assert_eq!(v["shapes"].as_array().unwrap().len(), 2);
        assert_eq!(from_clipboard(&text), Some(shapes));
        assert_eq!(from_clipboard(&to_clipboard(&[])), Some(vec![]));
    }

    #[test]
    fn from_clipboard_rejects_foreign_text() {
        let good = to_clipboard(&[shape(0.0, 0.0, 10.0, 10.0)]);
        let truncated = &good[..good.len() / 2];
        let version_2 = good.replace("\"version\":1", "\"version\":2");
        assert_ne!(version_2, good);
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
            &version_2,
            &other_marker,
        ] {
            assert_eq!(from_clipboard(text), None, "accepted {text:?}");
        }
    }

    #[test]
    fn from_clipboard_enforces_limits() {
        let one = serde_json::to_string(&shape(0.0, 0.0, 10.0, 10.0)).unwrap();
        let payload = |n: usize| {
            format!(
                r#"{{"mindweave":"shapes","version":1,"shapes":[{}]}}"#,
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
