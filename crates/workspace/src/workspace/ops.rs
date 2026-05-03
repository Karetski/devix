//! Mutating operations on a Workspace: tabs, frames/splits, sidebars,
//! file-open routing. Kept separate from focus/hit-test so each concern stays
//! reviewable on its own.

use std::path::PathBuf;

use anyhow::Result;

use crate::document::{DocId, Document};
use crate::frame::Frame;
use crate::layout::{Axis, Node, SidebarSlot};
use crate::view::{ScrollMode, View, ViewId};

use super::{LeafRef, Workspace, canonicalize_or_keep};

impl Workspace {
    /// Open a fresh empty buffer in a new tab on the active frame.
    pub fn new_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let did = self.documents.insert(Document::empty());
        let vid = self.views.insert(View::new(did));
        let frame = &mut self.frames[fid];
        frame.tabs.push(vid);
        let new_idx = frame.tabs.len() - 1;
        frame.set_active(new_idx);
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
            frame.set_active(0);
            self.views.remove(vid);
            self.try_remove_orphan_doc(did);
            return true;
        }

        let idx = frame.active_tab;
        frame.tabs.remove(idx);
        let next = idx.min(frame.tabs.len() - 1);
        frame.set_active(next);
        self.views.remove(vid);
        self.try_remove_orphan_doc(did);
        true
    }

    pub fn next_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let frame = &mut self.frames[fid];
        if frame.tabs.is_empty() { return; }
        let next = (frame.active_tab + 1) % frame.tabs.len();
        frame.set_active(next);
    }

    pub fn prev_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let frame = &mut self.frames[fid];
        if frame.tabs.is_empty() { return; }
        let prev = (frame.active_tab + frame.tabs.len() - 1) % frame.tabs.len();
        frame.set_active(prev);
    }

    /// Open `path` in the active frame's current tab (replace-current semantics).
    /// If a Document already exists for the canonicalized path, reuse it.
    /// Returns the new ViewId.
    pub fn open_path_replace_current(&mut self, path: PathBuf) -> Result<ViewId> {
        let key = canonicalize_or_keep(&path);
        let did = if let Some(&existing) = self.doc_index.get(&key) {
            existing
        } else {
            let mut doc = Document::from_path(path)?;
            if let Some(wiring) = self.lsp_wiring() {
                doc.attach_lsp(wiring.sink, wiring.encoding);
            }
            let id = self.documents.insert(doc);
            self.doc_index.insert(key, id);
            id
        };
        // Resolve the active frame and old view BEFORE allocating the new view,
        // so a missing frame or empty tabs short-circuits without leaving a
        // detached View in the slot-map.
        let Some(fid) = self.active_frame() else {
            return Err(anyhow::anyhow!("no active frame to open path into"));
        };
        let Some(old_view) = self.frames[fid].active_view() else {
            return Err(anyhow::anyhow!("active frame has no tabs"));
        };
        let new_view = self.views.insert(View::new(did));
        let frame = &mut self.frames[fid];
        frame.tabs[frame.active_tab] = new_view;
        let old_doc = self.views[old_view].doc;
        self.views.remove(old_view);
        self.try_remove_orphan_doc(old_doc);
        Ok(new_view)
    }

    /// Close the active frame if there are 2+ frames in the tree.
    /// The resulting Split with a single child collapses to that child.
    /// No-op when only one frame remains anywhere in the tree.
    pub fn close_active_frame(&mut self) {
        if self.frames.len() <= 1 { return; }
        let Some(fid) = self.active_frame() else { return };
        let path = self.focus.clone();
        if path.is_empty() { return; } // root is a single Frame; same as len==1

        let (parent_path, leaf_idx) = path.split_at(path.len() - 1);
        let leaf_idx = leaf_idx[0];
        let Some(parent) = node_at_mut(&mut self.layout, parent_path) else { return };
        if let Node::Split { children, .. } = parent {
            children.remove(leaf_idx);
        }
        self.layout.collapse_singleton_splits();
        let frame = self.frames.remove(fid).expect("frame existed");
        for vid in frame.tabs {
            let did = self.views[vid].doc;
            self.views.remove(vid);
            self.try_remove_orphan_doc(did);
        }
        // Re-anchor focus to the first remaining frame, deepest path.
        self.focus = first_frame_path(&self.layout);
    }

    /// Replace the focused Frame leaf with a Split containing two frames:
    /// the original frame, plus a new frame whose first tab clones the active view.
    pub fn split_active(&mut self, axis: Axis) {
        let Some(focus_path) = (if matches!(self.layout.leaf_at(&self.focus), Some(LeafRef::Frame(_))) {
            Some(self.focus.clone())
        } else { None }) else { return };

        let Some(active_fid) = self.active_frame() else { return };

        let cloned_view = {
            let Some(active_view_id) = self.frames[active_fid].active_view() else { return };
            let v = &self.views[active_view_id];
            View {
                doc: v.doc,
                selection: v.selection.clone(),
                target_col: v.target_col,
                scroll: v.scroll,
                scroll_mode: ScrollMode::Anchored,
            }
        };
        let new_view_id = self.views.insert(cloned_view);
        let new_frame_id = self.frames.insert(Frame::with_view(new_view_id));

        let new_node = Node::Split {
            axis,
            children: vec![
                (Node::Frame(active_fid), 1),
                (Node::Frame(new_frame_id), 1),
            ],
        };
        self.layout.replace_leaf_at(&focus_path, new_node);
        let mut new_focus = focus_path;
        new_focus.push(1);
        self.focus = new_focus;
    }

    pub fn toggle_sidebar(&mut self, slot: SidebarSlot) {
        // Lift the layout root into a horizontal Split if it isn't one.
        if !matches!(&self.layout, Node::Split { axis: Axis::Horizontal, .. }) {
            use slotmap::Key;
            let inner = std::mem::replace(
                &mut self.layout,
                Node::Frame(crate::frame::FrameId::null()),
            );
            self.layout = Node::Split {
                axis: Axis::Horizontal,
                children: vec![(inner, 80)],
            };
            // The focus path now needs a leading 0 (the editor body is child 0).
            let mut new_focus = vec![0];
            new_focus.extend(self.focus.iter().copied());
            self.focus = new_focus;
        }
        let Node::Split { children, .. } = &mut self.layout else {
            unreachable!("we just lifted the root into a horizontal Split")
        };

        let existing = children.iter().position(|(c, _)| matches!(c, Node::Sidebar(s) if *s == slot));
        if let Some(i) = existing {
            children.remove(i);
            if let Some(top) = self.focus.first_mut() {
                if *top >= i && *top > 0 { *top -= 1; }
            }
        } else {
            let insert_at = match slot {
                SidebarSlot::Left => 0,
                SidebarSlot::Right => children.len(),
            };
            children.insert(insert_at, (Node::Sidebar(slot), 20));
            if let Some(top) = self.focus.first_mut() {
                if *top >= insert_at { *top += 1; }
            }
        }
    }

    /// If no surviving view references `did`, drop the document and its
    /// path index entry. Sends `LspCommand::Close` to the LSP coordinator
    /// before the Document is removed.
    fn try_remove_orphan_doc(&mut self, did: DocId) {
        let still_used = self.views.values().any(|v| v.doc == did);
        if still_used { return; }
        if let Some(d) = self.documents.get_mut(did) {
            d.detach_lsp();
        }
        if let Some(d) = self.documents.remove(did) {
            if let Some(p) = d.buffer.path() {
                let key = canonicalize_or_keep(p);
                self.doc_index.remove(&key);
            }
        }
    }
}

fn node_at_mut<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    let mut n = node;
    for &i in path {
        match n {
            Node::Split { children, .. } => {
                n = &mut children.get_mut(i)?.0;
            }
            _ => return None,
        }
    }
    Some(n)
}

fn first_frame_path(node: &Node) -> Vec<usize> {
    fn go(node: &Node, path: &mut Vec<usize>) -> bool {
        match node {
            Node::Frame(_) => true,
            Node::Sidebar(_) => false, // skip sidebars when picking a default
            Node::Split { children, .. } => {
                for (i, (child, _)) in children.iter().enumerate() {
                    path.push(i);
                    if go(child, path) { return true; }
                    path.pop();
                }
                false
            }
        }
    }
    let mut p = Vec::new();
    if go(node, &mut p) { p } else { Vec::new() }
}
