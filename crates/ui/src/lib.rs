//! Pure ratatui widgets — design-system primitives with no awareness of
//! surface, buffer, or LSP types. Surface-coupled renderers (editor,
//! palette) live in `devix-editor` / `devix-surface`.

pub mod layout;
pub mod popup;
pub mod sidebar;
pub mod tabstrip;

pub use popup::{CompletionLine, Popup, PopupAnchor, PopupContent, render_popup};
pub use sidebar::{SidebarInfo, SidebarPane, render_sidebar};
pub use tabstrip::{
    MIN_TAB_WIDTH, TabHit, TabInfo, TabStripPane, TabStripRender, layout_tabstrip,
    render_tabstrip, tab_strip_layout,
};
