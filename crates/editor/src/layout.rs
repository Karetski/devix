//! Composite `Pane` types — the layout primitives.
//!
//! Three composites cover every layout shape the editor needs today:
//!
//! - [`SplitPane`]: ratatui-style proportional split along an axis. Children
//!   are `Box<dyn Pane>` plus an integer weight; recursion is just nesting.
//! - [`TabbedPane`]: a tab strip pinned to row 0 with one editor body
//!   beneath. Owns both children as fields so `children()` can hand back
//!   stable `&dyn Pane` references.
//! - [`SidebarSlotPane`]: a sidebar bound to an edge slot with optional
//!   content. For now content is empty (chrome-only), matching today's
//!   sidebar; the slot exists so plugins can drop a Pane in later.
//!
//! All three follow the same render shape: walk `children(area)` and
//! recurse. The Lattner answer to "what does a composite do?" is "the same
//! thing as the framework": ask each child for its rect, paint it. There's
//! no special composite API.

pub use devix_core::Axis;

use devix_core::{Event, HandleCtx, Outcome, Pane, Rect, RenderCtx, split_rects};
use devix_ui::{SidebarPane as SidebarChrome, TabStripPane};

use crate::editor::EditorPane;

/// Proportional split: children share the available area along `axis`,
/// each weighted by its `u16` factor. Mirrors `Node::Split` semantics.
pub struct SplitPane<'a> {
    pub axis: Axis,
    pub children: Vec<(Box<dyn Pane + 'a>, u16)>,
}

impl<'a> Pane for SplitPane<'a> {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        for (rect, child) in self.children(area) {
            child.render(rect, ctx);
        }
    }

    fn handle(&mut self, _ev: &Event, _area: Rect, _ctx: &mut HandleCtx<'_>) -> Outcome {
        // Composite layout: events route through the focused child via the
        // dispatcher walking `children()`, not via the parent matching.
        Outcome::Ignored
    }

    fn children(&self, area: Rect) -> Vec<(Rect, &dyn Pane)> {
        if self.children.is_empty() {
            return Vec::new();
        }
        let weights: Vec<u16> = self.children.iter().map(|(_, w)| *w).collect();
        let rects = split_rects(area, self.axis, &weights);
        self.children
            .iter()
            .zip(rects.into_iter())
            .map(|((child, _), rect)| (rect, child.as_ref()))
            .collect()
    }
}

/// One editor frame: tab strip pinned to row 0, active editor body below.
/// Owns its children as fields so `children()` returns stable references
/// — `EditorPane` borrows nothing it doesn't outlive.
pub struct TabbedPane<'a> {
    pub strip: TabStripPane,
    pub editor: EditorPane<'a>,
}

impl<'a> Pane for TabbedPane<'a> {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        for (rect, child) in self.children(area) {
            child.render(rect, ctx);
        }
    }

    fn handle(&mut self, _ev: &Event, _area: Rect, _ctx: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }

    fn children(&self, area: Rect) -> Vec<(Rect, &dyn Pane)> {
        let strip_rect = Rect { height: 1, ..area };
        let body_rect = Rect {
            y: area.y.saturating_add(1),
            height: area.height.saturating_sub(1),
            ..area
        };
        // Children must be returned in z-order: strip first, body second
        // so `pane_at` walking in reverse picks the body for clicks below
        // row 0 and the strip for clicks on row 0.
        vec![
            (strip_rect, &self.strip as &dyn Pane),
            (body_rect, &self.editor as &dyn Pane),
        ]
    }
}

/// A sidebar slot: chrome (bordered placeholder for now) plus an
/// optional content `Pane` that fills the inner area. Today's sidebars
/// have no content — the slot is the contract that lets a plugin (or
/// future file-tree / settings Pane) drop something in without the
/// framework needing to know about plugins.
pub struct SidebarSlotPane<'a> {
    pub chrome: SidebarChrome,
    pub content: Option<Box<dyn Pane + 'a>>,
}

impl<'a> Pane for SidebarSlotPane<'a> {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        self.chrome.render(area, ctx);
        if let Some(content) = &self.content {
            // Paint the inner area inside the 1-cell border. When
            // borders shrink past viability, skip — render must not panic.
            let inner = Rect {
                x: area.x.saturating_add(1),
                y: area.y.saturating_add(1),
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(2),
            };
            if inner.width > 0 && inner.height > 0 {
                content.render(inner, ctx);
            }
        }
    }

    fn handle(&mut self, _ev: &Event, _area: Rect, _ctx: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }

    fn children(&self, area: Rect) -> Vec<(Rect, &dyn Pane)> {
        let mut out: Vec<(Rect, &dyn Pane)> = vec![(area, &self.chrome as &dyn Pane)];
        if let Some(content) = &self.content {
            let inner = Rect {
                x: area.x.saturating_add(1),
                y: area.y.saturating_add(1),
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(2),
            };
            out.push((inner, content.as_ref()));
        }
        out
    }

    fn is_focusable(&self) -> bool {
        // The slot is focusable in the same way today's sidebar leaf is.
        // When content shows up, focus may pass through to it; that's a
        // dispatcher decision, not a property of this trait.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devix_core::{focusable_leaves, pane_at};

    /// A no-op leaf used to exercise the composites without dragging in
    /// the editor's borrow surface.
    struct StubLeaf {
        focusable: bool,
    }
    impl Pane for StubLeaf {
        fn render(&self, _: Rect, _: &mut RenderCtx<'_, '_>) {}
        fn handle(&mut self, _: &Event, _: Rect, _: &mut HandleCtx<'_>) -> Outcome {
            Outcome::Ignored
        }
        fn is_focusable(&self) -> bool {
            self.focusable
        }
    }

    fn full() -> Rect {
        Rect { x: 0, y: 0, width: 100, height: 50 }
    }

    #[test]
    fn split_horizontal_partitions_children_by_weight() {
        let split = SplitPane {
            axis: Axis::Horizontal,
            children: vec![
                (Box::new(StubLeaf { focusable: true }), 1),
                (Box::new(StubLeaf { focusable: true }), 3),
            ],
        };
        let kids = split.children(full());
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0].0.x, 0);
        assert_eq!(kids[0].0.width, 25); // 1/(1+3) of 100
        assert_eq!(kids[1].0.x, 25);
        assert_eq!(kids[1].0.width, 75);
    }

    #[test]
    fn split_vertical_partitions_along_y() {
        let split = SplitPane {
            axis: Axis::Vertical,
            children: vec![
                (Box::new(StubLeaf { focusable: true }), 1),
                (Box::new(StubLeaf { focusable: true }), 1),
            ],
        };
        let kids = split.children(full());
        assert_eq!(kids[0].0.y, 0);
        assert_eq!(kids[0].0.height, 25);
        assert_eq!(kids[1].0.y, 25);
    }

    #[test]
    fn split_pane_at_finds_correct_child() {
        let split = SplitPane {
            axis: Axis::Horizontal,
            children: vec![
                (Box::new(StubLeaf { focusable: true }), 1),
                (Box::new(StubLeaf { focusable: true }), 1),
            ],
        };
        // Click in left half lands on the first child.
        let (rect, _) = pane_at(&split, full(), 10, 10).unwrap();
        assert_eq!(rect.x, 0);
        assert_eq!(rect.width, 50);
        // Click in right half.
        let (rect, _) = pane_at(&split, full(), 80, 10).unwrap();
        assert_eq!(rect.x, 50);
    }

    #[test]
    fn split_focusable_leaves_returns_paths_in_order() {
        let split = SplitPane {
            axis: Axis::Horizontal,
            children: vec![
                (Box::new(StubLeaf { focusable: true }), 1),
                (Box::new(StubLeaf { focusable: false }), 1),
                (Box::new(StubLeaf { focusable: true }), 1),
            ],
        };
        let leaves = focusable_leaves(&split, full());
        assert_eq!(leaves.len(), 2, "non-focusable leaf is skipped");
        assert_eq!(leaves[0].0, vec![0]);
        assert_eq!(leaves[1].0, vec![2]);
    }

    #[test]
    fn nested_split_walks_recursively() {
        // Outer Vertical split, second child is a Horizontal split.
        let inner = SplitPane {
            axis: Axis::Horizontal,
            children: vec![
                (Box::new(StubLeaf { focusable: true }), 1),
                (Box::new(StubLeaf { focusable: true }), 1),
            ],
        };
        let outer = SplitPane {
            axis: Axis::Vertical,
            children: vec![
                (Box::new(StubLeaf { focusable: true }), 1),
                (Box::new(inner), 1),
            ],
        };
        let leaves = focusable_leaves(&outer, full());
        // Three leaves: one on top, two on bottom.
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[0].0, vec![0]);
        assert_eq!(leaves[1].0, vec![1, 0]);
        assert_eq!(leaves[2].0, vec![1, 1]);
    }
}
