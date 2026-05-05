//! Editor command helpers — shared utilities used by the `cmd` struct
//! impls. These are the bits of dispatch logic that benefit from being
//! shared across multiple commands.

use devix_text::{Buffer, delete_each_tx, delete_range_tx, replace_selection_tx};

use crate::commands::context::{Context, Viewport};
use crate::cursor::{Cursor, ScrollMode};

pub(crate) fn page_step(v: Viewport) -> usize { v.height.saturating_sub(1).max(1) as usize }

/// Apply a single-axis motion to every range. The motion sees each
/// range's head; with `extend`, only the head moves (anchor stays put);
/// without it, the range collapses to the new head.
pub(crate) fn move_to_with(
    cx: &mut Context<'_>,
    extend: bool,
    motion: impl Fn(&Buffer, usize) -> usize,
) {
    let Some((_, cid, did)) = cx.editor.active_ids() else { return };
    let buf = &cx.editor.documents[did].buffer;
    let mut sel = cx.editor.cursors[cid].selection.clone();
    sel.transform(|r| {
        let to = motion(buf, r.head);
        r.put_head(to, extend)
    });
    sel.normalize();
    let c = &mut cx.editor.cursors[cid];
    c.selection = sel;
    c.target_col = None;
    c.scroll_mode = ScrollMode::Anchored;
}

/// Vertical motion. With a single caret the sticky-column behavior on
/// `Cursor` keeps repeated Up/Down stable across short lines.
pub(crate) fn move_vertical(cx: &mut Context<'_>, down: bool, extend: bool) {
    let Some((_, cid, did)) = cx.editor.active_ids() else { return };
    let buf = &cx.editor.documents[did].buffer;
    let single = !cx.editor.cursors[cid].selection.is_multi();
    let sticky = cx.editor.cursors[cid].target_col;
    let mut sel = cx.editor.cursors[cid].selection.clone();

    let primary_idx = sel.primary_index();
    let primary_col_for_sticky = if single {
        Some(sticky.unwrap_or_else(|| buf.col_of_char(sel.primary().head)))
    } else {
        None
    };

    let mut i = 0usize;
    sel.transform(|r| {
        let col = if i == primary_idx {
            primary_col_for_sticky.unwrap_or_else(|| buf.col_of_char(r.head))
        } else {
            buf.col_of_char(r.head)
        };
        let new = if down {
            buf.move_down(r.head, Some(col))
        } else {
            buf.move_up(r.head, Some(col))
        };
        i += 1;
        r.put_head(new, extend)
    });
    sel.normalize();

    let c = &mut cx.editor.cursors[cid];
    c.selection = sel;
    c.target_col = primary_col_for_sticky;
    c.scroll_mode = ScrollMode::Anchored;
}

pub(crate) fn replace_selection(cx: &mut Context<'_>, text: &str) {
    let Some((_, cid, did)) = cx.editor.active_ids() else { return };
    let tx = replace_selection_tx(
        &cx.editor.documents[did].buffer,
        &cx.editor.cursors[cid].selection,
        text,
    );
    let after = tx.selection_after.clone();
    cx.editor.documents[did].apply_tx(tx);
    let c = &mut cx.editor.cursors[cid];
    c.selection = after;
    reset_motion_state(c);
}

/// Per-range delete. For each range: if non-empty, delete its span; if
/// empty (point caret), call `builder` to compute a 1-char-or-word span
/// to delete. All resulting changes bundle into one transaction.
pub(crate) fn delete_each_or(
    cx: &mut Context<'_>,
    builder: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) {
    let Some((_, cid, did)) = cx.editor.active_ids() else { return };
    let buf = &cx.editor.documents[did].buffer;
    let sel = cx.editor.cursors[cid].selection.clone();
    let tx = delete_each_tx(&sel, |r| {
        if !r.is_empty() {
            return Some((r.start(), r.end()));
        }
        let span = builder(buf, r.head)?;
        if span.0 == span.1 { None } else { Some(span) }
    });
    if tx.changes.is_empty() {
        return;
    }
    let after = tx.selection_after.clone();
    cx.editor.documents[did].apply_tx(tx);
    let c = &mut cx.editor.cursors[cid];
    c.selection = after;
    reset_motion_state(c);
}

fn reset_motion_state(c: &mut Cursor) {
    c.target_col = None;
    c.scroll_mode = ScrollMode::Anchored;
}

fn current_line_span(buf: &Buffer, head: usize) -> (usize, usize) {
    let line = buf.line_of_char(head);
    let start = buf.line_start(line);
    let end_no_nl = start + buf.line_len_chars(line);
    let end = if line + 1 < buf.line_count() {
        buf.line_start(line + 1)
    } else {
        end_no_nl
    };
    (start, end)
}

pub(crate) fn do_copy(cx: &mut Context<'_>) {
    let Some((_, cid, did)) = cx.editor.active_ids() else { return };
    let prim = cx.editor.cursors[cid].primary();
    let (start, end) = if prim.is_empty() {
        current_line_span(&cx.editor.documents[did].buffer, prim.head)
    } else {
        (prim.start(), prim.end())
    };
    if start == end { return; }
    let text = cx.editor.documents[did].buffer.slice_to_string(start, end);
    let _ = cx.clipboard.set_text(text);
}

pub(crate) fn do_cut(cx: &mut Context<'_>) {
    let Some((_, cid, did)) = cx.editor.active_ids() else { return };
    let prim = cx.editor.cursors[cid].primary();
    let (start, end) = if prim.is_empty() {
        current_line_span(&cx.editor.documents[did].buffer, prim.head)
    } else {
        (prim.start(), prim.end())
    };
    if start == end { return; }
    let text = cx.editor.documents[did].buffer.slice_to_string(start, end);
    if !cx.clipboard.set_text(text) { return; }
    let tx = delete_range_tx(
        &cx.editor.documents[did].buffer,
        &cx.editor.cursors[cid].selection,
        start, end,
    );
    let after = tx.selection_after.clone();
    cx.editor.documents[did].apply_tx(tx);
    cx.editor.cursors[cid].adopt_selection(after);
}

pub(crate) fn do_paste(cx: &mut Context<'_>) {
    let text = match cx.clipboard.get_text() {
        Some(t) => t,
        None => return,
    };
    if text.is_empty() { return; }
    replace_selection(cx, &text);
}

pub(crate) fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
    let v = cx.viewport;
    if row < v.y || row >= v.y + v.height { return None; }
    let text_x = v.x + v.gutter_width;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - v.y) as usize;
    let cursor = cx.editor.active_cursor()?;
    let buf = &cx.editor.documents.get(cursor.doc)?.buffer;
    let line = (cursor.scroll_top() + row_in_view).min(buf.line_count().saturating_sub(1));
    let local_col = click_col.min(buf.line_len_chars(line));
    Some(buf.line_start(line) + local_col)
}
