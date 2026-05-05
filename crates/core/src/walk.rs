//! Generic walkers over `&dyn Pane` trees.
//!
//! All consumers that need to find a Pane by some property (under the
//! mouse, focusable, on the path) go through these. The walkers are
//! totally generic — they ask each Pane via `Pane::children(area)` and
//! `Pane::is_focusable()`. Specific behavior lives on the Pane impl, not
//! on a kind enum the framework matches against.
//!
//! This is the Lattner shape: composition of one open primitive
//! (`Pane`) is enough to express every traversal.

use crate::geom::Rect;
use crate::pane::Pane;

/// Find the deepest Pane whose assigned rect contains `(col, row)`.
/// Returns `(rect, pane)` for that leaf, or `None` if no Pane covers the
/// point. Children later in `children()` win on overlap (z-order).
pub fn pane_at<'a>(
    root: &'a dyn Pane,
    area: Rect,
    col: u16,
    row: u16,
) -> Option<(Rect, &'a dyn Pane)> {
    if !contains(area, col, row) {
        return None;
    }
    // Walk children in reverse so later (top z-order) wins. First match
    // recurses to the deepest descendant that still contains the point.
    let kids = root.children(area);
    for (child_rect, child) in kids.iter().rev() {
        if let Some(found) = pane_at(*child, *child_rect, col, row) {
            return Some(found);
        }
    }
    Some((area, root))
}

/// Find the deepest focusable Pane whose rect contains `(col, row)`.
/// Same shape as `pane_at`, but skips Panes that report `is_focusable()
/// == false`. Used to translate a click into a focus-target leaf.
pub fn focusable_at<'a>(
    root: &'a dyn Pane,
    area: Rect,
    col: u16,
    row: u16,
) -> Option<(Rect, &'a dyn Pane)> {
    if !contains(area, col, row) {
        return None;
    }
    let kids = root.children(area);
    for (child_rect, child) in kids.iter().rev() {
        if let Some(found) = focusable_at(*child, *child_rect, col, row) {
            return Some(found);
        }
    }
    if root.is_focusable() {
        Some((area, root))
    } else {
        None
    }
}

/// Resolve a focus path (sequence of child indices) to its target Pane
/// and the rect that path occupies. Returns `None` if any index is out
/// of range. Used by the dispatcher to find the focused leaf without
/// having to remember its concrete type.
pub fn pane_at_path<'a>(
    root: &'a dyn Pane,
    area: Rect,
    path: &[usize],
) -> Option<(Rect, &'a dyn Pane)> {
    let mut cur_pane: &dyn Pane = root;
    let mut cur_area = area;
    for &idx in path {
        let kids = cur_pane.children(cur_area);
        let (rect, child) = kids.into_iter().nth(idx)?;
        cur_pane = child;
        cur_area = rect;
    }
    Some((cur_area, cur_pane))
}

/// Collect every focusable leaf in tree order: `(path, rect, pane)`.
/// Used by directional focus traversal to enumerate candidates and pick
/// the geometrically closest one.
pub fn focusable_leaves<'a>(
    root: &'a dyn Pane,
    area: Rect,
) -> Vec<(Vec<usize>, Rect, &'a dyn Pane)> {
    let mut out = Vec::new();
    walk_focusable(root, area, &mut Vec::new(), &mut out);
    out
}

fn walk_focusable<'a>(
    pane: &'a dyn Pane,
    area: Rect,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, Rect, &'a dyn Pane)>,
) {
    let kids = pane.children(area);
    if kids.is_empty() {
        if pane.is_focusable() {
            out.push((path.clone(), area, pane));
        }
        return;
    }
    for (i, (child_rect, child)) in kids.into_iter().enumerate() {
        path.push(i);
        walk_focusable(child, child_rect, path, out);
        path.pop();
    }
}

fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;
    use crate::pane::{HandleCtx, Outcome, RenderCtx};

    /// Minimal leaf for tests: ignores everything, optionally focusable.
    struct Leaf {
        focusable: bool,
    }
    impl Pane for Leaf {
        fn render(&self, _: Rect, _: &mut RenderCtx<'_, '_>) {}
        fn handle(&mut self, _: &Event, _: Rect, _: &mut HandleCtx<'_>) -> Outcome {
            Outcome::Ignored
        }
        fn is_focusable(&self) -> bool {
            self.focusable
        }
    }

    /// Composite that lays its two children out side by side, full height.
    struct HSplit {
        left: Leaf,
        right: Leaf,
    }
    impl Pane for HSplit {
        fn render(&self, _: Rect, _: &mut RenderCtx<'_, '_>) {}
        fn handle(&mut self, _: &Event, _: Rect, _: &mut HandleCtx<'_>) -> Outcome {
            Outcome::Ignored
        }
        fn children(&self, area: Rect) -> Vec<(Rect, &dyn Pane)> {
            let half = area.width / 2;
            vec![
                (Rect { x: area.x, y: area.y, width: half, height: area.height }, &self.left),
                (
                    Rect { x: area.x + half, y: area.y, width: area.width - half, height: area.height },
                    &self.right,
                ),
            ]
        }
    }

    fn full() -> Rect {
        Rect { x: 0, y: 0, width: 100, height: 50 }
    }

    #[test]
    fn pane_at_returns_left_child_for_point_in_left_half() {
        let tree = HSplit { left: Leaf { focusable: true }, right: Leaf { focusable: true } };
        let (rect, _) = pane_at(&tree, full(), 10, 10).unwrap();
        assert_eq!(rect.x, 0);
        assert_eq!(rect.width, 50);
    }

    #[test]
    fn pane_at_returns_right_child_for_point_in_right_half() {
        let tree = HSplit { left: Leaf { focusable: true }, right: Leaf { focusable: true } };
        let (rect, _) = pane_at(&tree, full(), 80, 10).unwrap();
        assert_eq!(rect.x, 50);
    }

    #[test]
    fn pane_at_returns_none_for_point_outside_area() {
        let tree = HSplit { left: Leaf { focusable: true }, right: Leaf { focusable: true } };
        assert!(pane_at(&tree, full(), 200, 10).is_none());
    }

    #[test]
    fn focusable_at_skips_unfocusable_leaves() {
        let tree = HSplit { left: Leaf { focusable: false }, right: Leaf { focusable: true } };
        // Left half is not focusable; the walker should not return it.
        assert!(focusable_at(&tree, full(), 10, 10).is_none());
        assert!(focusable_at(&tree, full(), 80, 10).is_some());
    }

    #[test]
    fn focusable_leaves_returns_paths_in_tree_order() {
        let tree = HSplit { left: Leaf { focusable: true }, right: Leaf { focusable: true } };
        let leaves = focusable_leaves(&tree, full());
        assert_eq!(leaves.len(), 2);
        assert_eq!(leaves[0].0, vec![0]);
        assert_eq!(leaves[1].0, vec![1]);
    }

    #[test]
    fn pane_at_path_resolves_path_to_correct_leaf() {
        let tree = HSplit { left: Leaf { focusable: true }, right: Leaf { focusable: true } };
        let (rect, _) = pane_at_path(&tree, full(), &[0]).unwrap();
        assert_eq!(rect.x, 0);
        let (rect, _) = pane_at_path(&tree, full(), &[1]).unwrap();
        assert_eq!(rect.x, 50);
        // Out-of-range index → None.
        assert!(pane_at_path(&tree, full(), &[2]).is_none());
    }
}
