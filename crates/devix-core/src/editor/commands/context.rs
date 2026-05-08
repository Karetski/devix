//! Dispatcher context.

use crate::Clipboard;
use crate::Rect;

use crate::editor::commands::registry::CommandRegistry;
use crate::editor::editor::RenderCache;
use crate::editor::Editor;

#[derive(Copy, Clone, Default)]
pub struct Viewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub gutter_width: u16,
}

impl From<(Rect, u16)> for Viewport {
    fn from((rect, gutter_width): (Rect, u16)) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            gutter_width,
        }
    }
}

pub struct Context<'a> {
    pub editor: &'a mut Editor,
    pub clipboard: &'a mut dyn Clipboard,
    pub quit: &'a mut bool,
    pub viewport: Viewport,
    pub commands: &'a CommandRegistry,
    /// Layout/render cache owned by the tui client. Commands that
    /// hit-test or query frame geometry read it through this borrow.
    /// Carved out of `Editor` per T-92 (the cache is tui-internal).
    pub layout_cache: &'a RenderCache,
}
