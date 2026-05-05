//! Buffer transactions.
//!
//! Every mutation flows through a `Transaction`. Applying one returns the
//! inverse, which the buffer pushes onto its undo stack. Undo pops and
//! re-applies (recording its own inverse for redo).
//!
//! Changes within a transaction must be sorted ascending by `start` and may
//! not overlap — they're applied right-to-left so earlier offsets stay valid.

use ropey::Rope;

use crate::selection::Selection;

#[derive(Clone, Debug)]
pub struct Change {
    pub start: usize,
    pub remove_len: usize,
    pub insert: String,
}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub changes: Vec<Change>,
    pub selection_before: Selection,
    pub selection_after: Selection,
}

impl Transaction {
    /// Apply to `rope`; returns the inverse (so re-applying it undoes this).
    pub fn apply(&self, rope: &mut Rope) -> Transaction {
        let mut inverse_changes: Vec<Change> = Vec::with_capacity(self.changes.len());
        // Right-to-left so each change's offsets are still valid in `rope`.
        for ch in self.changes.iter().rev() {
            let removed: String = rope
                .slice(ch.start..ch.start + ch.remove_len)
                .to_string();
            if ch.remove_len > 0 {
                rope.remove(ch.start..ch.start + ch.remove_len);
            }
            if !ch.insert.is_empty() {
                rope.insert(ch.start, &ch.insert);
            }
            inverse_changes.push(Change {
                start: ch.start,
                remove_len: ch.insert.chars().count(),
                insert: removed,
            });
        }
        inverse_changes.reverse();
        Transaction {
            changes: inverse_changes,
            selection_before: self.selection_after.clone(),
            selection_after: self.selection_before.clone(),
        }
    }
}
