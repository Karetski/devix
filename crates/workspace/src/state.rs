//! Per-buffer editor state: buffer + selection + sticky column + scroll.
//!
//! Methods migrate the motion/edit helpers that previously lived as free
//! functions in `crates/app/src/main.rs`. No logic change.

use devix_buffer::{Buffer, Range, Selection, Transaction};

pub struct EditorState {
    pub buffer: Buffer,
    pub selection: Selection,
    /// Sticky column for vertical motion. Reset on horizontal motion or edit.
    pub target_col: Option<usize>,
    pub scroll_top: usize,
}

impl EditorState {
    pub fn new(buffer: Buffer) -> Self {
        Self {
            buffer,
            selection: Selection::point(0),
            target_col: None,
            scroll_top: 0,
        }
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

    pub fn move_vertical(&mut self, down: bool, extend: bool) {
        let head = self.primary().head;
        let col = self
            .target_col
            .unwrap_or_else(|| self.buffer.col_of_char(head));
        let new = if down {
            self.buffer.move_down(head, Some(col))
        } else {
            self.buffer.move_up(head, Some(col))
        };
        self.target_col = Some(col);
        self.move_to(new, extend, true);
    }

    /// Apply a transaction; updates selection and clears the sticky column.
    pub fn apply_tx(&mut self, tx: Transaction) {
        let after = tx.selection_after.clone();
        self.buffer.apply(tx);
        self.selection = after;
        self.target_col = None;
    }
}
