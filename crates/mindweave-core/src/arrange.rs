//! Groups and stacking order of a layer's shapes.
//!
//! A shape's `groups` lists the groups it belongs to, outermost first;
//! groups may hold groups. A click selects a shape's whole outermost group
//! (its *unit*). Order commands move units, never splitting a group.

use std::collections::HashSet;

use uuid::Uuid;

use crate::shape::Shape;

/// A stacking-order command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    /// Above everything else on the layer.
    ToFront,
    /// Above the next shape or group above it.
    Forward,
    /// Below the next shape or group below it.
    Backward,
    /// Below everything else on the layer.
    ToBack,
}

/// The id naming `shape`'s unit: its outermost group, else itself.
pub fn unit_of(shape: &Shape) -> Uuid {
    shape.groups.first().copied().unwrap_or(shape.id)
}

/// The shapes of `shapes` in the same unit as `id`, in order (just `id`
/// when it is in no group); empty when `id` is not there.
pub fn unit_members(shapes: &[Shape], id: Uuid) -> Vec<Uuid> {
    let Some(unit) = shapes.iter().find(|s| s.id == id).map(unit_of) else {
        return Vec::new();
    };
    shapes
        .iter()
        .filter(|s| unit_of(s) == unit)
        .map(|s| s.id)
        .collect()
}

/// `ids` grown to whole units, in layer order.
pub fn expand_to_units(shapes: &[Shape], ids: &[Uuid]) -> Vec<Uuid> {
    let picked: HashSet<Uuid> = ids.iter().copied().collect();
    let units: HashSet<Uuid> = shapes
        .iter()
        .filter(|s| picked.contains(&s.id))
        .map(unit_of)
        .collect();
    shapes
        .iter()
        .filter(|s| units.contains(&unit_of(s)))
        .map(|s| s.id)
        .collect()
}

/// `ids` without the units they hold only part of, in layer order: what
/// a marquee selects when it must take groups whole.
pub fn whole_units_only(shapes: &[Shape], ids: &[Uuid]) -> Vec<Uuid> {
    let picked: HashSet<Uuid> = ids.iter().copied().collect();
    let partial: HashSet<Uuid> = shapes
        .iter()
        .filter(|s| !picked.contains(&s.id))
        .map(unit_of)
        .collect();
    shapes
        .iter()
        .filter(|s| picked.contains(&s.id) && !partial.contains(&unit_of(s)))
        .map(|s| s.id)
        .collect()
}

/// Groups the shapes with `ids` (at least two) into new group `group`:
/// it becomes their outermost group, and they move together, keeping
/// their order, to where the topmost of them was. Returns whether
/// anything changed.
pub fn group(shapes: &mut Vec<Shape>, ids: &[Uuid], group: Uuid) -> bool {
    let picked: HashSet<Uuid> = ids.iter().copied().collect();
    let Some(top) = shapes.iter().rposition(|s| picked.contains(&s.id)) else {
        return false;
    };
    if shapes.iter().filter(|s| picked.contains(&s.id)).count() < 2 {
        return false;
    }
    let below = shapes[..top]
        .iter()
        .filter(|s| !picked.contains(&s.id))
        .count();
    let (mut members, rest): (Vec<Shape>, Vec<Shape>) = std::mem::take(shapes)
        .into_iter()
        .partition(|s| picked.contains(&s.id));
    for m in &mut members {
        m.groups.insert(0, group);
    }
    *shapes = rest;
    shapes.splice(below..below, members);
    true
}

/// Takes the shapes with `ids` out of their outermost group. Returns
/// whether anything changed.
pub fn ungroup(shapes: &mut [Shape], ids: &[Uuid]) -> bool {
    let picked: HashSet<Uuid> = ids.iter().copied().collect();
    let mut changed = false;
    for s in shapes.iter_mut().filter(|s| picked.contains(&s.id)) {
        if !s.groups.is_empty() {
            s.groups.remove(0);
            changed = true;
        }
    }
    changed
}

/// Moves the shapes with `ids` in the stacking order (`shapes` is back to
/// front). Runs of neighbouring shapes in one unit, selected or not, move
/// as blocks, so a group is never split by a shape moving past part of
/// it. Returns whether the order changed.
pub fn arrange(shapes: &mut Vec<Shape>, ids: &[Uuid], how: Arrange) -> bool {
    let picked: HashSet<Uuid> = ids.iter().copied().collect();
    // Blocks of neighbours sharing selectedness and unit.
    let mut blocks: Vec<(bool, Vec<Shape>)> = Vec::new();
    for s in std::mem::take(shapes) {
        let selected = picked.contains(&s.id);
        match blocks.last_mut() {
            Some((sel, members))
                if *sel == selected && members.last().map(unit_of) == Some(unit_of(&s)) =>
            {
                members.push(s)
            }
            _ => blocks.push((selected, vec![s])),
        }
    }
    let before: Vec<Uuid> = blocks
        .iter()
        .flat_map(|(_, m)| m.iter().map(|s| s.id))
        .collect();
    match how {
        Arrange::ToFront => blocks.sort_by_key(|(sel, _)| *sel),
        Arrange::ToBack => blocks.sort_by_key(|(sel, _)| !*sel),
        Arrange::Forward => {
            for i in (0..blocks.len().saturating_sub(1)).rev() {
                if blocks[i].0 && !blocks[i + 1].0 {
                    blocks.swap(i, i + 1);
                }
            }
        }
        Arrange::Backward => {
            for i in 1..blocks.len() {
                if blocks[i].0 && !blocks[i - 1].0 {
                    blocks.swap(i, i - 1);
                }
            }
        }
    }
    *shapes = blocks.into_iter().flat_map(|(_, m)| m).collect();
    shapes.iter().map(|s| s.id).ne(before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes(n: usize) -> Vec<Shape> {
        (0..n)
            .map(|i| {
                let mut s = Shape::new_text_box((i as f32, 0.0));
                s.name = Some(format!("{i}"));
                s
            })
            .collect()
    }

    fn names(shapes: &[Shape]) -> String {
        shapes
            .iter()
            .map(|s| s.name.clone().unwrap_or_default())
            .collect()
    }

    fn ids(shapes: &[Shape], which: &[usize]) -> Vec<Uuid> {
        which.iter().map(|&i| shapes[i].id).collect()
    }

    #[test]
    fn grouping_gathers_members_where_the_topmost_was() {
        let mut s = shapes(5);
        let g = Uuid::new_v4();
        let pick = ids(&s, &[0, 3]);
        assert!(group(&mut s, &pick, g));
        assert_eq!(names(&s), "12034");
        assert_eq!(s[2].groups, vec![g]);
        assert_eq!(unit_members(&s, pick[0]), pick);
        // A click on either member selects both; a lone shape is its own.
        assert_eq!(expand_to_units(&s, &pick[1..]), pick);
        assert_eq!(unit_members(&s, s[0].id), vec![s[0].id]);
        // One shape is not a group.
        let lone = ids(&s, &[4]);
        assert!(!group(&mut s, &lone, Uuid::new_v4()));
    }

    #[test]
    fn groups_nest_and_ungroup_one_level_at_a_time() {
        let mut s = shapes(3);
        let (inner, outer) = (Uuid::new_v4(), Uuid::new_v4());
        let ab = ids(&s, &[0, 1]);
        group(&mut s, &ab, inner);
        let all = ids(&s, &[0, 1, 2]);
        group(&mut s, &all, outer);
        assert_eq!(s[0].groups, vec![outer, inner]);
        assert_eq!(unit_members(&s, s[2].id).len(), 3);
        assert!(ungroup(&mut s, &all));
        assert_eq!(s[0].groups, vec![inner]);
        assert_eq!(unit_members(&s, s[2].id), vec![s[2].id]);
        assert_eq!(unit_members(&s, ab[0]), ab);
        assert!(ungroup(&mut s, &ab));
        assert!(!ungroup(&mut s, &ab));
    }

    #[test]
    fn marquees_take_groups_whole_or_not_at_all() {
        let mut s = shapes(4);
        let pick = ids(&s, &[1, 2]);
        group(&mut s, &pick, Uuid::new_v4());
        let lone = s[0].id;
        assert_eq!(whole_units_only(&s, &[lone, pick[0]]), vec![lone]);
        assert_eq!(whole_units_only(&s, &[lone, pick[0], pick[1]]).len(), 3);
    }

    #[test]
    fn order_commands_move_selected_shapes_and_keep_groups_whole() {
        let mut s = shapes(5);
        let one = ids(&s, &[1]);
        assert!(arrange(&mut s, &one, Arrange::ToFront));
        assert_eq!(names(&s), "02341");
        assert!(!arrange(&mut s, &one, Arrange::ToFront));
        assert!(arrange(&mut s, &one, Arrange::ToBack));
        assert_eq!(names(&s), "10234");
        assert!(arrange(&mut s, &one, Arrange::Forward));
        assert_eq!(names(&s), "01234");
        assert!(arrange(&mut s, &one, Arrange::Backward));
        assert_eq!(names(&s), "10234");

        // "2" and "3" form a group: "1" steps over it as a whole.
        let mut s = shapes(5);
        let pair = ids(&s, &[2, 3]);
        group(&mut s, &pair, Uuid::new_v4());
        let one = ids(&s, &[1]);
        arrange(&mut s, &one, Arrange::Forward);
        assert_eq!(names(&s), "02314");
        // The group moves down past "0" as a block.
        arrange(&mut s, &pair, Arrange::ToBack);
        assert_eq!(names(&s), "23014");
        // Several selected shapes keep their order among themselves.
        let mut s = shapes(5);
        let two = ids(&s, &[0, 2]);
        arrange(&mut s, &two, Arrange::ToFront);
        assert_eq!(names(&s), "13402");
    }
}
