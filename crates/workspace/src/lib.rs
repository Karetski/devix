//! Editor state, layout tree, action enum, and dispatcher.

pub mod action;
pub mod context;
pub mod dispatch;
pub mod document;
pub mod frame;
pub mod layout;
pub mod state;
pub mod view;
pub mod workspace;

pub use action::Action;
pub use context::{Context, StatusLine, Viewport};
pub use dispatch::dispatch;
pub use state::EditorState;
