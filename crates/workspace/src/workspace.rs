//! Workspace = aggregate of all editor state owned across the layout tree:
//! documents, views, frames, plus the (Phase 3, single-frame for now) layout
//! root, focus path, and the per-frame render-rect cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ratatui::layout::Rect;
use slotmap::{SecondaryMap, SlotMap};

use crate::document::{DocId, Document};
use crate::frame::{Frame, FrameId};
use crate::layout::{Node, SidebarSlot};
use crate::view::{View, ViewId};
use devix_buffer::Buffer;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LeafRef {
    Frame(FrameId),
    Sidebar(SidebarSlot),
}

#[derive(Default)]
pub struct RenderCache {
    pub frame_rects: SecondaryMap<FrameId, Rect>,
    pub sidebar_rects: HashMap<SidebarSlot, Rect>,
}

pub struct Workspace {
    pub documents: SlotMap<DocId, Document>,
    pub views: SlotMap<ViewId, View>,
    pub frames: SlotMap<FrameId, Frame>,
    pub layout: Node,
    pub focus: Vec<usize>,
    pub doc_index: HashMap<PathBuf, DocId>,
    pub last_editor_focus: Vec<usize>,
    pub render_cache: RenderCache,
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
        let last_editor_focus = focus.clone();

        Ok(Self {
            documents,
            views,
            frames,
            layout,
            focus,
            doc_index,
            last_editor_focus,
            render_cache: RenderCache::default(),
        })
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

    pub fn insert_buffer(&mut self, buf: Buffer) -> DocId {
        self.documents.insert(Document::from_buffer(buf))
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

impl Workspace {
    /// Open a fresh empty buffer in a new tab on the active frame.
    pub fn new_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let did = self.documents.insert(Document::empty());
        let vid = self.views.insert(View::new(did));
        let frame = &mut self.frames[fid];
        frame.tabs.push(vid);
        frame.active_tab = frame.tabs.len() - 1;
    }

    /// Returns false if the active doc is dirty; the caller should warn.
    pub fn close_active_tab(&mut self, force: bool) -> bool {
        let Some(fid) = self.active_frame() else { return false };
        let frame = &self.frames[fid];
        let Some(vid) = frame.active_view() else { return false };
        let did = self.views[vid].doc;
        if !force && self.documents[did].buffer.dirty() { return false; }

        let frame = &mut self.frames[fid];
        if frame.tabs.len() == 1 {
            // Last tab in the frame: replace with a fresh scratch view.
            let new_did = self.documents.insert(Document::empty());
            let new_vid = self.views.insert(View::new(new_did));
            frame.tabs[0] = new_vid;
            frame.active_tab = 0;
            self.views.remove(vid);
            self.try_remove_orphan_doc(did);
            return true;
        }

        let idx = frame.active_tab;
        frame.tabs.remove(idx);
        if frame.active_tab >= frame.tabs.len() {
            frame.active_tab = frame.tabs.len() - 1;
        }
        self.views.remove(vid);
        self.try_remove_orphan_doc(did);
        true
    }

    pub fn next_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let frame = &mut self.frames[fid];
        if frame.tabs.is_empty() { return; }
        frame.active_tab = (frame.active_tab + 1) % frame.tabs.len();
    }

    pub fn prev_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let frame = &mut self.frames[fid];
        if frame.tabs.is_empty() { return; }
        frame.active_tab = (frame.active_tab + frame.tabs.len() - 1) % frame.tabs.len();
    }

    /// If no surviving view references `did`, drop the document and its
    /// path index entry.
    fn try_remove_orphan_doc(&mut self, did: DocId) {
        let still_used = self.views.values().any(|v| v.doc == did);
        if still_used { return; }
        if let Some(d) = self.documents.remove(did) {
            if let Some(p) = d.buffer.path() {
                let key = canonicalize_or_keep(p);
                self.doc_index.remove(&key);
            }
        }
    }
}

fn canonicalize_or_keep(p: &Path) -> PathBuf {
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
        // Mutate the active doc to make it dirty.
        let did = ws.active_view().unwrap().doc;
        let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), "hi");
        ws.documents[did].buffer.apply(tx);
        assert!(!ws.close_active_tab(false), "dirty close should refuse");
        assert!(ws.close_active_tab(true), "force close should succeed");
    }

    #[test]
    fn scroll_clamps_at_zero_and_at_end() {
        use devix_buffer::{Selection, replace_selection_tx};

        let mut ws = Workspace::open(None).unwrap();
        let did = ws.active_view().unwrap().doc;
        // 100 lines total.
        let txt = "x\n".repeat(100);
        let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), &txt);
        ws.documents[did].buffer.apply(tx);

        // Underflow clamps to 0.
        let v = ws.active_view_mut().unwrap();
        let next: isize = (v.scroll_top as isize).saturating_add(-1);
        v.scroll_top = next.clamp(0, 99) as usize;
        assert_eq!(v.scroll_top, 0);

        // Overflow clamps to last visible line index.
        let v = ws.active_view_mut().unwrap();
        let next: isize = (v.scroll_top as isize).saturating_add(1_000_000);
        v.scroll_top = next.clamp(0, 99) as usize;
        assert_eq!(v.scroll_top, 99);
    }
}
