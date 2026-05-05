//! Edit operations: undo/redo, selection, insertion, deletion, multicursor.

use devix_core::Action;
use devix_surface::view::ScrollMode;

use crate::context::Context;
use crate::dispatch;

pub struct Undo;
impl<'a> Action<Context<'a>> for Undo {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        if let Some(sel) = ctx.surface.documents[did].undo() {
            ctx.surface.views[vid].adopt_selection(sel);
        }
    }
}

pub struct Redo;
impl<'a> Action<Context<'a>> for Redo {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        if let Some(sel) = ctx.surface.documents[did].redo() {
            ctx.surface.views[vid].adopt_selection(sel);
        }
    }
}

pub struct SelectAll;
impl<'a> Action<Context<'a>> for SelectAll {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use devix_text::{Range, Selection};
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        let end = ctx.surface.documents[did].buffer.len_chars();
        ctx.surface.views[vid].adopt_selection(Selection::single(Range::new(0, end)));
    }
}

pub struct InsertNewline;
impl<'a> Action<Context<'a>> for InsertNewline {
    fn invoke(&self, ctx: &mut Context<'a>) {
        dispatch::replace_selection(ctx, "\n");
    }
}

pub struct InsertTab;
impl<'a> Action<Context<'a>> for InsertTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        dispatch::replace_selection(ctx, "    ");
    }
}

pub struct DeleteBack { pub word: bool }
impl<'a> Action<Context<'a>> for DeleteBack {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let word = self.word;
        dispatch::delete_each_or(ctx, |buf, head| {
            if head == 0 {
                return None;
            }
            let start = if word { buf.word_left(head) } else { head - 1 };
            Some((start, head))
        });
    }
}

pub struct DeleteForward { pub word: bool }
impl<'a> Action<Context<'a>> for DeleteForward {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let word = self.word;
        dispatch::delete_each_or(ctx, |buf, head| {
            let len = buf.len_chars();
            if head >= len {
                return None;
            }
            let end = if word { buf.word_right(head) } else { head + 1 };
            Some((head, end))
        });
    }
}

/// Add a point cursor one line above the primary head, at the same column
/// (clamped to the new line's width). Repeated presses extend upward.
pub struct AddCursorAbove;
impl<'a> Action<Context<'a>> for AddCursorAbove {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        let buf = &ctx.surface.documents[did].buffer;
        let head = ctx.surface.views[vid].primary().head;
        let line = buf.line_of_char(head);
        if line == 0 { return; }
        let col = buf.col_of_char(head);
        let new_line = line - 1;
        let new_col = col.min(buf.line_len_chars(new_line));
        let new_head = buf.line_start(new_line) + new_col;
        let v = &mut ctx.surface.views[vid];
        v.selection.push_range(devix_text::Range::point(new_head));
        v.target_col = None;
        v.scroll_mode = ScrollMode::Anchored;
    }
}

pub struct AddCursorBelow;
impl<'a> Action<Context<'a>> for AddCursorBelow {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        let buf = &ctx.surface.documents[did].buffer;
        let head = ctx.surface.views[vid].primary().head;
        let line = buf.line_of_char(head);
        let max_line = buf.line_count().saturating_sub(1);
        if line >= max_line { return; }
        let col = buf.col_of_char(head);
        let new_line = line + 1;
        let new_col = col.min(buf.line_len_chars(new_line));
        let new_head = buf.line_start(new_line) + new_col;
        let v = &mut ctx.surface.views[vid];
        v.selection.push_range(devix_text::Range::point(new_head));
        v.target_col = None;
        v.scroll_mode = ScrollMode::Anchored;
    }
}

/// Esc-equivalent: drop secondary cursors back to the primary. With a
/// single, non-empty range, collapse it to a point at the head.
pub struct CollapseSelection;
impl<'a> Action<Context<'a>> for CollapseSelection {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, _)) = ctx.surface.active_ids() else { return };
        let v = &mut ctx.surface.views[vid];
        if v.selection.is_multi() {
            v.selection.collapse_to_primary();
        } else {
            v.selection.collapse();
        }
        v.target_col = None;
        v.scroll_mode = ScrollMode::Anchored;
    }
}

pub struct InsertChar(pub char);
impl<'a> Action<Context<'a>> for InsertChar {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let mut buf = [0u8; 4];
        dispatch::replace_selection(ctx, self.0.encode_utf8(&mut buf));
    }
}
