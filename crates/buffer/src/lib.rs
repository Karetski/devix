//! Buffer = ropey rope + file path + dirty flag + transactional history.
//!
//! Phase 2 wires:
//! - multi-region `Selection` (single range in practice today)
//! - `Transaction` with apply/undo/redo
//! - char-class word motions
//! - external-reload helpers (`reload_from_disk`, `is_dirty_against_disk`)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ropey::{Rope, RopeSlice};

pub mod selection;
pub mod transaction;

pub use selection::{Range, Selection};
pub use transaction::{Change, Transaction};

pub struct Buffer {
    rope: Rope,
    path: Option<PathBuf>,
    /// Monotonic counter; advances on every applied transaction.
    revision: u64,
    /// Revision at last save (or load). `dirty()` compares against this.
    saved_revision: u64,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
}

impl Buffer {
    pub fn empty() -> Self {
        Self::from_rope(Rope::new(), None)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let rope = if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            Rope::from_str(&text)
        } else {
            Rope::new()
        };
        Ok(Self::from_rope(rope, Some(path.to_path_buf())))
    }

    fn from_rope(rope: Rope, path: Option<PathBuf>) -> Self {
        Self {
            rope,
            path,
            revision: 0,
            saved_revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn save(&mut self) -> Result<()> {
        let path = self.path.as_ref().context("buffer has no path")?;
        let mut file = std::fs::File::create(path)
            .with_context(|| format!("creating {}", path.display()))?;
        use std::io::Write;
        for chunk in self.rope.chunks() {
            file.write_all(chunk.as_bytes())?;
        }
        file.sync_all()?;
        self.saved_revision = self.revision;
        Ok(())
    }

    pub fn path(&self) -> Option<&Path> { self.path.as_deref() }
    pub fn dirty(&self) -> bool { self.revision != self.saved_revision }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn len_chars(&self) -> usize { self.rope.len_chars() }

    pub fn line_count(&self) -> usize {
        // ropey reports an extra empty trailing line after `\n`. We always want at
        // least 1 so an empty buffer still has a line to render the cursor on.
        self.rope.len_lines().max(1)
    }

    pub fn line(&self, idx: usize) -> RopeSlice<'_> {
        self.rope.line(idx)
    }

    pub fn line_string(&self, idx: usize) -> String {
        let mut s = self.rope.line(idx).to_string();
        if s.ends_with('\n') { s.pop(); }
        if s.ends_with('\r') { s.pop(); }
        s
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    pub fn line_of_char(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx.min(self.rope.len_chars()))
    }

    pub fn col_of_char(&self, char_idx: usize) -> usize {
        let idx = char_idx.min(self.rope.len_chars());
        idx - self.rope.line_to_char(self.rope.char_to_line(idx))
    }

    /// Length of `line` in chars excluding the trailing newline.
    pub fn line_len_chars(&self, line: usize) -> usize {
        let s = self.rope.line(line);
        let mut len = s.len_chars();
        if len > 0 && s.char(len - 1) == '\n' { len -= 1; }
        if len > 0 && s.char(len - 1) == '\r' { len -= 1; }
        len
    }

    pub fn char_at(&self, idx: usize) -> char {
        self.rope.char(idx)
    }

    pub fn slice_to_string(&self, start: usize, end: usize) -> String {
        self.rope.slice(start..end).to_string()
    }

    pub fn rope(&self) -> &Rope { &self.rope }

    /// Apply a transaction; pushes its inverse onto the undo stack and clears redo.
    pub fn apply(&mut self, tx: Transaction) {
        let inverse = tx.apply(&mut self.rope);
        self.undo.push(inverse);
        self.redo.clear();
        self.revision += 1;
    }

    /// Undo the top of the undo stack. Returns the selection to restore.
    pub fn undo(&mut self) -> Option<Selection> {
        let inverse = self.undo.pop()?;
        let redo = inverse.apply(&mut self.rope);
        let sel = inverse.selection_after.clone();
        self.redo.push(redo);
        self.revision += 1;
        Some(sel)
    }

    pub fn redo(&mut self) -> Option<Selection> {
        let tx = self.redo.pop()?;
        let inverse = tx.apply(&mut self.rope);
        let sel = tx.selection_after.clone();
        self.undo.push(inverse);
        self.revision += 1;
        Some(sel)
    }

    /// Replace whole-buffer contents (used by external file reload).
    /// Clears history; the in-memory buffer becomes the disk state.
    pub fn reload_from_disk(&mut self) -> Result<()> {
        let path = self.path.as_ref().context("buffer has no path")?;
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        self.rope = Rope::from_str(&text);
        self.undo.clear();
        self.redo.clear();
        self.revision = 0;
        self.saved_revision = 0;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Motion helpers (work on a Rope; agnostic to selection shape).
// ---------------------------------------------------------------------------

/// Char classification used for word-boundary motion.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punct,
}

fn class_of(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

impl Buffer {
    pub fn move_left(&self, idx: usize) -> usize {
        idx.saturating_sub(1)
    }

    pub fn move_right(&self, idx: usize) -> usize {
        (idx + 1).min(self.len_chars())
    }

    pub fn move_up(&self, idx: usize, target_col: Option<usize>) -> usize {
        let line = self.line_of_char(idx);
        if line == 0 {
            return 0;
        }
        let col = target_col.unwrap_or_else(|| self.col_of_char(idx));
        let new_line = line - 1;
        let new_col = col.min(self.line_len_chars(new_line));
        self.line_start(new_line) + new_col
    }

    pub fn move_down(&self, idx: usize, target_col: Option<usize>) -> usize {
        let line = self.line_of_char(idx);
        let max_line = self.line_count().saturating_sub(1);
        if line >= max_line {
            return self.len_chars();
        }
        let col = target_col.unwrap_or_else(|| self.col_of_char(idx));
        let new_line = line + 1;
        let new_col = col.min(self.line_len_chars(new_line));
        self.line_start(new_line) + new_col
    }

    pub fn line_start_of(&self, idx: usize) -> usize {
        self.line_start(self.line_of_char(idx))
    }

    pub fn line_end_of(&self, idx: usize) -> usize {
        let line = self.line_of_char(idx);
        self.line_start(line) + self.line_len_chars(line)
    }

    pub fn doc_start(&self) -> usize { 0 }
    pub fn doc_end(&self) -> usize { self.len_chars() }

    /// Move one word right: skip whitespace, then skip the run of the next class.
    pub fn word_right(&self, idx: usize) -> usize {
        let len = self.len_chars();
        let mut i = idx;
        while i < len && self.rope.char(i).is_whitespace() {
            i += 1;
        }
        if i < len {
            let cls = class_of(self.rope.char(i));
            while i < len {
                let c = self.rope.char(i);
                if class_of(c) != cls || c.is_whitespace() {
                    break;
                }
                i += 1;
            }
        }
        i
    }

    pub fn word_left(&self, idx: usize) -> usize {
        let mut i = idx.min(self.len_chars());
        while i > 0 && self.rope.char(i - 1).is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let cls = class_of(self.rope.char(i - 1));
            while i > 0 {
                let c = self.rope.char(i - 1);
                if class_of(c) != cls || c.is_whitespace() {
                    break;
                }
                i -= 1;
            }
        }
        i
    }
}

// ---------------------------------------------------------------------------
// Convenience transaction builders.
// ---------------------------------------------------------------------------

/// Build a transaction that, for each range in `before`, deletes the range's
/// span and inserts `text` at its start. The resulting selection collapses
/// each range to the end of its inserted text.
pub fn replace_selection_tx(_buf: &Buffer, before: &Selection, text: &str) -> Transaction {
    let mut changes = Vec::with_capacity(before.ranges().len());
    let mut new_ranges: Vec<Range> = Vec::with_capacity(before.ranges().len());
    let insert_chars = text.chars().count();

    // ranges() are not guaranteed sorted in general, but Phase 2 only emits
    // ascending single-range selections. Sort defensively in case multi-cursor
    // arrives later.
    let mut ordered: Vec<(usize, Range)> = before.ranges().iter().copied().enumerate().collect();
    ordered.sort_by_key(|(_, r)| r.start());

    let mut net_shift: isize = 0;
    let mut head_by_orig: Vec<(usize, usize)> = Vec::with_capacity(ordered.len());
    for (orig_idx, r) in &ordered {
        let start = r.start();
        let remove_len = r.len();
        changes.push(Change {
            start,
            remove_len,
            insert: text.to_string(),
        });
        let new_head = (start as isize + net_shift) as usize + insert_chars;
        head_by_orig.push((*orig_idx, new_head));
        net_shift += insert_chars as isize - remove_len as isize;
    }

    head_by_orig.sort_by_key(|(idx, _)| *idx);
    for (_, head) in head_by_orig {
        new_ranges.push(Range::point(head));
    }
    let after = Selection::single(new_ranges[before.primary_index()]);
    // Promote first to primary for now; multi-range will need richer tracking.
    let _ = new_ranges;

    Transaction {
        changes,
        selection_before: before.clone(),
        selection_after: after,
    }
    // Note: when multi-range edits land, build `after` from `new_ranges`, not
    // just the primary. For now Phase 2 only ever has one range.
}

/// Delete `[start, end)` in chars and place the cursor at `start`.
pub fn delete_range_tx(_buf: &Buffer, before: &Selection, start: usize, end: usize) -> Transaction {
    let after = Selection::single(Range::point(start));
    Transaction {
        changes: vec![Change {
            start,
            remove_len: end - start,
            insert: String::new(),
        }],
        selection_before: before.clone(),
        selection_after: after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf_with(text: &str) -> Buffer {
        let mut b = Buffer::empty();
        let tx = replace_selection_tx(&b, &Selection::point(0), text);
        b.apply(tx);
        b
    }

    #[test]
    fn empty_buffer_has_one_line() {
        let b = Buffer::empty();
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line_len_chars(0), 0);
    }

    #[test]
    fn apply_then_undo_redo_round_trip() {
        let mut b = Buffer::empty();
        let tx = replace_selection_tx(&b, &Selection::point(0), "hello");
        b.apply(tx);
        assert_eq!(b.len_chars(), 5);
        assert!(b.dirty());

        let sel = b.undo().expect("can undo");
        assert_eq!(b.len_chars(), 0);
        assert_eq!(sel.primary().head, 0);

        let sel = b.redo().expect("can redo");
        assert_eq!(b.len_chars(), 5);
        assert_eq!(sel.primary().head, 5);
    }

    #[test]
    fn delete_range_undo_restores_text() {
        let mut b = buf_with("hello world");
        let sel = Selection::point(11);
        let tx = delete_range_tx(&b, &sel, 5, 11);
        b.apply(tx);
        assert_eq!(b.line_string(0), "hello");
        b.undo();
        assert_eq!(b.line_string(0), "hello world");
    }

    #[test]
    fn word_motion_skips_punct_and_whitespace() {
        let b = buf_with("foo  bar_baz, qux");
        // From 0, word_right ends at the end of "foo".
        assert_eq!(b.word_right(0), 3);
        // From 3, skip whitespace, then end of "bar_baz".
        assert_eq!(b.word_right(3), 12);
        // ',' is its own punct class.
        assert_eq!(b.word_right(12), 13);
    }

    #[test]
    fn word_left_mirrors_right() {
        let b = buf_with("foo bar");
        assert_eq!(b.word_left(7), 4);
        assert_eq!(b.word_left(4), 0);
    }

    #[test]
    fn dirty_flag_clears_on_save_and_reload() {
        let dir = std::env::temp_dir().join(format!("teditor-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "abc").unwrap();
        let mut b = Buffer::from_path(&path).unwrap();
        assert!(!b.dirty());
        let tx = replace_selection_tx(&b, &Selection::point(3), "d");
        b.apply(tx);
        assert!(b.dirty());
        b.save().unwrap();
        assert!(!b.dirty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
