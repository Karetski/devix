//! View = per-frame editor state (selection + sticky col + scroll).
//! Owned by Surface, indexed by ViewId.

use devix_text::{Range, Selection, Transaction};
use slotmap::new_key_type;

use devix_editor::{CompletionState, DocId, HoverState};

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
    /// Editor scroll offset in cells. `.1` is the line index of the topmost
    /// visible line (one cell per line for now); `.0` is reserved for
    /// horizontal scrolling within long lines. Pure data — the render layer
    /// applies layout-aware clamping via `devix_ui::layout` free functions.
    pub scroll: (u32, u32),
    pub scroll_mode: ScrollMode,
    /// Active hover popup state, or `None` when no hover is in flight or
    /// being shown. Cleared on cursor motion / edit (the dispatcher resets
    /// this anywhere `target_col` is reset).
    pub hover: Option<HoverState>,
    /// Active completion popup, or `None` when no popup is in flight or
    /// shown. Survives prefix-extending insertions (the popup re-filters
    /// against the typed prefix); dismissed on cursor motion outside the
    /// query span, on Esc, on accept, or on edits that aren't simple
    /// trailing inserts/backspaces.
    pub completion: Option<CompletionState>,
}

impl View {
    pub fn new(doc: DocId) -> Self {
        Self {
            doc,
            selection: Selection::point(0),
            target_col: None,
            scroll: (0, 0),
            scroll_mode: ScrollMode::Anchored,
            hover: None,
            completion: None,
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
        // Any motion dismisses an open hover popup. Even within the same
        // logical position, the user's intent on pressing a motion key is
        // "move on", and an anchored popup would feel sticky.
        self.hover = None;
        // Cursor-key motion also dismisses completion. Typing-driven motion
        // (InsertChar / DeleteBack) bypasses move_to and refilters instead.
        self.completion = None;
        // Cursor moved → next render must keep the cursor on screen. Wheel
        // scrolls flip back to Free, so a deliberate scroll that's then
        // followed by a key press doesn't get stuck in Free.
        self.scroll_mode = ScrollMode::Anchored;
    }

    /// Replace the selection and reset transient view state (sticky col,
    /// hover, completion, scroll mode). Used by jump-style updates (undo,
    /// redo, select-all, completion-accept) where the new position has no
    /// continuity with prior state.
    pub fn adopt_selection(&mut self, sel: Selection) {
        self.selection = sel;
        self.target_col = None;
        self.hover = None;
        self.completion = None;
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
