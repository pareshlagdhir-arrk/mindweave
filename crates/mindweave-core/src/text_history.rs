//! Word-grouped undo/redo for typing inside one text-editing session.

use crate::text::RichText;

/// Maximum number of undo steps kept.
const MAX_STEPS: usize = 500;

/// Text plus selection at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct EditSnapshot {
    pub text: RichText,
    pub caret: usize,
    pub anchor: usize,
}

/// What an edit did; decides whether it merges into the previous step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    InsertWord,
    InsertSpace,
    Delete,
    Style,
    Paste,
    Other,
}

#[derive(Clone, Debug)]
struct Step {
    snapshot: EditSnapshot,
    kind: EditKind,
}

/// Undo/redo stacks for one editing session.
#[derive(Debug, Default)]
pub struct TextHistory {
    undo: Vec<Step>,
    redo: Vec<Step>,
    last_kind: Option<EditKind>,
}

impl TextHistory {
    /// A history for a session that starts at `initial`.
    pub fn new(_initial: EditSnapshot) -> Self {
        TextHistory::default()
    }

    /// Records an edit that has just been made; `before` is the state
    /// before it. Consecutive word insertions or deletions merge into one
    /// step unless [`break_group`](Self::break_group) intervened.
    pub fn record(&mut self, before: EditSnapshot, kind: EditKind) {
        self.redo.clear();
        let mergeable = matches!(kind, EditKind::InsertWord | EditKind::Delete);
        if mergeable && self.last_kind == Some(kind) && !self.undo.is_empty() {
            return;
        }
        self.undo.push(Step {
            snapshot: before,
            kind,
        });
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.last_kind = Some(kind);
    }

    /// Ends the current merge group (caret jump or selection change).
    pub fn break_group(&mut self) {
        self.last_kind = None;
    }

    /// Steps back; `current` becomes redoable. `None` when nothing to undo.
    pub fn undo(&mut self, current: EditSnapshot) -> Option<EditSnapshot> {
        let step = self.undo.pop()?;
        self.redo.push(Step {
            snapshot: current,
            kind: step.kind,
        });
        self.last_kind = None;
        Some(step.snapshot)
    }

    /// Steps forward; `current` becomes undoable again.
    pub fn redo(&mut self, current: EditSnapshot) -> Option<EditSnapshot> {
        let step = self.redo.pop()?;
        self.undo.push(Step {
            snapshot: current,
            kind: step.kind,
        });
        self.last_kind = None;
        Some(step.snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{HAlign, RunStyle};

    fn snap(s: &str, caret: usize) -> EditSnapshot {
        let mut text = RichText::new(RunStyle::default(), HAlign::Left);
        text.insert(0, s, &RunStyle::default());
        EditSnapshot {
            text,
            caret,
            anchor: caret,
        }
    }

    #[test]
    fn typing_a_word_is_one_step() {
        let mut h = TextHistory::new(snap("", 0));
        h.record(snap("", 0), EditKind::InsertWord);
        h.record(snap("a", 1), EditKind::InsertWord);
        h.record(snap("ab", 2), EditKind::InsertWord);
        assert_eq!(h.undo(snap("abc", 3)), Some(snap("", 0)));
        assert_eq!(h.undo(snap("", 0)), None);
    }

    #[test]
    fn space_starts_a_new_step() {
        let mut h = TextHistory::new(snap("", 0));
        h.record(snap("", 0), EditKind::InsertWord);
        h.record(snap("a", 1), EditKind::InsertSpace);
        h.record(snap("a ", 2), EditKind::InsertWord);
        assert_eq!(h.undo(snap("a b", 3)), Some(snap("a ", 2)));
        assert_eq!(h.undo(snap("a ", 2)), Some(snap("a", 1)));
        assert_eq!(h.undo(snap("a", 1)), Some(snap("", 0)));
    }

    #[test]
    fn caret_jump_breaks_group() {
        let mut h = TextHistory::new(snap("", 0));
        h.record(snap("", 0), EditKind::InsertWord);
        h.break_group();
        h.record(snap("a", 0), EditKind::InsertWord);
        assert_eq!(h.undo(snap("ba", 1)), Some(snap("a", 0)));
        assert_eq!(h.undo(snap("a", 0)), Some(snap("", 0)));
    }

    #[test]
    fn style_change_is_its_own_step() {
        let mut h = TextHistory::new(snap("", 0));
        h.record(snap("", 0), EditKind::InsertWord);
        h.record(snap("a", 1), EditKind::Style);
        h.record(snap("a", 1), EditKind::Style);
        h.record(snap("a", 1), EditKind::InsertWord);
        assert_eq!(h.undo(snap("ab", 2)), Some(snap("a", 1)));
        assert_eq!(h.undo(snap("a", 1)), Some(snap("a", 1)));
        assert_eq!(h.undo(snap("a", 1)), Some(snap("a", 1)));
        assert_eq!(h.undo(snap("a", 1)), Some(snap("", 0)));
    }

    #[test]
    fn undo_redo_restore_caret() {
        let mut h = TextHistory::new(snap("hi", 2));
        h.record(snap("hi", 2), EditKind::Paste);
        let after = snap("hi there", 8);
        let back = h.undo(after.clone()).unwrap();
        assert_eq!((back.caret, back.anchor), (2, 2));
        assert_eq!(h.redo(back), Some(after.clone()));
        assert_eq!(h.undo(after), Some(snap("hi", 2)));
        // A new edit clears redo.
        h.record(snap("hi", 2), EditKind::Other);
        assert_eq!(h.redo(snap("x", 1)), None);
    }

    #[test]
    fn history_is_capped() {
        let mut h = TextHistory::new(snap("", 0));
        for i in 0..600 {
            h.record(snap("", i), EditKind::Other);
        }
        let mut n = 0;
        while h.undo(snap("", 0)).is_some() {
            n += 1;
        }
        assert_eq!(n, 500);
    }

    #[test]
    fn undo_and_redo_reset_the_merge_group() {
        let mut h = TextHistory::new(snap("", 0));
        h.record(snap("", 0), EditKind::InsertWord);
        h.record(snap("a", 1), EditKind::Other);
        h.record(snap("ab", 2), EditKind::InsertWord);
        // Undo the word, then type again: must not merge into the older
        // InsertWord step, even though it is the same kind.
        let back = h.undo(snap("abc", 3)).unwrap();
        assert_eq!(back, snap("ab", 2));
        h.record(back, EditKind::InsertWord);
        assert_eq!(h.undo(snap("abd", 3)), Some(snap("ab", 2)));
        assert_eq!(h.undo(snap("ab", 2)), Some(snap("a", 1)));
        assert_eq!(h.undo(snap("a", 1)), Some(snap("", 0)));
        // Redo also resets the group.
        let again = h.redo(snap("", 0)).unwrap();
        h.record(again, EditKind::InsertWord);
        assert!(h.undo(snap("x", 1)).is_some());
    }
}
