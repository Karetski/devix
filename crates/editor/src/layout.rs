//! Composite `Pane` types — the per-frame render-tree primitives.
//!
//! Two composites are owned per render call (built on the stack inside
//! the structural Panes' `render` impls in `devix-editor`'s `tree.rs`):
//!
//! - [`TabbedPane`]: a tab strip pinned to row 0 with one editor body
//!   beneath. Owns both children as fields so `children()` can hand back
//!   stable `&dyn Pane` references.
//! - [`SidebarSlotPane`]: a sidebar bound to an edge slot with optional
//!   content. The slot exists so plugins can drop a Pane in later.
//!
//! Splits don't appear here — `LayoutSplit` in `devix-editor` is the
//! single split primitive; it owns its layout state and has been
//! taught to render itself directly. There is no parallel render-tree
//! `SplitPane` anymore.
//!
//! Both composites follow the same render shape: walk `children(area)`
//! and recurse. The Lattner answer to "what does a composite do?" is
//! "the same thing as the framework": ask each child for its rect,
//! paint it.

pub use devix_core::Axis;

use devix_core::{Event, HandleCtx, Outcome, Pane, Rect, RenderCtx};
use devix_ui::{SidebarPane as SidebarChrome, TabStripPane};

use crate::buffer::EditorPane;

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

