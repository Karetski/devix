//! Structural Pane tree — the layout source-of-truth.
//!
//! Phase 3c of the architecture refactor: replace the closed `Node` enum
//! with a tree of `Box<dyn Pane>` rooted at `Workspace.root`. The
//! structural Panes here are *long-lived* (owned by the workspace) and
//! `'static` (so they can opt into `Pane::as_any` for downcasting). They
//! hold IDs / slots, not borrowed editor state — render-time Panes
//! (`devix-views`'s `TabbedPane`, `SidebarSlotPane`) are still built per
//! frame with workspace borrows.
//!
//! Why two trees?
//!
//! - The structural tree is *the* layout. Hit-test, focus walk, and
//!   directional navigation all walk it via `core::walk::*`.
//! - The render tree is built per frame because `EditorPane` and friends
//!   borrow buffer/view/highlight data that's only valid while paint runs.
//!
//! Both trees agree on shape: every structural leaf corresponds to one
//! render leaf. The transition to a single tree happens once `View`
//! ownership migrates onto `TabbedPane` (Phase 3c follow-up) — at that
//! point `EditorPane` can own its data and the render tree disappears.

use devix_core::{Event, HandleCtx, Outcome, Pane, RenderCtx};
use ratatui::layout::{Constraint, Direction as RatDirection, Layout, Rect};
use std::any::Any;

use crate::frame::FrameId;
use crate::layout::{Axis, SidebarSlot};

/// Recursive split. Mirrors `Node::Split` semantics; `children()`
/// computes child rects via ratatui `Layout`, identical math to the
/// existing `Node::leaves_with_rects`.
pub struct LayoutSplit {
    pub axis: Axis,
    pub children: Vec<(Box<dyn Pane>, u16)>,
}

impl Pane for LayoutSplit {
    fn render(&self, _: Rect, _: &mut RenderCtx<'_, '_>) {
        // Structural — the render tree paints. SplitPane in `devix-views`
        // is the render-side equivalent.
    }

    fn handle(&mut self, _: &Event, _: Rect, _: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }

    fn children(&self, area: Rect) -> Vec<(Rect, &dyn Pane)> {
        if self.children.is_empty() {
            return Vec::new();
        }
        let total: u32 = self
            .children
            .iter()
            .map(|(_, w)| *w as u32)
            .sum::<u32>()
            .max(1);
        let constraints: Vec<Constraint> = self
            .children
            .iter()
            .map(|(_, w)| Constraint::Ratio(*w as u32, total))
            .collect();
        let dir = match self.axis {
            Axis::Horizontal => RatDirection::Horizontal,
            Axis::Vertical => RatDirection::Vertical,
        };
        let rects = Layout::default()
            .direction(dir)
            .constraints(constraints)
            .split(area);
        self.children
            .iter()
            .zip(rects.iter())
            .map(|((child, _), rect)| (*rect, child.as_ref()))
            .collect()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Editor-frame leaf. Owns the per-frame state directly — tabs, active
/// index, tab-strip scroll, and the one-shot recenter flag — so each
/// "frame" in the layout tree is its own self-contained Pane. Phase 3c
/// follow-up: replaced the indirection through `Workspace.frames:
/// SlotMap<FrameId, Frame>` with direct ownership.
///
/// `frame: FrameId` stays as a stable identifier the render cache
/// (`render_cache.frame_rects`, `tab_strips`) keys against. New frames
/// are minted via `crate::frame::mint_id()`.
pub struct LayoutFrame {
    pub frame: FrameId,
    pub tabs: Vec<crate::view::ViewId>,
    pub active_tab: usize,
    /// Scroll offset for this frame's tab strip, in cells.
    pub tab_strip_scroll: (u32, u32),
    /// One-shot signal asking the next tab-strip render to scroll the
    /// active tab into view. Set by mutators that change `active_tab`
    /// (keyboard nav, new tab, close), cleared by the renderer.
    pub recenter_active: bool,
}

impl LayoutFrame {
    pub fn with_view(frame: FrameId, view: crate::view::ViewId) -> Self {
        Self {
            frame,
            tabs: vec![view],
            active_tab: 0,
            tab_strip_scroll: (0, 0),
            recenter_active: true,
        }
    }

    /// Activate a tab and request scroll-to-visible. Use for keyboard
    /// nav and tab-mutating operations (new/close).
    pub fn set_active(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
        }
        self.recenter_active = true;
    }

    /// Activate a tab without disturbing scroll. Use for click activation
    /// — the user already pointed at the tab they want.
    pub fn select_visible(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
        }
    }

    /// Returns `None` if `tabs` is empty or `active_tab` is out of bounds.
    pub fn active_view(&self) -> Option<crate::view::ViewId> {
        self.tabs.get(self.active_tab).copied()
    }
}

impl Pane for LayoutFrame {
    fn render(&self, _: Rect, _: &mut RenderCtx<'_, '_>) {}

    fn handle(&mut self, _: &Event, _: Rect, _: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Sidebar-slot leaf. The slot enum (`Left` / `Right`) acts as the
/// identity; one of each can exist in the tree.
pub struct LayoutSidebar {
    pub slot: SidebarSlot,
}

impl Pane for LayoutSidebar {
    fn render(&self, _: Rect, _: &mut RenderCtx<'_, '_>) {}

    fn handle(&mut self, _: &Event, _: Rect, _: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Walk the structural tree by `children()` *index* — no area needed,
/// because we navigate through `LayoutSplit.children` directly. Use
/// this for path-only navigation (e.g. resolving `Workspace.focus` to
/// its leaf); use `core::walk::pane_at_path` when you also need the
/// rect each child occupies.
pub fn pane_at_indices<'a>(root: &'a dyn Pane, path: &[usize]) -> Option<&'a dyn Pane> {
    let mut cur = root;
    for &idx in path {
        let split = cur.as_any()?.downcast_ref::<LayoutSplit>()?;
        let (child, _) = split.children.get(idx)?;
        cur = child.as_ref();
    }
    Some(cur)
}

/// Recover a leaf identity (`LeafRef`) from a structural Pane leaf.
/// Returns `None` for splits or unrecognized leaf types.
pub fn pane_leaf_id(pane: &dyn Pane) -> Option<LeafRef> {
    let any = pane.as_any()?;
    if let Some(f) = any.downcast_ref::<LayoutFrame>() {
        return Some(LeafRef::Frame(f.frame));
    }
    if let Some(s) = any.downcast_ref::<LayoutSidebar>() {
        return Some(LeafRef::Sidebar(s.slot));
    }
    None
}

/// Identity of a leaf in the structural tree. `LeafId` was an internal
/// alias kept while the legacy `LeafRef` lived in `workspace.rs`; now
/// it's just a re-export so callers can `use tree::LeafRef` without
/// going through the parent module.
pub use crate::workspace::LeafRef;
pub type LeafId = LeafRef;

/// Find the `LayoutFrame` for `frame` by walking the tree. Returns
/// `None` if no leaf with that `FrameId` exists. The structural tree
/// is the only place per-frame state lives now, so all `Workspace`
/// accessors that used to do `ws.frames[fid]` go through this.
pub fn find_frame(root: &dyn Pane, frame: FrameId) -> Option<&LayoutFrame> {
    if let Some(f) = root.as_any().and_then(|a| a.downcast_ref::<LayoutFrame>()) {
        if f.frame == frame { return Some(f); }
    }
    if let Some(split) = root.as_any().and_then(|a| a.downcast_ref::<LayoutSplit>()) {
        for (child, _) in &split.children {
            if let Some(found) = find_frame(child.as_ref(), frame) {
                return Some(found);
            }
        }
    }
    None
}

/// Mutable counterpart of [`find_frame`]. Walks the tree and returns
/// `&mut LayoutFrame` for the matching id.
pub fn find_frame_mut(root: &mut Box<dyn Pane>, frame: FrameId) -> Option<&mut LayoutFrame> {
    // Match-or-recurse, but with mutable borrows the borrow checker
    // demands a careful structure: try the root downcast first as an
    // owned check, then descend via split children.
    let is_target = root
        .as_any()
        .and_then(|a| a.downcast_ref::<LayoutFrame>())
        .map(|f| f.frame == frame)
        .unwrap_or(false);
    if is_target {
        return root
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<LayoutFrame>());
    }
    let split = root.as_any_mut().and_then(|a| a.downcast_mut::<LayoutSplit>())?;
    for (child, _) in &mut split.children {
        if let Some(found) = find_frame_mut(child, frame) {
            return Some(found);
        }
    }
    None
}

/// Iterate every `FrameId` in the tree in left-to-right order. Used by
/// the LSP and watcher modules to fan out per-frame queries without
/// having to hold a borrow across the walk.
pub fn frame_ids(root: &dyn Pane) -> Vec<FrameId> {
    let mut out = Vec::new();
    collect_frames(root, &mut out);
    out
}

fn collect_frames(pane: &dyn Pane, out: &mut Vec<FrameId>) {
    if let Some(f) = pane.as_any().and_then(|a| a.downcast_ref::<LayoutFrame>()) {
        out.push(f.frame);
        return;
    }
    if let Some(split) = pane.as_any().and_then(|a| a.downcast_ref::<LayoutSplit>()) {
        for (child, _) in &split.children {
            collect_frames(child.as_ref(), out);
        }
    }
}

/// Walk the structural tree and collect every leaf with the rect it
/// occupies inside `area`. Sibling order matches the order in which
/// `children()` returns them. Replaces the old `Node::leaves_with_rects`.
pub fn leaves_with_rects(root: &dyn Pane, area: Rect) -> Vec<(LeafRef, Rect)> {
    let mut out = Vec::new();
    walk(root, area, &mut out);
    out
}

fn walk(pane: &dyn Pane, area: Rect, out: &mut Vec<(LeafRef, Rect)>) {
    if let Some(id) = pane_leaf_id(pane) {
        out.push((id, area));
        return;
    }
    for (child_rect, child) in pane.children(area) {
        walk(child, child_rect, out);
    }
}

/// Tree-mutation helpers — the write-side counterpart to
/// `pane_at_indices`. They take `&mut Box<dyn Pane>` rooted at
/// `Workspace.root` and rewrite it in place.
///
/// All four ops the editor needs (replace a leaf with a subtree, remove
/// the leaf at a path, lift the root into a Split when toggling the
/// first sidebar, collapse a Split that's been reduced to a single
/// child) live here. Workspace `ops` calls them; the structural tree is
/// the source of truth.
pub mod mutate {
    use super::{LayoutSplit, Pane};

    /// Replace the Pane at `path` with `new`. Empty path replaces the
    /// root itself. Returns `true` on success; `false` if any index in
    /// the path is out of range or hits a non-`LayoutSplit` mid-walk.
    pub fn replace_at(root: &mut Box<dyn Pane>, path: &[usize], new: Box<dyn Pane>) -> bool {
        if path.is_empty() {
            *root = new;
            return true;
        }
        let mut cur: &mut Box<dyn Pane> = root;
        for (i, &idx) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            let split = match cur.as_any_mut().and_then(|a| a.downcast_mut::<LayoutSplit>()) {
                Some(s) => s,
                None => return false,
            };
            if idx >= split.children.len() {
                return false;
            }
            if last {
                split.children[idx].0 = new;
                return true;
            }
            cur = &mut split.children[idx].0;
        }
        false
    }

    /// Remove the child at `path` from its parent split. The path must
    /// have at least one element (you can't remove the root itself
    /// through this helper). Returns `true` on success.
    pub fn remove_at(root: &mut Box<dyn Pane>, path: &[usize]) -> bool {
        if path.is_empty() {
            return false;
        }
        let (parent_path, last) = path.split_at(path.len() - 1);
        let leaf_idx = last[0];
        let mut cur: &mut Box<dyn Pane> = root;
        for &idx in parent_path {
            let split = match cur.as_any_mut().and_then(|a| a.downcast_mut::<LayoutSplit>()) {
                Some(s) => s,
                None => return false,
            };
            if idx >= split.children.len() {
                return false;
            }
            cur = &mut split.children[idx].0;
        }
        let split = match cur.as_any_mut().and_then(|a| a.downcast_mut::<LayoutSplit>()) {
            Some(s) => s,
            None => return false,
        };
        if leaf_idx >= split.children.len() {
            return false;
        }
        split.children.remove(leaf_idx);
        true
    }

    /// Recursively replace any `LayoutSplit` with one child by that
    /// child. Mirrors `Node::collapse_singleton_splits` for the
    /// structural Pane tree.
    pub fn collapse_singletons(root: &mut Box<dyn Pane>) {
        // Walk children first (post-order) so a chain of single-child
        // splits collapses fully in one pass. Limited recursion: TUI
        // trees are dozens of nodes, not thousands.
        if let Some(split) = root.as_any_mut().and_then(|a| a.downcast_mut::<LayoutSplit>()) {
            for (child, _) in split.children.iter_mut() {
                collapse_singletons(child);
            }
            if split.children.len() == 1 {
                let (only, _) = split.children.remove(0);
                *root = only;
            }
        }
    }

    /// Replace the root with a horizontal `LayoutSplit` containing it
    /// as the only child (weighted 80). Used by `toggle_sidebar` when
    /// the first sidebar opens against a non-split root.
    pub fn lift_into_horizontal_split(root: &mut Box<dyn Pane>) {
        // Take the current root by replacing it with a placeholder
        // empty split, then move the original into the new split's
        // first child.
        let placeholder: Box<dyn Pane> = Box::new(LayoutSplit {
            axis: super::Axis::Horizontal,
            children: Vec::new(),
        });
        let inner = std::mem::replace(root, placeholder);
        let split = root
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<LayoutSplit>())
            .expect("just installed a LayoutSplit");
        split.children.push((inner, 80));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devix_core::{focusable_leaves, pane_at};
    fn full() -> Rect {
        Rect { x: 0, y: 0, width: 100, height: 50 }
    }

    fn fake_frame_id() -> FrameId {
        crate::frame::mint_id()
    }

    fn frame(fid: FrameId) -> Box<dyn Pane> {
        Box::new(LayoutFrame {
            frame: fid,
            tabs: Vec::new(),
            active_tab: 0,
            tab_strip_scroll: (0, 0),
            recenter_active: false,
        })
    }

    fn sidebar(slot: SidebarSlot) -> Box<dyn Pane> {
        Box::new(LayoutSidebar { slot })
    }

    fn split(axis: Axis, children: Vec<(Box<dyn Pane>, u16)>) -> Box<dyn Pane> {
        Box::new(LayoutSplit { axis, children })
    }

    #[test]
    fn frame_leaf_downcasts_to_frame_id() {
        let fid = fake_frame_id();
        let tree = frame(fid);
        let (rect, leaf) = pane_at(tree.as_ref(), full(), 50, 25).unwrap();
        assert_eq!(rect, full());
        let f = leaf.as_any().unwrap().downcast_ref::<LayoutFrame>().unwrap();
        assert_eq!(f.frame, fid);
    }

    #[test]
    fn split_distributes_children_by_weight() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let tree = split(Axis::Horizontal, vec![(frame(f1), 1), (frame(f2), 3)]);
        let (rect, leaf) = pane_at(tree.as_ref(), full(), 10, 25).unwrap();
        assert_eq!(rect.x, 0);
        assert_eq!(rect.width, 25);
        assert_eq!(leaf.as_any().unwrap().downcast_ref::<LayoutFrame>().unwrap().frame, f1);
        let (_, leaf) = pane_at(tree.as_ref(), full(), 80, 25).unwrap();
        assert_eq!(leaf.as_any().unwrap().downcast_ref::<LayoutFrame>().unwrap().frame, f2);
    }

    #[test]
    fn focusable_leaves_visits_every_frame() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let f3 = fake_frame_id();
        let inner = split(Axis::Vertical, vec![(frame(f2), 1), (frame(f3), 1)]);
        let outer = split(Axis::Horizontal, vec![(frame(f1), 1), (inner, 1)]);
        let leaves = focusable_leaves(outer.as_ref(), full());
        assert_eq!(leaves.len(), 3);
        let ids: Vec<FrameId> = leaves
            .iter()
            .map(|(_, _, p)| p.as_any().unwrap().downcast_ref::<LayoutFrame>().unwrap().frame)
            .collect();
        assert_eq!(ids, vec![f1, f2, f3]);
    }

    #[test]
    fn sidebar_leaf_downcasts_to_slot() {
        let tree = sidebar(SidebarSlot::Left);
        let (_, leaf) = pane_at(tree.as_ref(), full(), 10, 10).unwrap();
        let sb = leaf.as_any().unwrap().downcast_ref::<LayoutSidebar>().unwrap();
        assert_eq!(sb.slot, SidebarSlot::Left);
    }

    #[test]
    fn mutate_replace_at_root_swaps_root() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let mut tree = frame(f1);
        assert!(mutate::replace_at(&mut tree, &[], frame(f2)));
        let f = tree.as_any().unwrap().downcast_ref::<LayoutFrame>().unwrap();
        assert_eq!(f.frame, f2);
    }

    #[test]
    fn mutate_remove_at_drops_one_child_and_collapse_flattens() {
        let f1 = fake_frame_id();
        let f2 = fake_frame_id();
        let mut tree = split(Axis::Horizontal, vec![(frame(f1), 1), (frame(f2), 1)]);
        assert!(mutate::remove_at(&mut tree, &[1]));
        // Now a single-child Split.
        mutate::collapse_singletons(&mut tree);
        // After collapse the root is the surviving frame.
        let f = tree.as_any().unwrap().downcast_ref::<LayoutFrame>().unwrap();
        assert_eq!(f.frame, f1);
    }
}
