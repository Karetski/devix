//! `Pane` — the universal display unit.
//!
//! Like `UIView` in UIKit: a rect-bounded thing that renders itself,
//! handles events, and may own children. The editor, a tab strip, a
//! sidebar, the command palette, a future plugin panel — every visual
//! element in devix is a `Pane` implementation. Composition and tree
//! walks (focus, hit-test, dispatch) are *generic* over `&dyn Pane`; the
//! framework never matches on a kind enum.
//!
//! ## Trait location (T-93, locked 2026-05-07)
//!
//! Per `docs/specs/crates.md` Q2, the `Pane` trait lives in
//! `devix-core` (this crate) — not in `devix-protocol`. Reason:
//! `Pane` has non-trivial method bodies (`render`, `handle`,
//! `children`) that take crate-local types (`Rect`, `Event`,
//! `RenderCtx`, `HandleCtx`) and ratatui's `Frame`. `devix-protocol`
//! is pure data + serde; widening it to carry render-trait surfaces
//! would force a `ratatui` dep on every plugin author wanting to
//! type-check against the protocol crate.
//!
//! Re-evaluation trigger: when third-party plugins ship as their
//! own `cargo` crates, we may want a `PaneSpec` (data-only) in
//! `devix-protocol` and a `PaneRenderer` trait in `devix-core` so
//! plugin authors can type the spec without pulling in `devix-core`.
//! Until that landscape exists, keep the trait here.
//!
//! Lattner's MLIR principle: a few open primitives compose, and new
//! features extend by implementing the primitive — not by adding new
//! top-level concepts. The trait surface is deliberately tiny:
//!
//! - `render(&self, ...)` — pure paint. The compiler enforces purity.
//! - `handle(&mut self, ...)` — event entry point. Composite Panes
//!   recurse into their own children inside `handle`.
//! - `children(&self, area)` — read-only structural walk used by the
//!   framework for hit-testing, focus discovery, and any future
//!   tree-walking consumer. Default: leaf, no children.
//! - `is_focusable(&self)` — does this Pane participate in the focus
//!   chain? Default: no (composite layout containers).
//!
//! Notably absent:
//! - No `PaneId` enum tagging kinds. The framework does not know about
//!   "frames" vs. "sidebars" vs. "tab strips". Plugin Panes look
//!   identical to built-in Panes from the framework's perspective.
//! - No `LayoutSink` with role-typed methods. Geometry comes back via
//!   `children()`'s `Rect`s; consumers query the Pane directly through
//!   the tree walk.

use std::any::Any;

use crate::event::Event;
use crate::geom::Rect;

/// Result of `Pane::handle`. Drives the responder chain: the dispatcher
/// hands an event to the focused Pane first, walks up through ancestors
/// until one returns `Consumed`, then stops.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// This Pane handled the event. Stop walking the chain.
    Consumed,
    /// This Pane did not care; let the next responder try.
    Ignored,
}

/// Render-time context. Carries the ratatui frame and any read-only
/// services a Pane needs to draw.
///
/// `layout` is `Some` when the structural-render walker
/// (`PaneRegistry::render`) is in progress — it carries the editor
/// borrows (documents, cursors, theme, render-cache, focused leaf)
/// that the layout-tree leaves (`FramePane`, `SidebarLayoutPane`)
/// need to actually paint content. Chrome panes and modal panes
/// (`TabbedPane`, `LayoutSidebar` chrome, `PalettePane`, plugin panes)
/// pass `None` and ignore it. Decision locked 2026-05-08 — see
/// `foundations-review.md` § *Amendment log* for the alternatives
/// considered (parallel render paths, sub-trait `LayoutPane`).
pub struct RenderCtx<'a, 'frame> {
    pub frame: &'a mut ratatui::Frame<'frame>,
    pub layout: Option<&'a crate::editor::tree::LayoutCtx<'a>>,
}

/// Event-time context. The mutable counterpart to `RenderCtx`. Carries
/// handles to the document registry, the focus chain, the action
/// dispatcher, and anything else a Pane may need to mutate while handling
/// an event. Phase 0 stub; fields populate as features migrate.
#[derive(Default)]
pub struct HandleCtx<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

/// The universal display unit.
pub trait Pane {
    /// Paint into `area`. Pure.
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>);

    /// Handle one input event. The Pane is responsible for forwarding to
    /// its own children if it has any — `area` lets composite Panes
    /// hit-test against the geometry their parent assigned them.
    fn handle(&mut self, ev: &Event, area: Rect, ctx: &mut HandleCtx<'_>) -> Outcome;

    /// Direct children of this Pane laid out within `area`, in z-order
    /// (later items paint on top / take precedence in hit-tests). Default:
    /// no children.
    ///
    /// This is the *only* structural hook on the trait. Hit-test, focus
    /// traversal, and any future tree-walking consumer go through it.
    /// Composite Panes (`SplitPane`, `TabbedPane`, `SidebarPane`) override
    /// to expose their layout; leaf Panes inherit the empty default.
    fn children(&self, _area: Rect) -> Vec<(Rect, &dyn Pane)> {
        Vec::new()
    }

    /// Mutable counterpart of [`Self::children`]. Same shape, same
    /// rect math, but yields `&mut dyn Pane` so mutate helpers
    /// (T-91 phase 2: tree-mutation ops like `replace_at`,
    /// `remove_at`, `collapse_singletons`,
    /// `lift_into_horizontal_split`) can walk and rewrite the tree
    /// without downcasting to a concrete composite struct. Default:
    /// no children.
    fn children_mut(&mut self, _area: Rect) -> Vec<(Rect, &mut dyn Pane)> {
        Vec::new()
    }

    /// Does this Pane accept focus? Layout containers (`SplitPane`)
    /// return false; leaves that take input (`EditorPane`, `SidebarPane`)
    /// return true. Used by the framework's focus-traversal walker to
    /// skip over passthrough composites.
    fn is_focusable(&self) -> bool {
        false
    }

    /// Type-erased view, for consumers that need to recover the concrete
    /// Pane type (e.g. extract a `FrameId` from a layout-tree leaf, or
    /// query a plugin Pane for its specific interface). Default: `None`,
    /// which is correct for borrowed Panes (`'static` is required for
    /// `&dyn Any`). Owned Panes (`'static`) override with `Some(self)`.
    ///
    /// MLIR analogue: `Op::dyn_cast<DerivedOp>()`. Lets the framework
    /// stay generic over `&dyn Pane` while specific consumers downcast
    /// when they have a reason to.
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }

    /// Mutable counterpart of [`Self::as_any`]. Same shape, same default;
    /// owned (`'static`) Panes override with `Some(self)` so callers can
    /// `downcast_mut::<DerivedPane>()` and mutate.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
}
