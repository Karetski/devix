//! Pure ratatui widgets — design-system primitives with no awareness
//! of editor state. Editor-coupled renderers live in `devix-editor`.

pub mod layout;
pub mod palette;
pub mod popup;
pub mod sidebar;
pub mod tabstrip;

pub use palette::{PaletteRow, format_chord, palette_area, render_palette};
pub use popup::{CompletionLine, Popup, PopupAnchor, PopupContent, render_popup};
pub use sidebar::{SidebarInfo, SidebarPane, render_sidebar};
pub use tabstrip::{
    MIN_TAB_WIDTH, TabHit, TabInfo, TabStripPane, TabStripRender, layout_tabstrip,
    render_tabstrip, tab_strip_layout,
};
