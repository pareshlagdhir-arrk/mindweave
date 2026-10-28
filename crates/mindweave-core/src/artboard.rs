//! Artboards: named frames on the canvas (library
//! [`crate::shape::TEXT_LIBRARY`], shape [`ARTBOARD_SHAPE`]) that hold and
//! clip the shapes drawn inside them, like Figma frames.
//!
//! Every other shape records the artboard it belongs to in
//! [`Shape::artboard`]. After a shape is placed, moved, resized, pasted or
//! dropped, [`membership`] decides it: the topmost artboard of the layer
//! whose frame holds the shape's bounding-box centre (a line's: the middle
//! of its ends), else none. Groups are decided as a whole, and a mind map
//! always shares its central topic's membership. Creating, moving or
//! resizing an artboard never changes other shapes' membership.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::arrange::unit_of;
use crate::geometry::{line_endpoints, Pt, Rect};
use crate::shape::Shape;
use crate::transform::{frame_aabb, selection_bounds};

/// Shape id of an artboard in [`crate::shape::TEXT_LIBRARY`].
pub const ARTBOARD_SHAPE: &str = "artboard";
/// Space between the rightmost artboard and a new one placed beside it.
pub const ARTBOARD_GAP: f32 = 80.0;

fn yes() -> bool {
    true
}

/// What makes a shape an artboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtboardProps {
    /// Members are painted clipped to the artboard's frame.
    #[serde(default = "yes")]
    pub clip: bool,
}

impl Default for ArtboardProps {
    fn default() -> Self {
        ArtboardProps { clip: true }
    }
}

/// The group of the Artboard ribbon tab a preset is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetCategory {
    /// Phones and tablets.
    Device,
    /// Desktop and web.
    Desktop,
    /// Paper and slides.
    Paper,
    /// Social media.
    Social,
}

/// A ready-made artboard size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArtboardPreset {
    /// Shown on its button and given to new artboards.
    pub name: &'static str,
    /// Width and height in canvas units (paper sizes in points).
    pub size: (f32, f32),
    pub category: PresetCategory,
    /// Lucide icon name of its button.
    pub icon: &'static str,
}

/// Shorthand for the table below.
const fn preset(
    name: &'static str,
    w: f32,
    h: f32,
    category: PresetCategory,
    icon: &'static str,
) -> ArtboardPreset {
    ArtboardPreset {
        name,
        size: (w, h),
        category,
        icon,
    }
}

/// The built-in presets, in ribbon order.
pub const ARTBOARD_PRESETS: &[ArtboardPreset] = {
    use PresetCategory::*;
    &[
        preset("iPhone 16", 393.0, 852.0, Device, "smartphone"),
        preset("iPhone 16 Pro Max", 440.0, 956.0, Device, "smartphone"),
        preset("Android", 412.0, 917.0, Device, "smartphone"),
        preset("iPad", 820.0, 1180.0, Device, "tablet"),
        preset("Android tablet", 800.0, 1280.0, Device, "tablet"),
        preset("Desktop", 1440.0, 1024.0, Desktop, "monitor"),
        preset("MacBook", 1512.0, 982.0, Desktop, "laptop"),
        preset("HD screen", 1920.0, 1080.0, Desktop, "monitor"),
        preset("Web page", 1280.0, 3000.0, Desktop, "monitor"),
        preset("A4", 595.0, 842.0, Paper, "file-text"),
        preset("A4 landscape", 842.0, 595.0, Paper, "file-text"),
        preset("A3", 842.0, 1191.0, Paper, "file-text"),
        preset("Letter", 612.0, 792.0, Paper, "file-text"),
        preset("Slide 16:9", 1920.0, 1080.0, Paper, "presentation"),
        preset("Slide 4:3", 1024.0, 768.0, Paper, "presentation"),
        preset("Square post", 1080.0, 1080.0, Social, "image"),
        preset("Story", 1080.0, 1920.0, Social, "image"),
        preset("Video thumbnail", 1280.0, 720.0, Social, "square-play"),
        preset("Banner", 1584.0, 396.0, Social, "rectangle-horizontal"),
    ]
};

/// The point that decides a lone shape's membership: a line's middle, else
/// the centre of its bounding box.
fn anchor(shape: &Shape) -> Pt {
    if shape.is_line() {
        let (a, b) = line_endpoints(&shape.frame);
        return ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    }
    frame_aabb(&shape.frame).center()
}

/// The topmost of `boards` (back to front) whose frame holds `p`.
fn board_at(boards: &[(Uuid, Rect)], p: Pt) -> Option<Uuid> {
    boards
        .iter()
        .rev()
        .find(|(_, r)| r.contains(p))
        .map(|(id, _)| *id)
}

/// The central topic of the mind map `shape` is part of: a topic's map, a
/// boundary's (from its first member), a relationship's (from the topic
/// at either end); `None` for other shapes. `maps` gives each topic's map.
fn map_of(shape: &Shape, maps: &HashMap<Uuid, Uuid>) -> Option<Uuid> {
    if let Some(t) = &shape.topic {
        return Some(t.map);
    }
    if let Some(b) = &shape.boundary {
        return b.members.iter().find_map(|m| maps.get(m).copied());
    }
    let line = shape.line.as_ref().filter(|l| l.relationship)?;
    [line.start, line.end]
        .into_iter()
        .flatten()
        .find_map(|e| maps.get(&e).copied())
}

/// The membership the shapes `moved` (of one layer's `shapes`, back to
/// front) get by the artboard rule, as `(shape, artboard)` in layer order.
/// Artboards are skipped. A grouped shape brings its whole outermost
/// group, decided by the centre of the group's bounds. A mind map shape
/// brings its whole map, which follows its central topic: by the rule when
/// the central topic is among `moved`, else the central topic's current
/// membership (a map without its central topic is decided shape by shape).
pub fn membership(shapes: &[Shape], moved: &[Uuid]) -> Vec<(Uuid, Option<Uuid>)> {
    let boards: Vec<(Uuid, Rect)> = shapes
        .iter()
        .filter(|s| s.is_artboard())
        .map(|s| (s.id, frame_aabb(&s.frame)))
        .collect();
    let maps: HashMap<Uuid, Uuid> = shapes
        .iter()
        .filter_map(|s| Some((s.id, s.topic.as_ref()?.map)))
        .collect();
    let by_id: HashMap<Uuid, &Shape> = shapes.iter().map(|s| (s.id, s)).collect();
    // A map is whole only while its central topic is there.
    let map_of_shape = |s: &Shape| map_of(s, &maps).filter(|m| by_id.contains_key(m));
    let picked: HashSet<Uuid> = moved.iter().copied().collect();
    let mut units: HashSet<Uuid> = HashSet::new();
    let mut picked_maps: HashSet<Uuid> = HashSet::new();
    for s in shapes.iter().filter(|s| picked.contains(&s.id)) {
        if s.is_artboard() {
            continue;
        }
        match map_of_shape(s) {
            Some(m) => picked_maps.insert(m),
            None => units.insert(unit_of(s)),
        };
    }
    // Each map's membership: its central topic's.
    let map_board: HashMap<Uuid, Option<Uuid>> = picked_maps
        .iter()
        .map(|&m| {
            let central = by_id[&m];
            let board = if picked.contains(&m) {
                board_at(&boards, anchor(central))
            } else {
                central.artboard
            };
            (m, board)
        })
        .collect();
    // Each unit's membership: a lone shape's anchor, a group's centre.
    let mut unit_frames: HashMap<Uuid, Vec<&Shape>> = HashMap::new();
    for s in shapes
        .iter()
        .filter(|s| !s.is_artboard() && map_of_shape(s).is_none())
    {
        let unit = unit_of(s);
        if units.contains(&unit) {
            unit_frames.entry(unit).or_default().push(s);
        }
    }
    let unit_board: HashMap<Uuid, Option<Uuid>> = unit_frames
        .into_iter()
        .map(|(unit, members)| {
            let p = match members.as_slice() {
                [one] => anchor(one),
                many => {
                    let frames: Vec<_> = many.iter().map(|s| s.frame).collect();
                    selection_bounds(&frames).map_or((0.0, 0.0), |r| r.center())
                }
            };
            (unit, board_at(&boards, p))
        })
        .collect();
    shapes
        .iter()
        .filter(|s| !s.is_artboard())
        .filter_map(|s| {
            let board = match map_of_shape(s) {
                Some(m) => map_board.get(&m)?,
                None => unit_board.get(&unit_of(s))?,
            };
            Some((s.id, *board))
        })
        .collect()
}

/// `ids` with the members of every artboard among them, each once, in the
/// order of `shapes`: what moving, deleting or copying `ids` acts on.
pub fn with_members(shapes: &[Shape], ids: &[Uuid]) -> Vec<Uuid> {
    let picked: HashSet<Uuid> = ids.iter().copied().collect();
    let boards: HashSet<Uuid> = shapes
        .iter()
        .filter(|s| s.is_artboard() && picked.contains(&s.id))
        .map(|s| s.id)
        .collect();
    shapes
        .iter()
        .filter(|s| picked.contains(&s.id) || s.artboard.is_some_and(|a| boards.contains(&a)))
        .map(|s| s.id)
        .collect()
}

/// `ids` with the shapes inside the containers among them (and inside
/// those, and so on), in layer order: what moving, copying or duplicating
/// a container takes along. A shape is inside when its box lies within the
/// container's; artboards, mind map parts and locked shapes are never
/// taken along.
pub fn with_contents(shapes: &[Shape], ids: &[Uuid]) -> Vec<Uuid> {
    let mut taken: HashSet<Uuid> = ids.iter().copied().collect();
    let mut boxes: Vec<crate::geometry::Rect> = shapes
        .iter()
        .filter(|s| taken.contains(&s.id) && s.container_header().is_some())
        .map(|s| frame_aabb(&s.frame))
        .collect();
    // Each round takes what the new containers hold; nesting ends it.
    while !boxes.is_empty() {
        let inside = |r: &crate::geometry::Rect| {
            boxes.iter().any(|b| {
                const SLACK: f32 = 0.5;
                r.x >= b.x - SLACK
                    && r.y >= b.y - SLACK
                    && r.x + r.w <= b.x + b.w + SLACK
                    && r.y + r.h <= b.y + b.h + SLACK
            })
        };
        let found: Vec<&Shape> = shapes
            .iter()
            .filter(|s| !taken.contains(&s.id) && !s.locked)
            .filter(|s| !s.is_artboard() && s.topic.is_none() && !s.is_boundary())
            .filter(|s| inside(&frame_aabb(&s.frame)))
            .collect();
        boxes = found
            .iter()
            .filter(|s| s.container_header().is_some())
            .map(|s| frame_aabb(&s.frame))
            .collect();
        taken.extend(found.iter().map(|s| s.id));
    }
    shapes
        .iter()
        .filter(|s| taken.contains(&s.id))
        .map(|s| s.id)
        .collect()
}

/// The top-left corner of a new artboard of `size`: [`ARTBOARD_GAP`] right
/// of the artboard reaching furthest right among `shapes` (tops aligned),
/// else centred on `view_center`.
pub fn artboard_placement(shapes: &[Shape], size: (f32, f32), view_center: Pt) -> Pt {
    let rightmost = shapes
        .iter()
        .filter(|s| s.is_artboard())
        .map(|s| frame_aabb(&s.frame))
        .max_by(|a, b| (a.x + a.w).total_cmp(&(b.x + b.w)));
    match rightmost {
        Some(r) => (r.x + r.w + ARTBOARD_GAP, r.y),
        None => (view_center.0 - size.0 / 2.0, view_center.1 - size.1 / 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Frame;
    use crate::mindmap::TopicProps;

    fn frame(x: f32, y: f32, w: f32, h: f32) -> Frame {
        Frame {
            x,
            y,
            w,
            h,
            ..Frame::default()
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Shape {
        let def = crate::library::builtin_libraries()[0].shapes[0].clone();
        Shape::new(&def, "basic-shapes", frame(x, y, w, h))
    }

    fn board(x: f32, y: f32, w: f32, h: f32) -> Shape {
        Shape::new_artboard(frame(x, y, w, h), "Board")
    }

    #[test]
    fn a_new_artboard_has_auto_colours_a_name_and_clips() {
        let mut a = board(1.0, 2.0, 300.0, 200.0);
        assert!(a.is_artboard() && !a.is_text_box());
        assert_eq!(a.name.as_deref(), Some("Board"));
        // Auto, like other objects: the page background and the theme's
        // outline colour, never a bright white.
        assert_eq!(a.style.fill.as_deref(), Some(crate::AUTO_COLOR));
        assert_eq!(a.style.stroke.as_deref(), Some(crate::AUTO_COLOR));
        assert_eq!(a.artboard_props, Some(ArtboardProps { clip: true }));
        assert!(a.label.is_none());
        // Never rotated, flipped or a member itself.
        a.frame.rotation = 30.0;
        a.frame.flip_y = true;
        a.artboard = Some(Uuid::new_v4());
        a.normalize();
        assert_eq!(
            (a.frame.rotation, a.frame.flip_y, a.artboard),
            (0.0, false, None)
        );
        // Other shapes never carry artboard properties.
        let mut r = rect(0.0, 0.0, 1.0, 1.0);
        r.artboard_props = Some(ArtboardProps::default());
        r.normalize();
        assert!(r.artboard_props.is_none() && !r.is_artboard());
    }

    #[test]
    fn centre_inside_joins_topmost_artboard() {
        let (a1, a2) = (
            board(0.0, 0.0, 100.0, 100.0),
            board(50.0, 0.0, 100.0, 100.0),
        );
        // Centres (70, 50): in both; (20, 50): in a1 only; (140, 140): none.
        let both = rect(60.0, 40.0, 20.0, 20.0);
        let first = rect(10.0, 40.0, 20.0, 20.0);
        let mut out = rect(90.0, 90.0, 100.0, 100.0);
        out.artboard = Some(a1.id);
        let ids = [both.id, first.id, out.id, a1.id];
        let shapes = vec![
            a1.clone(),
            a2.clone(),
            both.clone(),
            first.clone(),
            out.clone(),
        ];
        assert_eq!(
            membership(&shapes, &ids),
            vec![
                (both.id, Some(a2.id)),
                (first.id, Some(a1.id)),
                (out.id, None)
            ]
        );
        // Only the shapes asked about are decided.
        assert_eq!(
            membership(&shapes, &[first.id]),
            vec![(first.id, Some(a1.id))]
        );
    }

    #[test]
    fn group_joins_as_a_unit() {
        let a = board(0.0, 0.0, 100.0, 100.0);
        let g = Uuid::new_v4();
        let (mut inside, mut outside) =
            (rect(10.0, 10.0, 20.0, 20.0), rect(150.0, 10.0, 20.0, 20.0));
        inside.groups = vec![g];
        outside.groups = vec![g];
        let lone = rect(300.0, 10.0, 10.0, 10.0);
        // The group spans x 10..170: its centre (90, 20) is inside.
        let shapes = vec![a.clone(), inside.clone(), outside.clone(), lone];
        assert_eq!(
            membership(&shapes, &[outside.id]),
            vec![(inside.id, Some(a.id)), (outside.id, Some(a.id))]
        );
        // Moved further, the group's centre leaves: both leave.
        let mut shapes = shapes;
        shapes[2].frame.x = 400.0;
        assert_eq!(
            membership(&shapes, &[inside.id]),
            vec![(inside.id, None), (outside.id, None)]
        );
    }

    #[test]
    fn line_uses_midpoint() {
        let a = board(0.0, 0.0, 100.0, 100.0);
        let def = crate::library::builtin_libraries()[0]
            .shape("line")
            .unwrap()
            .clone();
        let line = |p, q| Shape::new(&def, "basic-shapes", crate::line::line_frame(p, q));
        // Drawn backwards: the middle of its ends is (50, 50).
        let inside = line((80.0, 80.0), (20.0, 20.0));
        let outside = line((90.0, 50.0), (300.0, 50.0));
        assert!(inside.is_line());
        let shapes = vec![a.clone(), inside.clone(), outside.clone()];
        assert_eq!(
            membership(&shapes, &[inside.id, outside.id]),
            vec![(inside.id, Some(a.id)), (outside.id, None)]
        );
    }

    #[test]
    fn subtopic_follows_central_topic() {
        let a = board(0.0, 0.0, 400.0, 400.0);
        let mut central = Shape::new_topic(frame(100.0, 100.0, 140.0, 48.0));
        central.artboard = Some(a.id);
        let mut sub = Shape::new_topic(frame(900.0, 900.0, 100.0, 30.0));
        sub.topic = Some(TopicProps::child(central.id, central.id, 1.0));
        let mut bound = Shape::new_boundary(vec![sub.id], None);
        bound.frame = frame(890.0, 890.0, 120.0, 50.0);
        let shapes = vec![a.clone(), central.clone(), sub.clone(), bound.clone()];
        // A subtopic dragged out stays with its map.
        let got = membership(&shapes, &[sub.id]);
        assert_eq!(
            got,
            vec![
                (central.id, Some(a.id)),
                (sub.id, Some(a.id)),
                (bound.id, Some(a.id))
            ]
        );
        // The central topic moved out takes the whole map along.
        let mut shapes = shapes;
        shapes[1].frame.x = 1000.0;
        let got = membership(&shapes, &[central.id]);
        assert!(got.iter().all(|(_, b)| b.is_none()) && got.len() == 3);
    }

    #[test]
    fn with_members_adds_each_member_once() {
        let a = board(0.0, 0.0, 100.0, 100.0);
        let mut m = rect(10.0, 10.0, 10.0, 10.0);
        m.artboard = Some(a.id);
        let other = rect(20.0, 20.0, 10.0, 10.0);
        let shapes = vec![a.clone(), m.clone(), other.clone()];
        assert_eq!(with_members(&shapes, &[m.id, a.id]), vec![a.id, m.id]);
        assert_eq!(with_members(&shapes, &[other.id]), vec![other.id]);
        // A member alone does not bring its artboard.
        assert_eq!(with_members(&shapes, &[m.id]), vec![m.id]);
    }

    #[test]
    fn placement_right_of_last_or_centred() {
        let r = rect(5000.0, 0.0, 10.0, 10.0);
        assert_eq!(
            artboard_placement(std::slice::from_ref(&r), (100.0, 50.0), (500.0, 500.0)),
            (450.0, 475.0)
        );
        // The artboard reaching furthest right wins, wherever it is stacked.
        let far = board(300.0, -50.0, 200.0, 100.0);
        let near = board(0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            artboard_placement(&[far, near, r], (100.0, 50.0), (500.0, 500.0)),
            (580.0, -50.0)
        );
    }

    #[test]
    fn presets_table_has_19_in_spec_order() {
        let names: Vec<&str> = ARTBOARD_PRESETS.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "iPhone 16",
                "iPhone 16 Pro Max",
                "Android",
                "iPad",
                "Android tablet",
                "Desktop",
                "MacBook",
                "HD screen",
                "Web page",
                "A4",
                "A4 landscape",
                "A3",
                "Letter",
                "Slide 16:9",
                "Slide 4:3",
                "Square post",
                "Story",
                "Video thumbnail",
                "Banner",
            ]
        );
        let find = |n: &str| ARTBOARD_PRESETS.iter().find(|p| p.name == n).unwrap();
        assert_eq!(find("iPhone 16").size, (393.0, 852.0));
        assert_eq!(find("A4").size, (595.0, 842.0));
        assert_eq!(find("Banner").size, (1584.0, 396.0));
        assert_eq!(find("iPad").icon, "tablet");
        assert_eq!(find("MacBook").icon, "laptop");
        assert_eq!(find("Slide 4:3").icon, "presentation");
        assert_eq!(find("Video thumbnail").icon, "square-play");
        let count = |c| ARTBOARD_PRESETS.iter().filter(|p| p.category == c).count();
        use PresetCategory::*;
        assert_eq!(
            [count(Device), count(Desktop), count(Paper), count(Social)],
            [5, 4, 6, 4]
        );
        // Categories come in one run each.
        let runs = ARTBOARD_PRESETS
            .windows(2)
            .filter(|w| w[0].category != w[1].category)
            .count();
        assert_eq!(runs, 3);
    }

    /// A container at `(x, y)` sized `w × h`.
    fn container(x: f32, y: f32, w: f32, h: f32) -> Shape {
        let mut s = rect(x, y, w, h);
        s.shape = "container".into();
        s.set_param("header", 28.0);
        s
    }

    #[test]
    fn a_container_takes_what_lies_inside_it_and_inside_that() {
        let outer = container(0.0, 0.0, 400.0, 300.0);
        let inner = container(20.0, 40.0, 200.0, 150.0);
        let in_inner = rect(40.0, 80.0, 50.0, 50.0);
        let in_outer = rect(250.0, 50.0, 100.0, 50.0);
        let half_out = rect(350.0, 250.0, 100.0, 100.0);
        let mut locked = rect(300.0, 200.0, 20.0, 20.0);
        locked.locked = true;
        let mut topic = rect(260.0, 150.0, 40.0, 20.0);
        topic.topic = Some(TopicProps::central(topic.id));
        let shapes = vec![
            outer.clone(),
            inner.clone(),
            in_inner.clone(),
            in_outer.clone(),
            half_out,
            locked,
            topic,
        ];
        assert_eq!(
            with_contents(&shapes, &[outer.id]),
            vec![outer.id, inner.id, in_inner.id, in_outer.id]
        );
        assert_eq!(
            with_contents(&shapes, &[inner.id]),
            vec![inner.id, in_inner.id]
        );
        // A plain shape takes nothing along.
        assert_eq!(with_contents(&shapes, &[in_outer.id]), vec![in_outer.id]);
    }
}
