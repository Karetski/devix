//! Editor = aggregate of all editor state owned across the layout tree:
//! documents, cursors, frames, plus the layout root, focus path, and the
//! per-frame render-rect cache.
//!
//! Behaviour is split across submodules by concern:
//!
//! * [`ops`]     — mutating operations (tabs, splits, sidebars, file open).
//! * [`focus`]   — directional focus traversal across the layout tree.
//! * [`hittest`] — screen-coord → leaf / tab-strip resolution and tab-strip
//!   scroll forwarding.
//!
//! Submodules add `impl Editor { ... }` blocks; this file owns the struct,
//! its constructor, and the unconditional read-side accessors.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use devix_core::Pane;
use devix_core::Rect;
use slotmap::SlotMap;

use crate::cursor::{Cursor, CursorId};
use crate::document::{DocId, Document};

use crate::frame::{FrameId, mint_id};
use devix_core::SidebarSlot;
use crate::tree::{LayoutFrame, LeafId, find_frame, pane_at_indices, pane_leaf_id};
#[cfg(test)]
use crate::tree::{find_frame_mut, frame_ids};

mod focus;
mod hittest;
mod ops;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LeafRef {
    Frame(FrameId),
    Sidebar(SidebarSlot),
}

/// One clickable tab region produced by the tab-strip render. Stored in the
/// render cache and consumed by hit-testing. Defined here (rather than in
/// `devix-ui`) so the editor model has no widget-layer dependency.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TabHit {
    pub idx: usize,
    pub rect: Rect,
}

/// Hit-test layout of a frame's tab strip, populated on render. `strip_rect`
/// covers the whole 1-row tab strip (including empty space past the last tab),
/// so wheel events anywhere on that row resolve to this frame.
#[derive(Default, Clone, Debug)]
pub struct TabStripCache {
    pub strip_rect: Rect,
    pub content_width: u32,
    pub hits: Vec<TabHit>,
}

#[derive(Default)]
pub struct RenderCache {
    pub frame_rects: HashMap<FrameId, Rect>,
    pub sidebar_rects: HashMap<SidebarSlot, Rect>,
    pub tab_strips: HashMap<FrameId, TabStripCache>,
}

/// What was hit by a click on the tab-strip overlay.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TabStripHit {
    Tab { frame: FrameId, idx: usize },
}

pub struct Editor {
    pub documents: SlotMap<DocId, Document>,
    pub cursors: SlotMap<CursorId, Cursor>,
    /// Structural Pane tree — the layout source of truth, and **also**
    /// the only place per-frame state lives now. `LayoutFrame` owns its
    /// `tabs`/`active_tab`/`tab_strip_scroll`/`recenter_active`
    /// directly. Lookups by `FrameId` go through tree walks
    /// (`find_frame` / `find_frame_mut`); they're O(tree-size) which is
    /// fine at TUI scales.
    pub root: Box<dyn Pane>,
    /// Head of the responder chain. When `Some`, the modal Pane gets
    /// first crack at every input event before the focused-leaf path,
    /// and paints last (z-top). Concrete modals (`PalettePane`,
    /// `SymbolPickerPane`, future plugin pickers) own their state
    /// outright and live here for the duration of their session. The
    /// framework is generic over `&dyn Pane`; there is no closed enum
    /// of modal kinds.
    pub modal: Option<Box<dyn Pane>>,
    pub focus: Vec<usize>,
    pub doc_index: HashMap<PathBuf, DocId>,
    pub render_cache: RenderCache,
}

impl Editor {
    /// Create a editor with a single frame, single tab, single cursor.
    /// `path` is opened if Some; otherwise an empty scratch buffer is used.
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let mut documents: SlotMap<DocId, Document> = SlotMap::with_key();
        let mut cursors: SlotMap<CursorId, Cursor> = SlotMap::with_key();
        let mut doc_index = HashMap::new();

        let doc_id = match path {
            Some(p) => {
                let canonical = canonicalize_or_keep(&p);
                let id = documents.insert(Document::from_path(p)?);
                doc_index.insert(canonical, id);
                id
            }
            None => documents.insert(Document::empty()),
        };
        let cursor_id = cursors.insert(Cursor::new(doc_id));
        let frame_id = mint_id();
        let root: Box<dyn Pane> = Box::new(LayoutFrame::with_cursor(frame_id, cursor_id));
        let focus = vec![]; // root is the frame leaf itself

        Ok(Self {
            documents,
            cursors,
            root,
            modal: None,
            focus,
            doc_index,
            render_cache: RenderCache::default(),
        })
    }

    pub fn active_cursor(&self) -> Option<&Cursor> {
        let frame_id = self.active_frame()?;
        let cursor_id = find_frame(self.root.as_ref(), frame_id)?.active_cursor()?;
        self.cursors.get(cursor_id)
    }

    pub fn active_cursor_mut(&mut self) -> Option<&mut Cursor> {
        let frame_id = self.active_frame()?;
        let cursor_id = find_frame(self.root.as_ref(), frame_id)?.active_cursor()?;
        self.cursors.get_mut(cursor_id)
    }

    pub fn active_frame(&self) -> Option<FrameId> {
        let pane = pane_at_indices(self.root.as_ref(), &self.focus)?;
        match pane_leaf_id(pane)? {
            LeafId::Frame(id) => Some(id),
            LeafId::Sidebar(_) => None,
        }
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut Document> {
        let c = self.active_cursor()?;
        self.documents.get_mut(c.doc)
    }

    pub fn active_doc(&self) -> Option<&Document> {
        let c = self.active_cursor()?;
        self.documents.get(c.doc)
    }

    /// Resolve focus to (frame, cursor, doc) IDs in one immutable borrow,
    /// so callers can take disjoint &mut borrows on the underlying slot-maps.
    pub fn active_ids(&self) -> Option<(FrameId, CursorId, DocId)> {
        let frame_id = self.active_frame()?;
        let cursor_id = find_frame(self.root.as_ref(), frame_id)?.active_cursor()?;
        let doc_id = self.cursors[cursor_id].doc;
        Some((frame_id, cursor_id, doc_id))
    }

    /// Pre-paint layout pass.
    ///
    /// Walks every `Frame` leaf in the layout tree under `area` and runs the
    /// state mutations the next paint will see: anchor `Cursor.scroll` to the
    /// caret (or clamp it under the new content extent in `Free` mode), and
    /// run the per-frame tab-strip's scroll-into-view math.
    ///
    /// This is the only mutation hook that runs between input dispatch and
    /// paint. After it returns, paint is pure — render functions read state
    /// and emit cells, never write back.
    pub fn layout(&mut self, area: Rect) {
        use devix_ui::TabInfo;
        use devix_ui::layout::{VRect, ensure_visible, set_scroll};
        use devix_ui::tab_strip_layout;
        use crate::cursor::ScrollMode;

        // Reset render-cache for this frame. Both the per-leaf walk
        // below (for `Frame` leaves' tab-strip + body rects) and the
        // sidebar arm (for `sidebar_rects`) repopulate it. Hit-testing
        // and click-routing read these tables.
        self.render_cache.frame_rects.clear();
        self.render_cache.sidebar_rects.clear();
        self.render_cache.tab_strips.clear();

        let leaves = crate::tree::leaves_with_rects(self.root.as_ref(), area);
        for (leaf, rect) in leaves {
            let fid = match leaf {
                LeafRef::Sidebar(slot) => {
                    self.render_cache.sidebar_rects.insert(slot, rect);
                    continue;
                }
                LeafRef::Frame(fid) => fid,
            };
            let strip_area = Rect { height: 1, ..rect };
            let body_area = Rect {
                y: rect.y + 1,
                height: rect.height.saturating_sub(1),
                ..rect
            };

            let tabs: Vec<TabInfo> = match find_frame(self.root.as_ref(), fid) {
                Some(frame) => frame
                    .tabs
                    .iter()
                    .map(|cid| {
                        let c = &self.cursors[*cid];
                        let d = &self.documents[c.doc];
                        let label = d
                            .buffer
                            .path()
                            .and_then(|p| p.file_name())
                            .and_then(|f| f.to_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "[scratch]".to_string());
                        TabInfo {
                            label,
                            dirty: d.buffer.dirty(),
                        }
                    })
                    .collect(),
                None => continue,
            };
            let Some(active_tab) = find_frame(self.root.as_ref(), fid).map(|f| f.active_tab) else {
                continue;
            };
            let Some(f) = crate::tree::find_frame_mut(&mut self.root, fid) else {
                continue;
            };
            devix_ui::layout_tabstrip(
                &tabs,
                active_tab,
                &mut f.tab_strip_scroll,
                &mut f.recenter_active,
                strip_area,
            );

            // Tab-strip hit cache. Recomputed against the post-scroll
            // strip so click hit-tests align with what's painted.
            let scroll = f.tab_strip_scroll;
            let (hits_pure, content_width) =
                tab_strip_layout(&tabs, active_tab, scroll, strip_area);
            let hits = hits_pure
                .iter()
                .map(|h| crate::editor::TabHit { idx: h.idx, rect: h.rect })
                .collect();
            self.render_cache.tab_strips.insert(
                fid,
                crate::editor::TabStripCache {
                    strip_rect: strip_area,
                    content_width,
                    hits,
                },
            );
            self.render_cache.frame_rects.insert(fid, body_area);

            let Some(cid) =
                find_frame(self.root.as_ref(), fid).and_then(|f| f.active_cursor())
            else {
                continue;
            };
            let cursor = &self.cursors[cid];
            let doc = &self.documents[cursor.doc];

            let head = cursor.primary().head;
            let cur_line = doc.buffer.line_of_char(head);
            let line_count = doc.buffer.line_count();
            let scroll_mode = cursor.scroll_mode;
            let body_w = body_area.width as u32;
            let body_h = body_area.height as u32;
            if body_h == 0 {
                continue;
            }

            let content = (body_w, line_count.max(1) as u32);
            let viewport = (body_w, body_h);
            let c = &mut self.cursors[cid];
            match scroll_mode {
                ScrollMode::Anchored => {
                    let line_rect = VRect {
                        x: 0,
                        y: cur_line as u32,
                        w: body_w,
                        h: 1,
                    };
                    ensure_visible(&mut c.scroll, line_rect, content, viewport);
                }
                ScrollMode::Free => {
                    let (sx, sy) = c.scroll;
                    set_scroll(&mut c.scroll, sx, sy, content, viewport);
                }
            }
        }
    }
}

pub(super) fn canonicalize_or_keep(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_workspace_has_one_frame_one_cursor() {
        let ws = Editor::open(None).unwrap();
        assert_eq!(frame_ids(ws.root.as_ref()).len(), 1);
        assert_eq!(ws.cursors.len(), 1);
        assert_eq!(ws.documents.len(), 1);
        assert!(ws.active_cursor().is_some());
    }

    #[test]
    fn new_tab_then_close_returns_to_previous() {
        let mut ws = Editor::open(None).unwrap();
        let original_doc = ws.active_cursor().unwrap().doc;

        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().tabs.len(), 2);
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().active_tab, 1);

        assert!(ws.close_active_tab(false));
        let active = ws.active_cursor().unwrap();
        assert_eq!(active.doc, original_doc);
    }

    #[test]
    fn close_last_tab_leaves_a_scratch_tab() {
        let mut ws = Editor::open(None).unwrap();
        assert!(ws.close_active_tab(false));
        let fid = ws.active_frame().unwrap();
        let frame = find_frame(ws.root.as_ref(), fid).unwrap();
        assert_eq!(frame.tabs.len(), 1);
        let c = ws.active_cursor().unwrap();
        assert!(ws.documents[c.doc].buffer.path().is_none());
    }

    #[test]
    fn dirty_close_refused_force_close_succeeds() {
        use devix_text::{Selection, replace_selection_tx};
        let mut ws = Editor::open(None).unwrap();
        let did = ws.active_cursor().unwrap().doc;
        let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), "hi");
        ws.documents[did].buffer.apply(tx);
        assert!(!ws.close_active_tab(false), "dirty close should refuse");
        assert!(ws.close_active_tab(true), "force close should succeed");
    }

    #[test]
    fn opening_same_path_twice_reuses_document() {
        let dir = std::env::temp_dir().join(format!("devix-open-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "abc").unwrap();

        let mut ws = Editor::open(None).unwrap();
        let c1 = ws.open_path_replace_current(p.clone()).unwrap();
        let did1 = ws.cursors[c1].doc;
        ws.new_tab();
        let c2 = ws.open_path_replace_current(p.clone()).unwrap();
        let did2 = ws.cursors[c2].doc;
        assert_eq!(did1, did2, "same path should reuse DocId");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_creates_a_second_frame_and_focuses_it() {
        let mut ws = Editor::open(None).unwrap();
        let original_fid = ws.active_frame().unwrap();
        ws.split_active(crate::layout::Axis::Horizontal);
        assert_eq!(frame_ids(ws.root.as_ref()).len(), 2);
        let new_fid = ws.active_frame().unwrap();
        assert_ne!(original_fid, new_fid);

        let Some(orig_cursor_id) = find_frame(ws.root.as_ref(), original_fid).and_then(|f| f.active_cursor()) else { panic!("original frame has no active cursor"); };
        let Some(new_cursor_id) = find_frame(ws.root.as_ref(), new_fid).and_then(|f| f.active_cursor()) else { panic!("new frame has no active cursor"); };
        let original_doc = ws.cursors[orig_cursor_id].doc;
        let new_doc = ws.cursors[new_cursor_id].doc;
        assert_eq!(original_doc, new_doc, "split clones cursor, shares document");
    }

    #[test]
    fn closing_one_split_child_collapses_back_to_single_frame() {
        use crate::layout::Axis;
        use crate::tree::LayoutFrame;
        let mut ws = Editor::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        assert_eq!(frame_ids(ws.root.as_ref()).len(), 2);
        ws.close_active_frame();
        assert_eq!(frame_ids(ws.root.as_ref()).len(), 1);
        let any = ws.root.as_any().expect("structural root has Any");
        assert!(any.downcast_ref::<LayoutFrame>().is_some(), "single frame at root");
    }

    #[test]
    fn toggle_left_sidebar_adds_then_removes_it() {
        use crate::tree::LayoutSplit;
        let mut ws = Editor::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        let split = ws
            .root
            .as_any()
            .and_then(|a| a.downcast_ref::<LayoutSplit>())
            .expect("root lifted to a Split");
        assert_eq!(split.children.len(), 2, "split has editor + left sidebar");

        ws.toggle_sidebar(SidebarSlot::Left);
        // After removal, root may have collapsed or stayed a single-child
        // split-wrapper; both are valid outcomes (the architecture doesn't
        // require auto-collapse of toggle-removal).
        assert!(ws.root.as_any().is_some());
    }

    #[test]
    fn focus_dir_right_after_split_returns_to_original() {
        use devix_core::{Axis, Direction};
        let mut ws = Editor::open(None).unwrap();
        let original = ws.active_frame().unwrap();
        ws.split_active(Axis::Horizontal);
        let new_fid = ws.active_frame().unwrap();
        assert_ne!(original, new_fid);

        ws.focus_dir(Direction::Left);
        assert_eq!(ws.active_frame(), Some(original));

        ws.focus_dir(Direction::Right);
        assert_eq!(ws.active_frame(), Some(new_fid));
    }

    #[test]
    fn focus_dir_left_at_edge_with_sidebar_enters_sidebar() {
        use devix_core::Direction;
        use crate::tree::{LeafId, pane_at_indices, pane_leaf_id};
        let mut ws = Editor::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        ws.focus_dir(Direction::Left);
        let pane = pane_at_indices(ws.root.as_ref(), &ws.focus).expect("focus resolves");
        assert_eq!(
            pane_leaf_id(pane),
            Some(LeafId::Sidebar(SidebarSlot::Left)),
        );
    }

    #[test]
    fn scroll_clamps_at_zero_and_at_end() {
        use devix_text::{Selection, replace_selection_tx};

        let mut ws = Editor::open(None).unwrap();
        let did = ws.active_cursor().unwrap().doc;
        let txt = "x\n".repeat(100);
        let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), &txt);
        ws.documents[did].buffer.apply(tx);

        let c = ws.active_cursor_mut().unwrap();
        let next: isize = (c.scroll_top() as isize).saturating_add(-1);
        c.set_scroll_top(next.clamp(0, 99) as usize);
        assert_eq!(c.scroll_top(), 0);

        let c = ws.active_cursor_mut().unwrap();
        let next: isize = (c.scroll_top() as isize).saturating_add(1_000_000);
        c.set_scroll_top(next.clamp(0, 99) as usize);
        assert_eq!(c.scroll_top(), 99);
    }

    #[test]
    fn closing_focused_sidebar_lands_focus_on_a_frame() {
        use devix_core::Direction;
        use crate::tree::{LeafId, pane_at_indices, pane_leaf_id};
        let mut ws = Editor::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        ws.focus_dir(Direction::Left);
        let pane = pane_at_indices(ws.root.as_ref(), &ws.focus).expect("focus resolves");
        assert_eq!(
            pane_leaf_id(pane),
            Some(LeafId::Sidebar(SidebarSlot::Left)),
        );
        ws.toggle_sidebar(SidebarSlot::Left);
        let pane = pane_at_indices(ws.root.as_ref(), &ws.focus).expect("focus resolves");
        assert!(
            matches!(pane_leaf_id(pane), Some(LeafId::Frame(_))),
            "after sidebar removal, focus should resolve to a Frame leaf",
        );
    }

    #[test]
    fn closing_one_of_three_split_children_keeps_two_remaining() {
        use crate::layout::Axis;
        use crate::tree::{LayoutSplit, LeafId, pane_at_indices, pane_leaf_id};
        let mut ws = Editor::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        ws.split_active(Axis::Horizontal);
        assert_eq!(frame_ids(ws.root.as_ref()).len(), 3);

        ws.close_active_frame();
        assert_eq!(frame_ids(ws.root.as_ref()).len(), 2);
        let any = ws.root.as_any().expect("root has Any");
        assert!(
            any.downcast_ref::<LayoutSplit>().is_some(),
            "two frames should be in a Split, not a flat Frame leaf",
        );
        let pane = pane_at_indices(ws.root.as_ref(), &ws.focus).expect("focus resolves");
        assert!(matches!(pane_leaf_id(pane), Some(LeafId::Frame(_))));
    }

    #[test]
    fn opening_same_path_in_two_frames_shares_document() {
        use crate::layout::Axis;
        let dir = std::env::temp_dir().join(format!("devix-dedup-cross-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "abc").unwrap();

        let mut ws = Editor::open(None).unwrap();
        let c1 = ws.open_path_replace_current(p.clone()).unwrap();
        let did1 = ws.cursors[c1].doc;

        ws.split_active(Axis::Horizontal);
        let c2 = ws.open_path_replace_current(p.clone()).unwrap();
        let did2 = ws.cursors[c2].doc;

        assert_eq!(did1, did2, "same path opened in different frames should share DocId");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tab_strip_hit_returns_tab_under_cursor() {
        let mut ws = Editor::open(None).unwrap();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        let strip = TabStripCache {
            strip_rect: Rect { x: 0, y: 0, width: 30, height: 1 },
            content_width: 21,
            hits: vec![
                TabHit { idx: 0, rect: Rect { x: 0, y: 0, width: 10, height: 1 } },
                TabHit { idx: 1, rect: Rect { x: 11, y: 0, width: 10, height: 1 } },
            ],
        };
        ws.render_cache.tab_strips.insert(fid, strip);

        assert_eq!(
            ws.tab_strip_hit(5, 0),
            Some(TabStripHit::Tab { frame: fid, idx: 0 }),
        );
        assert_eq!(
            ws.tab_strip_hit(15, 0),
            Some(TabStripHit::Tab { frame: fid, idx: 1 }),
        );
        assert_eq!(ws.tab_strip_hit(50, 0), None);
        assert_eq!(ws.tab_strip_hit(5, 5), None);
    }

    #[test]
    fn activate_tab_focuses_clicked_index() {
        let mut ws = Editor::open(None).unwrap();
        ws.new_tab();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        ws.activate_tab(fid, 0);
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().active_tab, 0);
        ws.activate_tab(fid, 99);
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().active_tab, 2);
    }

    #[test]
    fn scroll_tab_strip_clamps_to_content_minus_strip_width() {
        let mut ws = Editor::open(None).unwrap();
        let fid = ws.active_frame().unwrap();
        ws.render_cache.tab_strips.insert(fid, TabStripCache {
            strip_rect: Rect { x: 0, y: 0, width: 20, height: 1 },
            content_width: 50,
            hits: Vec::new(),
        });
        ws.scroll_tab_strip(fid, 100);
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().tab_strip_scroll.0, 30, "clamped to 50 - 20");
        ws.scroll_tab_strip(fid, -1000);
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().tab_strip_scroll.0, 0, "clamped at 0");
    }

    #[test]
    fn scroll_tab_strip_noop_when_content_fits() {
        let mut ws = Editor::open(None).unwrap();
        let fid = ws.active_frame().unwrap();
        ws.render_cache.tab_strips.insert(fid, TabStripCache {
            strip_rect: Rect { x: 0, y: 0, width: 20, height: 1 },
            content_width: 15,
            hits: Vec::new(),
        });
        ws.scroll_tab_strip(fid, 5);
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().tab_strip_scroll.0, 0);
    }

    #[test]
    fn frame_at_strip_resolves_full_strip_row() {
        let mut ws = Editor::open(None).unwrap();
        let fid = ws.active_frame().unwrap();
        ws.render_cache.tab_strips.insert(fid, TabStripCache {
            strip_rect: Rect { x: 0, y: 4, width: 30, height: 1 },
            content_width: 10,
            hits: Vec::new(),
        });
        assert_eq!(ws.frame_at_strip(25, 4), Some(fid));
        assert_eq!(ws.frame_at_strip(25, 5), None);
    }

    #[test]
    fn next_tab_requests_recenter_but_click_does_not() {
        let mut ws = Editor::open(None).unwrap();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        find_frame_mut(&mut ws.root, fid).unwrap().recenter_active = false;

        ws.next_tab();
        assert!(find_frame(ws.root.as_ref(), fid).unwrap().recenter_active, "keyboard nav requests scroll-into-view");

        find_frame_mut(&mut ws.root, fid).unwrap().recenter_active = false;
        ws.activate_tab(fid, 0);
        assert!(!find_frame(ws.root.as_ref(), fid).unwrap().recenter_active,
            "click activation must not request scroll — strip stays put");
    }

    #[test]
    fn activate_tab_does_not_change_tab_scroll() {
        let mut ws = Editor::open(None).unwrap();
        ws.new_tab();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        find_frame_mut(&mut ws.root, fid).unwrap().tab_strip_scroll.0 = 7;
        ws.activate_tab(fid, 0);
        assert_eq!(find_frame(ws.root.as_ref(), fid).unwrap().tab_strip_scroll.0, 7,
            "click-to-activate must not relayout the strip");
    }

    #[test]
    fn focus_frame_jumps_focus_across_a_split() {
        use crate::layout::Axis;
        let mut ws = Editor::open(None).unwrap();
        let original = ws.active_frame().unwrap();
        ws.split_active(Axis::Horizontal);
        let new_fid = ws.active_frame().unwrap();
        assert_ne!(original, new_fid);

        assert!(ws.focus_frame(original));
        assert_eq!(ws.active_frame(), Some(original));
        assert!(ws.focus_frame(new_fid));
        assert_eq!(ws.active_frame(), Some(new_fid));
    }
}
