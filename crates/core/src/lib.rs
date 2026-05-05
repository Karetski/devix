//! `devix-core` — the stable surface every UI thing builds on.
//!
//! Phase 0 of the architecture refactor (see `ARCHITECTURE-REFACTOR.md`):
//! this crate defines the trait surface and primitive types that future
//! migrations will satisfy. **No implementations live here yet.** Today's
//! editor code does not depend on this crate; it will be migrated piece by
//! piece in Phases 1–5.
//!
//! The four core concepts:
//!
//! - [`Pane`] — the universal display unit (`UIView` analogue).
//! - [`Action`] — invocable behavior, first-class (one type per command).
//! - `Document` — text data, decoupled from any view (lives in
//!   `devix-document` and will surface here later).
//! - `Surface` — the editor root (will replace `Workspace`; lives outside
//!   `core` because it owns concrete state).
//!
//! Anything plugins ever depend on lives here. Keep the surface small.

pub mod action;
pub mod event;
pub mod geom;
pub mod pane;
pub mod theme;
pub mod walk;

pub use action::Action;
pub use event::Event;
pub use geom::{Anchor, AnchorEdge, Rect};
pub use pane::{HandleCtx, Outcome, Pane, RenderCtx};
pub use theme::Theme;
pub use walk::{focusable_at, focusable_leaves, pane_at, pane_at_path};
