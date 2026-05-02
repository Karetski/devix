//! Editor state, action enum, and dispatcher. Phase 3-ready scaffold.

pub mod action;
pub mod context;
pub mod dispatch;
pub mod state;

pub use action::Action;
pub use context::{Context, StatusLine, Viewport};
pub use dispatch::dispatch;
pub use state::EditorState;
