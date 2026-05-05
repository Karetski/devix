//! Dispatcher context.

use ratatui::layout::Rect;

use crate::command::CommandRegistry;
use crate::workspace::Workspace;

#[derive(Default)]
pub struct StatusLine(Option<String>);

impl StatusLine {
    pub fn set(&mut self, s: impl Into<String>) { self.0 = Some(s.into()); }
    pub fn clear(&mut self) { self.0 = None; }
    pub fn get(&self) -> Option<&str> { self.0.as_deref() }
}

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
    pub workspace: &'a mut Workspace,
    pub clipboard: &'a mut Option<arboard::Clipboard>,
    pub status: &'a mut StatusLine,
    pub quit: &'a mut bool,
    pub viewport: Viewport,
    pub commands: &'a CommandRegistry,
}
