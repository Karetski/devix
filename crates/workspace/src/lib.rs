//! Editor state, layout tree, action enum, and dispatcher.

pub mod action;
pub mod command;
pub mod context;
pub mod dispatch;
pub mod document;
pub mod frame;
pub mod layout;
pub mod overlay;
pub mod view;
pub mod workspace;

pub use action::Action;
pub use command::{Command, CommandId, CommandRegistry};
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
