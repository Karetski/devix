//! Mouse-driven commands: scroll, click, drag.

use devix_core::Action;
use devix_surface::view::ScrollMode;

use crate::context::Context;
use crate::dispatch;

pub struct ScrollBy(pub isize);
impl<'a> Action<Context<'a>> for ScrollBy {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        let line_count = ctx.surface.documents[did].buffer.line_count();
        let v = &mut ctx.surface.views[vid];
        let max_top = line_count.saturating_sub(1);
        let next = (v.scroll_top() as isize).saturating_add(self.0);
        let clamped = next.clamp(0, max_top as isize) as usize;
        v.set_scroll_top(clamped);
        v.scroll_mode = ScrollMode::Free;
    }
}

pub struct ClickAt {
    pub col: u16,
    pub row: u16,
    pub extend: bool,
}
impl<'a> Action<Context<'a>> for ClickAt {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.focus_at_screen(self.col, self.row);
        let Some(idx) = dispatch::click_to_char_idx(ctx, self.col, self.row) else {
            return;
        };
        if let Some(v) = ctx.surface.active_view_mut() {
            v.move_to(idx, self.extend, false);
        }
    }
}

pub struct DragAt { pub col: u16, pub row: u16 }
impl<'a> Action<Context<'a>> for DragAt {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some(idx) = dispatch::click_to_char_idx(ctx, self.col, self.row) else {
            return;
        };
        if let Some(v) = ctx.surface.active_view_mut() {
            v.move_to(idx, true, false);
        }
    }
}
