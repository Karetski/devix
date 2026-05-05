//! `devix-editor` — concrete `Pane` implementations.
//!
//! The `Document` model lives in `devix-workspace`; this crate provides
//! the editor view, tab/split/sidebar composites, and the render
//! function. `Document` and `DocId` are re-exported for callers that
//! built up imports against the previous one-stop-shop.

pub mod editor;
pub mod layout;

pub use devix_workspace::{DocId, Document};
pub use editor::{EditorPane, EditorRenderResult, EditorView, render_editor};
pub use layout::{Axis, SidebarSlotPane, SplitPane, TabbedPane};
