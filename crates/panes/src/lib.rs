//! `devix-panes` — the framework crate.
//!
//! Holds the four trait-surface concepts (`Pane`, `Action`, `Event`,
//! `Outcome`) plus the implementations that compose against them
//! (layout composites, walk helpers, chrome widgets, theme). Plugins
//! depend on this single crate.
//!
//! Internally split by role: trait-surface modules (`pane`, `action`,
//! `event`, `geom`, `clipboard`, `layout_geom`) sit alongside
//! implementation modules (`composites`, `walk`, `theme`, `widgets`).
//! That's a module boundary, not a crate boundary — UIKit's stable
//! abstractions live in the same framework as their implementations,
//! and so do the ones here.

pub mod action;
pub mod clipboard;
pub mod composites;
pub mod event;
pub mod geom;
pub mod layout_geom;
pub mod pane;
pub mod theme;
pub mod walk;
pub mod widgets;

pub use action::Action;
pub use clipboard::{Clipboard, NoClipboard};
pub use composites::{SidebarSlotPane, TabbedPane};
pub use event::Event;
pub use geom::{Anchor, AnchorEdge, Rect};
pub use layout_geom::{Axis, Direction, SidebarSlot, split_rects};
pub use pane::{HandleCtx, Outcome, Pane, RenderCtx};
pub use theme::Theme;
pub use walk::{focusable_at, focusable_leaves, pane_at, pane_at_path};

// Chrome widgets — re-exported at the crate root for ergonomics; the
// `widgets` module is also pub so callers can opt for the namespaced
// path if they prefer.
pub use widgets::{
    CompletionLine, MIN_TAB_WIDTH, PaletteRow, Popup, PopupAnchor, PopupContent, SidebarInfo,
    SidebarPane, TabHit, TabInfo, TabStripPane, TabStripRender, format_chord, layout_tabstrip,
    palette_area, render_palette, render_popup, render_sidebar, render_tabstrip, tab_strip_layout,
};
