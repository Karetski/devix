//! Pure ratatui widgets — design-system primitives with no awareness of
//! workspace, buffer, or LSP types. Workspace-coupled renderers (editor,
//! palette, symbols) live in `devix-views`.

pub mod layout;
pub mod popup;
pub mod sidebar;
pub mod status;
pub mod tabstrip;

pub use popup::{CompletionLine, Popup, PopupAnchor, PopupContent, render_popup};
pub use sidebar::{SidebarInfo, render_sidebar};
pub use status::{StatusInfo, render_status};
pub use tabstrip::{
    MIN_TAB_WIDTH, TabHit, TabInfo, TabStripRender, layout_tabstrip, render_tabstrip,
};
