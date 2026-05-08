//! Pane registry — owner of the structural layout tree.
//!
//! Carved out of the god-`Editor` per T-100. The Editor holds a
//! `PaneRegistry` instead of a bare layout tree; every walk / lookup /
//! mutation of the layout tree goes through the registry's API.
//!
//! Post-T-91 the structural skeleton is a `Box<dyn Pane>` rooted tree.
//! Splits, frames, and sidebars each implement `Pane` directly; there
//! is no closed `LayoutNode` enum. Walks delegate to `Pane::children`
//! / `children_mut` and downcast to the concrete struct
//! (`LayoutFrame`, `LayoutSidebar`, `LayoutSplit`) when typed access is
//! required.

use ratatui::Frame;

use crate::editor::frame::FrameId;
use crate::editor::tree::{LayoutCtx, LayoutFrame, LayoutSidebar, LayoutSplit, mutate};
use crate::editor::LeafRef;
use crate::pane::Pane;
use crate::{Axis, Rect, SidebarSlot};

pub struct PaneRegistry {
    /// `Box<dyn Pane>` per T-91 acceptance criterion. Concrete root
    /// types are the structural Pane impls (`LayoutFrame`,
    /// `LayoutSidebar`, `LayoutSplit`). Walks go through the trait;
    /// typed access uses `Pane::as_any` / `as_any_mut`.
    root: Box<dyn Pane>,
}

impl PaneRegistry {
    pub fn new(root: Box<dyn Pane>) -> Self {
        Self { root }
    }

    /// Read-only access to the root pane. Typed access (e.g.
    /// pattern-matching for tests) goes through `Pane::as_any` and
    /// downcasts to `LayoutFrame` / `LayoutSidebar` / `LayoutSplit`.
    pub fn root(&self) -> &dyn Pane {
        self.root.as_ref()
    }

    pub fn find_frame(&self, fid: FrameId) -> Option<&LayoutFrame> {
        pane_find_frame(self.root.as_ref(), fid)
    }

    pub fn find_frame_mut(&mut self, fid: FrameId) -> Option<&mut LayoutFrame> {
        pane_find_frame_mut(self.root.as_mut(), fid)
    }

    pub fn find_sidebar_mut(&mut self, slot: SidebarSlot) -> Option<&mut LayoutSidebar> {
        pane_find_sidebar_mut(self.root.as_mut(), slot)
    }

    pub fn sidebar_present(&self, slot: SidebarSlot) -> bool {
        pane_sidebar_present(self.root.as_ref(), slot)
    }

    pub fn frames(&self) -> Vec<FrameId> {
        let mut out = Vec::new();
        pane_collect_frames(self.root.as_ref(), &mut out);
        out
    }

    pub fn at_path(&self, path: &[usize]) -> Option<&dyn Pane> {
        pane_at_path(self.root.as_ref(), path)
    }

    pub fn at_path_mut(&mut self, path: &[usize]) -> Option<&mut dyn Pane> {
        pane_at_path_mut(self.root.as_mut(), path)
    }

    pub fn at_path_with_rect(&self, area: Rect, path: &[usize]) -> Option<(Rect, &dyn Pane)> {
        pane_at_path_with_rect(self.root.as_ref(), area, path)
    }

    pub fn leaves_with_rects(&self, area: Rect) -> Vec<(LeafRef, Rect)> {
        let mut out = Vec::new();
        pane_collect_leaves(self.root.as_ref(), area, &mut out);
        out
    }

    pub fn path_to_leaf(&self, target: LeafRef) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        if pane_path_to_leaf(self.root.as_ref(), target, &mut path) {
            Some(path)
        } else {
            None
        }
    }

    pub fn pane_at_xy(&self, area: Rect, col: u16, row: u16) -> Option<(Rect, &dyn Pane)> {
        pane_hit_test(self.root.as_ref(), area, col, row)
    }

    pub fn render(&self, area: Rect, frame: &mut Frame<'_>, ctx: &LayoutCtx<'_>) {
        let mut rctx = crate::pane::RenderCtx {
            frame,
            layout: Some(ctx),
        };
        self.root.render(area, &mut rctx);
    }

    /// Resolve a `/pane(/<i>)*` path to the corresponding `&dyn Pane`.
    /// Path segments after `pane` are 0-based indices into
    /// `Pane::children`. Per `docs/specs/namespace.md` § *Migration
    /// table* and T-52.
    pub fn pane_at(&self, path: &devix_protocol::path::Path) -> Option<&dyn Pane> {
        let indices = pane_path_indices(path)?;
        pane_at_path(self.root.as_ref(), &indices)
    }

    pub fn pane_at_mut(
        &mut self,
        path: &devix_protocol::path::Path,
    ) -> Option<&mut dyn Pane> {
        let indices = pane_path_indices(path)?;
        pane_at_path_mut(self.root.as_mut(), &indices)
    }

    /// Pre-order enumeration of every reachable pane path. `/pane` is
    /// the root; each composite pane (today: `LayoutSplit`; tomorrow:
    /// any `Pane` with non-empty `children`) adds child indices. Walks
    /// via `Pane::children` so the helper stays generic over the pane
    /// kind.
    pub fn pane_paths(&self) -> Vec<devix_protocol::path::Path> {
        let mut out = Vec::new();
        let root_path =
            devix_protocol::path::Path::parse("/pane").expect("/pane is canonical");
        // The pane-path encoding doesn't depend on rect math; pass a
        // zero rect — the walk only consults the children indices.
        let zero = Rect::default();
        walk_pane_paths_via_trait(self.root.as_ref(), zero, root_path, &mut out);
        out
    }

    /// Replace the node at `path`. Empty path replaces the root.
    pub fn replace_at(&mut self, path: &[usize], new: Box<dyn Pane>) -> bool {
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

    /// Lift the root into a horizontal split so a sidebar can be
    /// inserted alongside it.
    pub fn lift_into_horizontal_split(&mut self) {
        mutate::lift_into_horizontal_split(&mut self.root);
    }

    /// Mutable access to the root split for the (currently in-tree) op
    /// patterns that destructure the root after
    /// `lift_into_horizontal_split`. Crate-internal so external callers
    /// stay on the typed API.
    pub(crate) fn root_split_mut(&mut self) -> Option<&mut LayoutSplit> {
        self.root.as_any_mut()?.downcast_mut::<LayoutSplit>()
    }

    /// Whether the root is a horizontal split. Used by
    /// `toggle_sidebar` to decide whether to lift first.
    pub fn root_is_horizontal_split(&self) -> bool {
        self.root
            .as_any()
            .and_then(|a| a.downcast_ref::<LayoutSplit>())
            .is_some_and(|s| s.axis == Axis::Horizontal)
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

/// Pane-trait-driven walker for `/pane(/<i>)*`. Generic over the
/// concrete `Pane` impl — any composite that returns children via
/// `Pane::children` plugs in.
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

fn pane_find_frame<'a>(node: &'a dyn Pane, fid: FrameId) -> Option<&'a LayoutFrame> {
    if let Some(frame) = node.as_any().and_then(|a| a.downcast_ref::<LayoutFrame>()) {
        if frame.frame == fid {
            return Some(frame);
        }
    }
    let zero = Rect::default();
    for (_, child) in node.children(zero) {
        if let Some(found) = pane_find_frame(child, fid) {
            return Some(found);
        }
    }
    None
}

fn pane_find_frame_mut<'a>(
    node: &'a mut dyn Pane,
    fid: FrameId,
) -> Option<&'a mut LayoutFrame> {
    let direct_match = node
        .as_any()
        .and_then(|a| a.downcast_ref::<LayoutFrame>())
        .is_some_and(|f| f.frame == fid);
    if direct_match {
        return node.as_any_mut()?.downcast_mut::<LayoutFrame>();
    }
    for (_, child) in node.children_mut(Rect::default()) {
        if let Some(found) = pane_find_frame_mut(child, fid) {
            return Some(found);
        }
    }
    None
}

fn pane_find_sidebar_mut<'a>(
    node: &'a mut dyn Pane,
    slot: SidebarSlot,
) -> Option<&'a mut LayoutSidebar> {
    let direct_match = node
        .as_any()
        .and_then(|a| a.downcast_ref::<LayoutSidebar>())
        .is_some_and(|s| s.slot == slot);
    if direct_match {
        return node.as_any_mut()?.downcast_mut::<LayoutSidebar>();
    }
    for (_, child) in node.children_mut(Rect::default()) {
        if let Some(found) = pane_find_sidebar_mut(child, slot) {
            return Some(found);
        }
    }
    None
}

fn pane_sidebar_present(node: &dyn Pane, slot: SidebarSlot) -> bool {
    if let Some(sb) = node.as_any().and_then(|a| a.downcast_ref::<LayoutSidebar>()) {
        if sb.slot == slot {
            return true;
        }
    }
    let zero = Rect::default();
    node.children(zero)
        .into_iter()
        .any(|(_, child)| pane_sidebar_present(child, slot))
}

/// Walk `path` (a list of `Pane::children` indices) from `node`,
/// returning the resolved Pane.
fn pane_at_path<'a>(node: &'a dyn Pane, path: &[usize]) -> Option<&'a dyn Pane> {
    let mut cur: &dyn Pane = node;
    let zero = Rect::default();
    for &idx in path {
        let kids = cur.children(zero);
        let (_, child) = kids.into_iter().nth(idx)?;
        cur = child;
    }
    Some(cur)
}

fn pane_at_path_mut<'a>(
    node: &'a mut dyn Pane,
    path: &[usize],
) -> Option<&'a mut dyn Pane> {
    let mut cur: &mut dyn Pane = node;
    let zero = Rect::default();
    for &idx in path {
        let kids = cur.children_mut(zero);
        let (_, child) = kids.into_iter().nth(idx)?;
        cur = child;
    }
    Some(cur)
}

fn pane_at_path_with_rect<'a>(
    node: &'a dyn Pane,
    area: Rect,
    path: &[usize],
) -> Option<(Rect, &'a dyn Pane)> {
    let mut cur: &dyn Pane = node;
    let mut cur_area = area;
    for &idx in path {
        let kids = cur.children(cur_area);
        let (rect, child) = kids.into_iter().nth(idx)?;
        cur = child;
        cur_area = rect;
    }
    Some((cur_area, cur))
}

fn pane_hit_test<'a>(
    node: &'a dyn Pane,
    area: Rect,
    col: u16,
    row: u16,
) -> Option<(Rect, &'a dyn Pane)> {
    if !rect_contains(area, col, row) {
        return None;
    }
    let kids = node.children(area);
    for (child_rect, child) in kids.iter().rev() {
        if let Some(found) = pane_hit_test(*child, *child_rect, col, row) {
            return Some(found);
        }
    }
    Some((area, node))
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

/// Extract a `LeafRef` from `node` if it represents a layout leaf
/// (`LayoutFrame` or `LayoutSidebar`). Returns `None` for splits or
/// non-layout panes.
pub fn pane_leaf_id(node: &dyn Pane) -> Option<LeafRef> {
    let any = node.as_any()?;
    if let Some(f) = any.downcast_ref::<LayoutFrame>() {
        return Some(LeafRef::Frame(f.frame));
    }
    if let Some(s) = any.downcast_ref::<LayoutSidebar>() {
        return Some(LeafRef::Sidebar(s.slot));
    }
    None
}

fn pane_collect_leaves(node: &dyn Pane, area: Rect, out: &mut Vec<(LeafRef, Rect)>) {
    if let Some(id) = pane_leaf_id(node) {
        out.push((id, area));
        return;
    }
    for (rect, child) in node.children(area) {
        pane_collect_leaves(child, rect, out);
    }
}

fn pane_path_to_leaf(node: &dyn Pane, target: LeafRef, out: &mut Vec<usize>) -> bool {
    if pane_leaf_id(node) == Some(target) {
        return true;
    }
    let zero = Rect::default();
    for (idx, (_, child)) in node.children(zero).into_iter().enumerate() {
        out.push(idx);
        if pane_path_to_leaf(child, target, out) {
            return true;
        }
        out.pop();
    }
    false
}

fn pane_collect_frames(node: &dyn Pane, out: &mut Vec<FrameId>) {
    if let Some(frame) = node.as_any().and_then(|a| a.downcast_ref::<LayoutFrame>()) {
        out.push(frame.frame);
    }
    let zero = Rect::default();
    for (_, child) in node.children(zero) {
        pane_collect_frames(child, out);
    }
}
