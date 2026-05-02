//! Dispatcher context: bundles every mutable reference an Action handler may
//! touch, plus a per-frame copy of viewport geometry.

use crate::state::EditorState;

#[derive(Default)]
pub struct StatusLine(Option<String>);

impl StatusLine {
    pub fn set(&mut self, s: impl Into<String>) {
        self.0 = Some(s.into());
    }
    pub fn clear(&mut self) {
        self.0 = None;
    }
    pub fn get(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Copy, Clone, Default)]
pub struct Viewport {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub gutter_width: u16,
}

pub struct Context<'a> {
    pub editor: &'a mut EditorState,
    pub clipboard: &'a mut Option<arboard::Clipboard>,
    pub status: &'a mut StatusLine,
    pub quit: &'a mut bool,
    pub disk_changed_pending: &'a mut bool,
    pub viewport: Viewport,
}
