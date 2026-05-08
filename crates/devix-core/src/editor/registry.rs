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
use crate::pane::Pane;
use crate::{Axis, Rect, SidebarSlot};

pub struct PaneRegistry {
    /// `Box<dyn Pane>` per T-91 acceptance criterion. The concrete
    /// type is currently `LayoutNode` (which now `impl Pane`);
    /// later T-91 phases carve the variants into standalone Pane
    /// structs and retire the enum. Helper accessors `as_layout` /
    /// `as_layout_mut` recover the typed view for the existing
    /// per-variant operations until the carve completes.
    root: Box<dyn Pane>,
}

impl PaneRegistry {
    pub fn new(root: LayoutNode) -> Self {
        Self { root: Box::new(root) }
    }

    fn as_layout(&self) -> &LayoutNode {
        self.root
            .as_any()
            .and_then(|a| a.downcast_ref::<LayoutNode>())
            .expect("PaneRegistry root is currently always a LayoutNode (T-91 phase 1)")
    }

    fn as_layout_mut(&mut self) -> &mut LayoutNode {
        self.root
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<LayoutNode>())
            .expect("PaneRegistry root is currently always a LayoutNode (T-91 phase 1)")
    }

    /// Read-only access to the underlying layout tree. Wraps the
    /// downcast through `Pane::as_any`; pattern-matching on the
    /// LayoutNode variant stays available until the variants are
    /// carved into standalone Pane structs (T-91 phase 2).
    pub fn root(&self) -> &LayoutNode {
        self.as_layout()
    }

    pub fn find_frame(&self, fid: FrameId) -> Option<&LayoutFrame> {
        self.as_layout().find_frame(fid)
    }

    pub fn find_frame_mut(&mut self, fid: FrameId) -> Option<&mut LayoutFrame> {
        self.as_layout_mut().find_frame_mut(fid)
    }

    pub fn find_sidebar_mut(&mut self, slot: SidebarSlot) -> Option<&mut LayoutSidebar> {
        self.as_layout_mut().find_sidebar_mut(slot)
    }

    pub fn sidebar_present(&self, slot: SidebarSlot) -> bool {
        self.as_layout().sidebar_present(slot)
    }

    pub fn frames(&self) -> Vec<FrameId> {
        self.as_layout().frames()
    }

    pub fn at_path(&self, path: &[usize]) -> Option<&LayoutNode> {
        self.as_layout().at_path(path)
    }

    pub fn at_path_mut(&mut self, path: &[usize]) -> Option<&mut LayoutNode> {
        self.as_layout_mut().at_path_mut(path)
    }

    pub fn at_path_with_rect(&self, area: Rect, path: &[usize]) -> Option<(Rect, &LayoutNode)> {
        self.as_layout().at_path_with_rect(area, path)
    }

    pub fn leaves_with_rects(&self, area: Rect) -> Vec<(LeafRef, Rect)> {
        self.as_layout().leaves_with_rects(area)
    }

    pub fn path_to_leaf(&self, target: LeafRef) -> Option<Vec<usize>> {
        self.as_layout().path_to_leaf(target)
    }

    pub fn pane_at_xy(&self, area: Rect, col: u16, row: u16) -> Option<(Rect, &LayoutNode)> {
        self.as_layout().pane_at(area, col, row)
    }

    pub fn render(&self, area: Rect, frame: &mut Frame<'_>, ctx: &LayoutCtx<'_>) {
        self.as_layout().render(area, frame, ctx);
    }

    /// Resolve a `/pane(/<i>)*` path to the corresponding `LayoutNode`.
    /// Path segments after `pane` are 0-based `Split.children` indices.
    /// Per `docs/specs/namespace.md` § *Migration table* and T-52.
    pub fn pane_at(&self, path: &devix_protocol::path::Path) -> Option<&LayoutNode> {
        let indices = pane_path_indices(path)?;
        self.as_layout().at_path(&indices)
    }

    pub fn pane_at_mut(
        &mut self,
        path: &devix_protocol::path::Path,
    ) -> Option<&mut LayoutNode> {
        let indices = pane_path_indices(path)?;
        self.as_layout_mut().at_path_mut(&indices)
    }

    /// Pre-order enumeration of every reachable pane path. `/pane` is
    /// the root; each composite (today: `LayoutSplit`; future: any
    /// `Pane` with non-empty `children`) adds child indices.
    /// Walks via `Pane::children` so the helper stays generic over
    /// the pane kind — phase 2 of T-91 will introduce additional
    /// composite Pane impls and they'll plug in here without code
    /// changes.
    pub fn pane_paths(&self) -> Vec<devix_protocol::path::Path> {
        let mut out = Vec::new();
        let root_path =
            devix_protocol::path::Path::parse("/pane").expect("/pane is canonical");
        // The pane-path encoding doesn't depend on rect math; pass
        // a zero rect — the walk only consults the children indices.
        let zero = Rect::default();
        walk_pane_paths_via_trait(self.root.as_ref(), zero, root_path, &mut out);
        out
    }

    /// Replace the node at `path`. Empty path replaces the root.
    pub fn replace_at(&mut self, path: &[usize], new: LayoutNode) -> bool {
        mutate::replace_at(self.as_layout_mut(), path, new)
    }

    /// Remove the child at `path` from its parent split.
    pub fn remove_at(&mut self, path: &[usize]) -> bool {
        mutate::remove_at(self.as_layout_mut(), path)
    }

    /// Collapse single-child splits anywhere in the tree.
    pub fn collapse_singletons(&mut self) {
        mutate::collapse_singletons(self.as_layout_mut());
    }

    /// Lift the root into a horizontal split so a sidebar can be inserted
    /// alongside it.
    pub fn lift_into_horizontal_split(&mut self) {
        mutate::lift_into_horizontal_split(self.as_layout_mut());
    }

    /// Mutable access to the root split for the (currently in-tree) op
    /// patterns that destructure the root after `lift_into_horizontal_split`.
    /// Crate-internal so external callers stay on the typed API.
    pub(crate) fn root_split_mut(&mut self) -> Option<&mut LayoutSplit> {
        match self.as_layout_mut() {
            LayoutNode::Split(s) => Some(s),
            _ => None,
        }
    }

    /// Whether the root is a horizontal split. Used by `toggle_sidebar`
    /// to decide whether to lift first.
    pub fn root_is_horizontal_split(&self) -> bool {
        matches!(self.as_layout(), LayoutNode::Split(s) if s.axis == Axis::Horizontal)
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

/// Pane-trait-driven version of the pane-path walker (T-91 phase 2
/// prep). Generic over the concrete `Pane` impl — any composite that
/// returns children via `Pane::children` plugs in.
fn walk_pane_paths_via_trait(
    node: &dyn Pane,
    area: Rect,
    here: devix_protocol::path::Path,
    out: &mut Vec<devix_protocol::path::Path>,
) {
    out.push(here.clone());
    for (idx, (rect, child)) in node.children(area).into_iter().enumerate() {
        if let Ok(child_path) = here.join(&idx.to_string()) {
            walk_pane_paths_via_trait(child, rect, child_path, out);
        }
    }
}
