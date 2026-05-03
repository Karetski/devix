//! View = per-frame editor state (selection + sticky col + scroll).
//! Owned by Workspace, indexed by ViewId.

use devix_buffer::{Range, Selection, Transaction};
use devix_collection::CollectionState;
use slotmap::new_key_type;

use crate::document::DocId;

new_key_type! { pub struct ViewId; }

/// What the next render pass should do with the view's scroll offset.
///
/// * `Anchored` — bump scroll the minimum amount needed to keep the cursor
///   visible (the editor "follows the cursor"). The default for keyboard
///   navigation and edits.
/// * `Free` — leave scroll alone. Set by `Action::ScrollBy` so a wheel scroll
///   past the cursor doesn't snap back on the next frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollMode {
    Anchored,
    Free,
}

pub struct View {
    pub doc: DocId,
    pub selection: Selection,
    /// Sticky column for vertical motion.
    pub target_col: Option<usize>,
    /// Editor scroll state. `scroll_y` is the line index of the topmost
    /// visible line (one cell per line for now); `scroll_x` is reserved for
    /// horizontal scrolling within long lines.
    pub scroll: CollectionState,
    pub scroll_mode: ScrollMode,
    /// Active hover popup state, or `None` when no hover is in flight or
    /// being shown. Cleared on cursor motion / edit (the dispatcher resets
    /// this anywhere `target_col` is reset).
    pub hover: Option<HoverState>,
}

/// Per-view hover popup state. Hover is request-driven: dispatch records
/// `Pending` with the cursor's char index, the LSP coordinator pumps back
/// a response, and the App-side drain matches it against `anchor_char` —
/// stale answers (cursor moved) are dropped on the floor.
#[derive(Clone, Debug)]
pub struct HoverState {
    pub anchor_char: usize,
    pub status: HoverStatus,
}

#[derive(Clone, Debug)]
pub enum HoverStatus {
    Pending,
    Ready(Vec<String>),
    Empty,
}

impl View {
    pub fn new(doc: DocId) -> Self {
        Self {
            doc,
            selection: Selection::point(0),
            target_col: None,
            scroll: CollectionState::default(),
            scroll_mode: ScrollMode::Anchored,
            hover: None,
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
        // Any motion dismisses an open hover popup. Even within the same
        // logical position, the user's intent on pressing a motion key is
        // "move on", and an anchored popup would feel sticky.
        self.hover = None;
    }

    /// Apply a transaction's selection_after; the buffer mutation happens on
    /// the Document side (the caller does buffer.apply(tx) first).
    pub fn adopt_selection_after(&mut self, tx: &Transaction) {
        self.selection = tx.selection_after.clone();
        self.target_col = None;
        self.hover = None;
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
        assert_eq!(v.scroll_mode, ScrollMode::Anchored);
        assert!(v.target_col.is_none());
        assert_eq!(v.scroll_top(), 0);
    }
}
