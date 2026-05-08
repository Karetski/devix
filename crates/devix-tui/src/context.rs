//! `AppContext` — the unified `&mut` surface threaded through every
//! delivery (input handler, typed-pulse dispatch).
//!
//! Built fresh from `&mut Application` for the duration of one delivery.
//! No `Arc`, no `RefCell`; single owner, single thread, nothing to lock.
//!
//! T-63 retired the `Effect` enum: `request_redraw` and `quit` flip
//! direct flags on the context (which `Application::flush_context_flags`
//! folds back into the runtime's `dirty` / `quit` after each delivery
//! returns). The previous `Effect::Run(closure)` escape hatch is gone —
//! the one site that used it (wheel-scroll coalescing) was inlined.

use devix_core::{
    CommandRegistry, Context as EditorContext, Editor, EditorCommand, Keymap, RenderCache,
    Viewport,
};
use devix_core::Clipboard;

use crate::event_sink::EventSink;

pub struct AppContext<'a> {
    pub editor: &'a mut Editor,
    pub commands: &'a CommandRegistry,
    pub keymap: &'a Keymap,
    pub clipboard: &'a mut dyn Clipboard,
    pub sink: &'a EventSink,
    /// Layout/render cache. Lives on `Application` (T-92 carved it
    /// out of `Editor` — the cache is tui-internal). Commands read
    /// it for hit-testing and frame-rect queries.
    pub layout_cache: &'a RenderCache,
    /// Set by `request_redraw`; the runtime ORs this into its `dirty`
    /// flag after the delivery returns.
    pub(crate) dirty_request: &'a mut bool,
    /// Set by `quit`; the runtime ORs this into its `quit` flag.
    pub(crate) quit_request: &'a mut bool,
}

impl AppContext<'_> {
    pub fn request_redraw(&mut self) {
        *self.dirty_request = true;
    }

    pub fn quit(&mut self) {
        *self.quit_request = true;
    }

    /// Invoke an editor command.
    pub fn run(&mut self, action: &dyn EditorCommand) {
        let viewport = active_viewport(self.editor, self.layout_cache);
        let mut quit = false;
        {
            let mut cx = EditorContext {
                editor: &mut *self.editor,
                clipboard: &mut *self.clipboard,
                quit: &mut quit,
                viewport,
                commands: self.commands,
                layout_cache: self.layout_cache,
            };
            action.invoke(&mut cx);
        }
        if quit {
            *self.quit_request = true;
        }
        *self.dirty_request = true;
    }
}

/// Compute the viewport for the active frame: rect from the render
/// cache, gutter width derived from the active document's line count.
pub(crate) fn active_viewport(editor: &Editor, cache: &RenderCache) -> Viewport {
    let rect = editor
        .active_frame()
        .and_then(|fid| cache.frame_rects.get(&fid).copied())
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
