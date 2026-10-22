//! Mind maps: topics are shapes (library [`crate::shape::TEXT_LIBRARY`],
//! shape [`TOPIC_SHAPE`]) holding [`TopicProps`]: the map they belong to,
//! their parent and their place among its children. A map's central
//! topic stays where it is put; every other topic is placed by
//! [`layout_page`] from the tree and each topic's size (which the editor
//! derives from its text and picture); a topic dragged elsewhere keeps
//! that offset, its subtree moving with it. Branch lines, summary brackets
//! and boundaries are drawn from the tree, not stored.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::document::Page;
use crate::geometry::{flatten, FillRole, Geometry, PathCmd, Pt, Rect, SubPath};
use crate::image::ImageData;
use crate::text::RichText;

/// Shape id of a topic in [`crate::shape::TEXT_LIBRARY`].
pub const TOPIC_SHAPE: &str = "topic";

/// Space between a topic and its children, across the branch.
pub const LEVEL_GAP: f32 = 48.0;
/// Space between neighbouring subtrees.
pub const SIBLING_GAP: f32 = 14.0;
/// Space between a summarised range and its bracket, and between the
/// bracket and the summary topic.
pub const SUMMARY_GAP: f32 = 12.0;
/// Width of a summary bracket.
pub const BRACKET_WIDTH: f32 = 14.0;
/// Space between a boundary and what it encloses.
pub const BOUNDARY_PADDING: f32 = 10.0;

/// How a map's topics are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Structure {
    /// Main topics on both sides of the central topic.
    #[default]
    Map,
    /// Every branch to the right.
    Logic,
    /// Every branch downwards.
    Org,
}

impl Structure {
    pub const ALL: [Structure; 3] = [Structure::Map, Structure::Logic, Structure::Org];

    pub fn name(self) -> &'static str {
        match self {
            Structure::Map => "Map",
            Structure::Logic => "Logic",
            Structure::Org => "Org Chart",
        }
    }
}

/// How the line from a topic to its parent is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BranchStyle {
    /// A curve thick at the parent, thinning to a point at the child.
    #[default]
    Tapered,
    Curve,
    Elbow,
    Straight,
}

impl BranchStyle {
    pub const ALL: [BranchStyle; 4] = [
        BranchStyle::Tapered,
        BranchStyle::Curve,
        BranchStyle::Elbow,
        BranchStyle::Straight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BranchStyle::Tapered => "Tapered",
            BranchStyle::Curve => "Curve",
            BranchStyle::Elbow => "Elbow",
            BranchStyle::Straight => "Straight",
        }
    }
}

/// A LaTeX equation shown in a topic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Equation {
    /// The LaTeX source.
    pub latex: String,
    /// The rendered size in canvas units, kept current by the desktop's
    /// fit pass; `(0, 0)` until measured.
    pub size: (f32, f32),
}

/// What makes a shape a topic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicProps {
    /// The map's central topic (its own id for the central topic).
    pub map: Uuid,
    /// `None` for the central topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Uuid>,
    /// Place among the parent's children, ascending.
    #[serde(default)]
    pub order: f64,
    /// The map's structure (read from the central topic).
    #[serde(default)]
    pub structure: Structure,
    /// The style of the lines to this topic and its subtree; `None`
    /// follows the parent (the central topic's is the map's default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<BranchStyle>,
    /// For a summary topic: the first and last of its parent's children it
    /// sums up. Summary topics sit beside that range, not among the
    /// children.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<(Uuid, Uuid)>,
    /// The topic's picture (an icon or an image), centred above its text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<ImageData>,
    /// The size the picture is shown at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_size: Option<(f32, f32)>,
    /// A web or file link, opened from the topic's link badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// A note attached to the topic, shown in a popover from its note badge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<RichText>,
    /// A LaTeX equation shown below the topic's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equation: Option<Equation>,
    /// On a central topic: the map keeps its topics arranged (a dragged
    /// topic snaps back unless dropped on another topic). Off, topics stay
    /// where they are dropped.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub auto_layout: bool,
    /// How far the topic (with its subtree) was dragged from where the
    /// layout puts it; `None` where the layout puts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<(f32, f32)>,
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

/// Side of a topic's link badge.
pub const TOPIC_BADGE: f32 = 14.0;
/// Space between a topic's text and its picture or badge.
pub const TOPIC_GAP: f32 = 4.0;
/// Space between a topic's edge and its picture or badge.
pub const TOPIC_PADDING: f32 = 6.0;
/// Space between a topic's edge and its text, across and down: the
/// central topic's, and every other topic's.
pub const CENTRAL_TEXT_PADDING: (f32, f32) = (24.0, 14.0);
pub const TOPIC_TEXT_PADDING: (f32, f32) = (16.0, 9.0);
/// Longest side of a topic's icon when dropped on it.
pub const TOPIC_ICON_SIZE: f32 = 32.0;
/// Longest side of a topic's image when chosen.
pub const TOPIC_IMAGE_SIZE: f32 = 160.0;
/// Smallest and largest longest side of a topic's picture.
pub const TOPIC_PICTURE_RANGE: (f32, f32) = (12.0, 800.0);
/// Widest a topic's text runs before wrapping.
pub const TOPIC_TEXT_MAX: f32 = 220.0;

/// The size a picture `w × h` is shown at in a topic: scaled so its
/// longest side is `longest`, at most its own size.
pub fn fit_topic_picture((w, h): (f32, f32), longest: f32) -> (f32, f32) {
    if !(w > 0.0 && h > 0.0) {
        return (longest, longest);
    }
    let scale = (longest / w.max(h)).min(1.0);
    (w * scale, h * scale)
}

/// A picture size `w × h` with its longest side clamped to
/// [`TOPIC_PICTURE_RANGE`], proportions kept.
pub fn clamp_topic_picture((w, h): (f32, f32)) -> (f32, f32) {
    let long = w.max(h);
    if long.is_nan() || long <= 0.0 {
        return (TOPIC_ICON_SIZE, TOPIC_ICON_SIZE);
    }
    let k = long.clamp(TOPIC_PICTURE_RANGE.0, TOPIC_PICTURE_RANGE.1) / long;
    (w * k, h * k)
}

/// Parts of a topic `w × h`, in its local box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopicParts {
    /// Where the text goes: the text's own size plus [`TOPIC_TEXT_PADDING`]
    /// round it, as the label layout pads it.
    pub text: Rect,
    pub picture: Option<Rect>,
    pub badge: Option<Rect>,
    /// The note badge, left of the link badge.
    pub note: Option<Rect>,
    /// The equation, below the text.
    pub equation: Option<Rect>,
}

impl TopicProps {
    /// Space between the topic's edge and its text, across and down.
    pub fn text_padding(&self) -> (f32, f32) {
        if self.is_central() {
            CENTRAL_TEXT_PADDING
        } else {
            TOPIC_TEXT_PADDING
        }
    }

    /// The shown size of the picture, if any.
    fn picture_shown(&self) -> Option<(f32, f32)> {
        self.picture.as_ref().map(|_| {
            self.picture_size
                .unwrap_or((TOPIC_ICON_SIZE, TOPIC_ICON_SIZE))
        })
    }

    /// The measured size of the equation, if there is one to show (an
    /// unmeasured `(0, 0)` equation shows nothing yet).
    fn equation_shown(&self) -> Option<(f32, f32)> {
        self.equation
            .as_ref()
            .map(|e| e.size)
            .filter(|&(w, h)| w > 0.0 && h > 0.0)
    }

    /// How many badges (link, note) sit right of the text.
    fn badge_count(&self) -> usize {
        usize::from(self.link.is_some()) + usize::from(self.note.is_some())
    }

    /// Width reserved right of the text for `n` badges, beyond the text's
    /// own horizontal padding.
    fn badges_reserve(&self, n: usize) -> f32 {
        if n == 0 {
            return 0.0;
        }
        let n = n as f32;
        (n * TOPIC_BADGE + (n - 1.0) * TOPIC_GAP + TOPIC_GAP + TOPIC_PADDING
            - self.text_padding().0)
            .max(0.0)
    }

    /// Height the equation adds below the text row.
    fn equation_extra(&self) -> f32 {
        self.equation_shown().map_or(0.0, |(_, eh)| {
            eh + TOPIC_GAP + TOPIC_PADDING - self.text_padding().1
        })
    }

    /// Where a topic `w × h` with these properties shows its picture (at
    /// the top, centred), its text below (or alone), its link and note
    /// badges (right of the text, link rightmost) and its equation (below
    /// the text, centred).
    pub fn parts(&self, w: f32, h: f32) -> TopicParts {
        let p = TOPIC_PADDING;
        let picture = self.picture_shown().map(|(pw, ph)| Rect {
            x: (w - pw) / 2.0,
            y: p,
            w: pw,
            h: ph,
        });
        let top = picture.map_or(0.0, |r| r.y + r.h + TOPIC_GAP - self.text_padding().1);
        let top = top.max(0.0);
        let row = Rect {
            x: 0.0,
            y: top,
            w,
            h: (h - top - self.equation_extra()).max(0.0),
        };
        let equation = self.equation_shown().map(|(ew, eh)| Rect {
            x: (w - ew) / 2.0,
            y: row.y + row.h - self.text_padding().1 + TOPIC_GAP,
            w: ew,
            h: eh,
        });
        let badge_at = |x: f32| Rect {
            x,
            y: row.y + (row.h - TOPIC_BADGE) / 2.0,
            w: TOPIC_BADGE,
            h: TOPIC_BADGE,
        };
        let last = w - p - TOPIC_BADGE;
        let badge = self.link.as_ref().map(|_| badge_at(last));
        let note = self.note.as_ref().map(|_| {
            badge_at(if badge.is_some() {
                last - TOPIC_BADGE - TOPIC_GAP
            } else {
                last
            })
        });
        let right = self.badges_reserve(self.badge_count());
        TopicParts {
            text: Rect {
                x: 0.0,
                y: row.y,
                w: (w - right).max(1.0),
                h: row.h,
            },
            picture,
            badge,
            note,
            equation,
        }
    }

    /// The size of a topic whose text lays out `tw × th` (empty text:
    /// one line), with [`TOPIC_TEXT_PADDING`] round the text and room for
    /// its picture, badges and equation.
    pub fn size_for_text(&self, (tw, th): (f32, f32)) -> (f32, f32) {
        let (px, py) = self.text_padding();
        let row_w = tw + 2.0 * px + self.badges_reserve(self.badge_count());
        let row_h = th + 2.0 * py;
        let (pw, ph) = self.picture_shown().unwrap_or((0.0, 0.0));
        let picture_h = if self.picture.is_some() {
            TOPIC_PADDING + ph + TOPIC_GAP - py
        } else {
            0.0
        };
        let (ew, _) = self.equation_shown().unwrap_or((0.0, 0.0));
        (
            row_w
                .max(ew + 2.0 * TOPIC_PADDING)
                .max(pw + 2.0 * TOPIC_PADDING)
                .max(60.0)
                .ceil(),
            (row_h + picture_h + self.equation_extra()).max(30.0).ceil(),
        )
    }
}

impl TopicProps {
    /// Properties of a new central topic `id`.
    pub fn central(id: Uuid) -> TopicProps {
        TopicProps {
            map: id,
            parent: None,
            order: 0.0,
            structure: Structure::Map,
            branch: Some(BranchStyle::Tapered),
            summary: None,
            picture: None,
            picture_size: None,
            link: None,
            note: None,
            equation: None,
            auto_layout: true,
            offset: None,
        }
    }

    /// Properties of a new child of `parent` (in map `map`) at `order`.
    pub fn child(map: Uuid, parent: Uuid, order: f64) -> TopicProps {
        TopicProps {
            parent: Some(parent),
            order,
            branch: None,
            ..TopicProps::central(map)
        }
    }

    pub fn is_central(&self) -> bool {
        self.parent.is_none()
    }
}

/// Branch colours given to main topics in turn.
pub const BRANCH_COLORS: [&str; 8] = [
    "#E5534B", "#F0883E", "#D4A72C", "#57AB5A", "#39C5CF", "#539BF5", "#986EE2", "#E275AD",
];

/// A topic as the layout sees it.
#[derive(Clone, Debug)]
struct Node {
    w: f32,
    h: f32,
    children: Vec<Uuid>,
    summaries: Vec<Uuid>,
}

/// A map's tree: nodes by id, children in order.
struct Tree {
    nodes: HashMap<Uuid, Node>,
    props: HashMap<Uuid, TopicProps>,
}

impl Tree {
    /// The tree of map `map` on `page`; topics whose parent is not in the
    /// map are left out (with their subtrees).
    fn of(page: &Page, map: Uuid) -> Tree {
        let topics: Vec<(&crate::shape::Shape, &TopicProps)> = page
            .layers
            .iter()
            .flat_map(|l| &l.objects)
            .filter_map(|s| Some((s, s.topic.as_ref().filter(|t| t.map == map)?)))
            .collect();
        let mut nodes: HashMap<Uuid, Node> = topics
            .iter()
            .map(|(s, _)| {
                (
                    s.id,
                    Node {
                        w: s.frame.w,
                        h: s.frame.h,
                        children: Vec::new(),
                        summaries: Vec::new(),
                    },
                )
            })
            .collect();
        let mut kids: Vec<(Uuid, Uuid, f64, bool)> = topics
            .iter()
            .filter_map(|(s, t)| Some((t.parent?, s.id, t.order, t.summary.is_some())))
            .collect();
        kids.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.1.cmp(&b.1)));
        for (parent, id, _, summary) in kids {
            if let Some(p) = nodes.get_mut(&parent) {
                if summary {
                    p.summaries.push(id);
                } else {
                    p.children.push(id);
                }
            }
        }
        let props = topics.into_iter().map(|(s, t)| (s.id, t.clone())).collect();
        Tree { nodes, props }
    }
}

/// Which way a branch grows from its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Right,
    Left,
    Down,
}

/// The members of a boundary: the topics it encloses with their subtrees.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BoundaryProps {
    pub members: Vec<Uuid>,
    /// The outline round the members as laid out: a convex hull, in the
    /// boundary's local box (see [`layout_page`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outline: Vec<Pt>,
}

/// Shape id of a boundary in [`crate::shape::TEXT_LIBRARY`].
pub const BOUNDARY_SHAPE: &str = "boundary";

/// Places map topics' frames: each map's central topic stays, its main
/// topics around it by its structure, every other topic beside or below
/// its parent; summary topics beside the range they sum up. Returns the
/// new frame of every topic it placed, by id.
fn layout_map(page: &Page, central: Uuid) -> HashMap<Uuid, Rect> {
    let tree = Tree::of(page, central);
    let mut out = HashMap::new();
    let Some(c) = page
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .find(|s| s.id == central)
    else {
        return out;
    };
    let structure = tree
        .props
        .get(&central)
        .map(|t| t.structure)
        .unwrap_or_default();
    let root = Rect {
        x: c.frame.x,
        y: c.frame.y,
        w: c.frame.w,
        h: c.frame.h,
    };
    out.insert(central, root);
    let mains = tree.nodes[&central].children.clone();
    match structure {
        Structure::Logic => place_children(&tree, &mains, Direction::Right, root, &mut out),
        Structure::Org => place_children(&tree, &mains, Direction::Down, root, &mut out),
        Structure::Map => {
            let right = mains.len().div_ceil(2);
            place_children(&tree, &mains[..right], Direction::Right, root, &mut out);
            place_children(&tree, &mains[right..], Direction::Left, root, &mut out);
        }
    }
    place_summaries(&tree, central, structure_direction(structure), &mut out);
    out
}

fn structure_direction(s: Structure) -> Direction {
    match s {
        Structure::Org => Direction::Down,
        _ => Direction::Right,
    }
}

/// The size of the block `id`'s subtree takes along the stacking axis
/// (height for sideways branches, width for downward ones).
fn block(tree: &Tree, id: Uuid, dir: Direction) -> f32 {
    let node = &tree.nodes[&id];
    let own = if dir == Direction::Down {
        node.w
    } else {
        node.h
    };
    if node.children.is_empty() {
        return own;
    }
    let kids: f32 = node
        .children
        .iter()
        .map(|c| block(tree, *c, dir))
        .sum::<f32>()
        + SIBLING_GAP * (node.children.len() - 1) as f32;
    own.max(kids)
}

/// Places `children` of the topic at `parent` growing `dir`, their blocks
/// stacked and centred on the parent.
fn place_children(
    tree: &Tree,
    children: &[Uuid],
    dir: Direction,
    parent: Rect,
    out: &mut HashMap<Uuid, Rect>,
) {
    if children.is_empty() {
        return;
    }
    let blocks: Vec<f32> = children.iter().map(|c| block(tree, *c, dir)).collect();
    let total = blocks.iter().sum::<f32>() + SIBLING_GAP * (children.len() - 1) as f32;
    let (pcx, pcy) = parent.center();
    let mut along = if dir == Direction::Down { pcx } else { pcy } - total / 2.0;
    for (child, b) in children.iter().zip(blocks) {
        let node = &tree.nodes[child];
        let mid = along + b / 2.0;
        let rect = match dir {
            Direction::Right => Rect {
                x: parent.x + parent.w + LEVEL_GAP,
                y: mid - node.h / 2.0,
                w: node.w,
                h: node.h,
            },
            Direction::Left => Rect {
                x: parent.x - LEVEL_GAP - node.w,
                y: mid - node.h / 2.0,
                w: node.w,
                h: node.h,
            },
            Direction::Down => Rect {
                x: mid - node.w / 2.0,
                y: parent.y + parent.h + LEVEL_GAP,
                w: node.w,
                h: node.h,
            },
        };
        // A topic dragged elsewhere keeps its offset; its subtree follows.
        let rect = match tree.props.get(child).and_then(|t| t.offset) {
            Some((dx, dy)) => Rect {
                x: rect.x + dx,
                y: rect.y + dy,
                ..rect
            },
            None => rect,
        };
        out.insert(*child, rect);
        let kids = node.children.clone();
        place_children(tree, &kids, dir, rect, out);
        along += b + SIBLING_GAP;
    }
}

/// The bounds of `id`'s placed subtree.
fn subtree_bounds(tree: &Tree, id: Uuid, placed: &HashMap<Uuid, Rect>) -> Option<Rect> {
    let mut r = *placed.get(&id)?;
    for c in &tree.nodes[&id].children {
        if let Some(b) = subtree_bounds(tree, *c, placed) {
            r = r.union(&b);
        }
    }
    Some(r)
}

/// Which way the branch to topic `id` grows: away from its parent.
fn direction_of(
    id: Uuid,
    parent: Rect,
    placed: &HashMap<Uuid, Rect>,
    default: Direction,
) -> Direction {
    match placed.get(&id) {
        Some(r) if r.y >= parent.y + parent.h => Direction::Down,
        Some(r) if r.x + r.w <= parent.x => Direction::Left,
        Some(_) => Direction::Right,
        None => default,
    }
}

/// The summarised range's bounds, its side, and where its bracket sits.
pub struct SummarySpan {
    pub range: Rect,
    pub dir: Direction,
}

/// Places every summary topic of the tree under `id` beside the range it
/// sums up, past the bracket.
fn place_summaries(tree: &Tree, id: Uuid, default: Direction, out: &mut HashMap<Uuid, Rect>) {
    let node = &tree.nodes[&id];
    for s in node.summaries.clone() {
        let Some(span) = summary_span(tree, s, out, default) else {
            continue;
        };
        let n = &tree.nodes[&s];
        let r = span.range;
        let reach = SUMMARY_GAP * 2.0 + BRACKET_WIDTH;
        let rect = match span.dir {
            Direction::Right => Rect {
                x: r.x + r.w + reach,
                y: r.y + r.h / 2.0 - n.h / 2.0,
                w: n.w,
                h: n.h,
            },
            Direction::Left => Rect {
                x: r.x - reach - n.w,
                y: r.y + r.h / 2.0 - n.h / 2.0,
                w: n.w,
                h: n.h,
            },
            Direction::Down => Rect {
                x: r.x + r.w / 2.0 - n.w / 2.0,
                y: r.y + r.h + reach,
                w: n.w,
                h: n.h,
            },
        };
        out.insert(s, rect);
    }
    for c in node.children.clone() {
        place_summaries(tree, c, default, out);
    }
}

/// The range summary topic `s` sums up, as placed.
fn summary_span(
    tree: &Tree,
    s: Uuid,
    placed: &HashMap<Uuid, Rect>,
    default: Direction,
) -> Option<SummarySpan> {
    let props = tree.props.get(&s)?;
    let (first, last) = props.summary?;
    let parent = props.parent?;
    let kids = &tree.nodes.get(&parent)?.children;
    let (a, b) = (
        kids.iter().position(|k| *k == first)?,
        kids.iter().position(|k| *k == last)?,
    );
    let (a, b) = (a.min(b), a.max(b));
    let mut range: Option<Rect> = None;
    for k in &kids[a..=b] {
        if let Some(r) = subtree_bounds(tree, *k, placed) {
            range = Some(range.map_or(r, |acc| acc.union(&r)));
        }
    }
    let dir = direction_of(kids[a], *placed.get(&parent)?, placed, default);
    Some(SummarySpan { range: range?, dir })
}

/// Lays out every mind map on `page` (see the module docs). Returns
/// whether any topic moved.
pub fn layout_page(page: &mut Page) -> bool {
    let centrals: Vec<Uuid> = page
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .filter(|s| s.topic.as_ref().is_some_and(TopicProps::is_central))
        .map(|s| s.id)
        .collect();
    let mut placed: HashMap<Uuid, Rect> = HashMap::new();
    for c in centrals {
        placed.extend(layout_map(page, c));
    }
    let frames: HashMap<Uuid, Rect> = page
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .filter(|s| s.topic.is_some())
        .map(|s| {
            let f = &s.frame;
            (
                s.id,
                Rect {
                    x: f.x,
                    y: f.y,
                    w: f.w,
                    h: f.h,
                },
            )
        })
        .collect();
    // Boundaries hug their member topics: the hull of their boxes.
    let mut outlines: HashMap<Uuid, Vec<Pt>> = HashMap::new();
    for s in page.layers.iter().flat_map(|l| &l.objects) {
        let Some(b) = &s.boundary else { continue };
        let rects: Vec<Rect> = b
            .members
            .iter()
            .filter_map(|m| placed.get(m).copied().or_else(|| frames.get(m).copied()))
            .collect();
        let corners: Vec<Pt> = rects
            .iter()
            .map(padded)
            .flat_map(|r| {
                [
                    (r.x, r.y),
                    (r.x + r.w, r.y),
                    (r.x + r.w, r.y + r.h),
                    (r.x, r.y + r.h),
                ]
            })
            .collect();
        let hull = convex_hull(corners);
        let Some(bbox) = hull_bounds(&hull) else {
            continue;
        };
        placed.insert(s.id, bbox);
        outlines.insert(
            s.id,
            hull.iter().map(|p| (p.0 - bbox.x, p.1 - bbox.y)).collect(),
        );
    }
    let mut changed = false;
    for s in page.layers.iter_mut().flat_map(|l| l.objects.iter_mut()) {
        if let (Some(b), Some(outline)) = (s.boundary.as_mut(), outlines.remove(&s.id)) {
            if b.outline != outline {
                b.outline = outline;
                changed = true;
            }
        }
        if let Some(r) = placed.get(&s.id) {
            let f = &mut s.frame;
            if (f.x, f.y, f.w, f.h) != (r.x, r.y, r.w, r.h) || f.rotation != 0.0 {
                (f.x, f.y, f.w, f.h) = (r.x, r.y, r.w, r.h);
                (f.rotation, f.flip_x, f.flip_y) = (0.0, false, false);
                changed = true;
            }
        }
    }
    changed
}

/// The ids of `ids` and all their descendants (summary topics of a
/// removed range included) in maps on `page`.
pub fn with_descendants(page: &Page, ids: &[Uuid]) -> HashSet<Uuid> {
    let topics: Vec<(Uuid, &TopicProps)> = page
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .filter_map(|s| Some((s.id, s.topic.as_ref()?)))
        .collect();
    let mut out: HashSet<Uuid> = ids.iter().copied().collect();
    loop {
        let before = out.len();
        for (id, t) in &topics {
            let under = t.parent.is_some_and(|p| out.contains(&p));
            let sums_removed = t
                .summary
                .is_some_and(|(a, b)| out.contains(&a) || out.contains(&b));
            if under || sums_removed {
                out.insert(*id);
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// The style of the line to topic `id`: its own, else its nearest
/// ancestor's, else the map's default.
pub fn branch_style(page: &Page, id: Uuid) -> BranchStyle {
    let props: HashMap<Uuid, &TopicProps> = page
        .layers
        .iter()
        .flat_map(|l| &l.objects)
        .filter_map(|s| Some((s.id, s.topic.as_ref()?)))
        .collect();
    let mut at = Some(id);
    for _ in 0..props.len() + 1 {
        let Some(t) = at.and_then(|i| props.get(&i)) else {
            break;
        };
        if let Some(style) = t.branch {
            return style;
        }
        at = t.parent;
    }
    BranchStyle::default()
}

/// Segments of a curved branch.
const CURVE_STEPS: usize = 24;

/// Where the line between `parent` and `child` starts and ends, and the
/// way it grows: from the parent's side (or centre, for the central
/// topic) to the child's near side.
fn branch_ends(parent: &Rect, child: &Rect, central: bool, downward: bool) -> (Pt, Pt, Direction) {
    let (pcx, pcy) = parent.center();
    let (ccx, ccy) = child.center();
    // An org chart joins tops and bottoms; other structures always join
    // the sides facing each other, wherever a topic was dragged.
    if downward {
        let start = if central {
            (pcx, pcy)
        } else {
            (pcx, parent.y + parent.h)
        };
        return (start, (ccx, child.y), Direction::Down);
    }
    if ccx < pcx {
        let start = if central { (pcx, pcy) } else { (parent.x, pcy) };
        (start, (child.x + child.w, ccy), Direction::Left)
    } else {
        let start = if central {
            (pcx, pcy)
        } else {
            (parent.x + parent.w, pcy)
        };
        (start, (child.x, ccy), Direction::Right)
    }
}

/// The cubic from `a` to `b` leaving and arriving along `dir`.
fn branch_curve(a: Pt, b: Pt, dir: Direction) -> (Pt, Pt) {
    if dir == Direction::Down {
        let my = (a.1 + b.1) / 2.0;
        ((a.0, my), (b.0, my))
    } else {
        let mx = (a.0 + b.0) / 2.0;
        ((mx, a.1), (mx, b.1))
    }
}

fn cubic_at(a: Pt, c1: Pt, c2: Pt, b: Pt, t: f32) -> Pt {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        w0 * a.0 + w1 * c1.0 + w2 * c2.0 + w3 * b.0,
        w0 * a.1 + w1 * c1.1 + w2 * c2.1 + w3 * b.1,
    )
}

/// The line from topic `parent` to its child `child` (canvas
/// coordinates; `downward` in an org chart) in `style`, for a stroke `width` wide. Tapered branches
/// are filled outlines in the stroke colour; the others are strokes.
pub fn branch_geometry(
    parent: &Rect,
    child: &Rect,
    central: bool,
    downward: bool,
    style: BranchStyle,
    width: f32,
) -> Geometry {
    let (a, b, dir) = branch_ends(parent, child, central, downward);
    let (c1, c2) = branch_curve(a, b, dir);
    let stroke = |cmds: Vec<PathCmd>| Geometry {
        paths: vec![SubPath {
            cmds,
            fill: FillRole::None,
        }],
    };
    match style {
        BranchStyle::Straight => stroke(vec![PathCmd::MoveTo(a), PathCmd::LineTo(b)]),
        BranchStyle::Curve => stroke(vec![PathCmd::MoveTo(a), PathCmd::CubicTo { c1, c2, to: b }]),
        BranchStyle::Elbow => {
            let r = 8.0_f32;
            if dir == Direction::Down {
                let my = (a.1 + b.1) / 2.0;
                let sx = if b.0 >= a.0 { 1.0 } else { -1.0 };
                let r = r.min((b.0 - a.0).abs() / 2.0).min((my - a.1).abs());
                stroke(vec![
                    PathCmd::MoveTo(a),
                    PathCmd::LineTo((a.0, my - r)),
                    PathCmd::QuadTo {
                        ctrl: (a.0, my),
                        to: (a.0 + sx * r, my),
                    },
                    PathCmd::LineTo((b.0 - sx * r, my)),
                    PathCmd::QuadTo {
                        ctrl: (b.0, my),
                        to: (b.0, my + r),
                    },
                    PathCmd::LineTo(b),
                ])
            } else {
                let mx = (a.0 + b.0) / 2.0;
                let sy = if b.1 >= a.1 { 1.0 } else { -1.0 };
                let sx = if b.0 >= a.0 { 1.0 } else { -1.0 };
                let r = r.min((b.1 - a.1).abs() / 2.0).min((mx - a.0).abs());
                stroke(vec![
                    PathCmd::MoveTo(a),
                    PathCmd::LineTo((mx - sx * r, a.1)),
                    PathCmd::QuadTo {
                        ctrl: (mx, a.1),
                        to: (mx, a.1 + sy * r),
                    },
                    PathCmd::LineTo((mx, b.1 - sy * r)),
                    PathCmd::QuadTo {
                        ctrl: (mx, b.1),
                        to: (mx + sx * r, b.1),
                    },
                    PathCmd::LineTo(b),
                ])
            }
        }
        BranchStyle::Tapered => {
            let thick = (width.max(1.0) * 3.0 + 2.0) / 2.0;
            let thin = (width.max(1.0) * 0.4) / 2.0;
            let pts: Vec<Pt> = (0..=CURVE_STEPS)
                .map(|i| cubic_at(a, c1, c2, b, i as f32 / CURVE_STEPS as f32))
                .collect();
            let n = pts.len();
            let mut left = Vec::with_capacity(n);
            let mut right = Vec::with_capacity(n);
            for i in 0..n {
                let (p, q) = (pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)]);
                let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                let len = dx.hypot(dy).max(1e-6);
                let normal = (-dy / len, dx / len);
                let t = i as f32 / (n - 1) as f32;
                let half = thick + (thin - thick) * t;
                left.push((pts[i].0 + normal.0 * half, pts[i].1 + normal.1 * half));
                right.push((pts[i].0 - normal.0 * half, pts[i].1 - normal.1 * half));
            }
            let mut cmds = vec![PathCmd::MoveTo(left[0])];
            cmds.extend(left[1..].iter().map(|p| PathCmd::LineTo(*p)));
            cmds.extend(right.iter().rev().map(|p| PathCmd::LineTo(*p)));
            cmds.push(PathCmd::Close);
            Geometry {
                paths: vec![SubPath {
                    cmds,
                    fill: FillRole::StrokeColor,
                }],
            }
        }
    }
}

/// The bracket in front of summarised `range` on side `dir` (a brace
/// pointing at the summary topic), as a stroke.
pub fn summary_bracket(range: &Rect, dir: Direction) -> Geometry {
    let w = BRACKET_WIDTH;
    let cmds = match dir {
        Direction::Right | Direction::Left => {
            let s = if dir == Direction::Right { 1.0 } else { -1.0 };
            let x0 = if dir == Direction::Right {
                range.x + range.w + SUMMARY_GAP
            } else {
                range.x - SUMMARY_GAP
            };
            let (top, bottom) = (range.y, range.y + range.h);
            let mid = (top + bottom) / 2.0;
            let xm = x0 + s * w / 2.0;
            vec![
                PathCmd::MoveTo((x0, top)),
                PathCmd::QuadTo {
                    ctrl: (xm, top),
                    to: (xm, top + (mid - top).min(w)),
                },
                PathCmd::LineTo((xm, mid - w / 2.0)),
                PathCmd::QuadTo {
                    ctrl: (xm, mid),
                    to: (x0 + s * w, mid),
                },
                PathCmd::QuadTo {
                    ctrl: (xm, mid),
                    to: (xm, mid + w / 2.0),
                },
                PathCmd::LineTo((xm, bottom - (bottom - mid).min(w))),
                PathCmd::QuadTo {
                    ctrl: (xm, bottom),
                    to: (x0, bottom),
                },
            ]
        }
        Direction::Down => {
            let y0 = range.y + range.h + SUMMARY_GAP;
            let (l, r) = (range.x, range.x + range.w);
            let mid = (l + r) / 2.0;
            let ym = y0 + w / 2.0;
            vec![
                PathCmd::MoveTo((l, y0)),
                PathCmd::QuadTo {
                    ctrl: (l, ym),
                    to: (l + (mid - l).min(w), ym),
                },
                PathCmd::LineTo((mid - w / 2.0, ym)),
                PathCmd::QuadTo {
                    ctrl: (mid, ym),
                    to: (mid, y0 + w),
                },
                PathCmd::QuadTo {
                    ctrl: (mid, ym),
                    to: (mid + w / 2.0, ym),
                },
                PathCmd::LineTo((r - (r - mid).min(w), ym)),
                PathCmd::QuadTo {
                    ctrl: (r, ym),
                    to: (r, y0),
                },
            ]
        }
    };
    Geometry {
        paths: vec![SubPath {
            cmds,
            fill: FillRole::None,
        }],
    }
}

/// What to draw for the mind maps on `page`: branch lines (with the topic
/// whose stroke colours them) and summary brackets. Boundaries are shapes
/// of their own (see [`layout_page`]).
#[derive(Debug, Default)]
pub struct MapDecor {
    /// Branch lines, by the child topic they lead to.
    pub branches: Vec<(Uuid, Geometry)>,
    /// Summary brackets, by summary topic.
    pub brackets: Vec<(Uuid, Geometry)>,
}

/// The branch lines and summary brackets of `page`'s mind maps, as
/// laid out; `width_of` gives the stroke width of the topic a branch leads
/// to.
pub fn map_decor(page: &Page, width_of: impl Fn(Uuid) -> f32) -> MapDecor {
    let mut decor = MapDecor::default();
    let centrals: Vec<Uuid> = page
        .layers
        .iter()
        .rev()
        .filter(|l| l.visible)
        .flat_map(|l| &l.objects)
        .filter(|s| s.topic.as_ref().is_some_and(TopicProps::is_central))
        .map(|s| s.id)
        .collect();
    let rects: HashMap<Uuid, Rect> = page
        .layers
        .iter()
        .filter(|l| l.visible)
        .flat_map(|l| &l.objects)
        .filter(|s| s.topic.is_some())
        .map(|s| {
            let f = &s.frame;
            (
                s.id,
                Rect {
                    x: f.x,
                    y: f.y,
                    w: f.w,
                    h: f.h,
                },
            )
        })
        .collect();
    for central in centrals {
        let tree = Tree::of(page, central);
        let downward = tree
            .props
            .get(&central)
            .is_some_and(|t| t.structure == Structure::Org);
        for (id, props) in &tree.props {
            let (Some(parent), Some(rect)) = (props.parent, rects.get(id)) else {
                continue;
            };
            let Some(prect) = rects.get(&parent) else {
                continue;
            };
            if props.summary.is_none() {
                let central_parent = tree.props.get(&parent).is_some_and(TopicProps::is_central);
                decor.branches.push((
                    *id,
                    branch_geometry(
                        prect,
                        rect,
                        central_parent,
                        downward,
                        branch_style(page, *id),
                        width_of(*id),
                    ),
                ));
            } else if let Some(span) = summary_span(&tree, *id, &rects, Direction::Right) {
                decor
                    .brackets
                    .push((*id, summary_bracket(&span.range, span.dir)));
            }
        }
    }
    decor.branches.sort_by_key(|(id, _)| *id);
    decor.brackets.sort_by_key(|(id, _)| *id);
    decor
}

/// The convex hull of `points`, counter-clockwise on screen (Andrew's
/// monotone chain); fewer than three distinct points come back as they
/// are.
pub fn convex_hull(mut points: Vec<Pt>) -> Vec<Pt> {
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: Pt, a: Pt, b: Pt| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<Pt> = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Pt> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn hull_bounds(points: &[Pt]) -> Option<Rect> {
    let first = points.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for p in points {
        (x0, y0, x1, y1) = (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1));
    }
    Some(Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// A closed outline through `points` with its corners rounded by up to
/// `radius` (less where the sides are short).
pub fn rounded_polygon(points: &[Pt], radius: f32) -> Geometry {
    let n = points.len();
    if n < 3 {
        return Geometry::default();
    }
    let toward = |a: Pt, b: Pt, d: f32| {
        let l = (b.0 - a.0).hypot(b.1 - a.1).max(1e-6);
        (a.0 + (b.0 - a.0) * d / l, a.1 + (b.1 - a.1) * d / l)
    };
    let mut cmds = Vec::with_capacity(2 * n + 2);
    for i in 0..n {
        let (prev, p, next) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
        let half = |a: Pt, b: Pt| (b.0 - a.0).hypot(b.1 - a.1) / 2.0;
        let r = radius.min(half(prev, p)).min(half(p, next));
        let (a, b) = (toward(p, prev, r), toward(p, next, r));
        cmds.push(if i == 0 {
            PathCmd::MoveTo(a)
        } else {
            PathCmd::LineTo(a)
        });
        cmds.push(PathCmd::QuadTo { ctrl: p, to: b });
    }
    cmds.push(PathCmd::Close);
    Geometry {
        paths: vec![SubPath {
            cmds,
            fill: FillRole::Shape,
        }],
    }
}

fn padded(r: &Rect) -> Rect {
    Rect {
        x: r.x - BOUNDARY_PADDING,
        y: r.y - BOUNDARY_PADDING,
        w: r.w + 2.0 * BOUNDARY_PADDING,
        h: r.h + 2.0 * BOUNDARY_PADDING,
    }
}

/// Whether `g` holds no points (for tests and callers that skip empty
/// decor).
pub fn is_empty(g: &Geometry) -> bool {
    flatten(g, 1.0).iter().all(|(c, _)| c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Page;
    use crate::geometry::Frame;
    use crate::shape::Shape;

    fn topic(id: Uuid, props: TopicProps, w: f32, h: f32) -> Shape {
        let mut s = Shape::new_topic(Frame {
            w,
            h,
            ..Frame::default()
        });
        s.id = id;
        s.topic = Some(props);
        s
    }

    fn page(shapes: Vec<Shape>) -> Page {
        let mut p = Page::new("p");
        p.layers[0].objects = shapes;
        p
    }

    fn rect(p: &Page, id: Uuid) -> (f32, f32, f32, f32) {
        let s = p.layers[0].objects.iter().find(|s| s.id == id).unwrap();
        (s.frame.x, s.frame.y, s.frame.w, s.frame.h)
    }

    #[test]
    fn logic_maps_stack_children_to_the_right_centred_on_the_parent() {
        let c = Uuid::new_v4();
        let (a, b, a1) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut central = TopicProps::central(c);
        central.structure = Structure::Logic;
        let mut p = page(vec![
            topic(c, central, 100.0, 40.0),
            topic(b, TopicProps::child(c, c, 2.0), 80.0, 30.0),
            topic(a, TopicProps::child(c, c, 1.0), 80.0, 30.0),
            topic(a1, TopicProps::child(c, a, 1.0), 60.0, 20.0),
        ]);
        assert!(layout_page(&mut p));
        assert!(!layout_page(&mut p));
        // Two children 30 high with a gap: 74 in all, centred on y = 20.
        assert_eq!(rect(&p, a), (148.0, -17.0, 80.0, 30.0));
        assert_eq!(rect(&p, b), (148.0, 27.0, 80.0, 30.0));
        assert_eq!(rect(&p, a1), (276.0, -12.0, 60.0, 20.0));
    }

    #[test]
    fn map_structure_puts_main_topics_on_both_sides() {
        let c = Uuid::new_v4();
        let kids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        let mut shapes = vec![topic(c, TopicProps::central(c), 100.0, 40.0)];
        for (i, k) in kids.iter().enumerate() {
            shapes.push(topic(*k, TopicProps::child(c, c, i as f64), 80.0, 30.0));
        }
        let mut p = page(shapes);
        layout_page(&mut p);
        // Two on the right, one on the left.
        assert!(rect(&p, kids[0]).0 > 100.0 && rect(&p, kids[1]).0 > 100.0);
        assert_eq!(rect(&p, kids[2]).0, -LEVEL_GAP - 80.0);
    }

    #[test]
    fn org_charts_grow_downwards() {
        let c = Uuid::new_v4();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut central = TopicProps::central(c);
        central.structure = Structure::Org;
        let mut p = page(vec![
            topic(c, central, 100.0, 40.0),
            topic(a, TopicProps::child(c, c, 0.0), 80.0, 30.0),
            topic(b, TopicProps::child(c, c, 1.0), 80.0, 30.0),
        ]);
        layout_page(&mut p);
        // 80 + 14 + 80 = 174 wide, centred on x = 50.
        assert_eq!(rect(&p, a), (-37.0, 88.0, 80.0, 30.0));
        assert_eq!(rect(&p, b), (57.0, 88.0, 80.0, 30.0));
    }

    #[test]
    fn summaries_sit_past_their_range_and_get_a_bracket() {
        let c = Uuid::new_v4();
        let (a, b, s) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut central = TopicProps::central(c);
        central.structure = Structure::Logic;
        let mut sum = TopicProps::child(c, c, 9.0);
        sum.summary = Some((a, b));
        let mut p = page(vec![
            topic(c, central, 100.0, 40.0),
            topic(a, TopicProps::child(c, c, 0.0), 80.0, 30.0),
            topic(b, TopicProps::child(c, c, 1.0), 80.0, 30.0),
            topic(s, sum, 70.0, 30.0),
        ]);
        layout_page(&mut p);
        // The range ends at x = 228; the summary sits past the bracket.
        let reach = 2.0 * SUMMARY_GAP + BRACKET_WIDTH;
        assert_eq!(rect(&p, s).0, 228.0 + reach);
        assert_eq!(rect(&p, s).1, 20.0 - 15.0);
        let decor = map_decor(&p, |_| 2.0);
        assert_eq!(decor.branches.len(), 2);
        assert_eq!(decor.brackets.len(), 1);
    }

    #[test]
    fn removing_a_topic_takes_its_branch_and_summaries_of_it() {
        let c = Uuid::new_v4();
        let (a, a1, b, s) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let mut sum = TopicProps::child(c, c, 9.0);
        sum.summary = Some((a, b));
        let p = page(vec![
            topic(c, TopicProps::central(c), 100.0, 40.0),
            topic(a, TopicProps::child(c, c, 0.0), 80.0, 30.0),
            topic(a1, TopicProps::child(c, a, 0.0), 80.0, 30.0),
            topic(b, TopicProps::child(c, c, 1.0), 80.0, 30.0),
            topic(s, sum, 70.0, 30.0),
        ]);
        let gone = with_descendants(&p, &[a]);
        assert_eq!(gone, [a, a1, s].into_iter().collect());
    }

    #[test]
    fn branch_styles_inherit_from_the_nearest_ancestor() {
        let c = Uuid::new_v4();
        let (a, a1) = (Uuid::new_v4(), Uuid::new_v4());
        let mut pa = TopicProps::child(c, c, 0.0);
        pa.branch = Some(BranchStyle::Elbow);
        let p = page(vec![
            topic(c, TopicProps::central(c), 100.0, 40.0),
            topic(a, pa, 80.0, 30.0),
            topic(a1, TopicProps::child(c, a, 0.0), 80.0, 30.0),
        ]);
        assert_eq!(branch_style(&p, a1), BranchStyle::Elbow);
        assert_eq!(branch_style(&p, c), BranchStyle::Tapered);
    }

    #[test]
    fn topics_fit_their_text_picture_and_link_snugly() {
        let c = Uuid::new_v4();
        let plain = TopicProps::central(c);
        // The central topic: 24 across and 14 down round the text.
        assert_eq!(plain.size_for_text((100.0, 20.0)), (148.0, 48.0));
        let parts = plain.parts(148.0, 48.0);
        assert_eq!(
            parts.text,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 148.0,
                h: 48.0
            }
        );
        // Others: 16 across and 9 down.
        let child = TopicProps::child(c, c, 0.0);
        assert_eq!(child.size_for_text((100.0, 20.0)), (132.0, 38.0));
        let pic = ImageData::from_file_bytes(b"GIF89a..".to_vec()).unwrap();
        let mut rich = plain.clone();
        rich.picture = Some(pic);
        rich.picture_size = Some(fit_topic_picture((320.0, 160.0), TOPIC_IMAGE_SIZE));
        rich.link = Some("https://example.com".into());
        assert_eq!(rich.picture_size, Some((160.0, 80.0)));
        let (w, h) = rich.size_for_text((100.0, 20.0));
        // The text row (100 + 48 + badge 14 + 4 + 6 - 24) is narrower than
        // the picture (160 + 12); the picture sits on top, centred, the
        // text row below it.
        assert_eq!(w, 172.0);
        assert_eq!(h, 48.0 + 6.0 + 80.0 + 4.0 - 14.0);
        let parts = rich.parts(w, h);
        let p = parts.picture.unwrap();
        assert_eq!((p.x, p.y), ((172.0 - 160.0) / 2.0, TOPIC_PADDING));
        assert_eq!(parts.badge.unwrap().x, w - TOPIC_PADDING - TOPIC_BADGE);
        assert!(parts.text.y > p.y + p.h - CENTRAL_TEXT_PADDING.1);
        assert_eq!(clamp_topic_picture((2000.0, 1000.0)), (800.0, 400.0));
        assert_eq!(clamp_topic_picture((4.0, 2.0)), (12.0, 6.0));
    }

    #[test]
    fn note_badge_sits_left_of_the_link_badge() {
        let c = Uuid::new_v4();
        let mut t = TopicProps::child(c, c, 0.0);
        t.link = Some("https://example.com".into());
        t.note = Some(RichText::new(
            crate::text::RunStyle::default(),
            crate::text::HAlign::Left,
        ));
        let (w, h) = t.size_for_text((100.0, 20.0));
        assert_eq!(w, 158.0);
        let parts = t.parts(w, h);
        let link = parts.badge.unwrap();
        assert_eq!(parts.note.unwrap().x, link.x - TOPIC_BADGE - TOPIC_GAP);
        t.link = None;
        let (w, h) = t.size_for_text((100.0, 20.0));
        let parts = t.parts(w, h);
        assert!(parts.badge.is_none());
        assert_eq!(parts.note.unwrap().x, w - TOPIC_PADDING - TOPIC_BADGE);
    }

    #[test]
    fn equation_sits_below_the_text_and_widens_the_topic() {
        let c = Uuid::new_v4();
        let mut t = TopicProps::child(c, c, 0.0);
        t.equation = Some(Equation {
            latex: "x".into(),
            size: (200.0, 40.0),
        });
        let (w, h) = t.size_for_text((100.0, 20.0));
        assert_eq!((w, h), (212.0, 38.0 + 40.0 + 4.0 + 6.0 - 9.0));
        assert_eq!(
            t.parts(w, h).equation,
            Some(Rect {
                x: 6.0,
                y: 38.0 - 9.0 + 4.0,
                w: 200.0,
                h: 40.0
            })
        );
    }

    #[test]
    fn unmeasured_equation_adds_nothing() {
        let c = Uuid::new_v4();
        let plain = TopicProps::child(c, c, 0.0);
        let mut t = plain.clone();
        t.equation = Some(Equation {
            latex: "x".into(),
            size: (0.0, 0.0),
        });
        assert_eq!(
            t.size_for_text((100.0, 20.0)),
            plain.size_for_text((100.0, 20.0))
        );
        assert!(t.parts(132.0, 38.0).equation.is_none());
    }

    #[test]
    fn dragged_topics_keep_their_offset_and_carry_their_subtree() {
        let c = Uuid::new_v4();
        let (a, a1) = (Uuid::new_v4(), Uuid::new_v4());
        let mut central = TopicProps::central(c);
        central.structure = Structure::Logic;
        let mut pa = TopicProps::child(c, c, 0.0);
        pa.offset = Some((30.0, 200.0));
        let mut p = page(vec![
            topic(c, central, 100.0, 40.0),
            topic(a, pa, 80.0, 30.0),
            topic(a1, TopicProps::child(c, a, 0.0), 60.0, 20.0),
        ]);
        layout_page(&mut p);
        assert_eq!(rect(&p, a), (148.0 + 30.0, 5.0 + 200.0, 80.0, 30.0));
        let (ax, ay, aw, _) = rect(&p, a);
        assert_eq!(rect(&p, a1).0, ax + aw + LEVEL_GAP);
        assert_eq!(rect(&p, a1).1, ay + 15.0 - 10.0);
    }

    #[test]
    fn boundaries_hug_only_their_members() {
        let c = Uuid::new_v4();
        let (a, b, b1) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut central = TopicProps::central(c);
        central.structure = Structure::Logic;
        let boundary = Shape::new_boundary(vec![c, a], None);
        let bid = boundary.id;
        let mut p = page(vec![
            boundary,
            topic(c, central, 100.0, 40.0),
            topic(a, TopicProps::child(c, c, 0.0), 80.0, 30.0),
            topic(b, TopicProps::child(c, c, 1.0), 80.0, 30.0),
            topic(b1, TopicProps::child(c, b, 0.0), 60.0, 20.0),
        ]);
        layout_page(&mut p);
        let (ax, ay, aw, _) = rect(&p, a);
        let r = rect(&p, bid);
        let pad = BOUNDARY_PADDING;
        // From the central topic's left edge to a's right edge; b and its
        // subtopic, not selected, are left out.
        assert_eq!(r.0, -pad);
        assert_eq!(r.0 + r.2, ax + aw + pad);
        assert_eq!(r.1, ay.min(0.0) - pad);
        let outline = p.layers[0].objects[0].boundary.clone().unwrap().outline;
        // A hull round two offset boxes is not a plain rectangle.
        assert!(outline.len() > 4, "{outline:?}");
        assert!(outline.iter().all(|q| q.0 >= -1e-3 && q.0 <= r.2 + 1e-3));
    }

    #[test]
    fn hulls_wrap_their_points() {
        let hull = convex_hull(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (5.0, 2.0),
            (5.0, 8.0),
            (0.0, 0.0),
        ]);
        assert_eq!(hull.len(), 3);
        assert!(!hull.contains(&(5.0, 2.0)));
        let g = rounded_polygon(&hull, 2.0);
        assert!(!is_empty(&g));
    }

    #[test]
    fn tapered_branches_thin_towards_the_child() {
        let parent = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 40.0,
        };
        let child = Rect {
            x: 200.0,
            y: 100.0,
            w: 80.0,
            h: 30.0,
        };
        let g = branch_geometry(&parent, &child, true, false, BranchStyle::Tapered, 2.0);
        assert_eq!(g.paths[0].fill, FillRole::StrokeColor);
        let pts: Vec<Pt> = g.paths[0]
            .cmds
            .iter()
            .filter_map(|c| match *c {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(p),
                _ => None,
            })
            .collect();
        let n = pts.len() / 2;
        let width_at = |i: usize| {
            let (l, r) = (pts[i], pts[pts.len() - 1 - i]);
            (l.0 - r.0).hypot(l.1 - r.1)
        };
        assert!(width_at(0) > 6.0 && width_at(n - 1) < 1.0);
        for style in BranchStyle::ALL {
            assert!(!is_empty(&branch_geometry(
                &parent, &child, false, false, style, 2.0
            )));
        }
    }
}
