//! Frame composition: editor area + status line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use devix_ui::{EditorView, StatusInfo, render_editor, render_status as render_status_widget};
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
    for (leaf, rect) in &leaves {
        if let LeafRef::Sidebar(slot) = leaf {
            app.workspace.render_cache.sidebar_rects.insert(*slot, *rect);
        }
    }

    // Render every leaf. (Sidebar painting comes in Task 10.)
    for (leaf, rect) in &leaves {
        match leaf {
            LeafRef::Frame(id) => render_frame(*id, *rect, app, frame),
            LeafRef::Sidebar(_) => { /* painted in Task 10 */ }
        }
    }

    // Track the active frame's body rect for legacy fields. render_frame
    // wrote frame_rects[id] = body_area, so this picks up the correct rect
    // for click hit-testing (without the tab strip row).
    if let Some(active_id) = app.workspace.active_frame() {
        if let Some(rect) = app.workspace.render_cache.frame_rects.get(active_id) {
            app.last_editor_area = *rect;
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
    devix_ui::render_tabstrip(&tabs, active_tab, strip_area, frame);

    // Editor body: anchor-pass + draw flow against body_area.
    let view_id = match app.workspace.frames[id].active_view() {
        Some(v) => v,
        None => return, // empty frame — nothing to draw
    };
    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];

    let visible = body_area.height as usize;
    let mut scroll_top = view.scroll_top;
    let head = view.primary().head;
    if view.view_anchored && visible > 0 {
        let cur_line = doc.buffer.line_of_char(head);
        if cur_line < scroll_top {
            scroll_top = cur_line;
        } else if cur_line >= scroll_top + visible {
            scroll_top = cur_line + 1 - visible;
        }
    }
    app.workspace.views[view_id].scroll_top = scroll_top;

    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];
    let editor_view = EditorView {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll_top: view.scroll_top,
    };
    let r = render_editor(editor_view, body_area, frame);
    app.last_gutter_width = r.gutter_width;
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
