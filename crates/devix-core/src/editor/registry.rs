//! Pane registry — owner of the structural layout tree.
//!
//! Carved out of the god-`Editor` per T-100. The Editor now holds a
//! `PaneRegistry` instead of a bare `LayoutNode`; every walk / lookup /
//! mutation of the layout tree goes through the registry's API. Future
//! Stage-9 / T-91 work folds the closed `LayoutNode` enum into a unified
//! Pane tree, but the registry's public surface is the seam that lets
//! the focus chain (T-101), ops (T-102), and modal slot (T-103) take
//! their pieces independently.

use ratatui::Frame;

use crate::editor::frame::FrameId;
use crate::editor::tree::{
    LayoutCtx, LayoutFrame, LayoutNode, LayoutSidebar, LayoutSplit, mutate,
};
use crate::editor::LeafRef;
use crate::{Axis, Rect, SidebarSlot};

pub struct PaneRegistry {
    root: LayoutNode,
}

impl PaneRegistry {
    pub fn new(root: LayoutNode) -> Self {
        Self { root }
    }

    /// Read-only access to the underlying tree. The closed `LayoutNode`
    /// shape is still the source of truth pre-T-91; callers that need to
    /// pattern-match on it (e.g. tests asserting structural shape) go
    /// through this accessor.
    pub fn root(&self) -> &LayoutNode {
        &self.root
    }

    pub fn find_frame(&self, fid: FrameId) -> Option<&LayoutFrame> {
        self.root.find_frame(fid)
    }

    pub fn find_frame_mut(&mut self, fid: FrameId) -> Option<&mut LayoutFrame> {
        self.root.find_frame_mut(fid)
    }

    pub fn find_sidebar_mut(&mut self, slot: SidebarSlot) -> Option<&mut LayoutSidebar> {
        self.root.find_sidebar_mut(slot)
    }

    pub fn sidebar_present(&self, slot: SidebarSlot) -> bool {
        self.root.sidebar_present(slot)
    }

    pub fn frames(&self) -> Vec<FrameId> {
        self.root.frames()
    }

    pub fn at_path(&self, path: &[usize]) -> Option<&LayoutNode> {
        self.root.at_path(path)
    }

    pub fn at_path_mut(&mut self, path: &[usize]) -> Option<&mut LayoutNode> {
        self.root.at_path_mut(path)
    }

    pub fn at_path_with_rect(&self, area: Rect, path: &[usize]) -> Option<(Rect, &LayoutNode)> {
        self.root.at_path_with_rect(area, path)
    }

    pub fn leaves_with_rects(&self, area: Rect) -> Vec<(LeafRef, Rect)> {
        self.root.leaves_with_rects(area)
    }

    pub fn path_to_leaf(&self, target: LeafRef) -> Option<Vec<usize>> {
        self.root.path_to_leaf(target)
    }

    pub fn pane_at_xy(&self, area: Rect, col: u16, row: u16) -> Option<(Rect, &LayoutNode)> {
        self.root.pane_at(area, col, row)
    }

    pub fn render(&self, area: Rect, frame: &mut Frame<'_>, ctx: &LayoutCtx<'_>) {
        self.root.render(area, frame, ctx);
    }

    /// Resolve a `/pane(/<i>)*` path to the corresponding `LayoutNode`.
    /// Path segments after `pane` are 0-based `Split.children` indices.
    /// Per `docs/specs/namespace.md` § *Migration table* and T-52.
    pub fn pane_at(&self, path: &devix_protocol::path::Path) -> Option<&LayoutNode> {
        let indices = pane_path_indices(path)?;
        self.root.at_path(&indices)
    }

    pub fn pane_at_mut(
        &mut self,
        path: &devix_protocol::path::Path,
    ) -> Option<&mut LayoutNode> {
        let indices = pane_path_indices(path)?;
        self.root.at_path_mut(&indices)
    }

    /// Pre-order enumeration of every reachable pane path. `/pane` is
    /// the root; each `Split` adds child indices.
    pub fn pane_paths(&self) -> Vec<devix_protocol::path::Path> {
        let mut out = Vec::new();
        let root_path =
            devix_protocol::path::Path::parse("/pane").expect("/pane is canonical");
        walk_pane_paths(&self.root, root_path, &mut out);
        out
    }

    /// Replace the node at `path`. Empty path replaces the root.
    pub fn replace_at(&mut self, path: &[usize], new: LayoutNode) -> bool {
        mutate::replace_at(&mut self.root, path, new)
    }

    /// Remove the child at `path` from its parent split.
    pub fn remove_at(&mut self, path: &[usize]) -> bool {
        mutate::remove_at(&mut self.root, path)
    }

    /// Collapse single-child splits anywhere in the tree.
    pub fn collapse_singletons(&mut self) {
        mutate::collapse_singletons(&mut self.root);
    }

    /// Lift the root into a horizontal split so a sidebar can be inserted
    /// alongside it.
    pub fn lift_into_horizontal_split(&mut self) {
        mutate::lift_into_horizontal_split(&mut self.root);
    }

    /// Mutable access to the root split for the (currently in-tree) op
    /// patterns that destructure the root after `lift_into_horizontal_split`.
    /// Crate-internal so external callers stay on the typed API.
    pub(crate) fn root_split_mut(&mut self) -> Option<&mut LayoutSplit> {
        match &mut self.root {
            LayoutNode::Split(s) => Some(s),
            _ => None,
        }
    }

    /// Whether the root is a horizontal split. Used by `toggle_sidebar`
    /// to decide whether to lift first.
    pub fn root_is_horizontal_split(&self) -> bool {
        matches!(&self.root, LayoutNode::Split(s) if s.axis == Axis::Horizontal)
    }
}

fn pane_path_indices(path: &devix_protocol::path::Path) -> Option<Vec<usize>> {
    let mut segs = path.segments();
    if segs.next()? != "pane" {
        return None;
    }
    segs.map(|s| s.parse::<usize>().ok())
        .collect::<Option<Vec<_>>>()
}

fn walk_pane_paths(
    node: &LayoutNode,
    here: devix_protocol::path::Path,
    out: &mut Vec<devix_protocol::path::Path>,
) {
    out.push(here.clone());
    if let LayoutNode::Split(s) = node {
        for (idx, (child, _)) in s.children.iter().enumerate() {
            if let Ok(child_path) = here.join(&idx.to_string()) {
                walk_pane_paths(child, child_path, out);
            }
        }
    }
}
