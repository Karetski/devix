//! `devix-editor` — concrete `Pane` implementations + `Document` model.
//!
//! Holds the editor view (buffer rendering), tab/split/sidebar
//! composites, and `Document` (rope buffer + highlighter + filesystem
//! watcher).

pub mod document;
pub mod editor;
pub mod layout;

pub use document::{DocId, Document};
pub use editor::{EditorPane, EditorRenderResult, EditorView, render_editor};
pub use layout::{Axis, SidebarSlotPane, TabbedPane};
