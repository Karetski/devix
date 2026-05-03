//! Ratatui widgets for the editor view and status line.

pub mod editor;
pub mod palette;
pub mod popup;
pub mod sidebar;
pub mod status;
pub mod tabstrip;

pub use editor::{EditorRenderResult, EditorView, render_editor};
pub use palette::{format_chord, palette_area, render_palette};
pub use popup::{Popup, PopupAnchor, PopupContent, render_popup};
pub use sidebar::{SidebarInfo, render_sidebar};
pub use status::{StatusInfo, render_status};
pub use tabstrip::{MIN_TAB_WIDTH, TabInfo, TabStripRender, render_tabstrip};
