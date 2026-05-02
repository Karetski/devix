//! Ratatui widgets for the editor view and status line.

pub mod editor;
pub mod status;

pub use editor::{EditorRenderResult, EditorView, render_editor};
pub use status::{StatusInfo, render_status};
