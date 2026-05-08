//! Cursor = per-tab editing state (selection + sticky col + scroll).
//! Owned by Editor's `CursorStore`. One per open tab.
//!
//! `CursorId` is a process-monotonic `u64` (atomic counter, never
//! reused across the session) per `docs/specs/namespace.md`
//! § *Segment encoding rules → Resource ids*. Wire form on paths is
//! `/cur/<id>`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use devix_protocol::Lookup;
use devix_protocol::path::Path as DevixPath;
use devix_text::{Range, Selection, Transaction};
use crate::editor::document::DocId;

/// Process-monotonic `Cursor` id.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct CursorId(u64);

static CURSOR_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

impl CursorId {
    fn mint() -> Self {
        CursorId(CURSOR_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Decode a `/cur/<u64>` path back into a `CursorId`.
    pub fn id_from_path(path: &DevixPath) -> Option<Self> {
        let mut segs = path.segments();
        if segs.next()? != "cur" {
            return None;
        }
        let id_seg = segs.next()?;
        if segs.next().is_some() {
            return None;
        }
        id_seg.parse::<u64>().ok().map(CursorId)
    }

    /// Encode this id into its canonical path (`/cur/<id>`).
    pub fn to_path(self) -> DevixPath {
        DevixPath::parse(&format!("/cur/{}", self.0)).expect("/cur/<u64> is canonical")
    }
}

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
    /// applies layout-aware clamping via `crate::layout` free functions.
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

/// Per-session cursor registry. Implements
/// `Lookup<Resource = Cursor>` mounted at `/cur/<id>` per
/// `docs/specs/namespace.md` § *Migration table*.
#[derive(Default)]
pub struct CursorStore {
    cursors: HashMap<CursorId, Cursor>,
}

impl CursorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, cursor: Cursor) -> CursorId {
        let id = CursorId::mint();
        self.cursors.insert(id, cursor);
        id
    }

    pub fn remove(&mut self, id: CursorId) -> Option<Cursor> {
        self.cursors.remove(&id)
    }

    pub fn get(&self, id: CursorId) -> Option<&Cursor> {
        self.cursors.get(&id)
    }

    pub fn get_mut(&mut self, id: CursorId) -> Option<&mut Cursor> {
        self.cursors.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (CursorId, &Cursor)> {
        self.cursors.iter().map(|(id, c)| (*id, c))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (CursorId, &mut Cursor)> {
        self.cursors.iter_mut().map(|(id, c)| (*id, c))
    }

    pub fn values(&self) -> impl Iterator<Item = &Cursor> {
        self.cursors.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Cursor> {
        self.cursors.values_mut()
    }

    pub fn keys(&self) -> impl Iterator<Item = CursorId> + '_ {
        self.cursors.keys().copied()
    }

    pub fn contains_key(&self, id: CursorId) -> bool {
        self.cursors.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }
}

impl std::ops::Index<CursorId> for CursorStore {
    type Output = Cursor;
    fn index(&self, id: CursorId) -> &Cursor {
        self.cursors
            .get(&id)
            .unwrap_or_else(|| panic!("CursorStore: unknown CursorId {:?}", id))
    }
}

impl std::ops::IndexMut<CursorId> for CursorStore {
    fn index_mut(&mut self, id: CursorId) -> &mut Cursor {
        self.cursors
            .get_mut(&id)
            .unwrap_or_else(|| panic!("CursorStore: unknown CursorId {:?}", id))
    }
}

impl Lookup for CursorStore {
    type Resource = Cursor;

    fn lookup(&self, path: &DevixPath) -> Option<&Cursor> {
        CursorId::id_from_path(path).and_then(|id| self.get(id))
    }

    fn lookup_mut(&mut self, path: &DevixPath) -> Option<&mut Cursor> {
        CursorId::id_from_path(path).and_then(|id| self.get_mut(id))
    }

    fn paths(&self) -> Box<dyn Iterator<Item = DevixPath> + '_> {
        Box::new(self.cursors.keys().map(|id| id.to_path()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::{DocStore, Document};

    #[test]
    fn fresh_cursor_starts_at_origin_anchored() {
        let mut store = DocStore::new();
        let id = store.insert(Document::empty());
        let c = Cursor::new(id);
        assert_eq!(c.primary().head, 0);
        assert_eq!(c.scroll_mode, ScrollMode::Anchored);
        assert!(c.target_col.is_none());
        assert_eq!(c.scroll_top(), 0);
    }

    #[test]
    fn cursor_id_round_trips_through_path() {
        let id = CursorId::mint();
        let path = id.to_path();
        assert!(path.as_str().starts_with("/cur/"));
        let back = CursorId::id_from_path(&path).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn cursor_id_from_path_rejects_other_roots() {
        let p = DevixPath::parse("/buf/3").unwrap();
        assert!(CursorId::id_from_path(&p).is_none());
        let p = DevixPath::parse("/cur/abc").unwrap();
        assert!(CursorId::id_from_path(&p).is_none());
    }

    #[test]
    fn cursor_store_lookup_round_trips() {
        let mut docs = DocStore::new();
        let did = docs.insert(Document::empty());
        let mut cursors = CursorStore::new();
        let cid = cursors.insert(Cursor::new(did));
        let path = cid.to_path();
        assert!(cursors.lookup(&path).is_some());
        let paths: Vec<DevixPath> = cursors.paths().collect();
        assert_eq!(paths, vec![path]);
    }
}
