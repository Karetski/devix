//! `devix-editor` — concrete `Pane` implementations and the `Document` model.

pub mod document;
pub mod editor;
pub mod layout;

pub use document::{DocId, Document};
pub use editor::{EditorPane, EditorRenderResult, EditorView, render_editor};
pub use layout::{Axis, SidebarSlotPane, SplitPane, TabbedPane};
