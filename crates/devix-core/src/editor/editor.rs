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
use crate::{Pane, Rect};

use crate::editor::cursor::{Cursor, CursorId};
use crate::editor::document::{DocId, Document};

use crate::editor::focus_chain::FocusChain;
use crate::editor::frame::{FrameId, mint_id};
use crate::editor::modal_slot::ModalSlot;
use crate::SidebarSlot;
use crate::editor::registry::PaneRegistry;
use crate::editor::tree::{LayoutFrame, LayoutNode};

mod focus;
mod hittest;
mod ops;

pub use focus::path_to_leaf;

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
    pub documents: crate::editor::document::DocStore,
    pub cursors: crate::editor::cursor::CursorStore,
    /// In-process pulse bus per `docs/specs/pulse-bus.md`. Producers
    /// publish typed `Pulse` events (sync via `publish`, cross-thread
    /// via `publish_async`); subscribers register typed handlers.
    /// Stage 6 / T-60 introduces the bus; T-61 / T-62 / T-63 migrate
    /// remaining closure-as-message producers off `EventSink`.
    pub bus: crate::PulseBus,
    /// Pane registry — owner of the structural layout tree. Carved out
    /// of the god-`Editor` per T-100. Lookups (`find_frame`,
    /// `at_path`, `pane_at`) and tree mutations (`replace_at`,
    /// `remove_at`, `collapse_singletons`, `lift_into_horizontal_split`)
    /// flow through this owner; per-frame state still lives on the
    /// underlying `LayoutFrame`.
    pub panes: PaneRegistry,
    /// Modal slot — owner of the at-most-one active modal. Carved out
    /// per T-103. When occupied, the modal Pane gets first crack at
    /// every input event before the focused-leaf path, and paints last
    /// (z-top). `Editor::open_modal` / `dismiss_modal` go through this
    /// owner and emit `Pulse::ModalOpened` / `ModalDismissed` on
    /// transitions.
    pub modal: ModalSlot,
    /// Focus chain — owner of the active pane path. Carved out per
    /// T-101. Mutations route through `FocusChain::replace` /
    /// `transform`; real transitions emit `Pulse::FocusChanged`
    /// (via `Editor::set_focus`) exactly once.
    pub focus: FocusChain,
    pub doc_index: HashMap<PathBuf, DocId>,
    pub render_cache: RenderCache,
}

impl Editor {
    /// Create a editor with a single frame, single tab, single cursor.
    /// `path` is opened if Some; otherwise an empty scratch buffer is used.
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let mut documents = crate::editor::document::DocStore::new();
        let mut cursors = crate::editor::cursor::CursorStore::new();
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
        let panes = PaneRegistry::new(LayoutNode::Frame(LayoutFrame::with_cursor(
            frame_id, cursor_id,
        )));
        let focus = FocusChain::new(); // root is the frame leaf itself

        let bus = crate::PulseBus::new();
        // Install bus-flavored disk watchers on every initially-open
        // document so disk-change events flow as Pulse::DiskChanged.
        for id in documents.keys().collect::<Vec<_>>() {
            install_bus_watcher_for_doc(&mut documents, id, &bus);
        }
        Ok(Self {
            documents,
            cursors,
            panes,
            modal: ModalSlot::new(),
            focus,
            doc_index,
            render_cache: RenderCache::default(),
            bus,
        })
    }


    pub fn active_cursor(&self) -> Option<&Cursor> {
        let frame_id = self.active_frame()?;
        let cursor_id = self.panes.find_frame(frame_id)?.active_cursor()?;
        self.cursors.get(cursor_id)
    }

    pub fn active_cursor_mut(&mut self) -> Option<&mut Cursor> {
        let frame_id = self.active_frame()?;
        let cursor_id = self.panes.find_frame(frame_id)?.active_cursor()?;
        self.cursors.get_mut(cursor_id)
    }

    pub fn active_frame(&self) -> Option<FrameId> {
        match self.panes.at_path(self.focus.active())?.leaf_id()? {
            LeafRef::Frame(id) => Some(id),
            LeafRef::Sidebar(_) => None,
        }
    }

    /// Set the active focus path. Publishes `Pulse::FocusChanged` iff
    /// the path actually changes (T-101).
    pub fn set_focus(&mut self, new: Vec<usize>) {
        if let Some(t) = self.focus.replace(new) {
            self.bus.publish(t.into_pulse());
        }
    }

    /// Install `pane` of `kind` as the active modal. If a modal was
    /// already open, it's dismissed first (its `ModalDismissed`
    /// publishes before the new modal's `ModalOpened`). T-103.
    pub fn open_modal(&mut self, pane: Box<dyn Pane>, kind: devix_protocol::pulse::ModalKind) {
        let frame_path = self
            .active_frame()
            .map(|_| modal_frame_path(self.focus.active()));
        let prev = self.modal.open(pane, kind);
        if let Some(prev_kind) = prev {
            self.bus
                .publish(devix_protocol::pulse::Pulse::ModalDismissed { modal: prev_kind });
        }
        self.bus.publish(devix_protocol::pulse::Pulse::ModalOpened {
            modal: kind,
            frame: frame_path,
        });
    }

    /// Dismiss the active modal, if any. Emits `ModalDismissed` on
    /// transition. No-op if the slot is already empty. T-103.
    pub fn dismiss_modal(&mut self) {
        if let Some(kind) = self.modal.dismiss() {
            self.bus
                .publish(devix_protocol::pulse::Pulse::ModalDismissed { modal: kind });
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
        let cursor_id = self.panes.find_frame(frame_id)?.active_cursor()?;
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
        use crate::TabInfo;
        use crate::widgets::layout::{VRect, ensure_visible, set_scroll};
        use crate::tab_strip_layout;
        use crate::editor::cursor::ScrollMode;

        // Reset render-cache for this frame. Both the per-leaf walk
        // below (for `Frame` leaves' tab-strip + body rects) and the
        // sidebar arm (for `sidebar_rects`) repopulate it. Hit-testing
        // and click-routing read these tables.
        self.render_cache.frame_rects.clear();
        self.render_cache.sidebar_rects.clear();
        self.render_cache.tab_strips.clear();

        let leaves = self.panes.leaves_with_rects(area);
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

            let tabs: Vec<TabInfo> = match self.panes.find_frame(fid) {
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
            let Some(active_tab) = self.panes.find_frame(fid).map(|f| f.active_tab) else {
                continue;
            };
            let Some(f) = self.panes.find_frame_mut(fid) else {
                continue;
            };
            crate::layout_tabstrip(
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

            let Some(cid) = self.panes.find_frame(fid).and_then(|f| f.active_cursor())
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

/// Build a `/pane(/<i>)*` path from the focus index list. Used by
/// `Editor::open_modal` to populate `Pulse::ModalOpened.frame`.
fn modal_frame_path(indices: &[usize]) -> devix_protocol::path::Path {
    let mut s = String::from("/pane");
    for i in indices {
        s.push('/');
        s.push_str(&i.to_string());
    }
    devix_protocol::path::Path::parse(&s).expect("/pane(/<i>)* is canonical")
}

/// Install a notify watcher on `documents[id]` whose callback
/// publishes `Pulse::DiskChanged { path, fs_path }` into `bus` via
/// `publish_async`. Replaces the legacy closure-based DiskSink path
/// retired in T-61.
pub(crate) fn install_bus_watcher_for_doc(
    documents: &mut crate::editor::document::DocStore,
    id: DocId,
    bus: &crate::PulseBus,
) {
    let Some(doc) = documents.get_mut(id) else { return };
    let Some(fs_path) = doc.buffer.path().map(std::path::Path::to_path_buf) else {
        return;
    };
    let path = id.to_path();
    let bus = bus.clone();
    doc.install_disk_watcher(Box::new(move || {
        bus.publish_async(devix_protocol::pulse::Pulse::DiskChanged {
            path: path.clone(),
            fs_path: fs_path.clone(),
        });
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_workspace_has_one_frame_one_cursor() {
        let ws = Editor::open(None).unwrap();
        assert_eq!(ws.panes.frames().len(), 1);
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
        assert_eq!(ws.panes.find_frame(fid).unwrap().tabs.len(), 2);
        assert_eq!(ws.panes.find_frame(fid).unwrap().active_tab, 1);

        assert!(ws.close_active_tab(false));
        let active = ws.active_cursor().unwrap();
        assert_eq!(active.doc, original_doc);
    }

    #[test]
    fn close_last_tab_leaves_a_scratch_tab() {
        let mut ws = Editor::open(None).unwrap();
        assert!(ws.close_active_tab(false));
        let fid = ws.active_frame().unwrap();
        let frame = ws.panes.find_frame(fid).unwrap();
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
        ws.split_active(crate::Axis::Horizontal);
        assert_eq!(ws.panes.frames().len(), 2);
        let new_fid = ws.active_frame().unwrap();
        assert_ne!(original_fid, new_fid);

        let Some(orig_cursor_id) = ws.panes.find_frame(original_fid).and_then(|f| f.active_cursor()) else { panic!("original frame has no active cursor"); };
        let Some(new_cursor_id) = ws.panes.find_frame(new_fid).and_then(|f| f.active_cursor()) else { panic!("new frame has no active cursor"); };
        let original_doc = ws.cursors[orig_cursor_id].doc;
        let new_doc = ws.cursors[new_cursor_id].doc;
        assert_eq!(original_doc, new_doc, "split clones cursor, shares document");
    }

    #[test]
    fn closing_one_split_child_collapses_back_to_single_frame() {
        use crate::Axis;
        let mut ws = Editor::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        assert_eq!(ws.panes.frames().len(), 2);
        ws.close_active_frame();
        assert_eq!(ws.panes.frames().len(), 1);
        assert!(matches!(ws.panes.root(), LayoutNode::Frame(_)), "single frame at root");
    }

    #[test]
    fn toggle_left_sidebar_adds_then_removes_it() {
        let mut ws = Editor::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        let split = match ws.panes.root() {
            LayoutNode::Split(s) => s,
            _ => panic!("root lifted to a Split"),
        };
        assert_eq!(split.children.len(), 2, "split has editor + left sidebar");

        ws.toggle_sidebar(SidebarSlot::Left);
        // After removal the root may have collapsed or stayed a single-child
        // split-wrapper; both are valid outcomes (the architecture doesn't
        // require auto-collapse of toggle-removal).
        assert!(!ws.panes.frames().is_empty());
    }

    #[test]
    fn focus_dir_right_after_split_returns_to_original() {
        use crate::{Axis, Direction};
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
        use crate::Direction;
        let mut ws = Editor::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        ws.focus_dir(Direction::Left);
        let node = ws.panes.at_path(&ws.focus).expect("focus resolves");
        assert_eq!(node.leaf_id(), Some(LeafRef::Sidebar(SidebarSlot::Left)));
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
        use crate::Direction;
        let mut ws = Editor::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        ws.focus_dir(Direction::Left);
        let node = ws.panes.at_path(&ws.focus).expect("focus resolves");
        assert_eq!(node.leaf_id(), Some(LeafRef::Sidebar(SidebarSlot::Left)));
        ws.toggle_sidebar(SidebarSlot::Left);
        let node = ws.panes.at_path(&ws.focus).expect("focus resolves");
        assert!(
            matches!(node.leaf_id(), Some(LeafRef::Frame(_))),
            "after sidebar removal, focus should resolve to a Frame leaf",
        );
    }

    #[test]
    fn closing_one_of_three_split_children_keeps_two_remaining() {
        use crate::Axis;
        let mut ws = Editor::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        ws.split_active(Axis::Horizontal);
        assert_eq!(ws.panes.frames().len(), 3);

        ws.close_active_frame();
        assert_eq!(ws.panes.frames().len(), 2);
        assert!(
            matches!(ws.panes.root(), LayoutNode::Split(_)),
            "two frames should be in a Split, not a flat Frame leaf",
        );
        let node = ws.panes.at_path(&ws.focus).expect("focus resolves");
        assert!(matches!(node.leaf_id(), Some(LeafRef::Frame(_))));
    }

    #[test]
    fn opening_same_path_in_two_frames_shares_document() {
        use crate::Axis;
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
        assert_eq!(ws.panes.find_frame(fid).unwrap().active_tab, 0);
        ws.activate_tab(fid, 99);
        assert_eq!(ws.panes.find_frame(fid).unwrap().active_tab, 2);
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
        assert_eq!(ws.panes.find_frame(fid).unwrap().tab_strip_scroll.0, 30, "clamped to 50 - 20");
        ws.scroll_tab_strip(fid, -1000);
        assert_eq!(ws.panes.find_frame(fid).unwrap().tab_strip_scroll.0, 0, "clamped at 0");
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
        assert_eq!(ws.panes.find_frame(fid).unwrap().tab_strip_scroll.0, 0);
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
        ws.panes.find_frame_mut(fid).unwrap().recenter_active = false;

        ws.next_tab();
        assert!(ws.panes.find_frame(fid).unwrap().recenter_active, "keyboard nav requests scroll-into-view");

        ws.panes.find_frame_mut(fid).unwrap().recenter_active = false;
        ws.activate_tab(fid, 0);
        assert!(!ws.panes.find_frame(fid).unwrap().recenter_active,
            "click activation must not request scroll — strip stays put");
    }

    #[test]
    fn activate_tab_does_not_change_tab_scroll() {
        let mut ws = Editor::open(None).unwrap();
        ws.new_tab();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        ws.panes.find_frame_mut(fid).unwrap().tab_strip_scroll.0 = 7;
        ws.activate_tab(fid, 0);
        assert_eq!(ws.panes.find_frame(fid).unwrap().tab_strip_scroll.0, 7,
            "click-to-activate must not relayout the strip");
    }

    #[test]
    fn focus_frame_jumps_focus_across_a_split() {
        use crate::Axis;
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

    #[test]
    fn pane_at_root_returns_root_node() {
        let editor = Editor::open(None).unwrap();
        let p = devix_protocol::path::Path::parse("/pane").unwrap();
        let node = editor.panes.pane_at(&p).unwrap();
        assert!(node.is_focusable());
    }

    #[test]
    fn pane_at_resolves_indices_after_split() {
        use crate::Axis;
        let mut editor = Editor::open(None).unwrap();
        editor.split_active(Axis::Horizontal);
        // After a split, root is a Split with two Frame children.
        let p0 = devix_protocol::path::Path::parse("/pane/0").unwrap();
        let p1 = devix_protocol::path::Path::parse("/pane/1").unwrap();
        assert!(editor.panes.pane_at(&p0).is_some());
        assert!(editor.panes.pane_at(&p1).is_some());
        // Out-of-range index → None.
        let p_bad = devix_protocol::path::Path::parse("/pane/2").unwrap();
        assert!(editor.panes.pane_at(&p_bad).is_none());
    }

    #[test]
    fn pane_at_rejects_non_pane_root() {
        let editor = Editor::open(None).unwrap();
        let p = devix_protocol::path::Path::parse("/buf/42").unwrap();
        assert!(editor.panes.pane_at(&p).is_none());
    }

    #[test]
    fn pane_paths_enumerates_tree_in_pre_order() {
        use crate::Axis;
        let mut editor = Editor::open(None).unwrap();
        editor.split_active(Axis::Horizontal);
        let paths: Vec<String> = editor
            .panes
            .pane_paths()
            .into_iter()
            .map(|p| p.as_str().to_string())
            .collect();
        // Root + two children, in pre-order.
        assert_eq!(paths, vec!["/pane", "/pane/0", "/pane/1"]);
    }

    /// Helper: subscribe to every pulse on `bus` and dump captures
    /// into the returned `Arc<Mutex<Vec<Pulse>>>`. Used by ops-pulse
    /// tests (T-102).
    fn capture_pulses(
        bus: &crate::PulseBus,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<devix_protocol::pulse::Pulse>>> {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
        bus.subscribe(devix_protocol::pulse::PulseFilter::any(), move |p| {
            cap.lock().unwrap().push(p.clone());
        });
        captured
    }

    #[test]
    fn split_active_publishes_frame_split_pulse() {
        use crate::Axis;
        use devix_protocol::pulse::{Pulse, PulseKind};
        let mut ws = Editor::open(None).unwrap();
        let captured = capture_pulses(&ws.bus);
        ws.split_active(Axis::Horizontal);
        let pulses = captured.lock().unwrap();
        let split = pulses
            .iter()
            .find(|p| p.kind() == PulseKind::FrameSplit)
            .expect("FrameSplit pulse fired");
        if let Pulse::FrameSplit { source, new, .. } = split {
            assert_eq!(source.as_str(), "/pane");
            assert_eq!(new.as_str(), "/pane/1");
        }
    }

    #[test]
    fn close_active_frame_publishes_frame_closed_pulse() {
        use crate::Axis;
        use devix_protocol::pulse::{Pulse, PulseKind};
        let mut ws = Editor::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        let captured = capture_pulses(&ws.bus);
        ws.close_active_frame();
        let pulses = captured.lock().unwrap();
        let closed = pulses
            .iter()
            .find(|p| p.kind() == PulseKind::FrameClosed)
            .expect("FrameClosed pulse fired");
        if let Pulse::FrameClosed { frame } = closed {
            assert_eq!(frame.as_str(), "/pane/1");
        }
    }

    #[test]
    fn toggle_sidebar_publishes_sidebar_toggled_pulse() {
        use devix_protocol::pulse::{Pulse, PulseKind};
        let mut ws = Editor::open(None).unwrap();
        let captured = capture_pulses(&ws.bus);
        ws.toggle_sidebar(SidebarSlot::Left);
        ws.toggle_sidebar(SidebarSlot::Left);
        let pulses = captured.lock().unwrap();
        let events: Vec<bool> = pulses
            .iter()
            .filter_map(|p| match p {
                Pulse::SidebarToggled { open, .. } => Some(*open),
                _ => None,
            })
            .collect();
        assert_eq!(events, vec![true, false], "open then close");
        let count = pulses
            .iter()
            .filter(|p| p.kind() == PulseKind::SidebarToggled)
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn open_then_dismiss_modal_publishes_modal_pulses() {
        use crate::editor::commands::{CommandRegistry, modal::PalettePane};
        use devix_protocol::pulse::{ModalKind, Pulse, PulseKind};
        let mut ws = Editor::open(None).unwrap();
        let captured = capture_pulses(&ws.bus);
        let registry = CommandRegistry::default();
        ws.open_modal(
            Box::new(PalettePane::from_registry(&registry)),
            ModalKind::Palette,
        );
        ws.dismiss_modal();
        let pulses = captured.lock().unwrap();
        let kinds: Vec<PulseKind> = pulses
            .iter()
            .filter(|p| matches!(
                p.kind(),
                PulseKind::ModalOpened | PulseKind::ModalDismissed
            ))
            .map(|p| p.kind())
            .collect();
        assert_eq!(kinds, vec![PulseKind::ModalOpened, PulseKind::ModalDismissed]);
        if let Some(Pulse::ModalOpened { modal, .. }) =
            pulses.iter().find(|p| p.kind() == PulseKind::ModalOpened)
        {
            assert_eq!(*modal, ModalKind::Palette);
        }
    }

    #[test]
    fn focus_dir_publishes_focus_changed_pulse_only_on_change() {
        use crate::{Axis, Direction};
        use devix_protocol::pulse::PulseKind;
        let mut ws = Editor::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        let captured = capture_pulses(&ws.bus);
        ws.focus_dir(Direction::Left);
        ws.focus_dir(Direction::Left); // already at the leftmost — no change
        let pulses = captured.lock().unwrap();
        let count = pulses
            .iter()
            .filter(|p| p.kind() == PulseKind::FocusChanged)
            .count();
        assert_eq!(count, 1, "FocusChanged fires once for the real transition");
    }
}
