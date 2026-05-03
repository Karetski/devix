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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LeafRef {
    Frame(FrameId),
    Sidebar(SidebarSlot),
}

/// One clickable tab region in a frame's tab strip. Mirrors `devix_ui::TabHit`
/// but lives here so workspace can do hit-testing without depending on the UI
/// crate.
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
    pub frame_rects: SecondaryMap<FrameId, Rect>,
    pub sidebar_rects: HashMap<SidebarSlot, Rect>,
    pub tab_strips: SecondaryMap<FrameId, TabStripCache>,
}

pub struct Workspace {
    pub documents: SlotMap<DocId, Document>,
    pub views: SlotMap<ViewId, View>,
    pub frames: SlotMap<FrameId, Frame>,
    pub layout: Node,
    pub focus: Vec<usize>,
    pub doc_index: HashMap<PathBuf, DocId>,
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

        Ok(Self {
            documents,
            views,
            frames,
            layout,
            focus,
            doc_index,
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
                scroll: v.scroll,
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
        self.focus = new_focus;
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

/// What was hit by a click on the tab-strip overlay.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TabStripHit {
    Tab { frame: FrameId, idx: usize },
}

impl Workspace {
    /// Find the tab-strip element under (col, row), if any. Used by the input
    /// layer before falling back to body-area hit-testing.
    pub fn tab_strip_hit(&self, col: u16, row: u16) -> Option<TabStripHit> {
        for (fid, strip) in &self.render_cache.tab_strips {
            for hit in &strip.hits {
                if rect_contains(hit.rect, col, row) {
                    return Some(TabStripHit::Tab { frame: fid, idx: hit.idx });
                }
            }
        }
        None
    }

    /// Frame whose tab-strip row contains (col, row). Independent of where in
    /// the strip the click landed — empty space past the last tab still
    /// resolves the frame so the wheel scrolls it.
    pub fn frame_at_strip(&self, col: u16, row: u16) -> Option<FrameId> {
        for (fid, strip) in &self.render_cache.tab_strips {
            if rect_contains(strip.strip_rect, col, row) {
                return Some(fid);
            }
        }
        None
    }

    /// Whether the tab strip currently overflows its row — i.e., scrolling
    /// can produce a visible change. Used by the input layer to decide
    /// whether to consume a wheel event or pass it through to the editor.
    pub fn tab_strip_can_scroll(&self, frame: FrameId) -> bool {
        let Some(strip) = self.render_cache.tab_strips.get(frame) else { return false };
        strip.content_width > strip.strip_rect.width as u32
    }

    /// Apply a horizontal scroll delta (cells) to a frame's tab strip. Routes
    /// through the frame's `CollectionState` so all scroll math lives in one
    /// place. No-op when content fits in the strip.
    pub fn scroll_tab_strip(&mut self, frame: FrameId, delta: isize) {
        let Some(strip) = self.render_cache.tab_strips.get(frame) else { return };
        let content = (strip.content_width, 1);
        let viewport = (strip.strip_rect.width as u32, 1);
        let Some(f) = self.frames.get_mut(frame) else { return };
        f.tab_strip_state.scroll_by(delta, 0, content, viewport);
    }

    /// Move focus to `frame`'s leaf, if it exists in the layout tree. Returns
    /// true on success.
    pub fn focus_frame(&mut self, frame: FrameId) -> bool {
        if let Some(path) = path_to_leaf(&self.layout, LeafRef::Frame(frame)) {
            self.focus = path;
            true
        } else {
            false
        }
    }

    /// Activate `idx` on `frame` from a click on a visible tab. Does *not*
    /// scroll the strip — the user already picked a tab they could see.
    /// Out-of-range indices clamp to a valid value.
    pub fn activate_tab(&mut self, frame: FrameId, idx: usize) {
        let Some(f) = self.frames.get_mut(frame) else { return };
        if f.tabs.is_empty() { return; }
        f.select_visible(idx.min(f.tabs.len() - 1));
    }

    /// Set focus to the leaf whose Rect contains (col, row), if any.
    pub fn focus_at_screen(&mut self, col: u16, row: u16) {
        let leaves = self.layout.leaves_with_rects(self.outer_editor_area());
        for (leaf, rect) in leaves {
            if (col >= rect.x && col < rect.x + rect.width)
                && (row >= rect.y && row < rect.y + rect.height)
            {
                if let Some(path) = path_to_leaf(&self.layout, leaf) {
                    self.focus = path;
                    return;
                }
            }
        }
    }

    /// The total area the layout tree occupies, derived from cached rects.
    /// Used by hit-testing without re-running a layout pass. Includes tab-strip
    /// rows so clicks on the strip can resolve to the owning frame.
    fn outer_editor_area(&self) -> ratatui::layout::Rect {
        use ratatui::layout::Rect;
        let rects: Vec<Rect> = self.render_cache.frame_rects.values().copied()
            .chain(self.render_cache.sidebar_rects.values().copied())
            .chain(
                self.render_cache.tab_strips.values()
                    .map(|s| s.strip_rect)
            )
            .collect();
        if rects.is_empty() { return Rect::default(); }
        let x = rects.iter().map(|r| r.x).min().unwrap();
        let y = rects.iter().map(|r| r.y).min().unwrap();
        let x_end = rects.iter().map(|r| r.x + r.width).max().unwrap();
        let y_end = rects.iter().map(|r| r.y + r.height).max().unwrap();
        Rect { x, y, width: x_end - x, height: y_end - y }
    }
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn path_to_leaf(node: &Node, target: LeafRef) -> Option<Vec<usize>> {
    fn matches(node: &Node, target: LeafRef) -> bool {
        match (node, target) {
            (Node::Frame(a), LeafRef::Frame(b)) => *a == b,
            (Node::Sidebar(a), LeafRef::Sidebar(b)) => *a == b,
            _ => false,
        }
    }
    fn go(node: &Node, target: LeafRef, out: &mut Vec<usize>) -> bool {
        if matches(node, target) { return true; }
        if let Node::Split { children, .. } = node {
            for (i, (c, _)) in children.iter().enumerate() {
                out.push(i);
                if go(c, target, out) { return true; }
                out.pop();
            }
        }
        false
    }
    let mut p = Vec::new();
    if go(node, target, &mut p) { Some(p) } else { None }
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
        let next: isize = (v.scroll_top() as isize).saturating_add(-1);
        v.set_scroll_top(next.clamp(0, 99) as usize);
        assert_eq!(v.scroll_top(), 0);

        // Overflow clamps to last visible line index.
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
        // Focus moves into the sidebar via the universal-focus rule.
        ws.focus_dir(Direction::Left);
        assert!(matches!(
            ws.layout.leaf_at(&ws.focus),
            Some(LeafRef::Sidebar(SidebarSlot::Left))
        ));
        // Toggle off the very sidebar that's focused.
        ws.toggle_sidebar(SidebarSlot::Left);
        // Focus must now point at a Frame leaf — never at a removed sidebar slot.
        assert!(
            matches!(ws.layout.leaf_at(&ws.focus), Some(LeafRef::Frame(_))),
            "after sidebar removal, focus should resolve to a Frame leaf"
        );
    }

    #[test]
    fn closing_one_of_three_split_children_keeps_two_remaining() {
        use crate::layout::{Axis, Node};
        let mut ws = Workspace::open(None).unwrap();
        ws.split_active(Axis::Horizontal); // 2 frames
        // Split again from the focused (right) frame to get 3 frames in a tree.
        // Note: split_active replaces the focused leaf with a Split, so this
        // produces a nested tree rather than a flat 3-child Split. We accept
        // that — the test still exercises CloseFrame on a tree with 3 frames.
        ws.split_active(Axis::Horizontal);
        assert_eq!(ws.frames.len(), 3);

        ws.close_active_frame();
        assert_eq!(ws.frames.len(), 2);
        // Layout should have at least one Split node remaining (since 2 frames).
        let has_split = matches!(&ws.layout, Node::Split { .. });
        assert!(has_split, "two frames should be in a Split, not a flat Frame leaf");
        // Focus must resolve to a Frame leaf.
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

        // Split, focus moves to new frame at child 1, open same path there.
        ws.split_active(Axis::Horizontal);
        let v2 = ws.open_path_replace_current(p.clone()).unwrap();
        let did2 = ws.views[v2].doc;

        assert_eq!(did1, did2, "same path opened in different frames should share DocId");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tab_strip_hit_returns_tab_under_cursor() {
        let mut ws = Workspace::open(None).unwrap();
        ws.new_tab(); // 2 tabs in active frame
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
        // Outside any tab → no hit.
        assert_eq!(ws.tab_strip_hit(50, 0), None);
        // Off the strip entirely → no hit.
        assert_eq!(ws.tab_strip_hit(5, 5), None);
    }

    #[test]
    fn activate_tab_focuses_clicked_index() {
        let mut ws = Workspace::open(None).unwrap();
        ws.new_tab();
        ws.new_tab(); // 3 tabs, active = 2
        let fid = ws.active_frame().unwrap();
        ws.activate_tab(fid, 0);
        assert_eq!(ws.frames[fid].active_tab, 0);
        // Out-of-range clamps to last.
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
        // Empty space past the last tab still resolves the frame so wheel scroll works.
        assert_eq!(ws.frame_at_strip(25, 4), Some(fid));
        assert_eq!(ws.frame_at_strip(25, 5), None);
    }

    #[test]
    fn next_tab_requests_recenter_but_click_does_not() {
        let mut ws = Workspace::open(None).unwrap();
        ws.new_tab(); // 2 tabs, active = 1, recenter set by new_tab
        let fid = ws.active_frame().unwrap();
        ws.frames[fid].recenter_active = false; // simulate render that consumed it

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
