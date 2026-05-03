//! Focus traversal: directional moves across the layout tree, plus the
//! geometry-aware "step into the closest sibling" picker that makes
//! Ctrl+Alt+Arrow feel right when leaves are unevenly sized.

use ratatui::layout::Rect;

use crate::frame::FrameId;
use crate::layout::{Axis, Direction, Node, SidebarSlot};

use super::{LeafRef, RenderCache, Workspace};

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

fn leaf_rect_for(layout: &Node, path: &[usize], cache: &RenderCache) -> Option<Rect> {
    match layout.leaf_at(path)? {
        LeafRef::Frame(id) => cache.frame_rects.get(id).copied(),
        LeafRef::Sidebar(slot) => cache.sidebar_rects.get(&slot).copied(),
    }
}

fn first_leaf_rect(layout: &Node, path: &[usize], cache: &RenderCache) -> Option<Rect> {
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

pub(super) fn node_at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Node> {
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

pub(super) fn path_to_leaf(node: &Node, target: LeafRef) -> Option<Vec<usize>> {
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
