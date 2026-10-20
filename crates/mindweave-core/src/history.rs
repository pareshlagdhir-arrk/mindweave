//! Bounded undo/redo stacks of whole-state snapshots.

/// Most undo steps kept; older ones are dropped.
pub const HISTORY_CAP: usize = 200;

/// Undo/redo history of snapshots of `T`.
///
/// The caller records the state *before* each edit; `undo` and `redo`
/// swap the caller's current state for a stored one.
#[derive(Debug, Clone)]
pub struct History<T: Clone> {
    undo: Vec<T>,
    redo: Vec<T>,
    cap: usize,
}

impl<T: Clone> History<T> {
    pub fn new(cap: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cap,
        }
    }

    /// Records `before` as an undo step, clears redo, and drops the oldest
    /// step beyond the cap.
    pub fn record(&mut self, before: T) {
        self.redo.clear();
        self.undo.push(before);
        if self.undo.len() > self.cap {
            let excess = self.undo.len() - self.cap;
            self.undo.drain(..excess);
        }
    }

    /// Returns the state to restore, keeping `current` for redo.
    pub fn undo(&mut self, current: T) -> Option<T> {
        let prev = self.undo.pop()?;
        self.redo.push(current);
        Some(prev)
    }

    /// Returns the state to restore, keeping `current` for undo.
    pub fn redo(&mut self, current: T) -> Option<T> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_undo_redo_roundtrip() {
        let mut h = History::new(10);
        assert!(!h.can_undo() && !h.can_redo());
        assert_eq!(h.undo(0), None);
        h.record(0);
        h.record(1);
        // Current state is 2.
        assert_eq!(h.undo(2), Some(1));
        assert_eq!(h.undo(1), Some(0));
        assert!(!h.can_undo());
        assert_eq!(h.undo(0), None);
        assert!(h.can_redo());
        assert_eq!(h.redo(0), Some(1));
        assert_eq!(h.redo(1), Some(2));
        assert_eq!(h.redo(2), None);
        assert!(h.can_undo() && !h.can_redo());
    }

    #[test]
    fn cap_drops_oldest() {
        let mut h = History::new(3);
        for i in 0..5 {
            h.record(i);
        }
        assert_eq!(h.undo(5), Some(4));
        assert_eq!(h.undo(4), Some(3));
        assert_eq!(h.undo(3), Some(2));
        assert_eq!(h.undo(2), None);
        assert_eq!(HISTORY_CAP, 200);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut h = History::new(10);
        h.record(0);
        assert_eq!(h.undo(1), Some(0));
        assert!(h.can_redo());
        h.record(0);
        assert!(!h.can_redo());
        assert_eq!(h.redo(9), None);
    }
}
