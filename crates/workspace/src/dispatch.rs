//! Action dispatcher. Single flat match; the only place that mutates Context
//! state in response to commands.

use devix_buffer::{Buffer, Range, Selection, delete_range_tx, replace_selection_tx};

use crate::action::Action;
use crate::context::{Context, Viewport};

pub fn dispatch(action: Action, cx: &mut Context<'_>) {
    use Action::*;
    match action {
        // ---- motion ----
        MoveLeft { extend } => {
            let to = cx.editor.buffer.move_left(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveRight { extend } => {
            let to = cx.editor.buffer.move_right(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveUp { extend } => cx.editor.move_vertical(false, extend),
        MoveDown { extend } => cx.editor.move_vertical(true, extend),
        MoveWordLeft { extend } => {
            let to = cx.editor.buffer.word_left(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveWordRight { extend } => {
            let to = cx.editor.buffer.word_right(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveLineStart { extend } => {
            let to = cx.editor.buffer.line_start_of(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveLineEnd { extend } => {
            let to = cx.editor.buffer.line_end_of(cx.editor.primary().head);
            cx.editor.move_to(to, extend, false);
        }
        MoveDocStart { extend } => {
            let to = cx.editor.buffer.doc_start();
            cx.editor.move_to(to, extend, false);
        }
        MoveDocEnd { extend } => {
            let to = cx.editor.buffer.doc_end();
            cx.editor.move_to(to, extend, false);
        }
        PageUp { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step {
                cx.editor.move_vertical(false, extend);
            }
        }
        PageDown { extend } => {
            let step = page_step(cx.viewport);
            for _ in 0..step {
                cx.editor.move_vertical(true, extend);
            }
        }

        // ---- edits ----
        InsertChar(c) => {
            let mut buf = [0u8; 4];
            replace_selection(cx, c.encode_utf8(&mut buf));
        }
        InsertNewline => replace_selection(cx, "\n"),
        InsertTab => replace_selection(cx, "    "),
        DeleteBack { word } => {
            delete_primary_or(cx, |buf, head| {
                if head == 0 {
                    return None;
                }
                let start = if word { buf.word_left(head) } else { head - 1 };
                Some((start, head))
            });
        }
        DeleteForward { word } => {
            delete_primary_or(cx, |buf, head| {
                let len = buf.len_chars();
                if head >= len {
                    return None;
                }
                let end = if word { buf.word_right(head) } else { head + 1 };
                Some((head, end))
            });
        }

        // ---- history ----
        Undo => {
            if let Some(sel) = cx.editor.buffer.undo() {
                cx.editor.selection = sel;
                cx.editor.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to undo");
            }
        }
        Redo => {
            if let Some(sel) = cx.editor.buffer.redo() {
                cx.editor.selection = sel;
                cx.editor.target_col = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to redo");
            }
        }

        // ---- selection ----
        SelectAll => {
            let end = cx.editor.buffer.len_chars();
            cx.editor.selection = Selection::single(Range::new(0, end));
            cx.editor.target_col = None;
        }

        // ---- clipboard ----
        Copy => do_copy(cx),
        Cut => do_cut(cx),
        Paste => do_paste(cx),

        // ---- file / disk ----
        Save => {
            let msg = match cx.editor.buffer.save() {
                Ok(()) => "saved".to_string(),
                Err(e) => format!("save failed: {e}"),
            };
            cx.status.set(msg);
        }
        ReloadFromDisk => match cx.editor.buffer.reload_from_disk() {
            Ok(()) => {
                let max = cx.editor.buffer.len_chars();
                cx.editor.selection.clamp(max);
                *cx.disk_changed_pending = false;
                cx.status.set("reloaded from disk");
            }
            Err(e) => cx.status.set(format!("reload failed: {e}")),
        },
        KeepBufferIgnoreDisk => {
            *cx.disk_changed_pending = false;
            cx.status.set("kept buffer; disk change ignored");
        }

        // ---- app ----
        Quit => *cx.quit = true,

        // ---- mouse ----
        ClickAt { col, row, extend } => {
            if let Some(idx) = click_to_char_idx(cx, col, row) {
                cx.editor.move_to(idx, extend, false);
            }
        }
        DragAt { col, row } => {
            if let Some(idx) = click_to_char_idx(cx, col, row) {
                cx.editor.move_to(idx, true, false);
            }
        }
        // One line per event matches macOS trackpad behavior: the OS already
        // emits one event per line of intended scroll distance (including the
        // inertia tail). Multiplying here would overshoot.
        ScrollUp => {
            cx.editor.scroll_top = cx.editor.scroll_top.saturating_sub(1);
        }
        ScrollDown => {
            let max = cx.editor.buffer.line_count().saturating_sub(1);
            cx.editor.scroll_top = (cx.editor.scroll_top + 1).min(max);
        }
    }
}

// ---------------------------------------------------------------------------
// Edit helpers
// ---------------------------------------------------------------------------

fn replace_selection(cx: &mut Context<'_>, text: &str) {
    let tx = replace_selection_tx(&cx.editor.buffer, &cx.editor.selection, text);
    cx.editor.apply_tx(tx);
    cx.status.clear();
}

fn delete_primary_or(
    cx: &mut Context<'_>,
    builder: impl FnOnce(&Buffer, usize) -> Option<(usize, usize)>,
) {
    let prim = cx.editor.primary();
    if !prim.is_empty() {
        let tx = delete_range_tx(
            &cx.editor.buffer,
            &cx.editor.selection,
            prim.start(),
            prim.end(),
        );
        cx.editor.apply_tx(tx);
        cx.status.clear();
        return;
    }
    let Some((start, end)) = builder(&cx.editor.buffer, prim.head) else {
        return;
    };
    if start == end {
        return;
    }
    let tx = delete_range_tx(&cx.editor.buffer, &cx.editor.selection, start, end);
    cx.editor.apply_tx(tx);
    cx.status.clear();
}

// ---------------------------------------------------------------------------
// Clipboard handlers
// ---------------------------------------------------------------------------

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
    let prim = cx.editor.primary();
    let (start, end, msg) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.editor.buffer, prim.head);
        (s, e, "copied line")
    } else {
        (prim.start(), prim.end(), "copied")
    };
    if start == end {
        return;
    }
    let text = cx.editor.buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard");
        return;
    };
    if cb.set_text(text).is_err() {
        cx.status.set("clipboard error");
        return;
    }
    cx.status.set(msg);
}

fn do_cut(cx: &mut Context<'_>) {
    let prim = cx.editor.primary();
    let (start, end, line_cut) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.editor.buffer, prim.head);
        (s, e, true)
    } else {
        (prim.start(), prim.end(), false)
    };
    if start == end {
        return;
    }
    let text = cx.editor.buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard");
        return;
    };
    if cb.set_text(text).is_err() {
        cx.status.set("clipboard error");
        return;
    }
    let tx = delete_range_tx(&cx.editor.buffer, &cx.editor.selection, start, end);
    cx.editor.apply_tx(tx);
    cx.status.set(if line_cut { "cut line" } else { "cut" });
}

fn do_paste(cx: &mut Context<'_>) {
    let text = match cx.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) {
        Some(t) => t,
        None => {
            cx.status.set("clipboard empty");
            return;
        }
    };
    if text.is_empty() {
        return;
    }
    replace_selection(cx, &text);
    cx.status.set("pasted");
}

// ---------------------------------------------------------------------------
// Mouse helpers
// ---------------------------------------------------------------------------

fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
    let v = cx.viewport;
    if row < v.y || row >= v.y + v.height {
        return None;
    }
    let text_x = v.x + v.gutter_width;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - v.y) as usize;
    let buf = &cx.editor.buffer;
    let line = (cx.editor.scroll_top + row_in_view).min(buf.line_count().saturating_sub(1));
    let local_col = click_col.min(buf.line_len_chars(line));
    Some(buf.line_start(line) + local_col)
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

fn page_step(viewport: Viewport) -> usize {
    viewport.height.saturating_sub(1).max(1) as usize
}
