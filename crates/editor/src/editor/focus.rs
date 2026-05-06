//! Focus traversal: directional moves across the layout tree, plus the
//! geometry-aware "step into the closest sibling" picker that makes
//! Ctrl+Alt+Arrow feel right when leaves are unevenly sized.
//!
//! Walks `Editor.root` as a `LayoutNode` enum — every step is an
//! exhaustive match on Split / Frame / Sidebar; no `as_any` downcasts.
//! Geometry still comes from the cached frame / sidebar rects, since
//! the cache is the only thing that knows how splits laid out at the
//! last render.

use devix_panes::Rect;

use crate::frame::FrameId;
use devix_panes::{Axis, Direction, SidebarSlot};
use crate::tree::LayoutNode;

use super::{Editor, LeafRef, RenderCache};

impl Editor {
    pub fn focus_dir(&mut self, dir: Direction) {
        let area = root_area(&self.render_cache);
        if let Some(target_path) =
            compute_focus_target(&self.root, area, &self.focus, dir, &self.render_cache)
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
            if let Some(path) = self.root.path_to_leaf(LeafRef::Sidebar(slot)) {
                self.focus = path;
            }
        }
    }

    /// Move focus to `frame`'s leaf, if it exists in the layout tree.
    pub fn focus_frame(&mut self, frame: FrameId) -> bool {
        if let Some(path) = self.root.path_to_leaf(LeafRef::Frame(frame)) {
            self.focus = path;
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

fn compute_focus_target(
    root: &LayoutNode,
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
        let parent = root.at_path(&parent_path)?;
        if let LayoutNode::Split(split) = parent {
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
    root: &LayoutNode,
    root_area: Rect,
    mut path: Vec<usize>,
    dir: Direction,
    source_path: &[usize],
    cache: &RenderCache,
) -> Vec<usize> {
    loop {
        let Some(node) = root.at_path(&path) else {
            return path;
        };
        let LayoutNode::Split(split) = node else {
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
    root: &LayoutNode,
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
    let source_rect = leaf_rect_for(root, source_path, cache);
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

fn leaf_rect_for(root: &LayoutNode, path: &[usize], cache: &RenderCache) -> Option<Rect> {
    let leaf = root.at_path(path)?.leaf_id()?;
    leaf_rect(leaf, cache)
}

fn first_leaf_rect(
    root: &LayoutNode,
    root_area: Rect,
    path: &[usize],
    cache: &RenderCache,
) -> Option<Rect> {
    // Walk into the deepest leaf along the leftmost child at each split.
    let mut p = path.to_vec();
    loop {
        let node = root.at_path(&p)?;
        if matches!(node, LayoutNode::Split(_)) {
            p.push(0);
            continue;
        }
        let _ = root_area;
        return leaf_rect_for(root, &p, cache);
    }
}

fn leaf_rect(leaf: LeafRef, cache: &RenderCache) -> Option<Rect> {
    match leaf {
        LeafRef::Frame(id) => cache.frame_rects.get(&id).copied(),
        LeafRef::Sidebar(slot) => cache.sidebar_rects.get(&slot).copied(),
    }
}

/// Find the path to a `LeafRef` by walking the layout tree. Mirrors
/// `LayoutNode::path_to_leaf`; kept under `editor::focus` for the
/// pre-existing `path_to_leaf(root, area, target)` re-export the binary
/// imports.
pub fn path_to_leaf(root: &LayoutNode, _area: Rect, target: LeafRef) -> Option<Vec<usize>> {
    root.path_to_leaf(target)
}
