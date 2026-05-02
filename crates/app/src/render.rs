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

    // Compute every leaf's Rect from the layout tree. Cache for hit-testing
    // and viewport-aware actions (scroll, focus traversal).
    let leaves = app.workspace.layout.leaves_with_rects(editor_area);
    app.workspace.render_cache.frame_rects.clear();
    app.workspace.render_cache.sidebar_rects.clear();
    for (leaf, rect) in &leaves {
        match leaf {
            LeafRef::Frame(id) => { app.workspace.render_cache.frame_rects.insert(*id, *rect); }
            LeafRef::Sidebar(slot) => { app.workspace.render_cache.sidebar_rects.insert(*slot, *rect); }
        }
    }

    // Single-frame phase: track the active frame's rect for legacy fields.
    if let Some(active_id) = app.workspace.active_frame() {
        if let Some(rect) = app.workspace.render_cache.frame_rects.get(active_id) {
            app.last_editor_area = *rect;
        }
    }

    // Render every leaf. (Sidebar painting comes in Task 10.)
    for (leaf, rect) in &leaves {
        match leaf {
            LeafRef::Frame(id) => render_frame(*id, *rect, app, frame),
            LeafRef::Sidebar(_) => { /* painted in Task 10 */ }
        }
    }

    render_status(frame, status_area, app);
}

fn render_frame(id: devix_workspace::FrameId, area: Rect, app: &mut App, frame: &mut Frame<'_>) {
    // Step 5: each frame has exactly one tab; a tab strip widget arrives in Task 6.
    let view_id = app.workspace.frames[id].active_view();
    let view = &app.workspace.views[view_id];
    let doc = &app.workspace.documents[view.doc];

    let visible = area.height as usize;
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
    let r = render_editor(editor_view, area, frame);
    app.last_gutter_width = r.gutter_width;
    if app.workspace.active_frame() == Some(id) {
        if let Some((x, y)) = r.cursor_screen { frame.set_cursor_position((x, y)); }
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
