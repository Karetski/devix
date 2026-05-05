//! Dispatcher context.

use ratatui::layout::Rect;

use crate::command::CommandRegistry;
use crate::surface::Surface;

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
    pub surface: &'a mut Surface,
    pub clipboard: &'a mut Option<arboard::Clipboard>,
    pub quit: &'a mut bool,
    pub viewport: Viewport,
    pub commands: &'a CommandRegistry,
}
