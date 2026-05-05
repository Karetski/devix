//! Surface-coupled views: render functions that paint editor state from
//! `devix-surface` using primitives from `devix-ui`. The split between
//! this crate and `devix-ui` is deliberate — `devix-ui` is a pure design
//! system with no awareness of surface state, so its widgets can be
//! reused without dragging buffer / syntax / LSP types behind them.
//!
//! Modal Panes (palette, symbol picker) used to live here too; Phase 4
//! moved them into `devix-surface::modal` so the modal slot on
//! `Surface` owns concrete state. The free render helpers
//! `render_palette` / `render_symbols` come along for the ride and are
//! re-exported from the surface crate.

pub mod editor;
pub mod layout;

pub use editor::{
    CompletionPane, EditorPane, EditorRenderResult, EditorView, HoverPane, render_editor,
};
pub use layout::{Axis, SidebarSlotPane, SplitPane, TabbedPane};
