//! Focus traversal: directional moves across the layout tree, plus the
//! geometry-aware "step into the closest sibling" picker that makes
//! Ctrl+Alt+Arrow feel right when leaves are unevenly sized.
//!
//! Phase 3c: walks the structural `Surface.root` Pane tree via
//! `core::walk::pane_at_path` + downcasts, instead of matching on the
//! legacy `Node` enum. Geometry still comes from the cached frame /
//! sidebar rects — the cache is the only thing that knows how splits
//! were laid out at the last render.

use devix_core::{Pane, pane_at_path};
use devix_core::Rect;

use crate::frame::FrameId;
use crate::layout::{Axis, Direction, SidebarSlot};
use crate::tree::{LayoutFrame, LayoutSidebar, LayoutSplit};

use super::{LeafRef, RenderCache, Surface};

impl Surface {
    pub fn focus_dir(&mut self, dir: Direction) {
        let area = root_area(&self.render_cache);
        if let Some(target_path) =
            compute_focus_target(self.root.as_ref(), area, &self.focus, dir, &self.render_cache)
        {
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
            if let Some(path) = find_sidebar(self.root.as_ref(), area, slot) {
                self.focus = path;
            }
        }
    }

    /// Move focus to `frame`'s leaf, if it exists in the layout tree. Returns
    /// true on success.
    pub fn focus_frame(&mut self, frame: FrameId) -> bool {
        let area = root_area(&self.render_cache);
        if let Some(path) = path_to_leaf(self.root.as_ref(), area, LeafRef::Frame(frame)) {
            self.focus = path;
            true
        } else {
            false
        }
    }
}

/// Total area covered by the cached leaves. The render cache is populated
/// after every paint, so this is exact for the geometry the user sees.
fn root_area(cache: &RenderCache) -> Rect {
    let rects: Vec<Rect> = cache
        .frame_rects
        .values()
        .copied()
        .chain(cache.sidebar_rects.values().copied())
        .chain(cache.tab_strips.values().map(|s| s.strip_rect))
        .collect();
    if rects.is_empty() {
        return Rect::default();
    }
    let x = rects.iter().map(|r| r.x).min().unwrap();
    let y = rects.iter().map(|r| r.y).min().unwrap();
    let x_end = rects.iter().map(|r| r.x + r.width).max().unwrap();
    let y_end = rects.iter().map(|r| r.y + r.height).max().unwrap();
    Rect { x, y, width: x_end - x, height: y_end - y }
}

fn compute_focus_target(
    root: &dyn Pane,
    area: Rect,
    focus: &[usize],
    dir: Direction,
    cache: &RenderCache,
) -> Option<Vec<usize>> {
    let needed_axis = match dir {
        Direction::Left | Direction::Right => Axis::Horizontal,
        Direction::Up | Direction::Down => Axis::Vertical,
    };
    let step: isize = match dir {
        Direction::Left | Direction::Up => -1,
        Direction::Right | Direction::Down => 1,
    };

    // Walk up from the leaf, looking for a Split on the needed axis where we
    // can step in `step` direction.
    let mut path = focus.to_vec();
    while !path.is_empty() {
        let parent_path: Vec<usize> = path[..path.len() - 1].to_vec();
        let child_idx = *path.last().unwrap();
        let (_, parent_pane) = pane_at_path(root, area, &parent_path)?;
        if let Some(split) = parent_pane.as_any().and_then(|a| a.downcast_ref::<LayoutSplit>()) {
            if split.axis == needed_axis {
                let next = child_idx as isize + step;
                if next >= 0 && (next as usize) < split.children.len() {
                    let mut new_path = parent_path;
                    new_path.push(next as usize);
                    return Some(walk_into(root, area, new_path, dir, focus, cache));
                }
            }
        }
        path.pop();
    }
    None
}

fn walk_into(
    root: &dyn Pane,
    root_area: Rect,
    mut path: Vec<usize>,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> Vec<usize> {
    loop {
        let Some((_, pane)) = pane_at_path(root, root_area, &path) else {
            return path;
        };
        let Some(split) = pane.as_any().and_then(|a| a.downcast_ref::<LayoutSplit>()) else {
            return path;
        };
        let pick = pick_closest_child(
            root,
            root_area,
            &path,
            split.axis,
            split.children.len(),
            dir,
            source_path,
            cache,
        );
        path.push(pick);
    }
}

#[allow(clippy::too_many_arguments)]
fn pick_closest_child(
    root: &dyn Pane,
    root_area: Rect,
    parent_path: &[usize],
    axis: Axis,
    n_children: usize,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> usize {
    if n_children == 0 {
        return 0;
    }
    let source_rect = leaf_rect_for(root, root_area, source_path, cache);
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
    let mut best = 0usize;
    let mut best_d = i32::MAX;
    for i in 0..n_children {
        let mut child_path = parent_path.to_vec();
        child_path.push(i);
        let Some(rect) = first_leaf_rect(root, root_area, &child_path, cache) else { continue };
        let d = match axis {
            Axis::Horizontal => (rect.y as i32 + rect.height as i32 / 2 - centre_y as i32).abs(),
            Axis::Vertical => (rect.x as i32 + rect.width as i32 / 2 - centre_x as i32).abs(),
        };
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

fn leaf_rect_for(
    root: &dyn Pane,
    root_area: Rect,
    path: &[usize],
    cache: &RenderCache,
) -> Option<Rect> {
    let (_, pane) = pane_at_path(root, root_area, path)?;
    pane_to_leaf_ref(pane).and_then(|leaf| leaf_rect(leaf, cache))
}

fn first_leaf_rect(
    root: &dyn Pane,
    root_area: Rect,
    path: &[usize],
    cache: &RenderCache,
) -> Option<Rect> {
    // Walk into the deepest leaf along the leftmost child at each split.
    let mut p = path.to_vec();
    loop {
        let (_, pane) = pane_at_path(root, root_area, &p)?;
        if pane
            .as_any()
            .map(|a| a.downcast_ref::<LayoutSplit>().is_some())
            .unwrap_or(false)
        {
            p.push(0);
            continue;
        }
        return leaf_rect_for(root, root_area, &p, cache);
    }
}

/// Recover a `LeafRef` from a structural Pane leaf via `as_any` downcast.
fn pane_to_leaf_ref(pane: &dyn Pane) -> Option<LeafRef> {
    let any = pane.as_any()?;
    if let Some(f) = any.downcast_ref::<LayoutFrame>() {
        return Some(LeafRef::Frame(f.frame));
    }
    if let Some(s) = any.downcast_ref::<LayoutSidebar>() {
        return Some(LeafRef::Sidebar(s.slot));
    }
    None
}

fn leaf_rect(leaf: LeafRef, cache: &RenderCache) -> Option<Rect> {
    match leaf {
        LeafRef::Frame(id) => cache.frame_rects.get(&id).copied(),
        LeafRef::Sidebar(slot) => cache.sidebar_rects.get(&slot).copied(),
    }
}

fn find_sidebar(root: &dyn Pane, area: Rect, slot: SidebarSlot) -> Option<Vec<usize>> {
    fn go(pane: &dyn Pane, area: Rect, slot: SidebarSlot, out: &mut Vec<usize>) -> bool {
        if let Some(s) = pane.as_any().and_then(|a| a.downcast_ref::<LayoutSidebar>()) {
            if s.slot == slot {
                return true;
            }
        }
        for (i, (child_rect, child)) in pane.children(area).into_iter().enumerate() {
            out.push(i);
            if go(child, child_rect, slot, out) {
                return true;
            }
            out.pop();
        }
        false
    }
    let mut p = Vec::new();
    if go(root, area, slot, &mut p) { Some(p) } else { None }
}

/// Find the path to a `LeafRef` by walking the structural Pane tree.
/// Returns the sequence of `children()` indices that lead to the target.
pub(super) fn path_to_leaf(root: &dyn Pane, area: Rect, target: LeafRef) -> Option<Vec<usize>> {
    fn go(pane: &dyn Pane, area: Rect, target: LeafRef, out: &mut Vec<usize>) -> bool {
        if let Some(leaf) = pane_to_leaf_ref(pane) {
            if leaf == target {
                return true;
            }
        }
        for (i, (child_rect, child)) in pane.children(area).into_iter().enumerate() {
            out.push(i);
            if go(child, child_rect, target, out) {
                return true;
            }
            out.pop();
        }
        false
    }
    let mut p = Vec::new();
    if go(root, area, target, &mut p) { Some(p) } else { None }
}
