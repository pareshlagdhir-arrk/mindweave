//! Artboards on the canvas: which shapes a placement, move or resize
//! gives membership to, hit-testing their names and clipped members, and
//! renaming one in place by double-clicking its name.
//!
//! Membership follows the core rule ([`mindweave_core::membership`]) and
//! always joins the undo step of the edit that caused it: placing, pasting
//! and duplicating add and assign in one session edit, and a move, resize
//! or rotate assigns inside its open gesture.

use std::collections::HashSet;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use mindweave_core::geometry::{Frame, Pt, Rect};
use mindweave_core::{frame_aabb, DocumentSession, Shape, Viewport};
use uuid::Uuid;

use super::shape_paint::{name_label_rect, screen_rect};
use super::{EditorWindow, RenameTarget};

/// Size of an artboard placed with a click of the Custom Artboard tool.
pub const ARTBOARD_DEFAULT_SIZE: (f32, f32) = (400., 400.);
/// Name of an artboard drawn with the Custom Artboard tool.
pub const CUSTOM_ARTBOARD_NAME: &str = "Artboard";
/// Least width of the field renaming an artboard, in pixels.
const RENAME_MIN_WIDTH: f32 = 160.;
/// Extra width of the rename field beyond the name, in pixels.
const RENAME_SLACK: f32 = 40.;
/// Height of the rename field, in pixels.
const RENAME_HEIGHT: f32 = 24.;

/// The shapes among `moved` (on the layer of `shapes`) whose membership a
/// move, resize or drop decides: not artboards (which take nothing in), nor
/// members of an artboard among `moved`, which travel with it and stay its
/// members.
pub fn membership_targets(shapes: &[Shape], moved: &[Uuid]) -> Vec<Uuid> {
    let picked: HashSet<Uuid> = moved.iter().copied().collect();
    let boards: HashSet<Uuid> = shapes
        .iter()
        .filter(|s| s.is_artboard() && picked.contains(&s.id))
        .map(|s| s.id)
        .collect();
    shapes
        .iter()
        .filter(|s| picked.contains(&s.id) && !s.is_artboard())
        .filter(|s| !s.artboard.is_some_and(|a| boards.contains(&a)))
        .map(|s| s.id)
        .collect()
}

/// Gives the shapes `moved` on the active layer (see
/// [`membership_targets`]) their membership after a move, resize or drop.
/// Called inside the gesture of that move, it joins its undo step.
pub fn settle_membership(session: &mut DocumentSession, moved: &[Uuid]) {
    let targets = membership_targets(session.shapes_on_active_layer(), moved);
    if !targets.is_empty() {
        session.assign_membership(&targets);
    }
}

/// Adds `shapes` (placed, pasted or duplicated) to the active layer and
/// gives those not travelling with an artboard among them their
/// membership, as one undo step.
pub fn add_with_membership(session: &mut DocumentSession, shapes: Vec<Shape>) {
    let ids: Vec<Uuid> = shapes.iter().map(|s| s.id).collect();
    let targets = membership_targets(&shapes, &ids);
    session.add_shapes(shapes);
    if !targets.is_empty() {
        session.assign_membership(&targets);
    }
}

/// `ids` without the members of artboards among `shapes`: what to select
/// after adding `shapes`, so a duplicated or pasted artboard is selected
/// alone.
pub fn without_members(shapes: &[Shape], ids: Vec<Uuid>) -> Vec<Uuid> {
    let boards: HashSet<Uuid> = shapes
        .iter()
        .filter(|s| s.is_artboard())
        .map(|s| s.id)
        .collect();
    let members: HashSet<Uuid> = shapes
        .iter()
        .filter(|s| s.artboard.is_some_and(|a| boards.contains(&a)))
        .map(|s| s.id)
        .collect();
    ids.into_iter().filter(|id| !members.contains(id)).collect()
}

/// Renames artboard `id` on the active layer to `name` as one undo step.
/// A blank name keeps the old one; an unchanged name records nothing.
pub fn rename_artboard(session: &mut DocumentSession, id: Uuid, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    session.update_shapes(&[id], |s| {
        if s.is_artboard() {
            s.name = Some(name.to_string());
        }
    });
}

/// Whether canvas point `at` is on the name of an artboard with `frame`,
/// the name being `text_width` pixels wide (see [`name_label_rect`]).
pub fn name_hit(frame: &Frame, text_width: f32, vp: &Viewport, at: Pt) -> bool {
    let on_screen = screen_rect(&frame_aabb(frame), vp, 0.);
    let label = name_label_rect(on_screen, text_width);
    label.contains(vp.canvas_to_screen(at))
}

/// Whether canvas point `at` is within `tolerance` of artboard `shape`'s
/// frame: all of it, filled or not, is the artboard's body.
pub fn artboard_body_hit(shape: &Shape, at: Pt, tolerance: f32) -> bool {
    let r = frame_aabb(&shape.frame);
    let grown = Rect {
        x: r.x - tolerance,
        y: r.y - tolerance,
        w: r.w + 2. * tolerance,
        h: r.h + 2. * tolerance,
    };
    grown.contains(at)
}

/// The frames of the clipping artboards among `shapes` (one layer), by
/// id: their members show, and are hit, only inside them.
pub fn clip_frames(shapes: &[Shape]) -> Vec<(Uuid, Rect)> {
    shapes
        .iter()
        .filter(|s| s.is_artboard() && s.artboard_props.as_ref().is_none_or(|p| p.clip))
        .map(|s| (s.id, frame_aabb(&s.frame)))
        .collect()
}

/// Whether canvas point `at` is on a part of `shape` clipped away by its
/// artboard (see [`clip_frames`]).
pub fn clipped_away(shape: &Shape, clips: &[(Uuid, Rect)], at: Pt) -> bool {
    let Some(board) = shape.artboard else {
        return false;
    };
    clips
        .iter()
        .find(|(id, _)| *id == board)
        .is_some_and(|(_, r)| !r.contains(at))
}

impl EditorWindow {
    /// The topmost artboard of the active layer whose name is at canvas
    /// point `at`, as last painted; none on a hidden layer or for locked
    /// artboards.
    pub(super) fn artboard_name_at(&self, at: Pt) -> Option<Uuid> {
        if !self.session.active_layer_selectable() {
            return None;
        }
        let vp = &self.session.page().viewport;
        self.session
            .shapes_on_active_layer()
            .iter()
            .rev()
            .filter(|s| s.is_artboard() && !s.locked)
            .find(|s| {
                self.artboard_names
                    .iter()
                    .find(|(id, _)| *id == s.id)
                    .is_some_and(|(_, w)| name_hit(&s.frame, *w, vp, at))
            })
            .map(|s| s.id)
    }

    /// Whether the selection chrome has a rotate handle: not when an
    /// artboard is selected, as artboards are never rotated.
    pub(super) fn chrome_rotates(&self) -> bool {
        !self
            .selection
            .iter()
            .filter_map(|id| self.session.find_shape(*id))
            .any(Shape::is_artboard)
    }

    /// The field renaming an artboard, over its name: Enter or leaving it
    /// applies the name, Escape cancels.
    pub(super) fn render_artboard_rename(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rename = self.rename.as_ref()?;
        let RenameTarget::Artboard(id) = rename.target else {
            return None;
        };
        let shape = self.session.find_shape(id)?;
        let vp = &self.session.page().viewport;
        let width = self
            .artboard_names
            .iter()
            .find(|(n, _)| *n == id)
            .map_or(0., |(_, w)| *w);
        let label = name_label_rect(screen_rect(&frame_aabb(&shape.frame), vp, 0.), width);
        let top = label.y + (label.h - RENAME_HEIGHT) / 2.;
        let input = rename.input.clone();
        Some(
            div()
                .absolute()
                .left(px(label.x))
                .top(px(top))
                .w(px((width + RENAME_SLACK).max(RENAME_MIN_WIDTH)))
                .h(px(RENAME_HEIGHT))
                .bg(cx.theme().background)
                .child(self.render_rename_input(&input, cx))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob import would shadow `#[test]`.
    use super::{
        add_with_membership, artboard_body_hit, clip_frames, clipped_away, membership_targets,
        name_hit, rename_artboard, settle_membership, without_members,
    };
    use crate::editor::{GestureOwner, SessionGesture};
    use mindweave_core::geometry::Frame;
    use mindweave_core::{duplicate, DocumentSession, Shape, Viewport};
    use uuid::Uuid;

    fn frame(x: f32, y: f32, w: f32, h: f32) -> Frame {
        Frame {
            x,
            y,
            w,
            h,
            ..Frame::default()
        }
    }

    fn rect(x: f32, y: f32) -> Shape {
        let mut s = Shape::new_text_box((x, y));
        s.shape = "rectangle".into();
        s.library = "basic-shapes".into();
        s.text_box = None;
        s.label = None;
        s.frame = frame(x, y, 20., 20.);
        s
    }

    fn board(x: f32, y: f32, w: f32, h: f32) -> Shape {
        Shape::new_artboard(frame(x, y, w, h), "Board")
    }

    fn session() -> (tempfile::TempDir, DocumentSession) {
        let dir = tempfile::tempdir().unwrap();
        let session = DocumentSession::create(dir.path().join("a.mindweave")).unwrap();
        (dir, session)
    }

    fn member_of(s: &DocumentSession, id: Uuid) -> Option<Uuid> {
        s.find_shape(id).unwrap().artboard
    }

    fn pos(s: &DocumentSession, id: Uuid) -> (f32, f32) {
        let f = s.find_shape(id).unwrap().frame;
        (f.x, f.y)
    }

    #[test]
    fn moved_artboards_and_their_members_take_no_membership() {
        let a = board(0., 0., 100., 100.);
        let mut m = rect(150., 0.);
        m.artboard = Some(a.id);
        let loose = rect(10., 10.);
        let shapes = vec![a.clone(), m.clone(), loose.clone()];
        // The artboard and its member travel together; only the loose
        // shape is decided.
        assert_eq!(
            membership_targets(&shapes, &[a.id, m.id, loose.id]),
            vec![loose.id]
        );
        // A member moved on its own is decided again.
        assert_eq!(membership_targets(&shapes, &[m.id]), vec![m.id]);
        assert!(membership_targets(&shapes, &[a.id]).is_empty());
    }

    #[test]
    fn a_dropped_move_and_its_membership_are_one_undo_step() {
        let (_d, mut s) = session();
        let a = board(0., 0., 100., 100.);
        let r = rect(500., 0.);
        let (aid, rid) = (a.id, r.id);
        s.add_shapes(vec![a, r]);
        // As the Pointer tool drags: the gesture opens, frames change,
        // membership is settled on drop, then the gesture ends.
        let mut gesture = SessionGesture::default();
        gesture.begin(&mut s, GestureOwner::Pointer);
        s.update_shapes(&[rid], |sh| sh.frame.x = 40.);
        settle_membership(&mut s, &[rid]);
        gesture.end(&mut s, GestureOwner::Pointer);
        assert_eq!(member_of(&s, rid), Some(aid));
        assert!(s.undo());
        assert_eq!(pos(&s, rid), (500., 0.));
        assert_eq!(member_of(&s, rid), None);
        assert!(s.redo());
        assert_eq!(member_of(&s, rid), Some(aid));
    }

    #[test]
    fn moving_an_artboard_keeps_a_member_outside_its_frame() {
        let (_d, mut s) = session();
        let a = board(0., 0., 100., 100.);
        let mut m = rect(150., 0.);
        m.artboard = Some(a.id);
        let (aid, mid) = (a.id, m.id);
        s.add_shapes(vec![a, m]);
        let mut gesture = SessionGesture::default();
        gesture.begin(&mut s, GestureOwner::Pointer);
        s.move_shapes(&[aid], 10., 0.);
        settle_membership(&mut s, &[aid, mid]);
        gesture.end(&mut s, GestureOwner::Pointer);
        assert_eq!(member_of(&s, mid), Some(aid));
    }

    #[test]
    fn a_placed_shape_joins_the_artboard_in_the_placing_step() {
        let (_d, mut s) = session();
        let a = board(0., 0., 100., 100.);
        let aid = a.id;
        s.add_shapes(vec![a]);
        let r = rect(40., 40.);
        let rid = r.id;
        add_with_membership(&mut s, vec![r]);
        assert_eq!(member_of(&s, rid), Some(aid));
        // One undo removes the shape; redo brings it back a member.
        assert!(s.undo());
        assert!(s.find_shape(rid).is_none());
        assert!(s.find_shape(aid).is_some());
        assert!(s.redo());
        assert_eq!(member_of(&s, rid), Some(aid));
    }

    #[test]
    fn a_new_artboard_captures_nothing() {
        let (_d, mut s) = session();
        let r = rect(40., 40.);
        let rid = r.id;
        s.add_shapes(vec![r]);
        let a = board(0., 0., 100., 100.);
        let aid = a.id;
        add_with_membership(&mut s, vec![a]);
        assert_eq!(member_of(&s, rid), None);
        // Resizing it over the shape takes nothing in either.
        settle_membership(&mut s, &[aid]);
        assert_eq!(member_of(&s, rid), None);
    }

    #[test]
    fn a_duplicated_artboard_keeps_its_members_and_is_selected_alone() {
        let (_d, mut s) = session();
        let a = board(0., 0., 100., 100.);
        let mut m = rect(150., 0.);
        m.artboard = Some(a.id);
        let copies = duplicate(&[a, m]);
        let (copy_board, copy_member) = (copies[0].id, copies[1].id);
        let ids: Vec<Uuid> = copies.iter().map(|c| c.id).collect();
        assert_eq!(without_members(&copies, ids), vec![copy_board]);
        add_with_membership(&mut s, copies);
        // The copied member stays with the copied artboard, though it lies
        // outside its frame.
        assert_eq!(member_of(&s, copy_member), Some(copy_board));
    }

    #[test]
    fn renaming_an_artboard_is_one_step_and_blank_keeps_the_name() {
        let (_d, mut s) = session();
        let a = board(0., 0., 100., 100.);
        let aid = a.id;
        s.add_shapes(vec![a]);
        let name = |s: &DocumentSession| s.find_shape(aid).unwrap().name.clone();
        rename_artboard(&mut s, aid, "  Home  ");
        assert_eq!(name(&s).as_deref(), Some("Home"));
        rename_artboard(&mut s, aid, "   ");
        assert_eq!(name(&s).as_deref(), Some("Home"));
        assert!(s.undo());
        assert_eq!(name(&s).as_deref(), Some("Board"));
    }

    #[test]
    fn names_are_hit_above_the_top_left_corner_at_any_zoom() {
        let f = frame(100., 100., 200., 100.);
        let vp = Viewport {
            x: 0.,
            y: 0.,
            zoom: 2.,
        };
        // The frame's top is at 200 px; the label spans 180..196 px.
        assert!(name_hit(&f, 50., &vp, (105., 94.)));
        // Past its text, or inside the frame: not the name.
        assert!(!name_hit(&f, 50., &vp, (140., 94.)));
        assert!(!name_hit(&f, 50., &vp, (105., 101.)));
    }

    #[test]
    fn the_whole_artboard_frame_is_its_body() {
        let mut a = board(0., 0., 100., 100.);
        a.style.fill = None;
        assert!(artboard_body_hit(&a, (50., 50.), 0.));
        assert!(artboard_body_hit(&a, (102., 50.), 3.));
        assert!(!artboard_body_hit(&a, (110., 50.), 3.));
    }

    #[test]
    fn clipped_away_parts_of_members_are_not_hit() {
        let a = board(0., 0., 100., 100.);
        let mut m = rect(90., 0.);
        m.artboard = Some(a.id);
        let clips = clip_frames(std::slice::from_ref(&a));
        assert!(!clipped_away(&m, &clips, (95., 10.)));
        assert!(clipped_away(&m, &clips, (105., 10.)));
        assert!(!clipped_away(&rect(90., 0.), &clips, (105., 10.)));
        // An artboard that does not clip shows all of its members.
        let mut open = a.clone();
        open.artboard_props.as_mut().unwrap().clip = false;
        assert!(!clipped_away(&m, &clip_frames(&[open]), (105., 10.)));
    }
}
