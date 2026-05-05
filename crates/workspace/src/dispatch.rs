//! Editor command helpers — shared utilities used by the `cmd` struct
//! impls.
//!
//! Phase 5: the enum-based `dispatch()` fn is gone. Every chord and
//! palette command resolves to `Arc<dyn EditorCommand>` (in `crate::cmd`)
//! and invokes through the trait. This module retains the helpers
//! (motion, selection, completion, LSP-position) that those struct impls
//! delegate to — they're the bits of dispatch logic that benefit from
//! sharing across multiple commands.

use devix_buffer::{Buffer, Change, Range, Selection, Transaction, delete_range_tx, replace_selection_tx};
use devix_lsp::{char_in_rope, position_in_rope};
use lsp_types::CompletionTextEdit;

use crate::context::{Context, Viewport};
use crate::view::{CompletionState, ScrollMode, View, ViewId};
#[cfg(test)]
use crate::view::CompletionStatus;
use crate::workspace::{LspChannel, Workspace};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn page_step(v: Viewport) -> usize { v.height.saturating_sub(1).max(1) as usize }

/// Inputs every position-anchored LSP request needs: the active view, the
/// LSP channel, the document's URI, the cursor position translated to LSP
/// coordinates, and the head as a char index. Returns None when any of
/// those is unavailable (no active view, LSP not attached, doc not on
/// disk yet).
pub(crate) struct LspPositionRequest {
    pub(crate) vid: ViewId,
    pub(crate) wiring: LspChannel,
    pub(crate) uri: lsp_types::Uri,
    pub(crate) position: lsp_types::Position,
    pub(crate) head: usize,
}

pub(crate) fn lsp_position_request(ws: &Workspace) -> Option<LspPositionRequest> {
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
pub(crate) fn move_to_with(
    cx: &mut Context<'_>,
    extend: bool,
    motion: impl FnOnce(&Buffer, usize) -> usize,
) {
    let Some((_, vid, did)) = cx.workspace.active_ids() else { return };
    let head = cx.workspace.views[vid].primary().head;
    let to = motion(&cx.workspace.documents[did].buffer, head);
    cx.workspace.views[vid].move_to(to, extend, false);
}

pub(crate) fn move_vertical(cx: &mut Context<'_>, down: bool, extend: bool) {
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

pub(crate) fn replace_selection(cx: &mut Context<'_>, text: &str) {
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

pub(crate) fn delete_primary_or(
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

pub(crate) fn do_copy(cx: &mut Context<'_>) {
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

pub(crate) fn do_cut(cx: &mut Context<'_>) {
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

pub(crate) fn do_paste(cx: &mut Context<'_>) {
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
pub(crate) fn ident_start_at(buf: &Buffer, head: usize) -> usize {
    let rope = buf.rope();
    let mut i = head;
    while i > 0 {
        let c = rope.char(i - 1);
        if !(c.is_alphanumeric() || c == '_') { break; }
        i -= 1;
    }
    i
}

pub(crate) fn take_completion(cx: &mut Context<'_>) -> Option<CompletionState> {
    let (_, vid, _) = cx.workspace.active_ids()?;
    cx.workspace.views[vid].completion.take()
}

pub(crate) fn dismiss_completion(cx: &mut Context<'_>) {
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
pub(crate) fn apply_completion_accept(cx: &mut Context<'_>) {
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
pub(crate) fn jump_to_location(cx: &mut Context<'_>, loc: lsp_types::Location) {
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
        let mut owner: Option<(crate::frame::FrameId, usize)> = None;
        for fid in crate::tree::frame_ids(cx.workspace.root.as_ref()) {
            if let Some(frame) = crate::tree::find_frame(cx.workspace.root.as_ref(), fid) {
                if let Some(idx) = frame.tabs.iter().position(|&v| v == vid) {
                    owner = Some((fid, idx));
                    break;
                }
            }
        }
        if let Some((fid, idx)) = owner {
            cx.workspace.focus_frame(fid);
            cx.workspace.activate_tab(fid, idx);
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

pub(crate) fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
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
        let fid = ws.active_frame().unwrap();
        let v_id = crate::tree::find_frame(ws.root.as_ref(), fid)
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
        };
        use devix_core::Action as _;
        crate::cmd::CompletionAccept.invoke(&mut cx);

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
