//! Action dispatcher.

use devix_buffer::{Buffer, Change, Range, Selection, Transaction, delete_range_tx, replace_selection_tx};
use devix_lsp::{CompletionTrigger, LspCommand, char_in_rope, position_in_rope};
use lsp_types::CompletionTextEdit;

use crate::action::Action;
use crate::context::{Context, Viewport};
use crate::overlay::{Overlay, PaletteState, SymbolsKind, SymbolsState};
use crate::view::{CompletionState, CompletionStatus, HoverState, HoverStatus, ScrollMode, View, ViewId};
use crate::workspace::{LspChannel, Workspace};

/// Trigger characters that auto-fire completion. Hardcoded for slice 3;
/// a follow-up will surface the server's `ServerCapabilities.completion_provider
/// .trigger_characters` list per (root, language) so this can specialize.
const TRIGGER_CHARS: &[char] = &['.', ':'];

pub fn dispatch(action: Action, cx: &mut Context<'_>) {
    use Action::*;
    match action {
        // ---- motion ----
        MoveLeft { extend } => move_to_with(cx, extend, |b, h| b.move_left(h)),
        MoveRight { extend } => move_to_with(cx, extend, |b, h| b.move_right(h)),
        MoveUp { extend } => move_vertical(cx, false, extend),
        MoveDown { extend } => move_vertical(cx, true, extend),
        MoveWordLeft { extend } => move_to_with(cx, extend, |b, h| b.word_left(h)),
        MoveWordRight { extend } => move_to_with(cx, extend, |b, h| b.word_right(h)),
        MoveLineStart { extend } => move_to_with(cx, extend, |b, h| b.line_start_of(h)),
        MoveLineEnd { extend } => move_to_with(cx, extend, |b, h| b.line_end_of(h)),
        MoveDocStart { extend } => move_to_with(cx, extend, |b, _| b.doc_start()),
        MoveDocEnd { extend } => move_to_with(cx, extend, |b, _| b.doc_end()),
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
                dispatch(Action::TriggerCompletion(CompletionTrigger::Char(c)), cx);
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
                cx.workspace.views[vid].adopt_selection(sel);
                cx.status.clear();
            } else {
                cx.status.set("nothing to undo");
            }
        }
        Redo => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            if let Some(sel) = cx.workspace.documents[did].redo() {
                cx.workspace.views[vid].adopt_selection(sel);
                cx.status.clear();
            } else {
                cx.status.set("nothing to redo");
            }
        }

        // ---- selection ----
        SelectAll => {
            let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
            let end = cx.workspace.documents[did].buffer.len_chars();
            cx.workspace.views[vid].adopt_selection(Selection::single(Range::new(0, end)));
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
            let Some(req) = lsp_position_request(cx.workspace) else { return };
            let _ = req.wiring.sink.send(LspCommand::Hover {
                uri: req.uri,
                position: req.position,
                anchor_char: req.head,
            });
            cx.workspace.views[req.vid].hover = Some(HoverState {
                anchor_char: req.head,
                status: HoverStatus::Pending,
            });
        }
        GotoDefinition => {
            let Some(req) = lsp_position_request(cx.workspace) else { return };
            let _ = req.wiring.sink.send(LspCommand::GotoDefinition {
                uri: req.uri,
                position: req.position,
                anchor_char: req.head,
            });
        }
        TriggerCompletion(trigger) => {
            let Some(req) = lsp_position_request(cx.workspace) else { return };
            let _ = req.wiring.sink.send(LspCommand::Completion {
                uri: req.uri,
                position: req.position,
                anchor_char: req.head,
                trigger,
            });
            // The query starts where the user can still meaningfully filter.
            // For trigger-char (`.`, `:`), the cursor itself; for manual,
            // walk back over the identifier. ident_start_at handles both.
            let did = cx.workspace.views[req.vid].doc;
            let query_start = ident_start_at(&cx.workspace.documents[did].buffer, req.head);
            cx.workspace.views[req.vid].completion = Some(CompletionState {
                anchor_char: req.head,
                query_start,
                items: Vec::new(),
                labels_lower: Vec::new(),
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

        // ---- symbol picker overlay ----
        ShowDocumentSymbols => {
            let Some((_, _vid, did)) = cx.workspace.active_ids() else { return };
            let Some(wiring) = cx.workspace.lsp_channel() else {
                cx.status.set("LSP not attached for this document");
                return;
            };
            let Some(uri) = cx.workspace.documents[did].lsp_uri().cloned() else {
                cx.status.set("no symbols: doc not attached to a language server");
                return;
            };
            let state = SymbolsState::new(SymbolsKind::Document, Some(uri.clone()));
            let _ = wiring.sink.send(LspCommand::DocumentSymbols {
                uri,
                epoch: state.epoch,
            });
            *cx.overlay = Some(Overlay::Symbols(state));
        }
        ShowWorkspaceSymbols => {
            let Some(wiring) = cx.workspace.lsp_channel() else {
                cx.status.set("LSP not attached");
                return;
            };
            let state = SymbolsState::new(SymbolsKind::Workspace, None);
            let _ = wiring.sink.send(LspCommand::WorkspaceSymbols {
                query: state.query.clone(),
                epoch: state.epoch,
            });
            *cx.overlay = Some(Overlay::Symbols(state));
        }
        CloseSymbols => {
            if matches!(cx.overlay, Some(Overlay::Symbols(_))) {
                *cx.overlay = None;
            }
        }
        SymbolsMove(delta) => {
            if let Some(Overlay::Symbols(s)) = cx.overlay.as_mut() {
                s.move_selection(delta);
            }
        }
        SymbolsSetQuery(q) => {
            // Workspace mode re-fetches on every query change; document
            // mode just client-filters. set_query bumps the epoch either
            // way so any in-flight workspace response can be discarded.
            if let Some(Overlay::Symbols(s)) = cx.overlay.as_mut() {
                let needs_refetch = s.kind == SymbolsKind::Workspace;
                s.set_query(q);
                if needs_refetch {
                    let epoch = s.epoch;
                    let query = s.query.clone();
                    if let Some(wiring) = cx.workspace.lsp_channel() {
                        let _ = wiring.sink.send(LspCommand::WorkspaceSymbols { query, epoch });
                    }
                }
            }
        }
        SymbolsAccept => {
            let location = if let Some(Overlay::Symbols(s)) = cx.overlay.as_ref() {
                s.selected_symbol().map(|sym| sym.location.clone())
            } else {
                None
            };
            *cx.overlay = None;
            if let Some(loc) = location {
                jump_to_location(cx, loc);
            }
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
            // Wheel/trackpad scrolling expresses "I want to look here, not at
            // the cursor" — flip out of Anchored so the next render doesn't
            // immediately snap back. The next cursor-moving action restores
            // Anchored via move_to.
            v.scroll_mode = crate::view::ScrollMode::Free;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn page_step(v: Viewport) -> usize { v.height.saturating_sub(1).max(1) as usize }

/// Inputs every position-anchored LSP request needs: the active view, the
/// LSP channel, the document's URI, the cursor position translated to LSP
/// coordinates, and the head as a char index. Returns None when any of
/// those is unavailable (no active view, LSP not attached, doc not on
/// disk yet).
struct LspPositionRequest {
    vid: ViewId,
    wiring: LspChannel,
    uri: lsp_types::Uri,
    position: lsp_types::Position,
    head: usize,
}

fn lsp_position_request(ws: &Workspace) -> Option<LspPositionRequest> {
    let (_, vid, did) = ws.active_ids()?;
    let wiring = ws.lsp_channel()?;
    let doc = &ws.documents[did];
    let uri = doc.lsp_uri().cloned()?;
    let head = ws.views[vid].primary().head;
    let position = position_in_rope(doc.buffer.rope(), head, &wiring.encoding);
    Some(LspPositionRequest { vid, wiring, uri, position, head })
}

/// Apply a single-axis motion: pick the new char index from the active
/// buffer + current head, then `move_to` it. Used by every cursor-key
/// arm except the vertical pair (which threads `target_col`).
fn move_to_with(
    cx: &mut Context<'_>,
    extend: bool,
    motion: impl FnOnce(&Buffer, usize) -> usize,
) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let head = cx.workspace.views[vid].primary().head;
    let to = motion(&cx.workspace.documents[did].buffer, head);
    cx.workspace.views[vid].move_to(to, extend, false);
}

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
    reset_motion_state(v);
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
    reset_motion_state(v);
    cx.status.clear();
}

/// Reset transient view state shared with `adopt_selection` *minus* the
/// completion popup. Used by edit helpers (`replace_selection`,
/// `delete_primary_or`) where the caller (InsertChar / DeleteBack) wants
/// to preserve completion across the edit and re-filter it afterward.
fn reset_motion_state(v: &mut View) {
    v.target_col = None;
    v.hover = None;
    v.scroll_mode = ScrollMode::Anchored;
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
    cx.workspace.views[vid].adopt_selection(after);
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
        // labels_lower is built alongside items by `set_items`, so refilter
        // doesn't re-lowercase every label on every keystroke. The two
        // vecs are kept in lockstep; index by `i` into both.
        let prefix_lower = prefix.to_lowercase();
        for (i, label_lower) in state.labels_lower.iter().enumerate() {
            if let Some(pos) = label_lower.find(&prefix_lower) {
                // Earlier match position scores higher; exact-prefix match
                // ranks above mid-string. Tie-break by shorter label so
                // the canonical name shows first.
                let mut score = 1000 - (pos as i32 * 10);
                if pos == 0 { score += 500; }
                score -= state.items[i].label.len() as i32;
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
        .lsp_channel()
        .map(|w| w.encoding)
        .unwrap_or(lsp_types::PositionEncodingKind::UTF16);

    let (start, end, new_text) = if let Some(edit) = item.text_edit.as_ref() {
        let (range, txt) = match edit {
            CompletionTextEdit::Edit(e) => (e.range, e.new_text.clone()),
            CompletionTextEdit::InsertAndReplace(ir) => (ir.replace, ir.new_text.clone()),
        };
        let rope = cx.workspace.documents[did].buffer.rope();
        let len = rope.len_chars();
        let s = char_in_rope(rope, range.start.line, range.start.character, &encoding).unwrap_or(len);
        let e = char_in_rope(rope, range.end.line, range.end.character, &encoding).unwrap_or(len);
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
    // One Change with both remove and insert keeps the accept atomic: a
    // single undo press reverts to the pre-accept state. Two separate
    // transactions (delete then insert) showed up as two undo steps.
    let insert_chars = new_text.chars().count();
    let tx = Transaction {
        changes: vec![Change {
            start,
            remove_len: end - start,
            insert: new_text,
        }],
        selection_before: cx.workspace.views[vid].selection.clone(),
        selection_after: Selection::single(Range::point(start + insert_chars)),
    };
    let after = tx.selection_after.clone();
    cx.workspace.documents[did].apply_tx(tx);
    cx.workspace.views[vid].adopt_selection(after);
    cx.status.clear();
}

/// Open the file at `loc.uri` (reusing an open view if any) and place the
/// cursor at the LSP range start. Used by the symbol picker on accept;
/// the goto-def path applies the same logic in app/lsp.rs but we need a
/// dispatch-side variant because symbol-accept fires sync from a key
/// press rather than waiting on a server response.
fn jump_to_location(cx: &mut Context<'_>, loc: lsp_types::Location) {
    use devix_lsp::uri_to_path;
    let Ok(target_path) = uri_to_path(&loc.uri) else { return };
    let encoding = cx
        .workspace
        .lsp_channel()
        .map(|w| w.encoding)
        .unwrap_or(lsp_types::PositionEncodingKind::UTF16);

    // Prefer an already-open view of the target file.
    let mut hit: Option<crate::view::ViewId> = None;
    for (vid, view) in cx.workspace.views.iter() {
        let doc = &cx.workspace.documents[view.doc];
        let Some(p) = doc.buffer.path() else { continue };
        if p == target_path
            || std::fs::canonicalize(p).ok() == std::fs::canonicalize(&target_path).ok()
        {
            hit = Some(vid);
            break;
        }
    }
    if let Some(vid) = hit {
        // Find the frame owning this view and focus + activate.
        let mut owner_fid: Option<crate::frame::FrameId> = None;
        for (fid, frame) in cx.workspace.frames.iter() {
            if frame.tabs.contains(&vid) {
                owner_fid = Some(fid);
                break;
            }
        }
        if let Some(fid) = owner_fid {
            cx.workspace.focus_frame(fid);
            if let Some(idx) = cx.workspace.frames[fid].tabs.iter().position(|&v| v == vid) {
                cx.workspace.activate_tab(fid, idx);
            }
        }
        place_cursor_at_pos(cx, vid, &loc, &encoding);
        return;
    }

    if let Err(e) = cx.workspace.open_path_replace_current(target_path) {
        cx.status.set(format!("symbol open failed: {e}"));
        return;
    }
    if let Some((_, vid, _)) = cx.workspace.active_ids() {
        place_cursor_at_pos(cx, vid, &loc, &encoding);
    }
}

fn place_cursor_at_pos(
    cx: &mut Context<'_>,
    vid: crate::view::ViewId,
    loc: &lsp_types::Location,
    encoding: &lsp_types::PositionEncodingKind,
) {
    let did = cx.workspace.views[vid].doc;
    let rope = cx.workspace.documents[did].buffer.rope();
    let idx = char_in_rope(rope, loc.range.start.line, loc.range.start.character, encoding)
        .unwrap_or_else(|| rope.len_chars());
    let v = &mut cx.workspace.views[vid];
    v.move_to(idx, false, false);
    v.scroll_mode = crate::view::ScrollMode::Anchored;
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
            labels_lower: vec!["new".into(), "next".into()],
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
    fn completion_accept_replace_is_one_undo_step() {
        use crate::command::CommandRegistry;
        use crate::context::StatusLine;
        // Set up a doc containing "ne" with cursor at end. Completion item
        // carries a text_edit replacing [0, 2) with "next". A single undo
        // press must restore the original "ne" — pre-fix this took two
        // (delete then insert were separate transactions).
        let (mut ws, vid) = ws_with_text("ne");
        let mut overlay: Option<crate::overlay::Overlay> = None;
        let mut clipboard: Option<arboard::Clipboard> = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::new();

        let item = lsp_types::CompletionItem {
            label: "next".into(),
            text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                range: lsp_types::Range {
                    start: lsp_types::Position { line: 0, character: 0 },
                    end: lsp_types::Position { line: 0, character: 2 },
                },
                new_text: "next".into(),
            })),
            ..Default::default()
        };
        ws.views[vid].completion = Some(CompletionState {
            anchor_char: 2,
            query_start: 0,
            items: vec![item],
            labels_lower: vec!["next".into()],
            filtered: vec![0],
            selected: 0,
            status: CompletionStatus::Ready,
        });

        let mut cx = Context {
            workspace: &mut ws,
            clipboard: &mut clipboard,
            status: &mut status,
            quit: &mut quit,
            viewport: Viewport::default(),
            commands: &commands,
            overlay: &mut overlay,
        };
        dispatch(Action::CompletionAccept, &mut cx);

        let did = ws.views[vid].doc;
        assert_eq!(ws.documents[did].buffer.rope().to_string(), "next");
        let _ = ws.documents[did].undo();
        assert_eq!(ws.documents[did].buffer.rope().to_string(), "ne",
            "single undo should restore pre-accept text");
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
            labels_lower: vec!["next".into(), "into".into(), "new".into()],
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
