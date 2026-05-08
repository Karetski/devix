//! Focus traversal: directional moves across the layout tree, plus the
//! geometry-aware "step into the closest sibling" picker that makes
//! Ctrl+Alt+Arrow feel right when leaves are unevenly sized.
//!
//! Walks `Editor.panes` through the `Pane` trait — `Pane::children`
//! for structural recursion, downcasts to `LayoutSplit` for axis /
//! child-count access. Geometry still comes from the cached frame /
//! sidebar rects, since the cache is the only thing that knows how
//! splits laid out at the last render.

use crate::Pane;
use crate::Rect;

use crate::editor::frame::FrameId;
use crate::editor::registry::{PaneRegistry, pane_leaf_id};
use crate::editor::tree::LayoutSplit;
use crate::{Axis, Direction, SidebarSlot};

use super::{Editor, LeafRef, RenderCache};

impl Editor {
    pub fn focus_dir(&mut self, dir: Direction, cache: &RenderCache) {
        let area = root_area(cache);
        if let Some(target_path) = compute_focus_target(
            &self.panes,
            area,
            self.focus.active(),
            dir,
            cache,
        ) {
            self.set_focus(target_path);
            return;
        }
        // Edge: try to move into a sidebar.
        let needed: Option<SidebarSlot> = match dir {
            Direction::Left => Some(SidebarSlot::Left),
            Direction::Right => Some(SidebarSlot::Right),
            _ => None,
        };
        if let Some(slot) = needed {
            if let Some(path) = self.panes.path_to_leaf(LeafRef::Sidebar(slot)) {
                self.set_focus(path);
            }
        }
    }

    /// Move focus to `frame`'s leaf, if it exists in the layout tree.
    pub fn focus_frame(&mut self, frame: FrameId) -> bool {
        if let Some(path) = self.panes.path_to_leaf(LeafRef::Frame(frame)) {
            self.set_focus(path);
            true
        } else {
            false
        }
    }
}

/// Total area covered by cached leaves. Populated after every paint, so
/// this is exact for the geometry the user sees.
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

fn split_axis_and_children(node: &dyn Pane) -> Option<(Axis, usize)> {
    let split = node.as_any().and_then(|a| a.downcast_ref::<LayoutSplit>())?;
    Some((split.axis, split.children.len()))
}

fn compute_focus_target(
    panes: &PaneRegistry,
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

    // Walk up from the leaf, looking for a Split on the needed axis where
    // we can step in `step` direction.
    let mut path = focus.to_vec();
    while !path.is_empty() {
        let parent_path: Vec<usize> = path[..path.len() - 1].to_vec();
        let child_idx = *path.last().unwrap();
        let parent = panes.at_path(&parent_path)?;
        if let Some((axis, n_children)) = split_axis_and_children(parent) {
            if axis == needed_axis {
                let next = child_idx as isize + step;
                if next >= 0 && (next as usize) < n_children {
                    let mut new_path = parent_path;
                    new_path.push(next as usize);
                    return Some(walk_into(panes, area, new_path, dir, focus, cache));
                }
            }
        }
        path.pop();
    }
    None
}

fn walk_into(
    panes: &PaneRegistry,
    root_area: Rect,
    mut path: Vec<usize>,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> Vec<usize> {
    loop {
        let Some(node) = panes.at_path(&path) else {
            return path;
        };
        let Some((axis, n_children)) = split_axis_and_children(node) else {
            return path;
        };
        let pick = pick_closest_child(
            panes,
            root_area,
            &path,
            axis,
            n_children,
            dir,
            source_path,
            cache,
        );
        path.push(pick);
    }
}

#[allow(clippy::too_many_arguments)]
fn pick_closest_child(
    panes: &PaneRegistry,
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
    let source_rect = leaf_rect_for(panes, source_path, cache);
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
        let Some(rect) = first_leaf_rect(panes, root_area, &child_path, cache) else { continue };
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

fn leaf_rect_for(panes: &PaneRegistry, path: &[usize], cache: &RenderCache) -> Option<Rect> {
    let leaf = pane_leaf_id(panes.at_path(path)?)?;
    leaf_rect(leaf, cache)
}

fn first_leaf_rect(
    panes: &PaneRegistry,
    _root_area: Rect,
    path: &[usize],
    cache: &RenderCache,
) -> Option<Rect> {
    // Walk into the deepest leaf along the leftmost child at each split.
    let mut p = path.to_vec();
    loop {
        let node = panes.at_path(&p)?;
        if split_axis_and_children(node).is_some() {
            p.push(0);
            continue;
        }
        return leaf_rect_for(panes, &p, cache);
    }
}

fn leaf_rect(leaf: LeafRef, cache: &RenderCache) -> Option<Rect> {
    match leaf {
        LeafRef::Frame(id) => cache.frame_rects.get(&id).copied(),
        LeafRef::Sidebar(slot) => cache.sidebar_rects.get(&slot).copied(),
    }
}
