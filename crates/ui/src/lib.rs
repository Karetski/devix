//! Ratatui widgets for the editor view and status line.

pub mod editor;
pub mod sidebar;
pub mod status;
pub mod tabstrip;

pub use editor::{EditorRenderResult, EditorView, render_editor};
pub use sidebar::{SidebarInfo, render_sidebar};
pub use status::{StatusInfo, render_status};
pub use tabstrip::{MIN_TAB_WIDTH, TabHit, TabInfo, TabStripRender, render_tabstrip};
