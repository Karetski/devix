//! Mutating operations on a Surface: tabs, frames/splits, sidebars,
//! file-open routing. Kept separate from focus/hit-test so each concern stays
//! reviewable on its own.
//!
//! Phase 3c write-side: layout mutations rewrite `Surface.root`
//! directly via `crate::tree::mutate` helpers — no `Node` enum, no
//! `sync_root` rebuild. The structural Pane tree is the source of truth.

use std::path::PathBuf;

use anyhow::Result;
use devix_core::Pane;

use devix_document::{DocId, Document};
use crate::frame::mint_id;
use crate::layout::{Axis, SidebarSlot};
use crate::tree::{
    LayoutFrame, LayoutSidebar, LayoutSplit, LeafId, find_frame, find_frame_mut, frame_ids,
    mutate, pane_leaf_id,
};
use crate::view::{ScrollMode, View, ViewId};

use super::{Surface, canonicalize_or_keep};

impl Surface {
    /// Open a fresh empty buffer in a new tab on the active frame.
    pub fn new_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let did = self.documents.insert(Document::empty());
        let vid = self.views.insert(View::new(did));
        let Some(frame) = find_frame_mut(&mut self.root, fid) else { return };
        frame.tabs.push(vid);
        let new_idx = frame.tabs.len() - 1;
        frame.set_active(new_idx);
    }

    /// Returns false if the active doc is dirty; the caller should warn.
    pub fn close_active_tab(&mut self, force: bool) -> bool {
        let Some(fid) = self.active_frame() else { return false };
        let Some(frame) = find_frame(self.root.as_ref(), fid) else { return false };
        let Some(vid) = frame.active_view() else { return false };
        let did = self.views[vid].doc;
        if !force && self.documents[did].buffer.dirty() { return false; }

        // After this point we mutate. Re-borrow mutably; the immutable
        // `frame` ref above is dropped.
        let frame = match find_frame_mut(&mut self.root, fid) {
            Some(f) => f,
            None => return false,
        };
        if frame.tabs.len() == 1 {
            // Last tab in the frame: replace with a fresh scratch view.
            let new_did = self.documents.insert(Document::empty());
            let new_vid = self.views.insert(View::new(new_did));
            let frame = match find_frame_mut(&mut self.root, fid) {
                Some(f) => f,
                None => return false,
            };
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
        let Some(frame) = find_frame_mut(&mut self.root, fid) else { return };
        if frame.tabs.is_empty() { return; }
        let next = (frame.active_tab + 1) % frame.tabs.len();
        frame.set_active(next);
    }

    pub fn prev_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let Some(frame) = find_frame_mut(&mut self.root, fid) else { return };
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
            if let Some(wiring) = self.lsp_channel() {
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
        let Some(old_view) = find_frame(self.root.as_ref(), fid).and_then(|f| f.active_view())
        else {
            return Err(anyhow::anyhow!("active frame has no tabs"));
        };
        let new_view = self.views.insert(View::new(did));
        let Some(frame) = find_frame_mut(&mut self.root, fid) else {
            return Err(anyhow::anyhow!("active frame disappeared"));
        };
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
        if frame_ids(self.root.as_ref()).len() <= 1 { return; }
        let Some(fid) = self.active_frame() else { return };
        let path = self.focus.clone();
        if path.is_empty() { return; } // root is a single Frame; same as len==1

        // Snapshot the dying frame's tabs *before* the structural mutation
        // drops the LayoutFrame. After `remove_at` the frame is gone — its
        // owned `tabs` Vec went with it.
        let dying_views: Vec<ViewId> = find_frame(self.root.as_ref(), fid)
            .map(|f| f.tabs.clone())
            .unwrap_or_default();

        if !mutate::remove_at(&mut self.root, &path) {
            return;
        }
        mutate::collapse_singletons(&mut self.root);
        for vid in dying_views {
            let did = self.views[vid].doc;
            self.views.remove(vid);
            self.try_remove_orphan_doc(did);
        }
        // Re-anchor focus to the first remaining frame, deepest path.
        self.focus = first_frame_path(self.root.as_ref());
    }

    /// Replace the focused Frame leaf with a Split containing two frames:
    /// the original frame, plus a new frame whose first tab clones the active view.
    pub fn split_active(&mut self, axis: Axis) {
        let Some(active_fid) = self.active_frame() else { return };
        let focus_path = self.focus.clone();

        // Snapshot the active frame's state — both its current tab list
        // (which the new LayoutFrame will inherit) and its active view —
        // before we mutate the tree.
        let Some(active_frame) = find_frame(self.root.as_ref(), active_fid) else { return };
        let original_tabs = active_frame.tabs.clone();
        let original_active_tab = active_frame.active_tab;
        let original_scroll = active_frame.tab_strip_scroll;
        let Some(active_view_id) = active_frame.active_view() else { return };

        let cloned_view = {
            let v = &self.views[active_view_id];
            View {
                doc: v.doc,
                selection: v.selection.clone(),
                target_col: v.target_col,
                scroll: v.scroll,
                scroll_mode: ScrollMode::Anchored,
                hover: None,
                completion: None,
            }
        };
        let new_view_id = self.views.insert(cloned_view);
        let new_frame_id = mint_id();

        let original_replaced = LayoutFrame {
            frame: active_fid,
            tabs: original_tabs,
            active_tab: original_active_tab,
            tab_strip_scroll: original_scroll,
            recenter_active: true,
        };
        let new_split: Box<dyn Pane> = Box::new(LayoutSplit {
            axis,
            children: vec![
                (Box::new(original_replaced), 1),
                (Box::new(LayoutFrame::with_view(new_frame_id, new_view_id)), 1),
            ],
        });
        if !mutate::replace_at(&mut self.root, &focus_path, new_split) {
            return;
        }
        let mut new_focus = focus_path;
        new_focus.push(1);
        self.focus = new_focus;
    }

    pub fn toggle_sidebar(&mut self, slot: SidebarSlot) {
        // Lift the root into a horizontal Split if it isn't one.
        let needs_lift = !root_is_horizontal_split(self.root.as_ref());
        if needs_lift {
            mutate::lift_into_horizontal_split(&mut self.root);
            // The focus path now needs a leading 0 (the editor body is child 0).
            let mut new_focus = vec![0];
            new_focus.extend(self.focus.iter().copied());
            self.focus = new_focus;
        }

        let split = self
            .root
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<LayoutSplit>())
            .expect("root is a horizontal LayoutSplit after lift");

        let existing = split.children.iter().position(|(c, _)| {
            c.as_any()
                .and_then(|a| a.downcast_ref::<LayoutSidebar>())
                .map(|s| s.slot == slot)
                .unwrap_or(false)
        });
        if let Some(i) = existing {
            split.children.remove(i);
            if let Some(top) = self.focus.first_mut() {
                if *top >= i && *top > 0 { *top -= 1; }
            }
        } else {
            let insert_at = match slot {
                SidebarSlot::Left => 0,
                SidebarSlot::Right => split.children.len(),
            };
            split.children.insert(
                insert_at,
                (Box::new(LayoutSidebar { slot }), 20),
            );
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

/// Is the structural root a horizontal `LayoutSplit`? Used by
/// `toggle_sidebar` to decide whether to lift before inserting.
fn root_is_horizontal_split(root: &dyn Pane) -> bool {
    root.as_any()
        .and_then(|a| a.downcast_ref::<LayoutSplit>())
        .map(|s| s.axis == Axis::Horizontal)
        .unwrap_or(false)
}

/// Path to the first focusable Frame leaf in tree order. Sidebars are
/// skipped — picking a sidebar as the default focus would surprise the
/// user after closing a split.
fn first_frame_path(root: &dyn Pane) -> Vec<usize> {
    fn go(pane: &dyn Pane, path: &mut Vec<usize>) -> bool {
        if let Some(LeafId::Frame(_)) = pane_leaf_id(pane) {
            return true;
        }
        if let Some(split) = pane.as_any().and_then(|a| a.downcast_ref::<LayoutSplit>()) {
            for (i, (child, _)) in split.children.iter().enumerate() {
                path.push(i);
                if go(child.as_ref(), path) { return true; }
                path.pop();
            }
        }
        false
    }
    let mut p = Vec::new();
    if go(root, &mut p) { p } else { Vec::new() }
}

