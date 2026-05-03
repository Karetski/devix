//! Editor state, layout tree, action enum, and dispatcher.

pub mod action;
pub mod builtins;
pub mod command;
pub mod context;
pub mod dispatch;
pub mod document;
pub mod frame;
pub mod keymap;
pub mod layout;
pub mod overlay;
pub mod view;
pub mod workspace;

pub use action::Action;
// Re-exported so callers building Actions don't have to depend on devix-lsp
// directly for the trigger discriminator.
pub use devix_lsp::CompletionTrigger;
pub use builtins::{build_registry, register_builtins};
pub use command::{Command, CommandId, CommandRegistry};
pub use keymap::{Chord, Keymap, chord_from_key, default_keymap};
pub use context::{Context, StatusLine, Viewport};
pub use dispatch::{dispatch, refilter_completion};
pub use document::{DocDiagnostic, DocId, Document};
pub use frame::{Frame, FrameId};
pub use layout::{Axis, Direction, Node, SidebarSlot};
pub use overlay::{Overlay, PaletteState, SymbolsKind, SymbolsState, SymbolsStatus};
pub use view::{
    CompletionState, CompletionStatus, HoverState, HoverStatus, ScrollMode, View, ViewId,
};
pub use workspace::{LeafRef, RenderCache, TabStripCache, TabStripHit, Workspace};
