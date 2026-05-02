//! Recursive layout tree.

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
}
