//! `devix-editor` — the editor crate. Owns the root `Editor` struct
//! (the layout tree, focus, modal slot, document/cursor SlotMaps),
//! plus `Document`, `EditorPane`, commands, keymap, and palette logic.

pub mod buffer;
pub mod commands;
pub mod cursor;
pub mod document;
pub mod editor;
pub mod frame;
pub mod services;
pub mod tree;

pub use buffer::{BufferRender, EditorPane, EditorRenderResult, render_buffer};
pub use commands::{
    Chord, Command, CommandId, CommandRegistry, Context, EditorCommand, Keymap, ModalOutcome,
    PalettePane, PaletteState, Viewport, build_registry, chord_from_key, cmd, default_keymap,
    format_chord, register_builtins,
};
pub use cursor::{Cursor, CursorId, ScrollMode};
pub use document::{DocId, Document};
pub use editor::{Editor, LeafRef, RenderCache, TabHit, TabStripCache, TabStripHit};
pub use frame::FrameId;
pub use services::RenderServices;
pub use tree::{
    LayoutFrame, find_frame, find_frame_mut, frame_ids, leaves_with_rects, pane_at_indices,
    pane_leaf_id,
};

// Layout primitives + composites live in `devix-panes`; re-export the
// pieces the binary touches so it doesn't have to import both crates.
pub use devix_panes::{Axis, Direction, SidebarSlot, SidebarSlotPane, TabbedPane};
