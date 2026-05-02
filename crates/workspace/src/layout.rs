//! Recursive layout tree.

use ratatui::layout::{Constraint, Direction as RatDirection, Layout, Rect};

use crate::frame::FrameId;
use crate::workspace::LeafRef;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis { Horizontal, Vertical }

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SidebarSlot { Left, Right }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction { Left, Down, Up, Right }

pub enum Node {
    Split { axis: Axis, children: Vec<(Node, u16)> },
    Frame(FrameId),
    Sidebar(SidebarSlot),
}

impl Node {
    /// Resolve a focus path (Vec<usize>) to its leaf reference.
    /// Returns None if the path is invalid.
    pub fn leaf_at(&self, path: &[usize]) -> Option<LeafRef> {
        let mut node = self;
        for &idx in path {
            match node {
                Node::Split { children, .. } => {
                    let (child, _) = children.get(idx)?;
                    node = child;
                }
                _ => return None,
            }
        }
        match node {
            Node::Frame(id) => Some(LeafRef::Frame(*id)),
            Node::Sidebar(slot) => Some(LeafRef::Sidebar(*slot)),
            Node::Split { .. } => None,
        }
    }

    /// Walk the tree, producing `(LeafRef, Rect)` for every leaf in z-order.
    pub fn leaves_with_rects(&self, area: Rect) -> Vec<(LeafRef, Rect)> {
        let mut out = Vec::new();
        Self::walk(self, area, &mut out);
        out
    }

    fn walk(node: &Node, area: Rect, out: &mut Vec<(LeafRef, Rect)>) {
        match node {
            Node::Frame(id) => out.push((LeafRef::Frame(*id), area)),
            Node::Sidebar(slot) => out.push((LeafRef::Sidebar(*slot), area)),
            Node::Split { axis, children } => {
                if children.is_empty() { return; }
                let total: u32 = children.iter().map(|(_, w)| *w as u32).sum();
                let constraints: Vec<Constraint> = children
                    .iter()
                    .map(|(_, w)| Constraint::Ratio(*w as u32, total.max(1)))
                    .collect();
                let dir = match axis {
                    Axis::Horizontal => RatDirection::Horizontal,
                    Axis::Vertical => RatDirection::Vertical,
                };
                let rects = Layout::default()
                    .direction(dir)
                    .constraints(constraints)
                    .split(area);
                for ((child, _), rect) in children.iter().zip(rects.iter()) {
                    Self::walk(child, *rect, out);
                }
            }
        }
    }

    /// Walk to the leaf at `path` and replace it with `new`. Returns true if
    /// the path resolved to a leaf and the replacement happened.
    pub fn replace_leaf_at(&mut self, path: &[usize], new: Node) -> bool {
        if path.is_empty() {
            *self = new;
            return true;
        }
        let mut node = self;
        for (i, &idx) in path.iter().enumerate() {
            match node {
                Node::Split { children, .. } => {
                    if idx >= children.len() { return false; }
                    if i + 1 == path.len() {
                        children[idx].0 = new;
                        return true;
                    }
                    node = &mut children[idx].0;
                }
                _ => return false,
            }
        }
        false
    }

    /// Take the leaf at `path` out, leaving a placeholder Frame with a null FrameId.
    /// Caller is expected to replace the placeholder before next use.
    /// Returns the original child if the path was valid.
    pub fn take_leaf_at(&mut self, path: &[usize]) -> Option<Node> {
        use slotmap::Key;
        if path.is_empty() { return None; }
        let mut node = self;
        for (i, &idx) in path.iter().enumerate() {
            match node {
                Node::Split { children, .. } => {
                    if idx >= children.len() { return None; }
                    if i + 1 == path.len() {
                        let placeholder = Node::Frame(crate::frame::FrameId::null());
                        return Some(std::mem::replace(&mut children[idx].0, placeholder));
                    }
                    node = &mut children[idx].0;
                }
                _ => return None,
            }
        }
        None
    }

    /// Recursively collapse any Split with one child into that child.
    pub fn collapse_singleton_splits(&mut self) {
        if let Node::Split { children, .. } = self {
            for (child, _) in children.iter_mut() {
                child.collapse_singleton_splits();
            }
            if children.len() == 1 {
                let (only, _) = children.remove(0);
                *self = only;
            }
        }
    }
}
