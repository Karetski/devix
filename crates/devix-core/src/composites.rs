//! Composite `Pane` types — the per-frame render-tree primitives.
//!
//! [`TabbedPane`] is built on the stack inside `LayoutFrame::render`
//! to compose a tab strip pinned to row 0 with the active body Pane
//! beneath. Generic over the body so any focusable Pane (an editor
//! buffer today; a settings pane, a terminal pane tomorrow) can sit
//! inside the tab strip without `panes` knowing what it is.
//!
//! Splits don't appear here — `LayoutSplit` in `crate::editor::tree`
//! is the single split primitive; it owns its layout state and renders
//! itself directly via `Pane::render`. There is no parallel render-tree
//! `SplitPane` anymore.
//!
//! Sidebars don't appear here either: T-94 retired the standalone
//! `SidebarSlotPane` once `LayoutSidebar` (in `editor::tree`) absorbed
//! the chrome + optional-content render directly.
//!
//! `TabbedPane` follows the framework render shape: walk
//! `children(area)` and recurse. The Lattner answer to "what does a
//! composite do?" is "the same thing as the framework": ask each child
//! for its rect, paint it.

use crate::event::Event;
use crate::geom::Rect;
use crate::pane::{HandleCtx, Outcome, Pane, RenderCtx};
use crate::widgets::TabStripPane;

/// One frame: tab strip pinned to row 0, active body below. Owns both
/// children as fields so `children()` returns stable references — the
/// body borrows nothing it doesn't outlive.
pub struct TabbedPane<B: Pane> {
    pub strip: TabStripPane,
    pub body: B,
}

impl<B: Pane> Pane for TabbedPane<B> {
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
            (body_rect, &self.body as &dyn Pane),
        ]
    }
}

