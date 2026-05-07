//! Editor module — formerly `devix-editor`.
//!
//! Owns the root `Editor` struct (the layout tree, focus, modal slot,
//! document/cursor SlotMaps), plus `Document`, `EditorPane`, commands,
//! keymap, and palette logic. Folded into `devix-core` during T-11; the
//! pre-Stage-1 public surface is re-exported here unchanged so consumers
//! can keep importing `crate::editor::X`. Layout primitives that the
//! editor's tree references (`Axis`, `Direction`, `SidebarSlot`,
//! `SidebarSlotPane`, `TabbedPane`) live at the `devix-core` crate root.

pub mod buffer;
pub mod commands;
pub mod cursor;
pub mod document;
pub mod editor;
pub mod frame;
pub mod tree;

pub use buffer::{BufferRender, EditorPane, EditorRenderResult, render_buffer};
pub use commands::{
    Chord, Command, CommandId, CommandRegistry, Context, EditorCommand, Keymap, ModalOutcome,
    PalettePane, PaletteState, Viewport, build_registry, chord_from_key, cmd, default_keymap,
    format_chord, register_builtins,
};
pub use cursor::{Cursor, CursorId, ScrollMode};
pub use document::{DocId, Document};
pub use editor::{
    DiskSink, Editor, LeafRef, RenderCache, TabHit, TabStripCache, TabStripHit, path_to_leaf,
};
pub use frame::FrameId;
pub use tree::{LayoutCtx, LayoutFrame, LayoutNode, LayoutSidebar, LayoutSplit};
