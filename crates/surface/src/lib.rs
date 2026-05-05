//! Editor state, layout tree, and the trait-based command surface.

pub mod builtins;
pub mod cmd;
pub mod command;
pub mod context;
pub mod dispatch;
pub mod frame;
pub mod keymap;
pub mod layout;
pub mod modal;
pub mod surface;
pub mod tree;
pub mod view;

pub use cmd::EditorCommand;
pub use tree::{
    LayoutFrame, find_frame, find_frame_mut, frame_ids, leaves_with_rects, pane_at_indices,
    pane_leaf_id,
};
// Re-exported so callers building commands don't have to depend on devix-lsp
// directly for the trigger discriminator.
pub use devix_lsp::CompletionTrigger;
// Document model lives in `devix-editor`; re-exported so callers can keep
// importing via `devix_surface::*` (one import path for editor state).
pub use devix_editor::{DocDiagnostic, DocId, Document};
pub use builtins::{build_registry, register_builtins};
pub use command::{Command, CommandId, CommandRegistry};
pub use keymap::{Chord, Keymap, chord_from_key, default_keymap};
pub use context::{Context, StatusLine, Viewport};
pub use dispatch::refilter_completion;
pub use frame::FrameId;
pub use layout::{Axis, Direction, SidebarSlot};
pub use modal::{
    ModalOutcome, PaletteState, PalettePane, SettingsPane, SettingsRow, SettingsState,
    SymbolPickerPane, SymbolsKind, SymbolsState, SymbolsStatus, format_chord, palette_area,
    render_palette, render_settings, render_symbols, settings_area, symbols_area,
};
pub use devix_editor::{CompletionState, CompletionStatus, HoverState, HoverStatus};
pub use view::{ScrollMode, View, ViewId};
pub use surface::{LeafRef, RenderCache, Surface, TabHit, TabStripCache, TabStripHit};
