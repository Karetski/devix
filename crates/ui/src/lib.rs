//! Ratatui widgets for the editor view and status line.

pub mod editor;
pub mod status;
pub mod tabstrip;

pub use editor::{EditorRenderResult, EditorView, render_editor};
pub use status::{StatusInfo, render_status};
pub use tabstrip::{TabInfo, render_tabstrip};
