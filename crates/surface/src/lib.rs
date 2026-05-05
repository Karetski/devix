//! Editor state coordinator: owns the layout tree, focus, modal slot,
//! and the SlotMaps that index Documents and Views by id.

pub mod commands;
pub mod frame;
pub mod layout;
pub mod services;
pub mod surface;
pub mod tree;
pub mod view;

pub use commands::{
    Chord, Command, CommandId, CommandRegistry, Context, EditorCommand, Keymap, ModalOutcome,
    PalettePane, PaletteState, Viewport, build_registry, chord_from_key, cmd, default_keymap,
    format_chord, register_builtins,
};
pub use services::RenderServices;
pub use tree::{
    LayoutFrame, find_frame, find_frame_mut, frame_ids, leaves_with_rects, pane_at_indices,
    pane_leaf_id,
};
pub use view::{ScrollMode, View, ViewId};
pub use devix_workspace::{DocId, Document};
pub use frame::FrameId;
pub use layout::{Axis, Direction, SidebarSlot};
pub use surface::{LeafRef, RenderCache, Surface, TabHit, TabStripCache, TabStripHit};
