//! Action dispatcher.

use devix_buffer::{Buffer, Range, Selection, delete_range_tx, replace_selection_tx};

use crate::action::Action;
use crate::context::{Context, Viewport};

pub fn dispatch(action: Action, cx: &mut Context<'_>) {
    use Action::*;
    match action {
        // ---- motion ----
        MoveLeft { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.move_left(cx.workspace.views[vid].primary().head);
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        MoveRight { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.move_right(cx.workspace.views[vid].primary().head);
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        MoveUp { extend } => move_vertical(cx, false, extend),
        MoveDown { extend } => move_vertical(cx, true, extend),
        MoveWordLeft { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.word_left(cx.workspace.views[vid].primary().head);
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        MoveWordRight { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.word_right(cx.workspace.views[vid].primary().head);
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        MoveLineStart { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.line_start_of(cx.workspace.views[vid].primary().head);
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        MoveLineEnd { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.line_end_of(cx.workspace.views[vid].primary().head);
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        MoveDocStart { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.doc_start();
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        MoveDocEnd { extend } => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let to = cx.workspace.documents[did].buffer.doc_end();
            cx.workspace.views[vid].move_to(to, extend, false);
        }
        PageUp { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step { move_vertical(cx, false, extend); }
        }
        PageDown { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step { move_vertical(cx, true, extend); }
        }

        // ---- edits ----
        InsertChar(c) => {
            let mut buf = [0u8; 4];
            replace_selection(cx, c.encode_utf8(&mut buf));
        }
        InsertNewline => replace_selection(cx, "\n"),
        InsertTab => replace_selection(cx, "    "),
        DeleteBack { word } => delete_primary_or(cx, |buf, head| {
            if head == 0 { return None; }
            let start = if word { buf.word_left(head) } else { head - 1 };
            Some((start, head))
        }),
        DeleteForward { word } => delete_primary_or(cx, |buf, head| {
            let len = buf.len_chars();
            if head >= len { return None; }
            let end = if word { buf.word_right(head) } else { head + 1 };
            Some((head, end))
        }),

        // ---- history ----
        Undo => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            if let Some(sel) = cx.workspace.documents[did].buffer.undo() {
                let v = &mut cx.workspace.views[vid];
                v.selection = sel;
                v.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to undo");
            }
        }
        Redo => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            if let Some(sel) = cx.workspace.documents[did].buffer.redo() {
                let v = &mut cx.workspace.views[vid];
                v.selection = sel;
                v.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to redo");
            }
        }

        // ---- selection ----
        SelectAll => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let end = cx.workspace.documents[did].buffer.len_chars();
            let v = &mut cx.workspace.views[vid];
            v.selection = Selection::single(Range::new(0, end));
            v.target_col = None;
        }

        // ---- clipboard ----
        Copy => do_copy(cx),
        Cut => do_cut(cx),
        Paste => do_paste(cx),

        // ---- file / disk ----
        Save => {
            let Some(d) = cx.workspace.active_doc_mut() else { return };
            let msg = match d.buffer.save() {
                Ok(()) => "saved".to_string(),
                Err(e) => format!("save failed: {e}"),
            };
            cx.status.set(msg);
        }
        ReloadFromDisk => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let res = cx.workspace.documents[did].buffer.reload_from_disk();
            match res {
                Ok(()) => {
                    let max = cx.workspace.documents[did].buffer.len_chars();
                    cx.workspace.documents[did].disk_changed_pending = false;
                    cx.workspace.views[vid].selection.clamp(max);
                    cx.status.set("reloaded from disk");
                }
                Err(e) => cx.status.set(format!("reload failed: {e}")),
            }
        }
        KeepBufferIgnoreDisk => {
            if let Some(d) = cx.workspace.active_doc_mut() {
                d.disk_changed_pending = false;
            }
            cx.status.set("kept buffer; disk change ignored");
        }

        // ---- tabs ----
        NewTab => cx.workspace.new_tab(),
        CloseTab => {
            if !cx.workspace.close_active_tab(false) {
                cx.status.set("unsaved changes — Ctrl+S to save, Ctrl+Shift+W to force close");
            } else {
                cx.status.clear();
            }
        }
        ForceCloseTab => { cx.workspace.close_active_tab(true); cx.status.clear(); }
        NextTab => cx.workspace.next_tab(),
        PrevTab => cx.workspace.prev_tab(),
        OpenPath(p) => match cx.workspace.open_path_replace_current(p) {
            Ok(_) => cx.status.clear(),
            Err(e) => cx.status.set(format!("open failed: {e}")),
        },

        // ---- splits / frames ----
        SplitVertical => cx.workspace.split_active(crate::layout::Axis::Horizontal),
        SplitHorizontal => cx.workspace.split_active(crate::layout::Axis::Vertical),
        CloseFrame => cx.workspace.close_active_frame(),
        ToggleSidebar(slot) => cx.workspace.toggle_sidebar(slot),
        FocusDir(d) => cx.workspace.focus_dir(d),

        // ---- app ----
        Quit => *cx.quit = true,

        // ---- mouse ----
        ClickAt { col, row, extend } => {
            let Some(idx) = click_to_char_idx(cx, col, row) else { return };
            if let Some(v) = cx.workspace.active_view_mut() {
                v.move_to(idx, extend, false);
            }
        }
        DragAt { col, row } => {
            let Some(idx) = click_to_char_idx(cx, col, row) else { return };
            if let Some(v) = cx.workspace.active_view_mut() {
                v.move_to(idx, true, false);
            }
        }
        ScrollBy(delta) => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let max = cx.workspace.documents[did].buffer.line_count().saturating_sub(1) as isize;
            let v = &mut cx.workspace.views[vid];
            let next = (v.scroll_top as isize).saturating_add(delta);
            v.scroll_top = next.clamp(0, max) as usize;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn page_step(v: Viewport) -> usize { v.height.saturating_sub(1).max(1) as usize }

fn move_vertical(cx: &mut Context<'_>, down: bool, extend: bool) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let head = cx.workspace.views[vid].primary().head;
    let col = cx.workspace.views[vid]
        .target_col
        .unwrap_or_else(|| cx.workspace.documents[did].buffer.col_of_char(head));
    let new = if down {
        cx.workspace.documents[did].buffer.move_down(head, Some(col))
    } else {
        cx.workspace.documents[did].buffer.move_up(head, Some(col))
    };
    let v = &mut cx.workspace.views[vid];
    v.target_col = Some(col);
    v.move_to(new, extend, true);
}

fn replace_selection(cx: &mut Context<'_>, text: &str) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let tx = replace_selection_tx(
        &cx.workspace.documents[did].buffer,
        &cx.workspace.views[vid].selection,
        text,
    );
    let after = tx.selection_after.clone();
    cx.workspace.documents[did].buffer.apply(tx);
    let v = &mut cx.workspace.views[vid];
    v.selection = after;
    v.target_col = None;
    cx.status.clear();
}

fn delete_primary_or(
    cx: &mut Context<'_>,
    builder: impl FnOnce(&Buffer, usize) -> Option<(usize, usize)>,
) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let prim = cx.workspace.views[vid].primary();
    let (start, end) = if !prim.is_empty() {
        (prim.start(), prim.end())
    } else {
        let Some(span) = builder(&cx.workspace.documents[did].buffer, prim.head) else { return };
        if span.0 == span.1 { return; }
        span
    };
    let tx = delete_range_tx(
        &cx.workspace.documents[did].buffer,
        &cx.workspace.views[vid].selection,
        start, end,
    );
    let after = tx.selection_after.clone();
    cx.workspace.documents[did].buffer.apply(tx);
    let v = &mut cx.workspace.views[vid];
    v.selection = after;
    v.target_col = None;
    cx.status.clear();
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

fn do_copy(cx: &mut Context<'_>) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let prim = cx.workspace.views[vid].primary();
    let (start, end, msg) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.workspace.documents[did].buffer, prim.head);
        (s, e, "copied line")
    } else {
        (prim.start(), prim.end(), "copied")
    };
    if start == end { return; }
    let text = cx.workspace.documents[did].buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard"); return;
    };
    if cb.set_text(text).is_err() { cx.status.set("clipboard error"); return; }
    cx.status.set(msg);
}

fn do_cut(cx: &mut Context<'_>) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let prim = cx.workspace.views[vid].primary();
    let (start, end, line_cut) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.workspace.documents[did].buffer, prim.head);
        (s, e, true)
    } else {
        (prim.start(), prim.end(), false)
    };
    if start == end { return; }
    let text = cx.workspace.documents[did].buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard"); return;
    };
    if cb.set_text(text).is_err() { cx.status.set("clipboard error"); return; }
    let tx = delete_range_tx(
        &cx.workspace.documents[did].buffer,
        &cx.workspace.views[vid].selection,
        start, end,
    );
    let after = tx.selection_after.clone();
    cx.workspace.documents[did].buffer.apply(tx);
    let v = &mut cx.workspace.views[vid];
    v.selection = after;
    v.target_col = None;
    cx.status.set(if line_cut { "cut line" } else { "cut" });
}

fn do_paste(cx: &mut Context<'_>) {
    let text = match cx.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
        Some(t) => t,
        None => { cx.status.set("clipboard empty"); return; }
    };
    if text.is_empty() { return; }
    replace_selection(cx, &text);
    cx.status.set("pasted");
}

fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
    let v = cx.viewport;
    if row < v.y || row >= v.y + v.height { return None; }
    let text_x = v.x + v.gutter_width;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - v.y) as usize;
    let view = cx.workspace.active_view()?;
    let buf = &cx.workspace.documents.get(view.doc)?.buffer;
    let line = (view.scroll_top + row_in_view).min(buf.line_count().saturating_sub(1));
    let local_col = click_col.min(buf.line_len_chars(line));
    Some(buf.line_start(line) + local_col)
}
