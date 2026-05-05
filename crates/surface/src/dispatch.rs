//! Editor command helpers — shared utilities used by the `cmd` struct
//! impls.
//!
//! Phase 5: the enum-based `dispatch()` fn is gone. Every chord and
//! palette command resolves to `Arc<dyn EditorCommand>` (in `crate::cmd`)
//! and invokes through the trait. This module retains the helpers
//! (motion, selection, completion, LSP-position) that those struct impls
//! delegate to — they're the bits of dispatch logic that benefit from
//! sharing across multiple commands.

use devix_text::{Buffer, Change, Range, Selection, Transaction, delete_each_tx, delete_range_tx, replace_selection_tx};
use devix_lsp::{char_in_rope, position_in_rope};
use lsp_types::CompletionTextEdit;

use crate::context::{Context, Viewport};
use crate::view::{ScrollMode, View, ViewId};
use devix_editor::CompletionState;
#[cfg(test)]
use devix_editor::CompletionStatus;
use crate::surface::{LspChannel, Surface};

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

pub(crate) fn lsp_position_request(ws: &Surface) -> Option<LspPositionRequest> {
    let (_, vid, did) = ws.active_ids()?;
    let wiring = ws.lsp_channel()?;
    let doc = &ws.documents[did];
    let uri = doc.lsp_uri().cloned()?;
    let head = ws.views[vid].primary().head;
    let position = position_in_rope(doc.buffer.rope(), head, &wiring.encoding);
    Some(LspPositionRequest { vid, wiring, uri, position, head })
}

/// Apply a single-axis motion to every range. The motion sees each
/// range's head; with `extend`, only the head moves (anchor stays put);
/// without it, the range collapses to the new head. The post-step
/// selection is normalized so cursors that landed on the same position
/// merge.
pub(crate) fn move_to_with(
    cx: &mut Context<'_>,
    extend: bool,
    motion: impl Fn(&Buffer, usize) -> usize,
) {
    let Some((_, vid, did)) = cx.surface.active_ids() else { return };
    // Clone-then-write so we can borrow buffer immutably while transforming
    // the selection. Selection is shallow (Vec of two-usize ranges).
    let buf = &cx.surface.documents[did].buffer;
    let mut sel = cx.surface.views[vid].selection.clone();
    sel.transform(|r| {
        let to = motion(buf, r.head);
        r.put_head(to, extend)
    });
    sel.normalize();
    let v = &mut cx.surface.views[vid];
    v.selection = sel;
    v.target_col = None;
    v.hover = None;
    v.completion = None;
    v.scroll_mode = ScrollMode::Anchored;
}

/// Vertical motion. With a single cursor the sticky-column behavior on
/// `View` keeps repeated Up/Down stable across short lines. With multi
/// cursor the column is recomputed per-range each call — sticky-col across
/// many cursors is a polish item, not a correctness one.
pub(crate) fn move_vertical(cx: &mut Context<'_>, down: bool, extend: bool) {
    let Some((_, vid, did)) = cx.surface.active_ids() else { return };
    let buf = &cx.surface.documents[did].buffer;
    let single = !cx.surface.views[vid].selection.is_multi();
    let sticky = cx.surface.views[vid].target_col;
    let mut sel = cx.surface.views[vid].selection.clone();

    // Track the primary's resolved column so the View's sticky col stays
    // attached to the primary cursor (most-natural behavior — the cursor
    // the user is "leading with" keeps the snap line).
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

    let v = &mut cx.surface.views[vid];
    v.selection = sel;
    v.target_col = primary_col_for_sticky;
    v.hover = None;
    v.completion = None;
    v.scroll_mode = ScrollMode::Anchored;
}

pub(crate) fn replace_selection(cx: &mut Context<'_>, text: &str) {
    let Some((_, vid, did)) = cx.surface.active_ids() else { return };
    let tx = replace_selection_tx(
        &cx.surface.documents[did].buffer,
        &cx.surface.views[vid].selection,
        text,
    );
    let after = tx.selection_after.clone();
    cx.surface.documents[did].apply_tx(tx);
    let v = &mut cx.surface.views[vid];
    v.selection = after;
    reset_motion_state(v);
    cx.status.clear();
}

/// Per-range delete. For each range: if non-empty, delete its span; if
/// empty (point cursor), call `builder` to compute a 1-char-or-word span
/// to delete (returning `None` skips that range — used at doc start/end).
/// All resulting changes are bundled into one transaction so an undo
/// reverts every cursor's deletion in one step.
pub(crate) fn delete_each_or(
    cx: &mut Context<'_>,
    builder: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) {
    let Some((_, vid, did)) = cx.surface.active_ids() else { return };
    let buf = &cx.surface.documents[did].buffer;
    let sel = cx.surface.views[vid].selection.clone();
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
    cx.surface.documents[did].apply_tx(tx);
    let v = &mut cx.surface.views[vid];
    v.selection = after;
    reset_motion_state(v);
    cx.status.clear();
}

/// Reset transient view state shared with `adopt_selection` *minus* the
/// completion popup. Used by edit helpers (`replace_selection`,
/// `delete_each_or`) where the caller (InsertChar / DeleteBack) wants
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
    let Some((_, vid, did)) = cx.surface.active_ids() else { return };
    let prim = cx.surface.views[vid].primary();
    let (start, end, msg) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.surface.documents[did].buffer, prim.head);
        (s, e, "copied line")
    } else {
        (prim.start(), prim.end(), "copied")
    };
    if start == end { return; }
    let text = cx.surface.documents[did].buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard"); return;
    };
    if cb.set_text(text).is_err() { cx.status.set("clipboard error"); return; }
    cx.status.set(msg);
}

pub(crate) fn do_cut(cx: &mut Context<'_>) {
    let Some((_, vid, did)) = cx.surface.active_ids() else { return };
    let prim = cx.surface.views[vid].primary();
    let (start, end, line_cut) = if prim.is_empty() {
        let (s, e) = current_line_span(&cx.surface.documents[did].buffer, prim.head);
        (s, e, true)
    } else {
        (prim.start(), prim.end(), false)
    };
    if start == end { return; }
    let text = cx.surface.documents[did].buffer.slice_to_string(start, end);
    let Some(cb) = cx.clipboard.as_mut() else {
        cx.status.set("no system clipboard"); return;
    };
    if cb.set_text(text).is_err() { cx.status.set("clipboard error"); return; }
    let tx = delete_range_tx(
        &cx.surface.documents[did].buffer,
        &cx.surface.views[vid].selection,
        start, end,
    );
    let after = tx.selection_after.clone();
    cx.surface.documents[did].apply_tx(tx);
    cx.surface.views[vid].adopt_selection(after);
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
    let (_, vid, _) = cx.surface.active_ids()?;
    cx.surface.views[vid].completion.take()
}

pub(crate) fn dismiss_completion(cx: &mut Context<'_>) {
    let Some((_, vid, _)) = cx.surface.active_ids() else { return };
    cx.surface.views[vid].completion = None;
}

/// Re-rank `state.items` against the prefix `query_start..cursor` from the
/// rope. If the cursor moved left of `query_start` (user backspaced past
/// the query origin), drop the popup. Empty-prefix shows everything in
/// server-given order.
pub fn refilter_completion(ws: &mut Surface, vid: ViewId) {
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
    let Some((_, vid, did)) = cx.surface.active_ids() else { return };
    let Some(state) = cx.surface.views[vid].completion.take() else { return };
    let Some(&idx) = state.filtered.get(state.selected) else { return };
    let Some(item) = state.items.get(idx).cloned() else { return };

    let head = cx.surface.views[vid].primary().head;
    let encoding = cx
        .surface
        .lsp_channel()
        .map(|w| w.encoding)
        .unwrap_or(lsp_types::PositionEncodingKind::UTF16);

    let (start, end, new_text) = if let Some(edit) = item.text_edit.as_ref() {
        let (range, txt) = match edit {
            CompletionTextEdit::Edit(e) => (e.range, e.new_text.clone()),
            CompletionTextEdit::InsertAndReplace(ir) => (ir.replace, ir.new_text.clone()),
        };
        let rope = cx.surface.documents[did].buffer.rope();
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

    let buf = &cx.surface.documents[did].buffer;
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
        selection_before: cx.surface.views[vid].selection.clone(),
        selection_after: Selection::single(Range::point(start + insert_chars)),
    };
    let after = tx.selection_after.clone();
    cx.surface.documents[did].apply_tx(tx);
    cx.surface.views[vid].adopt_selection(after);
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
        .surface
        .lsp_channel()
        .map(|w| w.encoding)
        .unwrap_or(lsp_types::PositionEncodingKind::UTF16);

    // Prefer an already-open view of the target file.
    let mut hit: Option<crate::view::ViewId> = None;
    for (vid, view) in cx.surface.views.iter() {
        let doc = &cx.surface.documents[view.doc];
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
        for fid in crate::tree::frame_ids(cx.surface.root.as_ref()) {
            if let Some(frame) = crate::tree::find_frame(cx.surface.root.as_ref(), fid) {
                if let Some(idx) = frame.tabs.iter().position(|&v| v == vid) {
                    owner = Some((fid, idx));
                    break;
                }
            }
        }
        if let Some((fid, idx)) = owner {
            cx.surface.focus_frame(fid);
            cx.surface.activate_tab(fid, idx);
        }
        place_cursor_at_pos(cx, vid, &loc, &encoding);
        return;
    }

    if let Err(e) = cx.surface.open_path_replace_current(target_path) {
        cx.status.set(format!("symbol open failed: {e}"));
        return;
    }
    if let Some((_, vid, _)) = cx.surface.active_ids() {
        place_cursor_at_pos(cx, vid, &loc, &encoding);
    }
}

fn place_cursor_at_pos(
    cx: &mut Context<'_>,
    vid: crate::view::ViewId,
    loc: &lsp_types::Location,
    encoding: &lsp_types::PositionEncodingKind,
) {
    let did = cx.surface.views[vid].doc;
    let rope = cx.surface.documents[did].buffer.rope();
    let idx = char_in_rope(rope, loc.range.start.line, loc.range.start.character, encoding)
        .unwrap_or_else(|| rope.len_chars());
    let v = &mut cx.surface.views[vid];
    v.move_to(idx, false, false);
    v.scroll_mode = crate::view::ScrollMode::Anchored;
}

pub(crate) fn click_to_char_idx(cx: &Context<'_>, col: u16, row: u16) -> Option<usize> {
    let v = cx.viewport;
    if row < v.y || row >= v.y + v.height { return None; }
    let text_x = v.x + v.gutter_width;
    let click_col = col.saturating_sub(text_x) as usize;
    let row_in_view = (row - v.y) as usize;
    let view = cx.surface.active_view()?;
    let buf = &cx.surface.documents.get(view.doc)?.buffer;
    let line = (view.scroll_top() + row_in_view).min(buf.line_count().saturating_sub(1));
    let local_col = click_col.min(buf.line_len_chars(line));
    Some(buf.line_start(line) + local_col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Surface;
    use devix_text::{Buffer, Selection, replace_selection_tx};
    use lsp_types::CompletionItem;

    fn ws_with_text(text: &str) -> (Surface, crate::view::ViewId) {
        let mut ws = Surface::open(None).unwrap();
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
            surface: &mut ws,
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
