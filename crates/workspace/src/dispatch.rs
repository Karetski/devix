//! Action dispatcher.

use devix_buffer::{Buffer, Range, Selection, delete_range_tx, replace_selection_tx};
use devix_lsp::{CompletionTrigger, LspCommand, position_in_rope};
use lsp_types::CompletionTextEdit;

use crate::action::Action;
use crate::context::{Context, Viewport};
use crate::overlay::{Overlay, PaletteState};
use crate::view::{CompletionState, CompletionStatus, HoverState, HoverStatus, ViewId};
use crate::workspace::Workspace;

/// Trigger characters that auto-fire completion. Hardcoded for slice 3;
/// a follow-up will surface the server's `ServerCapabilities.completion_provider
/// .trigger_characters` list per (root, language) so this can specialize.
const TRIGGER_CHARS: &[char] = &['.', ':'];

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
            // Preserve completion across the insert: replace_selection nukes
            // it via the helper's reset, so save first, restore + refilter
            // after. Triggers ('.', ':') fire a fresh request regardless.
            let saved = take_completion(cx);
            let mut buf = [0u8; 4];
            replace_selection(cx, c.encode_utf8(&mut buf));
            if TRIGGER_CHARS.contains(&c) {
                drop(saved); // trigger replaces any prior popup
                dispatch(Action::TriggerCompletion, cx);
            } else if let Some(state) = saved {
                if let Some((_, vid, _)) = cx.workspace.active_ids() {
                    cx.workspace.views[vid].completion = Some(state);
                    refilter_completion(cx.workspace, vid);
                }
            }
        }
        InsertNewline => {
            dismiss_completion(cx);
            replace_selection(cx, "\n");
        }
        InsertTab => {
            dismiss_completion(cx);
            replace_selection(cx, "    ");
        }
        DeleteBack { word } => {
            let keep_completion = !word;
            let saved = if keep_completion { take_completion(cx) } else { None };
            delete_primary_or(cx, |buf, head| {
                if head == 0 { return None; }
                let start = if word { buf.word_left(head) } else { head - 1 };
                Some((start, head))
            });
            if let Some(state) = saved {
                if let Some((_, vid, _)) = cx.workspace.active_ids() {
                    cx.workspace.views[vid].completion = Some(state);
                    refilter_completion(cx.workspace, vid);
                }
            }
        }
        DeleteForward { word } => {
            // Forward delete past the cursor never extends the typed query
            // backward, so it always dismisses.
            let _ = word;
            dismiss_completion(cx);
            delete_primary_or(cx, |buf, head| {
                let len = buf.len_chars();
                if head >= len { return None; }
                let end = if word { buf.word_right(head) } else { head + 1 };
                Some((head, end))
            });
        }

        // ---- history ----
        Undo => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            if let Some(sel) = cx.workspace.documents[did].undo() {
                let v = &mut cx.workspace.views[vid];
                v.selection = sel;
                v.target_col = None;
                v.hover = None;
                v.completion = None;
                cx.status.clear();
            } else {
                cx.status.set("nothing to undo");
            }
        }
        Redo => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            if let Some(sel) = cx.workspace.documents[did].redo() {
                let v = &mut cx.workspace.views[vid];
                v.selection = sel;
                v.target_col = None;
                v.hover = None;
                v.completion = None;
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
            v.hover = None;
            v.completion = None;
        }

        // ---- clipboard ----
        Copy => do_copy(cx),
        Cut => { dismiss_completion(cx); do_cut(cx); }
        Paste => { dismiss_completion(cx); do_paste(cx); }

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
            let res = cx.workspace.documents[did].reload_from_disk();
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
        // Action names follow user intuition (a "vertical split" creates a
        // vertical dividing line); Axis names describe the layout direction
        // children are arranged in. So a vertical split means children laid out
        // horizontally, and vice versa.
        SplitVertical => cx.workspace.split_active(crate::layout::Axis::Horizontal),
        SplitHorizontal => cx.workspace.split_active(crate::layout::Axis::Vertical),
        CloseFrame => cx.workspace.close_active_frame(),
        ToggleSidebar(slot) => cx.workspace.toggle_sidebar(slot),
        FocusDir(d) => cx.workspace.focus_dir(d),

        // ---- app ----
        Quit => *cx.quit = true,

        // ---- palette overlay ----
        OpenPalette => {
            *cx.overlay = Some(Overlay::Palette(PaletteState::from_registry(cx.commands)));
        }
        ClosePalette => {
            if matches!(cx.overlay, Some(Overlay::Palette(_))) {
                *cx.overlay = None;
            }
        }
        PaletteMove(delta) => {
            if let Some(Overlay::Palette(p)) = cx.overlay.as_mut() {
                p.move_selection(delta);
            }
        }
        PaletteSetQuery(q) => {
            if let Some(Overlay::Palette(p)) = cx.overlay.as_mut() {
                p.set_query(q);
            }
        }
        PaletteAccept => {
            // Snapshot the selection, drop the overlay, then dispatch the
            // chosen command. Recursive dispatch is fine: ClosePalette already
            // ran (overlay = None) so re-entering can't loop.
            let chosen = if let Some(Overlay::Palette(p)) = cx.overlay.as_ref() {
                p.selected_command_id().and_then(|id| cx.commands.resolve(id))
            } else {
                None
            };
            *cx.overlay = None;
            if let Some(action) = chosen {
                dispatch(action, cx);
            }
        }

        // ---- language server ----
        Hover => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let Some(wiring) = cx.workspace.lsp_wiring() else { return };
            let doc = &cx.workspace.documents[did];
            let Some(uri) = doc.lsp_uri().cloned() else { return };
            let head = cx.workspace.views[vid].primary().head;
            let position = position_in_rope(doc.buffer.rope(), head, &wiring.encoding);
            let _ = wiring.sink.send(LspCommand::Hover {
                uri,
                position,
                anchor_char: head,
            });
            cx.workspace.views[vid].hover = Some(HoverState {
                anchor_char: head,
                status: HoverStatus::Pending,
            });
        }
        GotoDefinition => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let Some(wiring) = cx.workspace.lsp_wiring() else { return };
            let doc = &cx.workspace.documents[did];
            let Some(uri) = doc.lsp_uri().cloned() else { return };
            let head = cx.workspace.views[vid].primary().head;
            let position = position_in_rope(doc.buffer.rope(), head, &wiring.encoding);
            let _ = wiring.sink.send(LspCommand::GotoDefinition {
                uri,
                position,
                anchor_char: head,
            });
        }
        TriggerCompletion => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let Some(wiring) = cx.workspace.lsp_wiring() else { return };
            let doc = &cx.workspace.documents[did];
            let Some(uri) = doc.lsp_uri().cloned() else { return };
            let head = cx.workspace.views[vid].primary().head;
            let position = position_in_rope(doc.buffer.rope(), head, &wiring.encoding);
            // Decide whether this trigger is from a typed character or an
            // explicit invocation: peek the char immediately to the cursor's
            // left and check it against TRIGGER_CHARS. This is good enough
            // for slice 3; a more rigorous design threads the trigger
            // through the InsertChar arm.
            let prev_char = if head > 0 {
                doc.buffer.rope().char(head - 1)
            } else {
                '\0'
            };
            let trigger = if TRIGGER_CHARS.contains(&prev_char) {
                CompletionTrigger::Char(prev_char)
            } else {
                CompletionTrigger::Manual
            };
            let _ = wiring.sink.send(LspCommand::Completion {
                uri,
                position,
                anchor_char: head,
                trigger,
            });
            // The query starts where the user can still meaningfully filter.
            // For trigger-char (`.`, `:`), the cursor itself; for manual,
            // walk back over the identifier. ident_start_at handles both.
            let query_start = ident_start_at(&doc.buffer, head);
            cx.workspace.views[vid].completion = Some(CompletionState {
                anchor_char: head,
                query_start,
                items: Vec::new(),
                filtered: Vec::new(),
                selected: 0,
                status: CompletionStatus::Pending,
            });
        }
        CompletionMove(delta) => {
            let Some((_, vid, _)) = cx.workspace.active_ids() else { return };
            let Some(state) = cx.workspace.views[vid].completion.as_mut() else { return };
            if state.filtered.is_empty() { return; }
            let n = state.filtered.len() as isize;
            let cur = state.selected as isize;
            let next = (cur + delta).rem_euclid(n);
            state.selected = next as usize;
        }
        CompletionAccept => {
            apply_completion_accept(cx);
        }
        CompletionDismiss => {
            dismiss_completion(cx);
        }

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
            let line_count = cx.workspace.documents[did].buffer.line_count();
            let v = &mut cx.workspace.views[vid];
            let max = line_count.saturating_sub(1);
            let next = (v.scroll_top() as isize)
                .saturating_add(delta)
                .clamp(0, max as isize) as usize;
            v.set_scroll_top(next);
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
    cx.workspace.documents[did].apply_tx(tx);
    let v = &mut cx.workspace.views[vid];
    v.selection = after;
    v.target_col = None;
    v.hover = None;
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
    cx.workspace.documents[did].apply_tx(tx);
    let v = &mut cx.workspace.views[vid];
    v.selection = after;
    v.target_col = None;
    v.hover = None;
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
    cx.workspace.documents[did].apply_tx(tx);
    let v = &mut cx.workspace.views[vid];
    v.selection = after;
    v.target_col = None;
    v.hover = None;
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

// ---------------------------------------------------------------------------
// Completion helpers
// ---------------------------------------------------------------------------

/// Walk left from `head` while the previous char is an identifier char
/// (alphanumeric or `_`). Used to anchor the completion query at the
/// start of the partially-typed identifier.
fn ident_start_at(buf: &Buffer, head: usize) -> usize {
    let rope = buf.rope();
    let mut i = head;
    while i > 0 {
        let c = rope.char(i - 1);
        if !(c.is_alphanumeric() || c == '_') { break; }
        i -= 1;
    }
    i
}

fn take_completion(cx: &mut Context<'_>) -> Option<CompletionState> {
    let (_, vid, _) = cx.workspace.active_ids()?;
    cx.workspace.views[vid].completion.take()
}

fn dismiss_completion(cx: &mut Context<'_>) {
    let Some((_, vid, _)) = cx.workspace.active_ids() else { return };
    cx.workspace.views[vid].completion = None;
}

/// Re-rank `state.items` against the prefix `query_start..cursor` from the
/// rope. If the cursor moved left of `query_start` (user backspaced past
/// the query origin), drop the popup. Empty-prefix shows everything in
/// server-given order.
pub fn refilter_completion(ws: &mut Workspace, vid: ViewId) {
    let head = ws.views[vid].primary().head;
    let did = ws.views[vid].doc;
    let Some(state) = ws.views[vid].completion.as_mut() else { return };
    if head < state.query_start {
        ws.views[vid].completion = None;
        return;
    }
    let prefix: String = ws.documents[did]
        .buffer
        .rope()
        .slice(state.query_start..head)
        .to_string();
    // If the user typed a non-identifier character into the query span, the
    // completion context is over — dismiss instead of producing a confusing
    // empty popup that "filters" against e.g. "ne ".
    if prefix.chars().any(|c| !(c.is_alphanumeric() || c == '_')) {
        ws.views[vid].completion = None;
        return;
    }
    let mut scored: Vec<(usize, i32)> = Vec::with_capacity(state.items.len());
    if prefix.is_empty() {
        // No prefix yet: keep server order, full list.
        for i in 0..state.items.len() {
            scored.push((i, 0));
        }
    } else {
        let prefix_lower = prefix.to_lowercase();
        for (i, item) in state.items.iter().enumerate() {
            let label_lower = item.label.to_lowercase();
            if let Some(pos) = label_lower.find(&prefix_lower) {
                // Earlier match position scores higher; exact-prefix match
                // ranks above mid-string. Tie-break by shorter label so
                // the canonical name shows first.
                let mut score = 1000 - (pos as i32 * 10);
                if pos == 0 { score += 500; }
                score -= item.label.len() as i32;
                scored.push((i, score));
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1));
    }
    let filtered: Vec<usize> = scored.into_iter().map(|(i, _)| i).collect();
    let state = ws.views[vid].completion.as_mut().unwrap();
    state.filtered = filtered;
    state.selected = 0;
}

/// Apply the highlighted completion item: prefer an explicit `text_edit`
/// when the server provided one (rust-analyzer relies on this for `::`
/// insertions, qualified paths, etc.); otherwise replace the identifier
/// span around the cursor with `insert_text` or `label`.
fn apply_completion_accept(cx: &mut Context<'_>) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let Some(state) = cx.workspace.views[vid].completion.take() else { return };
    let Some(&idx) = state.filtered.get(state.selected) else { return };
    let Some(item) = state.items.get(idx).cloned() else { return };

    let head = cx.workspace.views[vid].primary().head;
    let encoding = cx
        .workspace
        .lsp_wiring()
        .map(|w| w.encoding)
        .unwrap_or(lsp_types::PositionEncodingKind::UTF16);

    let (start, end, new_text) = if let Some(edit) = item.text_edit.as_ref() {
        let (range, txt) = match edit {
            CompletionTextEdit::Edit(e) => (e.range, e.new_text.clone()),
            CompletionTextEdit::InsertAndReplace(ir) => (ir.replace, ir.new_text.clone()),
        };
        let rope = cx.workspace.documents[did].buffer.rope();
        let s = lsp_pos_to_char(rope, range.start.line, range.start.character, &encoding);
        let e = lsp_pos_to_char(rope, range.end.line, range.end.character, &encoding);
        (s, e, txt)
    } else {
        // Fallback: replace the typed-prefix span [query_start, cursor)
        // with the item's insert_text or label.
        let txt = item.insert_text.clone().unwrap_or_else(|| item.label.clone());
        (state.query_start, head, txt)
    };

    let buf = &cx.workspace.documents[did].buffer;
    let len = buf.len_chars();
    let start = start.min(len);
    let end = end.min(len);
    let (start, end) = if start <= end { (start, end) } else { (end, start) };
    let tx = if start == end {
        replace_selection_tx(buf, &Selection::point(start), &new_text)
    } else {
        let del = delete_range_tx(buf, &cx.workspace.views[vid].selection, start, end);
        // Apply delete first so we can re-anchor selection at `start`, then
        // insert. Two transactions keeps undo behavior clean.
        cx.workspace.documents[did].apply_tx(del);
        replace_selection_tx(
            &cx.workspace.documents[did].buffer,
            &Selection::point(start),
            &new_text,
        )
    };
    let after = tx.selection_after.clone();
    cx.workspace.documents[did].apply_tx(tx);
    let v = &mut cx.workspace.views[vid];
    v.selection = after;
    v.target_col = None;
    v.hover = None;
    v.completion = None;
    cx.status.clear();
}

/// Reverse-translate an LSP `(line, character)` to a char offset using
/// the negotiated encoding. Mirrors `Document::lsp_pos_to_char` but
/// works directly off a rope so we don't have to plumb back through
/// Document for completion edits.
fn lsp_pos_to_char(
    rope: &ropey::Rope,
    line: u32,
    character: u32,
    encoding: &lsp_types::PositionEncodingKind,
) -> usize {
    let line = (line as usize).min(rope.len_lines().saturating_sub(1));
    let line_start = rope.line_to_char(line);
    let line_slice = rope.line(line);
    let mut line_chars = line_slice.len_chars();
    if line_chars > 0 && line_slice.char(line_chars - 1) == '\n' {
        line_chars -= 1;
    }
    let char_in_line = if encoding == &lsp_types::PositionEncodingKind::UTF8 {
        let mut remaining = character as usize;
        let mut idx = 0;
        for c in line_slice.chars().take(line_chars) {
            let b = char::len_utf8(c);
            if remaining < b { break; }
            remaining -= b;
            idx += 1;
        }
        idx.min(line_chars)
    } else if encoding == &lsp_types::PositionEncodingKind::UTF32 {
        (character as usize).min(line_chars)
    } else {
        let mut remaining = character as usize;
        let mut idx = 0;
        for c in line_slice.chars().take(line_chars) {
            let u = char::len_utf16(c);
            if remaining < u { break; }
            remaining -= u;
            idx += 1;
        }
        idx.min(line_chars)
    };
    line_start + char_in_line
}

fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
    let v = cx.viewport;
    if row < v.y || row >= v.y + v.height { return None; }
    let text_x = v.x + v.gutter_width;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - v.y) as usize;
    let view = cx.workspace.active_view()?;
    let buf = &cx.workspace.documents.get(view.doc)?.buffer;
    let line = (view.scroll_top() + row_in_view).min(buf.line_count().saturating_sub(1));
    let local_col = click_col.min(buf.line_len_chars(line));
    Some(buf.line_start(line) + local_col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use devix_buffer::{Buffer, Selection, replace_selection_tx};
    use lsp_types::CompletionItem;

    fn ws_with_text(text: &str) -> (Workspace, crate::view::ViewId) {
        let mut ws = Workspace::open(None).unwrap();
        let did = ws.active_view().unwrap().doc;
        let buf: &Buffer = &ws.documents[did].buffer;
        let tx = replace_selection_tx(buf, &Selection::point(0), text);
        ws.documents[did].buffer.apply(tx);
        let vid = ws.active_view().unwrap().doc;
        // Move cursor to end of text.
        let len = ws.documents[did].buffer.len_chars();
        let v_id = ws
            .frames
            .values()
            .next()
            .unwrap()
            .active_view()
            .unwrap();
        ws.views[v_id].selection = Selection::point(len);
        let _ = vid;
        (ws, v_id)
    }

    #[test]
    fn ident_start_walks_left_through_word_chars() {
        let mut buf = Buffer::empty();
        let tx = replace_selection_tx(&buf, &Selection::point(0), "fn foo_bar()");
        buf.apply(tx);
        // Cursor at 10 (end of "foo_bar"); ident starts at 3.
        assert_eq!(ident_start_at(&buf, 10), 3);
        // Cursor at 11 (just past `(`); ident starts at 11 (no ident there).
        assert_eq!(ident_start_at(&buf, 11), 11);
    }

    #[test]
    fn refilter_drops_when_prefix_contains_non_ident() {
        let (mut ws, vid) = ws_with_text("ne ");
        // Pretend completion was fired at position 2 with two items.
        ws.views[vid].completion = Some(CompletionState {
            anchor_char: 2,
            query_start: 0,
            items: vec![
                CompletionItem { label: "new".into(), ..Default::default() },
                CompletionItem { label: "next".into(), ..Default::default() },
            ],
            filtered: vec![0, 1],
            selected: 0,
            status: CompletionStatus::Ready,
        });
        // Move cursor to after the space (index 3); prefix = "ne ".
        ws.views[vid].selection = Selection::point(3);
        refilter_completion(&mut ws, vid);
        assert!(ws.views[vid].completion.is_none(), "non-ident in prefix dismisses");
    }

    #[test]
    fn refilter_ranks_matches_by_prefix_position() {
        let (mut ws, vid) = ws_with_text("n");
        ws.views[vid].completion = Some(CompletionState {
            anchor_char: 0,
            query_start: 0,
            items: vec![
                CompletionItem { label: "next".into(), ..Default::default() },
                CompletionItem { label: "into".into(), ..Default::default() }, // contains 'n' but later
                CompletionItem { label: "new".into(), ..Default::default() },
            ],
            filtered: vec![],
            selected: 0,
            status: CompletionStatus::Ready,
        });
        ws.views[vid].selection = Selection::point(1);
        refilter_completion(&mut ws, vid);
        let state = ws.views[vid].completion.as_ref().unwrap();
        assert!(!state.filtered.is_empty(), "should have at least one match");
        // Both "next" and "new" start with 'n' (prefix-position 0); "into"
        // matches at position 1 so ranks lower. Either of the two prefix-0
        // hits is acceptable as the head.
        let head_label = &state.items[state.filtered[0]].label;
        assert!(head_label == "new" || head_label == "next",
            "prefix-0 match should rank first, got {head_label}");
    }
}
