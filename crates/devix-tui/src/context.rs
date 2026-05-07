//! `AppContext` — the unified `&mut` surface threaded through every
//! delivery (input handler, pulse, deferred closure).
//!
//! Built fresh from `&mut Application` for the duration of one delivery.
//! No `Arc`, no `RefCell`; single owner, single thread, nothing to lock.

use std::collections::VecDeque;

use devix_core::{
    CommandRegistry, Context as EditorContext, Editor, EditorCommand, Keymap, Viewport,
};
use devix_core::{Clipboard, Theme};

use crate::effect::Effect;
use crate::event_sink::EventSink;

pub struct AppContext<'a> {
    pub editor: &'a mut Editor,
    pub commands: &'a CommandRegistry,
    pub keymap: &'a Keymap,
    pub theme: &'a Theme,
    pub clipboard: &'a mut dyn Clipboard,
    pub sink: &'a EventSink,
    pub(crate) effects: &'a mut VecDeque<Effect>,
}

impl AppContext<'_> {
    pub fn request_redraw(&mut self) {
        self.effects.push_back(Effect::Redraw);
    }

    pub fn quit(&mut self) {
        self.effects.push_back(Effect::Quit);
    }

    pub fn defer<F>(&mut self, f: F)
    where
        F: for<'b> FnOnce(&mut AppContext<'b>) + Send + 'static,
    {
        self.effects.push_back(Effect::Run(Box::new(f)));
    }

    /// Invoke an editor command. Bridges to `devix_core::Context`
    /// (which expects an immediate `quit: &mut bool` flag); if the command
    /// sets it, translates to `Effect::Quit` so quit stays deferred at the
    /// runtime layer.
    pub fn run(&mut self, action: &dyn EditorCommand) {
        let viewport = active_viewport(self.editor);
        let mut quit = false;
        {
            let mut cx = EditorContext {
                editor: &mut *self.editor,
                clipboard: &mut *self.clipboard,
                quit: &mut quit,
                viewport,
                commands: self.commands,
            };
            action.invoke(&mut cx);
        }
        if quit {
            self.effects.push_back(Effect::Quit);
        }
        self.effects.push_back(Effect::Redraw);
    }
}

/// Compute the viewport for the active frame: rect from the render cache,
/// gutter width derived from the active document's line count.
pub(crate) fn active_viewport(editor: &Editor) -> Viewport {
    let rect = editor
        .active_frame()
        .and_then(|fid| editor.render_cache.frame_rects.get(&fid).copied())
        .unwrap_or_default();
    let gutter_width = editor
        .active_doc()
        .map(|d| (d.buffer.line_count().to_string().len() as u16) + 2)
        .unwrap_or(0);
    Viewport {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        gutter_width,
    }
}
