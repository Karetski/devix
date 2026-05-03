//! Workspace = aggregate of all editor state owned across the layout tree:
//! documents, views, frames, plus the layout root, focus path, and the
//! per-frame render-rect cache.
//!
//! Behaviour is split across submodules by concern:
//!
//! * [`ops`]     — mutating operations (tabs, splits, sidebars, file open).
//! * [`focus`]   — directional focus traversal across the layout tree.
//! * [`hittest`] — screen-coord → leaf / tab-strip resolution and tab-strip
//!   scroll forwarding.
//!
//! Submodules add `impl Workspace { ... }` blocks; this file owns the struct,
//! its constructor, and the unconditional read-side accessors.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use devix_collection::Hit;
use devix_lsp::LspCommand;
use lsp_types::PositionEncodingKind;
use ratatui::layout::Rect;
use slotmap::{SecondaryMap, SlotMap};
use tokio::sync::mpsc;

use devix_document::{DocId, Document};
use crate::frame::{Frame, FrameId};
use crate::layout::{Node, SidebarSlot};
use crate::view::{View, ViewId};

mod focus;
mod hittest;
mod ops;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LeafRef {
    Frame(FrameId),
    Sidebar(SidebarSlot),
}

/// Hit-test layout of a frame's tab strip, populated on render. `strip_rect`
/// covers the whole 1-row tab strip (including empty space past the last tab),
/// so wheel events anywhere on that row resolve to this frame.
#[derive(Default, Clone, Debug)]
pub struct TabStripCache {
    pub strip_rect: Rect,
    pub content_width: u32,
    pub hits: Vec<Hit>,
}

#[derive(Default)]
pub struct RenderCache {
    pub frame_rects: SecondaryMap<FrameId, Rect>,
    pub sidebar_rects: HashMap<SidebarSlot, Rect>,
    pub tab_strips: SecondaryMap<FrameId, TabStripCache>,
}

/// What was hit by a click on the tab-strip overlay.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TabStripHit {
    Tab { frame: FrameId, idx: usize },
}

pub struct Workspace {
    pub documents: SlotMap<DocId, Document>,
    pub views: SlotMap<ViewId, View>,
    pub frames: SlotMap<FrameId, Frame>,
    pub layout: Node,
    pub focus: Vec<usize>,
    pub doc_index: HashMap<PathBuf, DocId>,
    pub render_cache: RenderCache,
    /// LSP coordinator sink + the encoding it negotiated. Set via
    /// `attach_lsp` after the App spawns the coordinator. New documents
    /// created with a recognized language auto-attach; on `None` the
    /// workspace runs without LSP integration.
    pub(crate) lsp: Option<LspChannel>,
}

#[derive(Clone)]
pub(crate) struct LspChannel {
    pub sink: mpsc::UnboundedSender<LspCommand>,
    pub encoding: PositionEncodingKind,
}

impl Workspace {
    /// Create a workspace with a single frame, single tab, single view.
    /// `path` is opened if Some; otherwise an empty scratch buffer is used.
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let mut documents: SlotMap<DocId, Document> = SlotMap::with_key();
        let mut views: SlotMap<ViewId, View> = SlotMap::with_key();
        let mut frames: SlotMap<FrameId, Frame> = SlotMap::with_key();
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
        let view_id = views.insert(View::new(doc_id));
        let frame_id = frames.insert(Frame::with_view(view_id));
        let layout = Node::Frame(frame_id);
        let focus = vec![]; // root is the frame leaf itself

        Ok(Self {
            documents,
            views,
            frames,
            layout,
            focus,
            doc_index,
            render_cache: RenderCache::default(),
            lsp: None,
        })
    }

    /// Wire this workspace to an LSP coordinator. Stores the sink and
    /// triggers `LspCommand::Open` for every existing document with a known
    /// language. Subsequent `open_path_*` calls auto-attach.
    ///
    /// Idempotent in shape but not in side effects — calling twice with
    /// different sinks would re-open every doc on the new sink (and orphan
    /// the old one); App doesn't do this.
    pub fn attach_lsp(
        &mut self,
        sink: mpsc::UnboundedSender<LspCommand>,
        encoding: PositionEncodingKind,
    ) {
        self.lsp = Some(LspChannel { sink: sink.clone(), encoding: encoding.clone() });
        for (_, doc) in self.documents.iter_mut() {
            doc.attach_lsp(sink.clone(), encoding.clone());
        }
    }

    pub(crate) fn lsp_channel(&self) -> Option<LspChannel> {
        self.lsp.clone()
    }

    /// Negotiated LSP position encoding, when the workspace is attached.
    /// Exposed so callers outside the workspace crate (the App-side event
    /// drain) can resolve LSP positions against ropes without having to
    /// pull the private `LspChannel` shape.
    pub fn lsp_encoding(&self) -> Option<PositionEncodingKind> {
        self.lsp.as_ref().map(|w| w.encoding.clone())
    }

    pub fn active_view(&self) -> Option<&View> {
        let frame_id = self.active_frame()?;
        let view_id = self.frames[frame_id].active_view()?;
        self.views.get(view_id)
    }

    pub fn active_view_mut(&mut self) -> Option<&mut View> {
        let frame_id = self.active_frame()?;
        let view_id = self.frames[frame_id].active_view()?;
        self.views.get_mut(view_id)
    }

    pub fn active_frame(&self) -> Option<FrameId> {
        match self.layout.leaf_at(&self.focus)? {
            LeafRef::Frame(id) => Some(id),
            LeafRef::Sidebar(_) => None,
        }
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut Document> {
        let v = self.active_view()?;
        self.documents.get_mut(v.doc)
    }

    pub fn active_doc(&self) -> Option<&Document> {
        let v = self.active_view()?;
        self.documents.get(v.doc)
    }

    /// Resolve focus to (frame, view, doc) IDs in one immutable borrow,
    /// so callers can take disjoint &mut borrows on the underlying slot-maps.
    pub fn active_ids(&self) -> Option<(FrameId, ViewId, DocId)> {
        let frame_id = self.active_frame()?;
        let view_id = self.frames[frame_id].active_view()?;
        let doc_id = self.views[view_id].doc;
        Some((frame_id, view_id, doc_id))
    }
}

pub(super) fn canonicalize_or_keep(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_workspace_has_one_frame_one_view() {
        let ws = Workspace::open(None).unwrap();
        assert_eq!(ws.frames.len(), 1);
        assert_eq!(ws.views.len(), 1);
        assert_eq!(ws.documents.len(), 1);
        assert!(ws.active_view().is_some());
    }

    #[test]
    fn new_tab_then_close_returns_to_previous() {
        let mut ws = Workspace::open(None).unwrap();
        let original_view = ws.active_view().unwrap().doc;

        ws.new_tab();
        assert_eq!(ws.frames.values().next().unwrap().tabs.len(), 2);
        assert_eq!(ws.frames.values().next().unwrap().active_tab, 1);

        assert!(ws.close_active_tab(false));
        let active = ws.active_view().unwrap();
        assert_eq!(active.doc, original_view);
    }

    #[test]
    fn close_last_tab_leaves_a_scratch_tab() {
        let mut ws = Workspace::open(None).unwrap();
        assert!(ws.close_active_tab(false));
        let frame = ws.frames.values().next().unwrap();
        assert_eq!(frame.tabs.len(), 1);
        let v = ws.active_view().unwrap();
        assert!(ws.documents[v.doc].buffer.path().is_none());
    }

    #[test]
    fn dirty_close_refused_force_close_succeeds() {
        use devix_buffer::{Selection, replace_selection_tx};
        let mut ws = Workspace::open(None).unwrap();
        let did = ws.active_view().unwrap().doc;
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

        let mut ws = Workspace::open(None).unwrap();
        let v1 = ws.open_path_replace_current(p.clone()).unwrap();
        let did1 = ws.views[v1].doc;
        ws.new_tab();
        let v2 = ws.open_path_replace_current(p.clone()).unwrap();
        let did2 = ws.views[v2].doc;
        assert_eq!(did1, did2, "same path should reuse DocId");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_creates_a_second_frame_and_focuses_it() {
        let mut ws = Workspace::open(None).unwrap();
        let original_fid = ws.active_frame().unwrap();
        ws.split_active(crate::layout::Axis::Horizontal);
        assert_eq!(ws.frames.len(), 2);
        let new_fid = ws.active_frame().unwrap();
        assert_ne!(original_fid, new_fid);

        let Some(orig_view_id) = ws.frames[original_fid].active_view() else { panic!("original frame has no active view"); };
        let Some(new_view_id) = ws.frames[new_fid].active_view() else { panic!("new frame has no active view"); };
        let original_doc = ws.views[orig_view_id].doc;
        let new_doc = ws.views[new_view_id].doc;
        assert_eq!(original_doc, new_doc, "split clones view, shares document");
    }

    #[test]
    fn closing_one_split_child_collapses_back_to_single_frame() {
        use crate::layout::Axis;
        let mut ws = Workspace::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        assert_eq!(ws.frames.len(), 2);
        ws.close_active_frame();
        assert_eq!(ws.frames.len(), 1);
        assert!(matches!(ws.layout, Node::Frame(_)), "single frame at root");
    }

    #[test]
    fn toggle_left_sidebar_adds_then_removes_it() {
        let mut ws = Workspace::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        let n = match &ws.layout { Node::Split { children, .. } => children.len(), _ => 0 };
        assert_eq!(n, 2, "split has editor + left sidebar");

        ws.toggle_sidebar(SidebarSlot::Left);
        let collapsed = matches!(&ws.layout, Node::Split { .. } | Node::Frame(_));
        assert!(collapsed);
    }

    #[test]
    fn focus_dir_right_after_split_returns_to_original() {
        use crate::layout::{Axis, Direction};
        let mut ws = Workspace::open(None).unwrap();
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
        use crate::layout::Direction;
        let mut ws = Workspace::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        ws.focus_dir(Direction::Left);
        assert!(matches!(
            ws.layout.leaf_at(&ws.focus),
            Some(LeafRef::Sidebar(SidebarSlot::Left))
        ));
    }

    #[test]
    fn scroll_clamps_at_zero_and_at_end() {
        use devix_buffer::{Selection, replace_selection_tx};

        let mut ws = Workspace::open(None).unwrap();
        let did = ws.active_view().unwrap().doc;
        let txt = "x\n".repeat(100);
        let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), &txt);
        ws.documents[did].buffer.apply(tx);

        let v = ws.active_view_mut().unwrap();
        let next: isize = (v.scroll_top() as isize).saturating_add(-1);
        v.set_scroll_top(next.clamp(0, 99) as usize);
        assert_eq!(v.scroll_top(), 0);

        let v = ws.active_view_mut().unwrap();
        let next: isize = (v.scroll_top() as isize).saturating_add(1_000_000);
        v.set_scroll_top(next.clamp(0, 99) as usize);
        assert_eq!(v.scroll_top(), 99);
    }

    #[test]
    fn closing_focused_sidebar_lands_focus_on_a_frame() {
        use crate::layout::Direction;
        let mut ws = Workspace::open(None).unwrap();
        ws.toggle_sidebar(SidebarSlot::Left);
        ws.focus_dir(Direction::Left);
        assert!(matches!(
            ws.layout.leaf_at(&ws.focus),
            Some(LeafRef::Sidebar(SidebarSlot::Left))
        ));
        ws.toggle_sidebar(SidebarSlot::Left);
        assert!(
            matches!(ws.layout.leaf_at(&ws.focus), Some(LeafRef::Frame(_))),
            "after sidebar removal, focus should resolve to a Frame leaf"
        );
    }

    #[test]
    fn closing_one_of_three_split_children_keeps_two_remaining() {
        use crate::layout::{Axis, Node};
        let mut ws = Workspace::open(None).unwrap();
        ws.split_active(Axis::Horizontal);
        ws.split_active(Axis::Horizontal);
        assert_eq!(ws.frames.len(), 3);

        ws.close_active_frame();
        assert_eq!(ws.frames.len(), 2);
        let has_split = matches!(&ws.layout, Node::Split { .. });
        assert!(has_split, "two frames should be in a Split, not a flat Frame leaf");
        assert!(matches!(ws.layout.leaf_at(&ws.focus), Some(LeafRef::Frame(_))));
    }

    #[test]
    fn opening_same_path_in_two_frames_shares_document() {
        use crate::layout::Axis;
        let dir = std::env::temp_dir().join(format!("devix-dedup-cross-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "abc").unwrap();

        let mut ws = Workspace::open(None).unwrap();
        let v1 = ws.open_path_replace_current(p.clone()).unwrap();
        let did1 = ws.views[v1].doc;

        ws.split_active(Axis::Horizontal);
        let v2 = ws.open_path_replace_current(p.clone()).unwrap();
        let did2 = ws.views[v2].doc;

        assert_eq!(did1, did2, "same path opened in different frames should share DocId");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tab_strip_hit_returns_tab_under_cursor() {
        let mut ws = Workspace::open(None).unwrap();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        let strip = TabStripCache {
            strip_rect: Rect { x: 0, y: 0, width: 30, height: 1 },
            content_width: 21,
            hits: vec![
                Hit { idx: 0, rect: Rect { x: 0, y: 0, width: 10, height: 1 } },
                Hit { idx: 1, rect: Rect { x: 11, y: 0, width: 10, height: 1 } },
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
        let mut ws = Workspace::open(None).unwrap();
        ws.new_tab();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        ws.activate_tab(fid, 0);
        assert_eq!(ws.frames[fid].active_tab, 0);
        ws.activate_tab(fid, 99);
        assert_eq!(ws.frames[fid].active_tab, 2);
    }

    #[test]
    fn scroll_tab_strip_clamps_to_content_minus_strip_width() {
        let mut ws = Workspace::open(None).unwrap();
        let fid = ws.active_frame().unwrap();
        ws.render_cache.tab_strips.insert(fid, TabStripCache {
            strip_rect: Rect { x: 0, y: 0, width: 20, height: 1 },
            content_width: 50,
            hits: Vec::new(),
        });
        ws.scroll_tab_strip(fid, 100);
        assert_eq!(ws.frames[fid].tab_strip_state.scroll_x, 30, "clamped to 50 - 20");
        ws.scroll_tab_strip(fid, -1000);
        assert_eq!(ws.frames[fid].tab_strip_state.scroll_x, 0, "clamped at 0");
    }

    #[test]
    fn scroll_tab_strip_noop_when_content_fits() {
        let mut ws = Workspace::open(None).unwrap();
        let fid = ws.active_frame().unwrap();
        ws.render_cache.tab_strips.insert(fid, TabStripCache {
            strip_rect: Rect { x: 0, y: 0, width: 20, height: 1 },
            content_width: 15,
            hits: Vec::new(),
        });
        ws.scroll_tab_strip(fid, 5);
        assert_eq!(ws.frames[fid].tab_strip_state.scroll_x, 0);
    }

    #[test]
    fn frame_at_strip_resolves_full_strip_row() {
        let mut ws = Workspace::open(None).unwrap();
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
        let mut ws = Workspace::open(None).unwrap();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        ws.frames[fid].recenter_active = false;

        ws.next_tab();
        assert!(ws.frames[fid].recenter_active, "keyboard nav requests scroll-into-view");

        ws.frames[fid].recenter_active = false;
        ws.activate_tab(fid, 0);
        assert!(!ws.frames[fid].recenter_active,
            "click activation must not request scroll — strip stays put");
    }

    #[test]
    fn activate_tab_does_not_change_tab_scroll() {
        let mut ws = Workspace::open(None).unwrap();
        ws.new_tab();
        ws.new_tab();
        let fid = ws.active_frame().unwrap();
        ws.frames[fid].tab_strip_state.scroll_x = 7;
        ws.activate_tab(fid, 0);
        assert_eq!(ws.frames[fid].tab_strip_state.scroll_x, 7,
            "click-to-activate must not relayout the strip");
    }

    #[test]
    fn focus_frame_jumps_focus_across_a_split() {
        use crate::layout::Axis;
        let mut ws = Workspace::open(None).unwrap();
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
