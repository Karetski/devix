//! Mutating operations on an Editor: tabs, frames/splits, sidebars,
//! file-open routing.
//!
//! Layout mutations rewrite the tree via `Editor.panes` (the
//! `PaneRegistry` carved out at T-100). The structural tree remains
//! the source of truth; the registry is the seam.

use std::path::PathBuf;

use anyhow::Result;
use crate::Pane;
use devix_protocol::path::Path;
use devix_protocol::pulse::Pulse;

use crate::editor::document::{DocId, Document};
use crate::editor::frame::mint_id;
use crate::{Axis, SidebarSlot};
use crate::editor::registry::pane_leaf_id;
use crate::editor::tree::{
    LayoutFrame, LayoutSidebar, frame_pane, sidebar_pane, split_pane,
};
use crate::editor::cursor::{Cursor, CursorId, ScrollMode};

use super::{Editor, LeafRef, canonicalize_or_keep};

fn pane_path(indices: &[usize]) -> Path {
    let mut s = String::from("/pane");
    for i in indices {
        s.push('/');
        s.push_str(&i.to_string());
    }
    Path::parse(&s).expect("/pane(/<i>)* is canonical")
}

fn axis_to_protocol(axis: Axis) -> devix_protocol::view::Axis {
    match axis {
        Axis::Horizontal => devix_protocol::view::Axis::Horizontal,
        Axis::Vertical => devix_protocol::view::Axis::Vertical,
    }
}

impl Editor {
    /// Open a fresh empty buffer in a new tab on the active frame.
    pub fn new_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let did = self.documents.insert(Document::empty());
        let cid = self.cursors.insert(Cursor::new(did));
        let Some(frame) = self.panes.find_frame_mut(fid) else { return };
        frame.tabs.push(cid);
        let new_idx = frame.tabs.len() - 1;
        frame.set_active(new_idx);
    }

    /// Returns false if the active doc is dirty; the caller should warn.
    pub fn close_active_tab(&mut self, force: bool) -> bool {
        let Some(fid) = self.active_frame() else { return false };
        let Some(frame) = self.panes.find_frame(fid) else { return false };
        let Some(cid) = frame.active_cursor() else { return false };
        let did = self.cursors[cid].doc;
        if !force && self.documents[did].buffer.dirty() { return false; }

        // After this point we mutate. Re-borrow mutably; the immutable
        // `frame` ref above is dropped.
        let frame = match self.panes.find_frame_mut(fid) {
            Some(f) => f,
            None => return false,
        };
        if frame.tabs.len() == 1 {
            // Last tab in the frame: replace with a fresh scratch cursor.
            let new_did = self.documents.insert(Document::empty());
            let new_cid = self.cursors.insert(Cursor::new(new_did));
            let frame = match self.panes.find_frame_mut(fid) {
                Some(f) => f,
                None => return false,
            };
            frame.tabs[0] = new_cid;
            frame.set_active(0);
            self.cursors.remove(cid);
            self.try_remove_orphan_doc(did);
            return true;
        }

        let idx = frame.active_tab;
        frame.tabs.remove(idx);
        let next = idx.min(frame.tabs.len() - 1);
        frame.set_active(next);
        self.cursors.remove(cid);
        self.try_remove_orphan_doc(did);
        true
    }

    pub fn next_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let Some(frame) = self.panes.find_frame_mut(fid) else { return };
        if frame.tabs.is_empty() { return; }
        let next = (frame.active_tab + 1) % frame.tabs.len();
        frame.set_active(next);
    }

    pub fn prev_tab(&mut self) {
        let Some(fid) = self.active_frame() else { return };
        let Some(frame) = self.panes.find_frame_mut(fid) else { return };
        if frame.tabs.is_empty() { return; }
        let prev = (frame.active_tab + frame.tabs.len() - 1) % frame.tabs.len();
        frame.set_active(prev);
    }

    /// Open `path` in the active frame's current tab (replace-current
    /// semantics). If a Document already exists for the canonicalized
    /// path, reuse it. Returns the new CursorId.
    pub fn open_path_replace_current(&mut self, path: PathBuf) -> Result<CursorId> {
        let key = canonicalize_or_keep(&path);
        let did = if let Some(&existing) = self.doc_index.get(&key) {
            existing
        } else {
            let doc = Document::from_path(path)?;
            let id = self.documents.insert(doc);
            self.doc_index.insert(key, id);
            super::install_bus_watcher_for_doc(&mut self.documents, id, &self.bus);
            // F-5 follow-up 2026-05-12: announce the new buffer.
            // Fires only on the *new Document insert* branch — the
            // reuse-cached-doc branch above must stay silent so a
            // second tab onto the same path doesn't double-fire.
            let fs_path = self
                .documents
                .get(id)
                .and_then(|d| d.buffer.path().map(|p| p.to_path_buf()));
            self.bus
                .publish(devix_protocol::pulse::Pulse::BufferOpened {
                    path: id.to_path(),
                    fs_path,
                });
            id
        };
        // Resolve the active frame and old cursor BEFORE allocating the
        // new cursor, so a missing frame or empty tabs short-circuits
        // without leaving a detached Cursor in the slot-map.
        let Some(fid) = self.active_frame() else {
            return Err(anyhow::anyhow!("no active frame to open path into"));
        };
        let Some(old_cid) = self.panes.find_frame(fid).and_then(|f| f.active_cursor()) else {
            return Err(anyhow::anyhow!("active frame has no tabs"));
        };
        let new_cid = self.cursors.insert(Cursor::new(did));
        let Some(frame) = self.panes.find_frame_mut(fid) else {
            return Err(anyhow::anyhow!("active frame disappeared"));
        };
        frame.tabs[frame.active_tab] = new_cid;
        let old_doc = self.cursors[old_cid].doc;
        self.cursors.remove(old_cid);
        self.try_remove_orphan_doc(old_doc);
        Ok(new_cid)
    }

    /// Close the active frame if there are 2+ frames in the tree.
    pub fn close_active_frame(&mut self) {
        if self.panes.frames().len() <= 1 { return; }
        let Some(fid) = self.active_frame() else { return };
        let path = self.focus.as_vec();
        if path.is_empty() { return; } // root is a single Frame; same as len==1
        let frame_path = pane_path(&path);

        // Snapshot the dying frame's tabs *before* the structural mutation
        // drops the LayoutFrame.
        let dying_cursors: Vec<CursorId> = self
            .panes
            .find_frame(fid)
            .map(|f| f.tabs.clone())
            .unwrap_or_default();

        if !self.panes.remove_at(&path) {
            return;
        }
        self.panes.collapse_singletons();
        for cid in dying_cursors {
            let did = self.cursors[cid].doc;
            self.cursors.remove(cid);
            self.try_remove_orphan_doc(did);
        }
        self.bus.publish(Pulse::FrameClosed { frame: frame_path });
        // Re-anchor focus to the first remaining frame, deepest path.
        let new_focus = first_frame_path(self.panes.root());
        self.set_focus(new_focus);
    }

    /// Replace the focused Frame leaf with a Split containing two frames:
    /// the original frame plus a new frame whose first tab clones the
    /// active cursor.
    pub fn split_active(&mut self, axis: Axis) {
        let Some(active_fid) = self.active_frame() else { return };
        let focus_path = self.focus.as_vec();

        // Snapshot the active frame's state before mutating the tree.
        let Some(active_frame) = self.panes.find_frame(active_fid) else { return };
        let original_tabs = active_frame.tabs.clone();
        let original_active_tab = active_frame.active_tab;
        let original_scroll = active_frame.tab_strip_scroll;
        let Some(active_cursor_id) = active_frame.active_cursor() else { return };

        let cloned_cursor = {
            let c = &self.cursors[active_cursor_id];
            Cursor {
                doc: c.doc,
                selection: c.selection.clone(),
                target_col: c.target_col,
                scroll: c.scroll,
                scroll_mode: ScrollMode::Anchored,
            }
        };
        let new_cursor_id = self.cursors.insert(cloned_cursor);
        let new_frame_id = mint_id();

        let original_replaced: Box<dyn Pane> = Box::new(LayoutFrame {
            frame: active_fid,
            tabs: original_tabs,
            active_tab: original_active_tab,
            tab_strip_scroll: original_scroll,
            recenter_active: true,
        });
        let new_split = split_pane(
            axis,
            vec![
                (original_replaced, 1),
                (frame_pane(new_frame_id, new_cursor_id), 1),
            ],
        );
        let source_path = pane_path(&focus_path);
        if !self.panes.replace_at(&focus_path, new_split) {
            return;
        }
        let mut new_focus = focus_path;
        new_focus.push(1);
        let new_pane_path = pane_path(&new_focus);
        self.bus.publish(Pulse::FrameSplit {
            source: source_path,
            new: new_pane_path,
            axis: axis_to_protocol(axis),
        });
        self.set_focus(new_focus);
    }

    /// Install `pane` as the content of the sidebar slot `slot`. If the
    /// slot leaf doesn't exist yet, this also creates it (toggling the
    /// slot open). If a previous content was installed, it's replaced.
    pub fn install_sidebar_pane(&mut self, slot: SidebarSlot, pane: Box<dyn Pane>) {
        if !self.panes.sidebar_present(slot) {
            self.toggle_sidebar(slot);
        }
        if let Some(leaf) = self.panes.find_sidebar_mut(slot) {
            leaf.content = Some(pane);
        }
    }

    pub fn toggle_sidebar(&mut self, slot: SidebarSlot) {
        // Lift the root into a horizontal Split if it isn't one.
        if !self.panes.root_is_horizontal_split() {
            self.panes.lift_into_horizontal_split();
            // The focus path needs a leading 0 (editor body is child 0).
            // This preserves the user's logical focus across the lift, so
            // it goes through `transform` (no FocusChanged emit).
            self.focus.transform(|p| p.insert(0, 0));
        }

        let split = self
            .panes
            .root_split_mut()
            .expect("root is a horizontal LayoutSplit after lift");

        let existing = split.children.iter().position(|(c, _)| {
            c.as_ref()
                .as_any()
                .and_then(|a| a.downcast_ref::<LayoutSidebar>())
                .is_some_and(|sb| sb.slot == slot)
        });
        let (slot_path, opened) = if let Some(i) = existing {
            let path = pane_path(&[i]);
            split.children.remove(i);
            self.focus.transform(|p| {
                if let Some(top) = p.first_mut() {
                    if *top >= i && *top > 0 { *top -= 1; }
                }
            });
            (path, false)
        } else {
            let insert_at = match slot {
                SidebarSlot::Left => 0,
                SidebarSlot::Right => split.children.len(),
            };
            split.children.insert(insert_at, (sidebar_pane(slot), 20));
            self.focus.transform(|p| {
                if let Some(top) = p.first_mut() {
                    if *top >= insert_at { *top += 1; }
                }
            });
            (pane_path(&[insert_at]), true)
        };
        self.bus.publish(Pulse::SidebarToggled {
            slot: slot_path,
            open: opened,
        });
    }

    /// If no surviving cursor references `did`, drop the document and
    /// its path-index entry.
    fn try_remove_orphan_doc(&mut self, did: DocId) {
        let still_used = self.cursors.values().any(|c| c.doc == did);
        if still_used { return; }
        if let Some(d) = self.documents.remove(did) {
            if let Some(p) = d.buffer.path() {
                let key = canonicalize_or_keep(p);
                self.doc_index.remove(&key);
            }
        }
    }
}

/// Path to the first focusable Frame leaf in tree order. Sidebars are
/// skipped — picking a sidebar as the default focus would surprise the
/// user after closing a split.
fn first_frame_path(root: &dyn Pane) -> Vec<usize> {
    fn go(node: &dyn Pane, path: &mut Vec<usize>) -> bool {
        if matches!(pane_leaf_id(node), Some(LeafRef::Frame(_))) {
            return true;
        }
        for (i, (_, child)) in node.children(crate::Rect::default()).into_iter().enumerate() {
            path.push(i);
            if go(child, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut p = Vec::new();
    if go(root, &mut p) { p } else { Vec::new() }
}
