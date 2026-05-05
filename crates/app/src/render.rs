//! Frame composition: editor area + status line.
//!
//! Two distinct phases per draw cycle:
//!
//! 1. [`layout_pass`] — pre-paint state mutation. Walks every `Frame` leaf,
//!    runs the cursor-anchor pass on its active `View.scroll`, and clamps any
//!    stale scroll offsets against the new body geometry. This is the *only*
//!    place the renderer mutates editor state. Mirrors UIKit's
//!    `viewWillLayoutSubviews`.
//! 2. [`paint`] — pure draw + render-cache updates. Every cached rect /
//!    tab-strip hit-list / sidebar rect is written here as a record of what
//!    the frame just painted; no view, document, or scroll mutation happens.
//!
//! Per PLAN.md rule 3 ("render is pure"), the second pass MUST NOT touch
//! anything in `Surface` other than the `RenderCache`.

use devix_core::{Pane, RenderCtx};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use devix_ui::layout::{VRect, ensure_visible, set_scroll};
use devix_ui::{
    SidebarPane as SidebarChrome, StatusInfo, StatusPane, TabStripPane, layout_tabstrip,
    tab_strip_layout,
};
use devix_views::{EditorPane, SidebarSlotPane, TabbedPane};
use devix_surface::{
    Document, FrameId, LeafRef, PalettePane, ScrollMode, SidebarSlot, SymbolPickerPane, View,
    Surface, palette_area, render_palette, render_symbols, symbols_area,
};

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];

    let leaves =
        devix_surface::leaves_with_rects(app.surface.root.as_ref(), editor_area);

    // Phase 1 — layout: scroll-into-view + clamp.
    layout_pass(&leaves, &mut app.surface);

    // Phase 2 — paint (pure, plus render-cache writes).
    paint(&leaves, app, frame);

    render_status(frame, status_area, app);

    // Modal Panes paint last (z-order is paint order in ratatui). The
    // modal slot lives on `Surface`; the host downcasts to known modal
    // types for their surface-aux render path (palette needs the
    // command registry + keymap; symbols needs the theme), then falls
    // back to the modal Pane's own `render` for plugin-contributed
    // modals — the framework never matches on a kind enum.
    if let Some(modal) = app.surface.modal.as_ref() {
        let any = modal.as_any();
        if let Some(p) = any.and_then(|a| a.downcast_ref::<PalettePane>()) {
            render_palette(
                &p.state,
                &app.commands,
                &app.keymap,
                &app.theme,
                palette_area(editor_area),
                frame,
            );
        } else if let Some(s) = any.and_then(|a| a.downcast_ref::<SymbolPickerPane>()) {
            render_symbols(&s.state, &app.theme, symbols_area(editor_area), frame);
        } else {
            let mut overlay_ctx = RenderCtx { frame };
            modal.render(editor_area, &mut overlay_ctx);
        }
    }
}

/// Mutate every `Frame`'s active `View.scroll` so the next paint pass renders
/// the cursor in view (Anchored mode) or against a clamped offset (Free mode),
/// and run the tab-strip's pre-paint scroll math.  No painting, no cache
/// writes — those happen in [`paint`].
fn layout_pass(leaves: &[(LeafRef, Rect)], ws: &mut Surface) {
    for (leaf, rect) in leaves {
        let LeafRef::Frame(fid) = leaf else { continue };
        let strip_area = Rect { height: 1, ..*rect };
        let body_area = frame_body_rect(*rect);

        // Tab-strip layout: clamp on resize/tab-close and consume the
        // recenter-active one-shot. Done here so paint can stay pure.
        let tabs: Vec<devix_ui::TabInfo> = match devix_surface::find_frame(ws.root.as_ref(), *fid) {
            Some(frame) => frame
                .tabs
                .iter()
                .map(|vid| {
                    let v = &ws.views[*vid];
                    let d = &ws.documents[v.doc];
                    let label = d.buffer.path()
                        .and_then(|p| p.file_name())
                        .and_then(|f| f.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "[scratch]".to_string());
                    devix_ui::TabInfo { label, dirty: d.buffer.dirty() }
                })
                .collect(),
            None => continue,
        };
        let Some(active_tab) = devix_surface::find_frame(ws.root.as_ref(), *fid)
            .map(|f| f.active_tab) else { continue };
        let Some(f) = devix_surface::find_frame_mut(&mut ws.root, *fid) else { continue };
        layout_tabstrip(
            &tabs,
            active_tab,
            &mut f.tab_strip_scroll,
            &mut f.recenter_active,
            strip_area,
        );

        let Some(vid) = devix_surface::find_frame(ws.root.as_ref(), *fid)
            .and_then(|f| f.active_view()) else { continue };
        let view = &ws.views[vid];
        let doc = &ws.documents[view.doc];

        let head = view.primary().head;
        let cur_line = doc.buffer.line_of_char(head);
        let line_count = doc.buffer.line_count();
        let scroll_mode = view.scroll_mode;
        let body_w = body_area.width as u32;
        let body_h = body_area.height as u32;
        if body_h == 0 { continue; }

        let content = (body_w, line_count.max(1) as u32);
        let viewport = (body_w, body_h);
        let v = &mut ws.views[vid];
        match scroll_mode {
            ScrollMode::Anchored => {
                // One-line virtual rect for the cursor's line; ensure_visible
                // bumps scroll the minimum amount needed to show it. No-op
                // when the cursor is already in view.
                let line_rect = VRect { x: 0, y: cur_line as u32, w: body_w, h: 1 };
                ensure_visible(&mut v.scroll, line_rect, content, viewport);
            }
            ScrollMode::Free => {
                // Re-clamp so resize / line-count changes don't leave a stale
                // out-of-bounds scroll.
                let (sx, sy) = v.scroll;
                set_scroll(&mut v.scroll, sx, sy, content, viewport);
            }
        }
    }
}

/// Pure paint via the composite Pane tree. Run in two passes:
///
/// 1. `populate_cache` pre-fills the surface's `RenderCache` (sidebar
///    rects, frame body rects, tab-strip hit lists) using read-only
///    layout helpers — no painting, no view/document mutation.
/// 2. `paint_leaves` builds a `TabbedPane` or `SidebarSlotPane` per leaf
///    and calls its `render`. Each Pane is `&self`-pure; the surface
///    is borrowed shared, not mutably.
///
/// Splitting the work this way is what lets `Pane::render(&self)` stay
/// honest. Cache writes used to happen inside `paint_frame` while the
/// renderer was running — the new shape moves them ahead of paint into
/// pure layout math.
fn paint(leaves: &[(LeafRef, Rect)], app: &mut App, frame: &mut Frame<'_>) {
    populate_cache(leaves, &mut app.surface);
    paint_leaves(leaves, app, frame);
}

/// Pre-paint cache population. Walks the leaves once, computes geometry
/// (frame body rect, sidebar rect, tab-strip hits/content width) via
/// read-only helpers, and writes the result to `RenderCache`. No
/// painting, no scroll mutation (that already happened in `layout_pass`).
fn populate_cache(leaves: &[(LeafRef, Rect)], ws: &mut Surface) {
    ws.render_cache.frame_rects.clear();
    ws.render_cache.sidebar_rects.clear();
    ws.render_cache.tab_strips.clear();
    for (leaf, rect) in leaves {
        match leaf {
            LeafRef::Sidebar(slot) => {
                ws.render_cache.sidebar_rects.insert(*slot, *rect);
            }
            LeafRef::Frame(fid) => {
                let strip_area = Rect { height: 1, ..*rect };
                let body_area = frame_body_rect(*rect);
                let tabs = build_tab_infos(ws, *fid);
                let Some(frame_state) = devix_surface::find_frame(ws.root.as_ref(), *fid)
                else { continue };
                let active = frame_state.active_tab;
                let scroll = frame_state.tab_strip_scroll;
                let (hits_pure, content_width) =
                    tab_strip_layout(&tabs, active, scroll, strip_area);
                let hits = hits_pure
                    .iter()
                    .map(|h| devix_surface::TabHit { idx: h.idx, rect: h.rect })
                    .collect();
                ws.render_cache.tab_strips.insert(
                    *fid,
                    devix_surface::TabStripCache {
                        strip_rect: strip_area,
                        content_width,
                        hits,
                    },
                );
                ws.render_cache.frame_rects.insert(*fid, body_area);
            }
        }
    }
}

/// Paint pass: build a `Pane` per leaf and render it. The surface is
/// borrowed shared — every mutation already happened in `layout_pass` /
/// `populate_cache`.
fn paint_leaves(leaves: &[(LeafRef, Rect)], app: &App, frame: &mut Frame<'_>) {
    let mut ctx = RenderCtx { frame };
    for (leaf, rect) in leaves {
        match leaf {
            LeafRef::Frame(id) => {
                let pane = build_tabbed_pane(app, *id);
                pane.render(*rect, &mut ctx);
            }
            LeafRef::Sidebar(slot) => {
                let pane = build_sidebar_pane(app, *slot);
                pane.render(*rect, &mut ctx);
            }
        }
    }
}

/// Construct a `TabbedPane` for `frame`. Borrows from the surface; the
/// returned Pane lives only as long as the borrow.
fn build_tabbed_pane<'a>(app: &'a App, frame: FrameId) -> TabbedPane<'a> {
    let f = devix_surface::find_frame(app.surface.root.as_ref(), frame)
        .expect("active frame must exist in tree");
    let strip = TabStripPane {
        tabs: build_tab_infos(&app.surface, frame),
        active: f.active_tab,
        scroll: f.tab_strip_scroll,
    };
    // `TabbedPane` always wants an editor child. If the frame somehow has
    // no active view (transient state during tab close), build an empty
    // EditorPane against a zero-length scratch borrow — the renderer just
    // paints nothing.
    let view_id = f.active_view().expect("frame must have an active view");
    let view = &app.surface.views[view_id];
    let doc = &app.surface.documents[view.doc];
    // Highlights are scoped to a generous viewport; the actual paint area
    // lives downstream (TabbedPane.children() splits) but the over-set is
    // safe — highlights past the body just don't render. Using the cached
    // body rect from the previous frame would be exact but couples the
    // build to render order.
    let cached_body = app
        .surface
        .render_cache
        .frame_rects
        .get(&frame)
        .copied()
        .unwrap_or(Rect { x: 0, y: 0, width: 0, height: 0 });
    let height_rows = cached_body.height as usize;
    let (s, e) = visible_byte_range(doc, view, height_rows);
    let highlights = doc.highlights(s, e);
    let editor = EditorPane {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll: view.scroll,
        theme: &app.theme,
        highlights,
        diagnostics: doc.diagnostics(),
        active: app.surface.active_frame() == Some(frame),
        hover: view.hover.as_ref(),
        completion: view.completion.as_ref(),
    };
    TabbedPane { strip, editor }
}

/// Construct a `SidebarSlotPane` for `slot`. Chrome-only for now; future
/// plugins drop content via the `content` field.
fn build_sidebar_pane<'a>(app: &'a App, slot: SidebarSlot) -> SidebarSlotPane<'a> {
    let title = match slot {
        SidebarSlot::Left => "left",
        SidebarSlot::Right => "right",
    };
    let focused = devix_surface::pane_at_indices(
        app.surface.root.as_ref(),
        &app.surface.focus,
    )
    .and_then(devix_surface::pane_leaf_id)
    .map(|id| matches!(id, LeafRef::Sidebar(s) if s == slot))
    .unwrap_or(false);
    SidebarSlotPane {
        chrome: SidebarChrome { title: title.to_string(), focused },
        content: None,
    }
}

/// Build the per-tab label info for a frame's strip. Same logic as the
/// previous inline build in `layout_pass` / `paint_frame`, factored so
/// both the cache pass and the render pass produce identical labels.
fn build_tab_infos(ws: &Surface, frame: FrameId) -> Vec<devix_ui::TabInfo> {
    let Some(frame_state) = devix_surface::find_frame(ws.root.as_ref(), frame) else {
        return Vec::new();
    };
    frame_state
        .tabs
        .iter()
        .map(|vid| {
            let v = &ws.views[*vid];
            let d = &ws.documents[v.doc];
            let label = d
                .buffer
                .path()
                .and_then(|p| p.file_name())
                .and_then(|f| f.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "[scratch]".to_string());
            devix_ui::TabInfo { label, dirty: d.buffer.dirty() }
        })
        .collect()
}

fn frame_body_rect(area: Rect) -> Rect {
    Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    }
}

/// Byte range covering all lines currently visible in `view`'s editor body.
/// Used to scope tree-sitter highlight queries to the viewport rather than
/// the whole document — full-file queries on large buffers would push past
/// the 16ms frame budget.
fn visible_byte_range(doc: &Document, view: &View, height_rows: usize) -> (usize, usize) {
    let line_count = doc.buffer.line_count();
    let rope = doc.buffer.rope();
    let top = view.scroll_top().min(line_count);
    let bottom = (view.scroll_top() + height_rows).min(line_count);
    let start = rope.line_to_byte(top);
    let end = if bottom >= line_count {
        rope.len_bytes()
    } else {
        rope.line_to_byte(bottom)
    };
    (start, end)
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(view) = app.surface.active_view() else { return };
    let doc = &app.surface.documents[view.doc];
    let path_str = doc.buffer.path().map(|p| p.display().to_string());
    let head = view.primary().head;
    let (errors, warnings) = count_diagnostics(doc);
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: doc.buffer.dirty(),
        line: doc.buffer.line_of_char(head) + 1,
        col: doc.buffer.col_of_char(head) + 1,
        sel_len: view.primary().len(),
        message: app.status.get(),
        diag_errors: errors,
        diag_warnings: warnings,
    };
    // Phase 1 of the architecture refactor: drive the status line through
    // the Pane adapter so the new trait surface is exercised end-to-end.
    // Other render sites still call the free functions directly until their
    // own migration phase.
    let mut ctx = RenderCtx { frame };
    StatusPane { info }.render(area, &mut ctx);
}

fn count_diagnostics(doc: &Document) -> (usize, usize) {
    use lsp_types::DiagnosticSeverity;
    let mut e = 0;
    let mut w = 0;
    for d in doc.diagnostics() {
        match d.severity {
            DiagnosticSeverity::ERROR => e += 1,
            DiagnosticSeverity::WARNING => w += 1,
            _ => {}
        }
    }
    (e, w)
}
