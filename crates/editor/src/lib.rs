//! `devix-editor` — concrete `Pane` implementations, the `Document`
//! model, and per-view popup state.
//!
//! Phase 7 of the architecture refactor: the old `devix-views` and
//! `devix-document` crates merge here. `Document` is editor-internal —
//! its lifecycle (LSP attachment, filesystem watcher, syntax
//! highlighting) is the editor's concern, not generic state on the
//! framework's surface. Per-view popup state (hover, completion) lives
//! here for the same reason: the Panes that paint those popups are
//! defined here, and the bookkeeping is co-located with the renderer.
//!
//! `devix-surface` stores `SlotMap<DocId, Document>` and
//! `SlotMap<ViewId, View>` (View: this crate's `HoverState` /
//! `CompletionState`); the dep edge runs `surface -> editor`. There is
//! no edge back; editor consumes only `core`, `text`, `syntax`, `lsp`,
//! `ui`, and the standard library.

pub mod document;
pub mod editor;
pub mod layout;
pub mod popup_state;

pub use document::{DocDiagnostic, DocId, Document};
pub use editor::{
    CompletionPane, EditorPane, EditorRenderResult, EditorView, HoverPane, render_editor,
};
pub use layout::{Axis, SidebarSlotPane, SplitPane, TabbedPane};
pub use popup_state::{CompletionState, CompletionStatus, HoverState, HoverStatus};
