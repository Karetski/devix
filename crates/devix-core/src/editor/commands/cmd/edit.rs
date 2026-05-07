//! Edit operations: undo/redo, selection, insertion, deletion, multicursor.

use crate::Action;
use crate::editor::cursor::ScrollMode;

use crate::editor::commands::context::Context;
use crate::editor::commands::dispatch;

pub struct Undo;
impl<'a> Action<Context<'a>> for Undo {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, cid, did)) = ctx.editor.active_ids() else { return };
        if let Some(sel) = ctx.editor.documents[did].undo() {
            ctx.editor.cursors[cid].adopt_selection(sel);
        }
    }
}

pub struct Redo;
impl<'a> Action<Context<'a>> for Redo {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, cid, did)) = ctx.editor.active_ids() else { return };
        if let Some(sel) = ctx.editor.documents[did].redo() {
            ctx.editor.cursors[cid].adopt_selection(sel);
        }
    }
}

pub struct SelectAll;
impl<'a> Action<Context<'a>> for SelectAll {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use devix_text::{Range, Selection};
        let Some((_, cid, did)) = ctx.editor.active_ids() else { return };
        let end = ctx.editor.documents[did].buffer.len_chars();
        ctx.editor.cursors[cid].adopt_selection(Selection::single(Range::new(0, end)));
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

/// Add a point caret one line above the primary head, at the same column
/// (clamped to the new line's width). Repeated presses extend upward.
pub struct AddCursorAbove;
impl<'a> Action<Context<'a>> for AddCursorAbove {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, cid, did)) = ctx.editor.active_ids() else { return };
        let buf = &ctx.editor.documents[did].buffer;
        let head = ctx.editor.cursors[cid].primary().head;
        let line = buf.line_of_char(head);
        if line == 0 { return; }
        let col = buf.col_of_char(head);
        let new_line = line - 1;
        let new_col = col.min(buf.line_len_chars(new_line));
        let new_head = buf.line_start(new_line) + new_col;
        let c = &mut ctx.editor.cursors[cid];
        c.selection.push_range(devix_text::Range::point(new_head));
        c.target_col = None;
        c.scroll_mode = ScrollMode::Anchored;
    }
}

pub struct AddCursorBelow;
impl<'a> Action<Context<'a>> for AddCursorBelow {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, cid, did)) = ctx.editor.active_ids() else { return };
        let buf = &ctx.editor.documents[did].buffer;
        let head = ctx.editor.cursors[cid].primary().head;
        let line = buf.line_of_char(head);
        let max_line = buf.line_count().saturating_sub(1);
        if line >= max_line { return; }
        let col = buf.col_of_char(head);
        let new_line = line + 1;
        let new_col = col.min(buf.line_len_chars(new_line));
        let new_head = buf.line_start(new_line) + new_col;
        let c = &mut ctx.editor.cursors[cid];
        c.selection.push_range(devix_text::Range::point(new_head));
        c.target_col = None;
        c.scroll_mode = ScrollMode::Anchored;
    }
}

/// Esc-equivalent: drop secondary carets back to the primary. With a
/// single, non-empty range, collapse it to a point at the head.
pub struct CollapseSelection;
impl<'a> Action<Context<'a>> for CollapseSelection {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, cid, _)) = ctx.editor.active_ids() else { return };
        let c = &mut ctx.editor.cursors[cid];
        if c.selection.is_multi() {
            c.selection.collapse_to_primary();
        } else {
            c.selection.collapse();
        }
        c.target_col = None;
        c.scroll_mode = ScrollMode::Anchored;
    }
}

pub struct InsertChar(pub char);
impl<'a> Action<Context<'a>> for InsertChar {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let mut buf = [0u8; 4];
        dispatch::replace_selection(ctx, self.0.encode_utf8(&mut buf));
    }
}
