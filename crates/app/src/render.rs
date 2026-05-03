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
//! anything in `Workspace` other than the `RenderCache`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use devix_collection::VRect;
use devix_ui::{
    EditorView, StatusInfo, render_editor, render_status as render_status_widget,
    render_tabstrip,
};
use devix_workspace::{FrameId, LeafRef, ScrollMode, SidebarSlot, Workspace};

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];

    let leaves = app.workspace.layout.leaves_with_rects(editor_area);

    // Phase 1 — layout: scroll-into-view + clamp.
    layout_pass(&leaves, &mut app.workspace);

    // Phase 2 — paint (pure, plus render-cache writes).
    paint(&leaves, app, frame);

    render_status(frame, status_area, app);
}

/// Mutate every `Frame`'s active `View.scroll` so the next paint pass renders
/// the cursor in view (Anchored mode) or against a clamped offset (Free mode).
/// No painting, no cache writes — those happen in [`paint`].
fn layout_pass(leaves: &[(LeafRef, Rect)], ws: &mut Workspace) {
    for (leaf, rect) in leaves {
        let LeafRef::Frame(fid) = leaf else { continue };
        let body_area = frame_body_rect(*rect);
        let Some(vid) = ws.frames[*fid].active_view() else { continue };
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
                v.scroll.ensure_visible(line_rect, content, viewport);
            }
            ScrollMode::Free => {
                // Re-clamp so resize / line-count changes don't leave a stale
                // out-of-bounds scroll.
                v.scroll.set_scroll(v.scroll.scroll_x, v.scroll.scroll_y, content, viewport);
            }
        }
    }
}

/// Pure paint over already-laid-out state. Writes only the `RenderCache`,
/// never the document/view scroll/selection.
fn paint(leaves: &[(LeafRef, Rect)], app: &mut App, frame: &mut Frame<'_>) {
    app.workspace.render_cache.frame_rects.clear();
    app.workspace.render_cache.sidebar_rects.clear();
    app.workspace.render_cache.tab_strips.clear();
    for (leaf, rect) in leaves {
        if let LeafRef::Sidebar(slot) = leaf {
            app.workspace.render_cache.sidebar_rects.insert(*slot, *rect);
        }
    }
    for (leaf, rect) in leaves {
        match leaf {
            LeafRef::Frame(id) => paint_frame(*id, *rect, app, frame),
            LeafRef::Sidebar(slot) => paint_sidebar(*slot, *rect, app, frame),
        }
    }
}

fn paint_frame(id: FrameId, area: Rect, app: &mut App, frame: &mut Frame<'_>) {
    let strip_area = Rect { height: 1, ..area };
    let body_area = frame_body_rect(area);

    let tabs: Vec<devix_ui::TabInfo> = app.workspace.frames[id]
        .tabs
        .iter()
        .map(|vid| {
            let v = &app.workspace.views[*vid];
            let d = &app.workspace.documents[v.doc];
            let label = d.buffer.path()
                .and_then(|p| p.file_name())
                .and_then(|f| f.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "[scratch]".to_string());
            devix_ui::TabInfo { label, dirty: d.buffer.dirty() }
        })
        .collect();
    let active_tab = app.workspace.frames[id].active_tab;
    let f = &mut app.workspace.frames[id];
    let render = render_tabstrip(
        &tabs,
        active_tab,
        &mut f.tab_strip_state,
        &mut f.recenter_active,
        strip_area,
        frame,
    );
    app.workspace.render_cache.tab_strips.insert(
        id,
        devix_workspace::TabStripCache {
            strip_rect: strip_area,
            content_width: render.content_width,
            hits: render.hits.clone(),
        },
    );

    let Some(view_id) = app.workspace.frames[id].active_view() else { return };
    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];
    let editor_view = EditorView {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll: &view.scroll,
    };
    let r = render_editor(editor_view, body_area, frame);
    if app.workspace.active_frame() == Some(id) {
        if let Some((x, y)) = r.cursor_screen { frame.set_cursor_position((x, y)); }
    }
    // Cache the body rect (not strip) so hit-tests aim at the editor.
    app.workspace.render_cache.frame_rects.insert(id, body_area);
}

fn paint_sidebar(slot: SidebarSlot, area: Rect, app: &App, frame: &mut Frame<'_>) {
    let title = match slot {
        SidebarSlot::Left => "left",
        SidebarSlot::Right => "right",
    };
    let focused = matches!(
        app.workspace.layout.leaf_at(&app.workspace.focus),
        Some(LeafRef::Sidebar(s)) if s == slot
    );
    devix_ui::render_sidebar(&devix_ui::SidebarInfo { title, focused }, area, frame);
}

fn frame_body_rect(area: Rect) -> Rect {
    Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    }
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(view) = app.workspace.active_view() else { return };
    let doc = &app.workspace.documents[view.doc];
    let path_str = doc.buffer.path().map(|p| p.display().to_string());
    let head = view.primary().head;
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: doc.buffer.dirty(),
        line: doc.buffer.line_of_char(head) + 1,
        col: doc.buffer.col_of_char(head) + 1,
        sel_len: view.primary().len(),
        message: app.status.get(),
    };
    render_status_widget(&info, area, frame);
}
