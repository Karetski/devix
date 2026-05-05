//! Cursor = per-tab editing state (selection + sticky col + scroll).
//! Owned by Surface, indexed by CursorId. One per open tab.

use devix_text::{Range, Selection, Transaction};
use devix_workspace::DocId;
use slotmap::new_key_type;

new_key_type! { pub struct CursorId; }

/// What the next render pass should do with the cursor's scroll offset.
///
/// * `Anchored` — bump scroll the minimum amount needed to keep the caret
///   visible (the editor "follows the cursor"). The default for keyboard
///   navigation and edits.
/// * `Free` — leave scroll alone. Set by `Action::ScrollBy` so a wheel scroll
///   past the caret doesn't snap back on the next frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollMode {
    Anchored,
    Free,
}

pub struct Cursor {
    pub doc: DocId,
    pub selection: Selection,
    /// Sticky column for vertical motion.
    pub target_col: Option<usize>,
    /// Editor scroll offset in cells. `.1` is the line index of the topmost
    /// visible line (one cell per line for now); `.0` is reserved for
    /// horizontal scrolling within long lines. Pure data — the render layer
    /// applies layout-aware clamping via `devix_ui::layout` free functions.
    pub scroll: (u32, u32),
    pub scroll_mode: ScrollMode,
}

impl Cursor {
    pub fn new(doc: DocId) -> Self {
        Self {
            doc,
            selection: Selection::point(0),
            target_col: None,
            scroll: (0, 0),
            scroll_mode: ScrollMode::Anchored,
        }
    }

    pub fn scroll_top(&self) -> usize {
        self.scroll.1 as usize
    }

    pub fn set_scroll_top(&mut self, line: usize) {
        // Scroll is bounded to u32 — fine for any practical buffer (4B lines).
        self.scroll.1 = line.min(u32::MAX as usize) as u32;
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
        self.scroll_mode = ScrollMode::Anchored;
    }

    /// Replace the selection and reset transient state (sticky col, scroll
    /// mode). Used by jump-style updates (undo, redo, select-all) where the
    /// new position has no continuity with prior state.
    pub fn adopt_selection(&mut self, sel: Selection) {
        self.selection = sel;
        self.target_col = None;
        self.scroll_mode = ScrollMode::Anchored;
    }

    /// Apply a transaction's selection_after; the buffer mutation happens on
    /// the Document side (the caller does buffer.apply(tx) first).
    pub fn adopt_selection_after(&mut self, tx: &Transaction) {
        self.adopt_selection(tx.selection_after.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn fresh_cursor_starts_at_origin_anchored() {
        let mut docs: SlotMap<DocId, ()> = SlotMap::with_key();
        let id = docs.insert(());
        let c = Cursor::new(id);
        assert_eq!(c.primary().head, 0);
        assert_eq!(c.scroll_mode, ScrollMode::Anchored);
        assert!(c.target_col.is_none());
        assert_eq!(c.scroll_top(), 0);
    }
}
