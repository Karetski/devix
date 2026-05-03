//! Frame composition: editor area + status line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use devix_ui::{
    EditorView, StatusInfo, render_editor, render_status as render_status_widget,
    render_tabstrip,
};
use devix_workspace::LeafRef;

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];

    // Compute every leaf's Rect from the layout tree. Sidebar entries are
    // cached up-front; frame entries are written by render_frame using the
    // body rect (after carving out the 1-row tab strip).
    let leaves = app.workspace.layout.leaves_with_rects(editor_area);
    app.workspace.render_cache.frame_rects.clear();
    app.workspace.render_cache.sidebar_rects.clear();
    app.workspace.render_cache.tab_strips.clear();
    for (leaf, rect) in &leaves {
        if let LeafRef::Sidebar(slot) = leaf {
            app.workspace.render_cache.sidebar_rects.insert(*slot, *rect);
        }
    }

    // Render every leaf. (Sidebar painting comes in Task 10.)
    for (leaf, rect) in &leaves {
        match leaf {
            LeafRef::Frame(id) => render_frame(*id, *rect, app, frame),
            LeafRef::Sidebar(slot) => {
                let title = match slot {
                    devix_workspace::SidebarSlot::Left => "left",
                    devix_workspace::SidebarSlot::Right => "right",
                };
                let focused = matches!(
                    app.workspace.layout.leaf_at(&app.workspace.focus),
                    Some(devix_workspace::LeafRef::Sidebar(s)) if s == *slot
                );
                devix_ui::render_sidebar(
                    &devix_ui::SidebarInfo { title, focused },
                    *rect,
                    frame,
                );
            }
        }
    }

    render_status(frame, status_area, app);
}

fn render_frame(id: devix_workspace::FrameId, area: Rect, app: &mut App, frame: &mut Frame<'_>) {
    let strip_area = Rect { height: 1, ..area };
    let body_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };

    // Build TabInfo for each tab in this frame.
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
            hits: render.hits.iter()
                .map(|h| devix_workspace::TabHit { idx: h.idx, rect: h.rect })
                .collect(),
        },
    );

    // Editor body: anchor-pass + draw flow against body_area.
    let view_id = match app.workspace.frames[id].active_view() {
        Some(v) => v,
        None => return, // empty frame — nothing to draw
    };
    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];

    let head = view.primary().head;
    let cur_line = doc.buffer.line_of_char(head);
    let line_count = doc.buffer.line_count();
    let view_anchored = view.view_anchored;
    let body_h = body_area.height as u32;
    let body_w = body_area.width as u32;
    if view_anchored && body_h > 0 {
        // One-line virtual rect for the cursor's line; ensure_visible bumps
        // scroll the minimum amount needed to show it. No-op when the cursor
        // is already in view.
        let line_rect = devix_collection::VRect { x: 0, y: cur_line as u32, w: body_w, h: 1 };
        let content = (body_w, line_count.max(1) as u32);
        let viewport = (body_w, body_h);
        app.workspace.views[view_id].scroll
            .ensure_visible(line_rect, content, viewport);
    } else {
        // Re-clamp anyway so resize / line-count changes don't leave a stale
        // out-of-bounds scroll.
        let v = &mut app.workspace.views[view_id];
        let content = (body_w, line_count.max(1) as u32);
        let viewport = (body_w, body_h);
        v.scroll.set_scroll(v.scroll.scroll_x, v.scroll.scroll_y, content, viewport);
    }

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
