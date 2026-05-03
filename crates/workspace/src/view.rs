//! View = per-frame editor state (selection + sticky col + scroll).
//! Owned by Workspace, indexed by ViewId.

use devix_buffer::{Range, Selection, Transaction};
use devix_collection::CollectionState;
use slotmap::new_key_type;

use crate::document::DocId;

new_key_type! { pub struct ViewId; }

pub struct View {
    pub doc: DocId,
    pub selection: Selection,
    /// Sticky column for vertical motion.
    pub target_col: Option<usize>,
    /// Editor scroll state. `scroll_y` is the line index of the topmost
    /// visible line (one cell per line for now); `scroll_x` is reserved for
    /// horizontal scrolling within long lines.
    pub scroll: CollectionState,
    /// Anchored: render keeps the cursor in view. Detached: the scroll state
    /// floats — set by `Action::ScrollBy` so wheel scroll past the cursor
    /// doesn't snap back.
    pub view_anchored: bool,
}

impl View {
    pub fn new(doc: DocId) -> Self {
        Self {
            doc,
            selection: Selection::point(0),
            target_col: None,
            scroll: CollectionState::default(),
            view_anchored: true,
        }
    }

    pub fn scroll_top(&self) -> usize {
        self.scroll.scroll_y as usize
    }

    pub fn set_scroll_top(&mut self, line: usize) {
        // Scroll is bounded to u32 — fine for any practical buffer (4B lines).
        self.scroll.scroll_y = line.min(u32::MAX as usize) as u32;
    }

    pub fn primary(&self) -> Range {
        self.selection.primary()
    }

    pub fn move_to(&mut self, idx: usize, extend: bool, sticky_col: bool) {
        let r = self.primary().put_head(idx, extend);
        *self.selection.primary_mut() = r;
        if !sticky_col {
            self.target_col = None;
        }
    }

    /// Apply a transaction's selection_after; the buffer mutation happens on
    /// the Document side (the caller does buffer.apply(tx) first).
    pub fn adopt_selection_after(&mut self, tx: &Transaction) {
        self.selection = tx.selection_after.clone();
        self.target_col = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn fresh_view_starts_at_origin_anchored() {
        let mut docs: SlotMap<DocId, ()> = SlotMap::with_key();
        let id = docs.insert(());
        let v = View::new(id);
        assert_eq!(v.primary().head, 0);
        assert!(v.view_anchored);
        assert!(v.target_col.is_none());
        assert_eq!(v.scroll_top(), 0);
    }
}
