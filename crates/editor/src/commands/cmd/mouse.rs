//! Mouse-driven commands: scroll, click, drag.

use devix_core::Action;
use crate::cursor::ScrollMode;

use crate::commands::context::Context;
use crate::commands::dispatch;

pub struct ScrollBy(pub isize);
impl<'a> Action<Context<'a>> for ScrollBy {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, cid, did)) = ctx.editor.active_ids() else { return };
        let line_count = ctx.editor.documents[did].buffer.line_count();
        let c = &mut ctx.editor.cursors[cid];
        let max_top = line_count.saturating_sub(1);
        let next = (c.scroll_top() as isize).saturating_add(self.0);
        let clamped = next.clamp(0, max_top as isize) as usize;
        c.set_scroll_top(clamped);
        c.scroll_mode = ScrollMode::Free;
    }
}

pub struct ClickAt {
    pub col: u16,
    pub row: u16,
    pub extend: bool,
}
impl<'a> Action<Context<'a>> for ClickAt {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.editor.focus_at_screen(self.col, self.row);
        let Some(idx) = dispatch::click_to_char_idx(ctx, self.col, self.row) else {
            return;
        };
        if let Some(c) = ctx.editor.active_cursor_mut() {
            c.move_to(idx, self.extend, false);
        }
    }
}

pub struct DragAt { pub col: u16, pub row: u16 }
impl<'a> Action<Context<'a>> for DragAt {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some(idx) = dispatch::click_to_char_idx(ctx, self.col, self.row) else {
            return;
        };
        if let Some(c) = ctx.editor.active_cursor_mut() {
            c.move_to(idx, true, false);
        }
    }
}
