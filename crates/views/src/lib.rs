//! Workspace-coupled views: render functions that paint editor state from
//! `devix-workspace` using primitives from `devix-ui`. The split between
//! this crate and `devix-ui` is deliberate — `devix-ui` is a pure design
//! system with no awareness of workspace state, so its widgets can be
//! reused without dragging buffer / syntax / LSP types behind them.
//!
//! Modal Panes (palette, symbol picker) used to live here too; Phase 4
//! moved them into `devix-workspace::modal` so the modal slot on
//! `Workspace` owns concrete state. The free render helpers
//! `render_palette` / `render_symbols` come along for the ride and are
//! re-exported from the workspace crate.

pub mod editor;
pub mod layout;

pub use editor::{
    CompletionPane, EditorPane, EditorRenderResult, EditorView, HoverPane, render_editor,
};
pub use layout::{Axis, SidebarSlotPane, SplitPane, TabbedPane};
