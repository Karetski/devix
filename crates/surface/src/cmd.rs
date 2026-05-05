//! Editor commands as `Pane`-style trait impls.
//!
//! Phase 5 of the architecture refactor: the `Action` enum is being
//! flipped from a closed match-target to an open trait surface. Each
//! command becomes a struct that implements `core::Action<Context<'_>>`;
//! the keymap and palette eventually store `Box<dyn EditorCommand>`
//! instead of an enum value.
//!
//! This module is the partial migration: the trait alias is here, plus
//! a worked example (`Quit`). The legacy `Action::Quit` arm in
//! [`crate::dispatch`] now routes through this struct, proving the
//! pattern compiles and runs end-to-end. Porting the remaining variants
//! is mechanical — one struct per variant — and lands in a follow-up.

use devix_core::Action;

use crate::context::Context;

/// HRTB trait alias for actions that take the editor's `Context<'_>`.
/// Storage shape: `Box<dyn EditorCommand>`.
///
/// HRTB (`for<'a> Action<Context<'a>>`) is what makes the storage
/// possible — `Context<'a>` borrows from the surface, so its lifetime
/// is per-call, not `'static`. The action type itself stays `'static`
/// (no fields with lifetimes), which is what the trait's bound requires.
pub trait EditorCommand: for<'a> Action<Context<'a>> {}
impl<T> EditorCommand for T where T: for<'a> Action<Context<'a>> {}

/// Quit the editor. The simplest possible action: flips the run flag.
pub struct Quit;
impl<'a> Action<Context<'a>> for Quit {
    fn invoke(&self, ctx: &mut Context<'a>) {
        *ctx.quit = true;
    }
}

// --- File / disk -----------------------------------------------------------

pub struct Save;
impl<'a> Action<Context<'a>> for Save {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some(d) = ctx.surface.active_doc_mut() else { return };
        let msg = match d.buffer.save() {
            Ok(()) => "saved".to_string(),
            Err(e) => format!("save failed: {e}"),
        };
        ctx.status.set(msg);
    }
}

pub struct KeepBufferIgnoreDisk;
impl<'a> Action<Context<'a>> for KeepBufferIgnoreDisk {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(d) = ctx.surface.active_doc_mut() {
            d.disk_changed_pending = false;
        }
        ctx.status.set("kept buffer; disk change ignored");
    }
}

// --- Tabs -----------------------------------------------------------------

pub struct NewTab;
impl<'a> Action<Context<'a>> for NewTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.new_tab();
    }
}

pub struct NextTab;
impl<'a> Action<Context<'a>> for NextTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.next_tab();
    }
}

pub struct PrevTab;
impl<'a> Action<Context<'a>> for PrevTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.prev_tab();
    }
}

pub struct ForceCloseTab;
impl<'a> Action<Context<'a>> for ForceCloseTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.close_active_tab(true);
        ctx.status.clear();
    }
}

// --- Splits / frames -------------------------------------------------------
//
// `SplitVertical` / `SplitHorizontal` are named for the *dividing line* the
// user expects to see; the surface's `Axis` describes the layout direction
// children are arranged in. So a "vertical split" produces children laid
// out horizontally — flip in the impls.

pub struct SplitVertical;
impl<'a> Action<Context<'a>> for SplitVertical {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.split_active(crate::layout::Axis::Horizontal);
    }
}

pub struct SplitHorizontal;
impl<'a> Action<Context<'a>> for SplitHorizontal {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.split_active(crate::layout::Axis::Vertical);
    }
}

pub struct CloseFrame;
impl<'a> Action<Context<'a>> for CloseFrame {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.close_active_frame();
    }
}

// --- Modal Panes ----------------------------------------------------------
//
// All modal commands operate on `ctx.surface.modal`. Concrete pane types
// are looked up via `as_any_mut()` downcast — the framework treats the slot
// as a generic `Box<dyn Pane>` and never matches on a kind enum.

pub struct OpenPalette;
impl<'a> Action<Context<'a>> for OpenPalette {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.modal = Some(Box::new(crate::modal::PalettePane::from_registry(
            ctx.commands,
        )));
    }
}

pub struct ClosePalette;
impl<'a> Action<Context<'a>> for ClosePalette {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if modal_is::<crate::modal::PalettePane>(ctx) {
            ctx.surface.modal = None;
        }
    }
}

/// Generic "close whatever modal is open" action used by the responder
/// chain after a modal pane signals `ModalOutcome::Dismiss`. Symmetrical
/// to `OpenPalette` — it doesn't know or care which modal is in the slot.
pub struct CloseModal;
impl<'a> Action<Context<'a>> for CloseModal {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.modal = None;
    }
}

fn modal_is<T: 'static>(ctx: &Context<'_>) -> bool {
    ctx.surface
        .modal
        .as_ref()
        .and_then(|m| m.as_any())
        .map(|a| a.is::<T>())
        .unwrap_or(false)
}

// --- Parameterized actions ------------------------------------------------
//
// These carry per-invocation data on their fields. Pattern: each variant
// of the legacy enum becomes one struct; the data the variant held becomes
// public fields. Plugins contributing new actions follow the same shape —
// no central enum to grow.

pub struct ToggleSidebar(pub crate::layout::SidebarSlot);
impl<'a> Action<Context<'a>> for ToggleSidebar {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.toggle_sidebar(self.0);
    }
}

pub struct FocusDir(pub crate::layout::Direction);
impl<'a> Action<Context<'a>> for FocusDir {
    fn invoke(&self, ctx: &mut Context<'a>) {
        ctx.surface.focus_dir(self.0);
    }
}

pub struct PaletteMove(pub isize);
impl<'a> Action<Context<'a>> for PaletteMove {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(p) = downcast_modal_mut::<crate::modal::PalettePane>(ctx) {
            p.state.move_selection(self.0);
        }
    }
}

pub struct PaletteSetQuery(pub String);
impl<'a> Action<Context<'a>> for PaletteSetQuery {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(p) = downcast_modal_mut::<crate::modal::PalettePane>(ctx) {
            p.state.set_query(self.0.clone());
        }
    }
}

fn downcast_modal_mut<'a, T: 'static>(ctx: &'a mut Context<'_>) -> Option<&'a mut T> {
    ctx.surface
        .modal
        .as_mut()?
        .as_any_mut()?
        .downcast_mut::<T>()
}

pub struct CompletionMove(pub isize);
impl<'a> Action<Context<'a>> for CompletionMove {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, _)) = ctx.surface.active_ids() else { return };
        let Some(state) = ctx.surface.views[vid].completion.as_mut() else { return };
        if state.filtered.is_empty() {
            return;
        }
        let n = state.filtered.len() as isize;
        let cur = state.selected as isize;
        let next = (cur + self.0).rem_euclid(n);
        state.selected = next as usize;
    }
}

pub struct CompletionDismiss;
impl<'a> Action<Context<'a>> for CompletionDismiss {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, _)) = ctx.surface.active_ids() else { return };
        ctx.surface.views[vid].completion = None;
    }
}

pub struct CloseSymbols;
impl<'a> Action<Context<'a>> for CloseSymbols {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if modal_is::<crate::modal::SymbolPickerPane>(ctx) {
            ctx.surface.modal = None;
        }
    }
}

pub struct SymbolsMove(pub isize);
impl<'a> Action<Context<'a>> for SymbolsMove {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if let Some(s) = downcast_modal_mut::<crate::modal::SymbolPickerPane>(ctx) {
            s.state.move_selection(self.0);
        }
    }
}

// --- History --------------------------------------------------------------

pub struct Undo;
impl<'a> Action<Context<'a>> for Undo {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        if let Some(sel) = ctx.surface.documents[did].undo() {
            ctx.surface.views[vid].adopt_selection(sel);
            ctx.status.clear();
        } else {
            ctx.status.set("nothing to undo");
        }
    }
}

pub struct Redo;
impl<'a> Action<Context<'a>> for Redo {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        if let Some(sel) = ctx.surface.documents[did].redo() {
            ctx.surface.views[vid].adopt_selection(sel);
            ctx.status.clear();
        } else {
            ctx.status.set("nothing to redo");
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

// --- File / paths ---------------------------------------------------------

pub struct OpenPath(pub std::path::PathBuf);
impl<'a> Action<Context<'a>> for OpenPath {
    fn invoke(&self, ctx: &mut Context<'a>) {
        match ctx.surface.open_path_replace_current(self.0.clone()) {
            Ok(_) => ctx.status.clear(),
            Err(e) => ctx.status.set(format!("open failed: {e}")),
        }
    }
}

pub struct CloseTab;
impl<'a> Action<Context<'a>> for CloseTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        if !ctx.surface.close_active_tab(false) {
            ctx.status.set("unsaved changes — Ctrl+S to save, Ctrl+Shift+W to force close");
        } else {
            ctx.status.clear();
        }
    }
}

pub struct ReloadFromDisk;
impl<'a> Action<Context<'a>> for ReloadFromDisk {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        let res = ctx.surface.documents[did].reload_from_disk();
        match res {
            Ok(()) => {
                let max = ctx.surface.documents[did].buffer.len_chars();
                ctx.surface.documents[did].disk_changed_pending = false;
                ctx.surface.views[vid].selection.clamp(max);
                ctx.status.set("reloaded from disk");
            }
            Err(e) => ctx.status.set(format!("reload failed: {e}")),
        }
    }
}

// --- Clipboard ------------------------------------------------------------

pub struct Copy;
impl<'a> Action<Context<'a>> for Copy {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::do_copy(ctx);
    }
}

pub struct Cut;
impl<'a> Action<Context<'a>> for Cut {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::dismiss_completion(ctx);
        crate::dispatch::do_cut(ctx);
    }
}

pub struct Paste;
impl<'a> Action<Context<'a>> for Paste {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::dismiss_completion(ctx);
        crate::dispatch::do_paste(ctx);
    }
}

// --- Motion ---------------------------------------------------------------
//
// All motion variants share the `extend: bool` field — Shift+motion grows
// the selection, plain motion collapses it. Each delegates to the existing
// `move_to_with` / `move_vertical` helpers in `dispatch`.

pub struct MoveLeft { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveLeft {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, h| b.move_left(h));
    }
}

pub struct MoveRight { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveRight {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, h| b.move_right(h));
    }
}

pub struct MoveUp { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveUp {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::move_vertical(ctx, false, self.extend);
    }
}

pub struct MoveDown { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveDown {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::move_vertical(ctx, true, self.extend);
    }
}

pub struct MoveWordLeft { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveWordLeft {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, h| b.word_left(h));
    }
}

pub struct MoveWordRight { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveWordRight {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, h| b.word_right(h));
    }
}

pub struct MoveLineStart { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveLineStart {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, h| b.line_start_of(h));
    }
}

pub struct MoveLineEnd { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveLineEnd {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, h| b.line_end_of(h));
    }
}

pub struct MoveDocStart { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveDocStart {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, _| b.doc_start());
    }
}

pub struct MoveDocEnd { pub extend: bool }
impl<'a> Action<Context<'a>> for MoveDocEnd {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let extend = self.extend;
        crate::dispatch::move_to_with(ctx, extend, |b, _| b.doc_end());
    }
}

pub struct PageUp { pub extend: bool }
impl<'a> Action<Context<'a>> for PageUp {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let step = crate::dispatch::page_step(ctx.viewport);
        for _ in 0..step {
            crate::dispatch::move_vertical(ctx, false, self.extend);
        }
    }
}

pub struct PageDown { pub extend: bool }
impl<'a> Action<Context<'a>> for PageDown {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let step = crate::dispatch::page_step(ctx.viewport);
        for _ in 0..step {
            crate::dispatch::move_vertical(ctx, true, self.extend);
        }
    }
}

// --- Edits ----------------------------------------------------------------

pub struct InsertNewline;
impl<'a> Action<Context<'a>> for InsertNewline {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::dismiss_completion(ctx);
        crate::dispatch::replace_selection(ctx, "\n");
    }
}

pub struct InsertTab;
impl<'a> Action<Context<'a>> for InsertTab {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::dismiss_completion(ctx);
        crate::dispatch::replace_selection(ctx, "    ");
    }
}

pub struct DeleteBack { pub word: bool }
impl<'a> Action<Context<'a>> for DeleteBack {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let word = self.word;
        let keep_completion = !word;
        let saved = if keep_completion {
            crate::dispatch::take_completion(ctx)
        } else {
            None
        };
        crate::dispatch::delete_each_or(ctx, |buf, head| {
            if head == 0 {
                return None;
            }
            let start = if word { buf.word_left(head) } else { head - 1 };
            Some((start, head))
        });
        if let Some(state) = saved {
            if let Some((_, vid, _)) = ctx.surface.active_ids() {
                ctx.surface.views[vid].completion = Some(state);
                crate::dispatch::refilter_completion(ctx.surface, vid);
            }
        }
    }
}

pub struct DeleteForward { pub word: bool }
impl<'a> Action<Context<'a>> for DeleteForward {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let word = self.word;
        crate::dispatch::dismiss_completion(ctx);
        crate::dispatch::delete_each_or(ctx, |buf, head| {
            let len = buf.len_chars();
            if head >= len {
                return None;
            }
            let end = if word { buf.word_right(head) } else { head + 1 };
            Some((head, end))
        });
    }
}

// --- Multi-cursor ---------------------------------------------------------

/// Add a point cursor one line above the primary head, at the same column
/// (clamped to the new line's width). Repeated presses extend upward
/// because `push_range` makes the new range the primary.
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
        v.hover = None;
        v.completion = None;
        v.scroll_mode = crate::view::ScrollMode::Anchored;
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
        v.hover = None;
        v.completion = None;
        v.scroll_mode = crate::view::ScrollMode::Anchored;
    }
}

/// Esc-equivalent: drop secondary cursors back to the primary. With a
/// single, non-empty range, collapse it to a point at the head — same
/// "press Esc to deselect" UX modern editors share.
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
        v.hover = None;
        v.completion = None;
        v.scroll_mode = crate::view::ScrollMode::Anchored;
    }
}

// --- LSP: hover / goto / completion ---------------------------------------

pub struct Hover;
impl<'a> Action<Context<'a>> for Hover {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use devix_editor::{HoverState, HoverStatus};
        use devix_lsp::LspCommand;
        let Some(req) = crate::dispatch::lsp_position_request(ctx.surface) else { return };
        let _ = req.wiring.sink.send(LspCommand::Hover {
            uri: req.uri,
            position: req.position,
            anchor_char: req.head,
        });
        ctx.surface.views[req.vid].hover = Some(HoverState {
            anchor_char: req.head,
            status: HoverStatus::Pending,
        });
    }
}

pub struct GotoDefinition;
impl<'a> Action<Context<'a>> for GotoDefinition {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use devix_lsp::LspCommand;
        let Some(req) = crate::dispatch::lsp_position_request(ctx.surface) else { return };
        let _ = req.wiring.sink.send(LspCommand::GotoDefinition {
            uri: req.uri,
            position: req.position,
            anchor_char: req.head,
        });
    }
}

pub struct TriggerCompletion(pub devix_lsp::CompletionTrigger);
impl<'a> Action<Context<'a>> for TriggerCompletion {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use devix_editor::{CompletionState, CompletionStatus};
        use devix_lsp::LspCommand;
        let Some(req) = crate::dispatch::lsp_position_request(ctx.surface) else { return };
        let _ = req.wiring.sink.send(LspCommand::Completion {
            uri: req.uri,
            position: req.position,
            anchor_char: req.head,
            trigger: self.0.clone(),
        });
        let did = ctx.surface.views[req.vid].doc;
        let query_start =
            crate::dispatch::ident_start_at(&ctx.surface.documents[did].buffer, req.head);
        ctx.surface.views[req.vid].completion = Some(CompletionState {
            anchor_char: req.head,
            query_start,
            items: Vec::new(),
            labels_lower: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            status: CompletionStatus::Pending,
        });
    }
}

pub struct CompletionAccept;
impl<'a> Action<Context<'a>> for CompletionAccept {
    fn invoke(&self, ctx: &mut Context<'a>) {
        crate::dispatch::apply_completion_accept(ctx);
    }
}

// --- Scroll / Mouse ------------------------------------------------------

pub struct ScrollBy(pub isize);
impl<'a> Action<Context<'a>> for ScrollBy {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use crate::view::ScrollMode;
        let Some((_, vid, did)) = ctx.surface.active_ids() else { return };
        let line_count = ctx.surface.documents[did].buffer.line_count();
        let v = &mut ctx.surface.views[vid];
        let max_top = line_count.saturating_sub(1);
        let next = (v.scroll_top() as isize).saturating_add(self.0);
        let clamped = next.clamp(0, max_top as isize) as usize;
        v.set_scroll_top(clamped);
        v.scroll_mode = ScrollMode::Free;
    }
}

// --- Symbol picker --------------------------------------------------------

pub struct ShowDocumentSymbols;
impl<'a> Action<Context<'a>> for ShowDocumentSymbols {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use crate::modal::{SymbolPickerPane, SymbolsKind};
        use devix_lsp::LspCommand;
        let Some((_, _vid, did)) = ctx.surface.active_ids() else { return };
        let Some(wiring) = ctx.surface.lsp_channel() else {
            ctx.status.set("LSP not attached for this document");
            return;
        };
        let Some(uri) = ctx.surface.documents[did].lsp_uri().cloned() else {
            ctx.status.set("no symbols: doc not attached to a language server");
            return;
        };
        let pane = SymbolPickerPane::new(SymbolsKind::Document, Some(uri.clone()));
        let _ = wiring.sink.send(LspCommand::DocumentSymbols { uri, epoch: pane.state.epoch });
        ctx.surface.modal = Some(Box::new(pane));
    }
}

pub struct ShowWorkspaceSymbols;
impl<'a> Action<Context<'a>> for ShowWorkspaceSymbols {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use crate::modal::{SymbolPickerPane, SymbolsKind};
        use devix_lsp::LspCommand;
        let Some(wiring) = ctx.surface.lsp_channel() else {
            ctx.status.set("LSP not attached");
            return;
        };
        let pane = SymbolPickerPane::new(SymbolsKind::Surface, None);
        let _ = wiring.sink.send(LspCommand::WorkspaceSymbols {
            query: pane.state.query.clone(),
            epoch: pane.state.epoch,
        });
        ctx.surface.modal = Some(Box::new(pane));
    }
}

pub struct SymbolsSetQuery(pub String);
impl<'a> Action<Context<'a>> for SymbolsSetQuery {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use crate::modal::SymbolsKind;
        use devix_lsp::LspCommand;
        let Some(s) = downcast_modal_mut::<crate::modal::SymbolPickerPane>(ctx) else { return };
        let needs_refetch = s.state.kind == SymbolsKind::Surface;
        s.state.set_query(self.0.clone());
        if needs_refetch {
            let epoch = s.state.epoch;
            let query = s.state.query.clone();
            if let Some(wiring) = ctx.surface.lsp_channel() {
                let _ = wiring.sink.send(LspCommand::WorkspaceSymbols { query, epoch });
            }
        }
    }
}

/// Refetch surface-symbols using the modal's current query. Called by
/// the responder chain after the modal Pane signals
/// `ModalOutcome::Refetch` (typing a char / backspace in surface mode).
pub struct RefetchWorkspaceSymbols;
impl<'a> Action<Context<'a>> for RefetchWorkspaceSymbols {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use crate::modal::SymbolsKind;
        use devix_lsp::LspCommand;
        let Some(s) = downcast_modal_mut::<crate::modal::SymbolPickerPane>(ctx) else { return };
        if s.state.kind != SymbolsKind::Surface { return }
        let epoch = s.state.epoch;
        let query = s.state.query.clone();
        if let Some(wiring) = ctx.surface.lsp_channel() {
            let _ = wiring.sink.send(LspCommand::WorkspaceSymbols { query, epoch });
        }
    }
}

pub struct SymbolsAccept;
impl<'a> Action<Context<'a>> for SymbolsAccept {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let location = downcast_modal_mut::<crate::modal::SymbolPickerPane>(ctx)
            .and_then(|s| s.state.selected_symbol().map(|sym| sym.location.clone()));
        ctx.surface.modal = None;
        if let Some(loc) = location {
            crate::dispatch::jump_to_location(ctx, loc);
        }
    }
}

// --- Mouse ----------------------------------------------------------------

pub struct ClickAt {
    pub col: u16,
    pub row: u16,
    pub extend: bool,
}
impl<'a> Action<Context<'a>> for ClickAt {
    fn invoke(&self, ctx: &mut Context<'a>) {
        // Prefer focusing-by-click first; the existing dispatch arm relies on
        // `click_to_char_idx` to also resolve which frame's body the click
        // landed in. Keep that here.
        ctx.surface.focus_at_screen(self.col, self.row);
        let Some(idx) = crate::dispatch::click_to_char_idx(ctx, self.col, self.row) else {
            return;
        };
        if let Some(v) = ctx.surface.active_view_mut() {
            v.move_to(idx, self.extend, false);
        }
    }
}

pub struct DragAt { pub col: u16, pub row: u16 }
impl<'a> Action<Context<'a>> for DragAt {
    fn invoke(&self, ctx: &mut Context<'a>) {
        let Some(idx) = crate::dispatch::click_to_char_idx(ctx, self.col, self.row) else {
            return;
        };
        if let Some(v) = ctx.surface.active_view_mut() {
            v.move_to(idx, true, false);
        }
    }
}

// --- Recursive: PaletteAccept and InsertChar ------------------------------
//
// These two are the only variants that need to dispatch *another* action
// during their own execution. PaletteAccept hands off to whatever command
// the user picked; InsertChar tail-fires `TriggerCompletion` after typing
// a `.` or `:`. They keep a `dispatch::dispatch` call in `invoke` — the
// one piece of the action surface where the trait surface alone isn't
// expressive enough until the keymap also stores trait-objects.

pub struct PaletteAccept;
impl<'a> Action<Context<'a>> for PaletteAccept {
    fn invoke(&self, ctx: &mut Context<'a>) {
        // Resolve the chosen command into an `Arc` clone, then drop the
        // immutable registry borrow before invoking — `invoke` takes
        // `&mut Context`, and the registry borrow goes through `ctx`.
        let chosen = ctx
            .surface
            .modal
            .as_ref()
            .and_then(|m| m.as_any())
            .and_then(|a| a.downcast_ref::<crate::modal::PalettePane>())
            .and_then(|p| {
                p.state
                    .selected_command_id()
                    .and_then(|id| ctx.commands.resolve(id))
            });
        ctx.surface.modal = None;
        if let Some(action) = chosen {
            action.invoke(ctx);
        }
    }
}

pub struct InsertChar(pub char);
impl<'a> Action<Context<'a>> for InsertChar {
    fn invoke(&self, ctx: &mut Context<'a>) {
        use devix_lsp::CompletionTrigger;
        const TRIGGER_CHARS: &[char] = &['.', ':'];
        let saved = crate::dispatch::take_completion(ctx);
        let mut buf = [0u8; 4];
        crate::dispatch::replace_selection(ctx, self.0.encode_utf8(&mut buf));
        if TRIGGER_CHARS.contains(&self.0) {
            drop(saved);
            TriggerCompletion(CompletionTrigger::Char(self.0)).invoke(ctx);
        } else if let Some(state) = saved {
            if let Some((_, vid, _)) = ctx.surface.active_ids() {
                ctx.surface.views[vid].completion = Some(state);
                crate::dispatch::refilter_completion(ctx.surface, vid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandRegistry;
    use crate::context::{StatusLine, Viewport};
    use crate::surface::Surface;

    fn make_ctx<'a>(
        ws: &'a mut Surface,
        clipboard: &'a mut Option<arboard::Clipboard>,
        status: &'a mut StatusLine,
        quit: &'a mut bool,
        commands: &'a CommandRegistry,
    ) -> Context<'a> {
        Context {
            surface: ws,
            clipboard,
            status,
            quit,
            viewport: Viewport::default(),
            commands,
        }
    }

    #[test]
    fn quit_sets_the_quit_flag_through_the_trait() {
        let mut ws = Surface::open(None).unwrap();
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        Quit.invoke(&mut ctx);
        assert!(quit, "Quit action should set the quit flag");
    }

    #[test]
    fn quit_can_be_stored_as_box_dyn_editor_command() {
        let _: Box<dyn EditorCommand> = Box::new(Quit);
    }

    #[test]
    fn parameterized_commands_dispatch_through_trait_objects() {
        let mut ws = Surface::open(None).unwrap();
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();

        let actions: Vec<Box<dyn EditorCommand>> =
            vec![Box::new(NewTab), Box::new(NextTab), Box::new(NewTab)];

        for action in &actions {
            let mut ctx =
                make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
            action.invoke(&mut ctx);
        }
        let fid = ws.active_frame().unwrap();
        let frame = crate::tree::find_frame(ws.root.as_ref(), fid).unwrap();
        assert_eq!(frame.tabs.len(), 3);
    }

    #[test]
    fn open_palette_populates_modal_slot() {
        let mut ws = Surface::open(None).unwrap();
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        OpenPalette.invoke(&mut ctx);
        assert!(ws.modal.is_some());
        assert!(
            ws.modal
                .as_ref()
                .and_then(|m| m.as_any())
                .map(|a| a.is::<crate::modal::PalettePane>())
                .unwrap_or(false),
            "modal slot should hold a PalettePane",
        );
    }

    fn surface_with_text(text: &str) -> Surface {
        use devix_text::{Selection, replace_selection_tx};
        let mut ws = Surface::open(None).unwrap();
        let did = ws.active_view().unwrap().doc;
        let tx = replace_selection_tx(&ws.documents[did].buffer, &Selection::point(0), text);
        ws.documents[did].buffer.apply(tx);
        let vid = ws.active_ids().unwrap().1;
        // Place primary at start so AddCursorBelow lands inside the buffer.
        ws.views[vid].selection = Selection::point(0);
        ws
    }

    #[test]
    fn add_cursor_below_inserts_at_each_cursor() {
        let mut ws = surface_with_text("aa\nbb\ncc");
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        AddCursorBelow.invoke(&mut ctx);
        AddCursorBelow.invoke(&mut ctx);
        // Now three point cursors at start of lines 0, 1, 2.
        InsertChar('x').invoke(&mut ctx);
        let did = ws.active_view().unwrap().doc;
        assert_eq!(ws.documents[did].buffer.rope().to_string(), "xaa\nxbb\nxcc");
        // All three cursors survived and advanced past the inserted char.
        let vid = ws.active_ids().unwrap().1;
        let sel = &ws.views[vid].selection;
        assert_eq!(sel.len(), 3);
        for r in sel.ranges() {
            let buf = &ws.documents[did].buffer;
            assert_eq!(buf.col_of_char(r.head), 1);
        }
    }

    #[test]
    fn add_cursor_above_at_line_zero_is_noop() {
        let mut ws = surface_with_text("aa\nbb");
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        AddCursorAbove.invoke(&mut ctx);
        let vid = ws.active_ids().unwrap().1;
        assert_eq!(ws.views[vid].selection.len(), 1);
    }

    #[test]
    fn motion_transforms_every_cursor() {
        let mut ws = surface_with_text("aaaa\nbbbb");
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        AddCursorBelow.invoke(&mut ctx);
        // Two cursors at line 0 col 0 and line 1 col 0.
        MoveRight { extend: false }.invoke(&mut ctx);
        MoveRight { extend: false }.invoke(&mut ctx);
        let vid = ws.active_ids().unwrap().1;
        let did = ws.views[vid].doc;
        let buf = &ws.documents[did].buffer;
        let cols: Vec<usize> = ws.views[vid]
            .selection
            .ranges()
            .iter()
            .map(|r| buf.col_of_char(r.head))
            .collect();
        assert_eq!(cols, vec![2, 2]);
    }

    #[test]
    fn delete_back_removes_one_char_per_cursor() {
        let mut ws = surface_with_text("aa\nbb");
        let vid0 = ws.active_ids().unwrap().1;
        // Set both cursors at end of each line.
        ws.views[vid0].selection = devix_text::Selection::with_ranges(
            vec![devix_text::Range::point(2), devix_text::Range::point(5)],
            0,
        );
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        DeleteBack { word: false }.invoke(&mut ctx);
        let did = ws.active_view().unwrap().doc;
        assert_eq!(ws.documents[did].buffer.rope().to_string(), "a\nb");
    }

    #[test]
    fn collapse_selection_drops_secondary_cursors() {
        let mut ws = surface_with_text("aa\nbb\ncc");
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        AddCursorBelow.invoke(&mut ctx);
        AddCursorBelow.invoke(&mut ctx);
        CollapseSelection.invoke(&mut ctx);
        let vid = ws.active_ids().unwrap().1;
        assert_eq!(ws.views[vid].selection.len(), 1);
    }

    #[test]
    fn close_modal_clears_any_modal() {
        let mut ws = Surface::open(None).unwrap();
        ws.modal = Some(Box::new(crate::modal::PalettePane::from_registry(
            &CommandRegistry::default(),
        )));
        let mut clipboard = None;
        let mut status = StatusLine::default();
        let mut quit = false;
        let commands = CommandRegistry::default();
        let mut ctx = make_ctx(&mut ws, &mut clipboard, &mut status, &mut quit, &commands);
        CloseModal.invoke(&mut ctx);
        assert!(ws.modal.is_none());
    }
}
