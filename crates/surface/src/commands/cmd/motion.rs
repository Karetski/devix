//! Cursor motion: char / word / line / doc / page.

use devix_core::Action;

use crate::commands::context::Context;
use crate::commands::dispatch;

pub struct MoveLeft { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveLeft {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, h| b.move_left(h));
    }
}

pub struct MoveRight { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveRight {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, h| b.move_right(h));
    }
}

pub struct MoveUp { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveUp {
    fn invoke(&self, ctx: &mut Context<'a>) {
        dispatch::move_vertical(ctx, false, self.extend);
    }
}

pub struct MoveDown { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveDown {
    fn invoke(&self, ctx: &mut Context<'a>) {
        dispatch::move_vertical(ctx, true, self.extend);
    }
}

pub struct MoveWordLeft { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveWordLeft {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, h| b.word_left(h));
    }
}

pub struct MoveWordRight { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveWordRight {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, h| b.word_right(h));
    }
}

pub struct MoveLineStart { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveLineStart {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, h| b.line_start_of(h));
    }
}

pub struct MoveLineEnd { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveLineEnd {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, h| b.line_end_of(h));
    }
}

pub struct MoveDocStart { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveDocStart {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, _| b.doc_start());
    }
}

pub struct MoveDocEnd { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveDocEnd {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        dispatch::move_to_with(ctx, extend, |b, _| b.doc_end());
    }
}

pub struct PageUp { pub extend: bool }
impl<'a> Action<Context<'a>> for PageUp {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let step = dispatch::page_step(ctx.viewport);
        for _ in 0..step {
            dispatch::move_vertical(ctx, false, self.extend);
        }
    }
}

pub struct PageDown { pub extend: bool }
impl<'a> Action<Context<'a>> for PageDown {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let step = dispatch::page_step(ctx.viewport);
        for _ in 0..step {
            dispatch::move_vertical(ctx, true, self.extend);
        }
    }
}
