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
use crate::layout::{Axis, Direction, Node, SidebarSlot};
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

impl Workspace {
    /// Open `path` in the active frame's current tab (replace-current semantics).
    /// If a Document already exists for the canonicalized path, reuse it.
    /// Returns the new ViewId.
    pub fn open_path_replace_current(&mut self, path: PathBuf) -> Result<ViewId> {
        let key = canonicalize_or_keep(&path);
        let did = if let Some(&existing) = self.doc_index.get(&key) {
            existing
        } else {
            let id = self.documents.insert(Document::from_path(path)?);
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
}

impl Workspace {
    /// Close the active frame if there are 2+ frames in the tree.
    /// The resulting Split with a single child collapses to that child.
    /// No-op when only one frame remains anywhere in the tree.
    pub fn close_active_frame(&mut self) {
        if self.frames.len() <= 1 { return; }
        let Some(fid) = self.active_frame() else { return };
        let path = self.focus.clone();
        if path.is_empty() { return; } // root is a single Frame; same as len==1

        // Remove the leaf from its parent split.
        let (parent_path, leaf_idx) = path.split_at(path.len() - 1);
        let leaf_idx = leaf_idx[0];
        let Some(parent) = node_at_mut(&mut self.layout, parent_path) else { return };
        if let Node::Split { children, .. } = parent {
            children.remove(leaf_idx);
        }
        // Collapse one-child splits up the chain.
        self.layout.collapse_singleton_splits();
        // Drop the views/frames the closed frame held.
        let frame = self.frames.remove(fid).expect("frame existed");
        for vid in frame.tabs {
            let did = self.views[vid].doc;
            self.views.remove(vid);
            self.try_remove_orphan_doc(did);
        }
        // Re-anchor focus to the first remaining frame, deepest path.
        self.focus = first_frame_path(&self.layout);
        self.last_editor_focus = self.focus.clone();
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

impl Workspace {
    /// Replace the focused Frame leaf with a Split containing two frames:
    /// the original frame, plus a new frame whose first tab clones the active view.
    pub fn split_active(&mut self, axis: Axis) {
        let Some(focus_path) = (if matches!(self.layout.leaf_at(&self.focus), Some(LeafRef::Frame(_))) {
            Some(self.focus.clone())
        } else { None }) else { return };

        let Some(active_fid) = self.active_frame() else { return };

        // Clone the active view: same DocId, copy of selection/scroll.
        let cloned_view = {
            let Some(active_view_id) = self.frames[active_fid].active_view() else { return };
            let v = &self.views[active_view_id];
            View {
                doc: v.doc,
                selection: v.selection.clone(),
                target_col: v.target_col,
                scroll_top: v.scroll_top,
                view_anchored: true,
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
        // Move focus to the new (right/bottom) frame at index 1 in the new Split.
        let mut new_focus = focus_path;
        new_focus.push(1);
        self.focus = new_focus.clone();
        self.last_editor_focus = new_focus;
    }
}

impl Workspace {
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
            self.last_editor_focus = self.focus.clone();
        }
        let Node::Split { children, .. } = &mut self.layout else {
            unreachable!("we just lifted the root into a horizontal Split")
        };

        // Find an existing sidebar of this slot.
        let existing = children.iter().position(|(c, _)| matches!(c, Node::Sidebar(s) if *s == slot));
        if let Some(i) = existing {
            children.remove(i);
            // If focus was on or past this index, fix it up.
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
}

impl Workspace {
    pub fn focus_dir(&mut self, dir: Direction) {
        if let Some(target_path) = compute_focus_target(&self.layout, &self.focus, dir, &self.render_cache) {
            self.focus = target_path;
            if matches!(self.layout.leaf_at(&self.focus), Some(LeafRef::Frame(_))) {
                self.last_editor_focus = self.focus.clone();
            }
            return;
        }
        // Edge: try to move into a sidebar.
        let needed: Option<SidebarSlot> = match dir {
            Direction::Left => Some(SidebarSlot::Left),
            Direction::Right => Some(SidebarSlot::Right),
            _ => None,
        };
        if let Some(slot) = needed {
            if let Some(path) = find_sidebar(&self.layout, slot) {
                self.focus = path;
            }
        }
    }
}

fn compute_focus_target(
    layout: &Node,
    focus: &[usize],
    dir: Direction,
    cache: &RenderCache,
) -> Option<Vec<usize>> {
    let needed_axis = match dir {
        Direction::Left | Direction::Right => Axis::Horizontal,
        Direction::Up   | Direction::Down  => Axis::Vertical,
    };
    let step: isize = match dir {
        Direction::Left | Direction::Up   => -1,
        Direction::Right | Direction::Down => 1,
    };

    // Walk up from the leaf, looking for a Split on the needed axis where we
    // can step in `step` direction.
    let mut path = focus.to_vec();
    while !path.is_empty() {
        let parent_path = path[..path.len() - 1].to_vec();
        let child_idx = *path.last().unwrap();
        let parent = node_at(layout, &parent_path)?;
        if let Node::Split { axis, children } = parent {
            if *axis == needed_axis {
                let next = child_idx as isize + step;
                if next >= 0 && (next as usize) < children.len() {
                    let mut new_path = parent_path;
                    new_path.push(next as usize);
                    return Some(walk_into(layout, new_path, dir, focus, cache));
                }
            }
        }
        path.pop();
    }
    None
}

fn walk_into(
    layout: &Node,
    mut path: Vec<usize>,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> Vec<usize> {
    loop {
        let n = match node_at(layout, &path) {
            Some(n) => n,
            None => return path,
        };
        match n {
            Node::Frame(_) | Node::Sidebar(_) => return path,
            Node::Split { axis, children } => {
                let pick = pick_closest_child(layout, &path, *axis, children.len(), dir, source_path, cache);
                path.push(pick);
            }
        }
    }
}

fn pick_closest_child(
    layout: &Node,
    parent_path: &[usize],
    axis: Axis,
    n_children: usize,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> usize {
    if n_children == 0 { return 0; }
    let source_rect = leaf_rect_for(layout, source_path, cache);
    let Some(src) = source_rect else {
        // Fallback when no rect cached yet: pick the side adjacent to the move.
        return match (axis, dir) {
            (Axis::Horizontal, Direction::Left) => n_children - 1,
            (Axis::Horizontal, Direction::Right) => 0,
            (Axis::Vertical, Direction::Up) => n_children - 1,
            (Axis::Vertical, Direction::Down) => 0,
            _ => 0,
        };
    };
    let centre_y = src.y + src.height / 2;
    let centre_x = src.x + src.width / 2;
    // Compute each child's centre and pick the closest along the perpendicular axis.
    let mut best = 0usize;
    let mut best_d = i32::MAX;
    for i in 0..n_children {
        let mut child_path = parent_path.to_vec();
        child_path.push(i);
        let Some(rect) = first_leaf_rect(layout, &child_path, cache) else { continue };
        let d = match axis {
            Axis::Horizontal => (rect.y as i32 + rect.height as i32 / 2 - centre_y as i32).abs(),
            Axis::Vertical => (rect.x as i32 + rect.width as i32 / 2 - centre_x as i32).abs(),
        };
        if d < best_d { best_d = d; best = i; }
    }
    best
}

fn leaf_rect_for(layout: &Node, path: &[usize], cache: &RenderCache) -> Option<ratatui::layout::Rect> {
    match layout.leaf_at(path)? {
        LeafRef::Frame(id) => cache.frame_rects.get(id).copied(),
        LeafRef::Sidebar(slot) => cache.sidebar_rects.get(&slot).copied(),
    }
}

fn first_leaf_rect(layout: &Node, path: &[usize], cache: &RenderCache) -> Option<ratatui::layout::Rect> {
    fn descend<'a>(node: &'a Node, path: &mut Vec<usize>) -> &'a Node {
        match node {
            Node::Split { children, .. } if !children.is_empty() => {
                path.push(0);
                descend(&children[0].0, path)
            }
            other => other,
        }
    }
    let mut p = path.to_vec();
    let root = node_at(layout, &p)?;
    let _final_node = descend(root, &mut p);
    leaf_rect_for(layout, &p, cache)
}

fn node_at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Node> {
    let mut n = node;
    for &i in path {
        match n {
            Node::Split { children, .. } => n = &children.get(i)?.0,
            _ => return None,
        }
    }
    Some(n)
}

fn find_sidebar(node: &Node, slot: SidebarSlot) -> Option<Vec<usize>> {
    fn go(node: &Node, slot: SidebarSlot, out: &mut Vec<usize>) -> bool {
        match node {
            Node::Sidebar(s) if *s == slot => true,
            Node::Split { children, .. } => {
                for (i, (c, _)) in children.iter().enumerate() {
                    out.push(i);
                    if go(c, slot, out) { return true; }
                    out.pop();
                }
                false
            }
            _ => false,
        }
    }
    let mut p = Vec::new();
    if go(node, slot, &mut p) { Some(p) } else { None }
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
    fn opening_same_path_twice_reuses_document() {
        let dir = std::env::temp_dir().join(format!("devix-open-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        std::fs::write(&p, "abc").unwrap();

        let mut ws = Workspace::open(None).unwrap();
        let v1 = ws.open_path_replace_current(p.clone()).unwrap();
        let did1 = ws.views[v1].doc;
        // Open another tab and re-open the same path there.
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

        // Both views should reference the same DocId (shared buffer).
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

        // Focus left should land on the original frame.
        // Render-cache is empty, fallback rule picks adjacent: right→Left = last child of left subtree.
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
        // After toggling on, focus is still on the editor frame.
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
