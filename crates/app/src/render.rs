//! Frame composition: editor area + status line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use devix_ui::{EditorView, StatusInfo, render_editor, render_status as render_status_widget};

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());
    let editor_area = chunks[0];
    let status_area = chunks[1];
    app.last_editor_area = editor_area;

    let visible = editor_area.height as usize;
    // Anchor pass: read view + doc, compute new scroll_top, write back.
    let Some((_, vid, did)) = app.workspace.active_ids() else { return };
    let head = app.workspace.views[vid].primary().head;
    let cur_anchored = app.workspace.views[vid].view_anchored;
    let mut scroll_top = app.workspace.views[vid].scroll_top;
    if cur_anchored && visible > 0 {
        let cur_line = app.workspace.documents[did].buffer.line_of_char(head);
        if cur_line < scroll_top {
            scroll_top = cur_line;
        } else if cur_line >= scroll_top + visible {
            scroll_top = cur_line + 1 - visible;
        }
    }
    app.workspace.views[vid].scroll_top = scroll_top;

    let view = &app.workspace.views[vid];
    let doc = &app.workspace.documents[did];
    let editor_view = EditorView {
        buffer: &doc.buffer,
        selection: &view.selection,
        scroll_top: view.scroll_top,
    };
    let r = render_editor(editor_view, editor_area, frame);
    app.last_gutter_width = r.gutter_width;
    if let Some((x, y)) = r.cursor_screen { frame.set_cursor_position((x, y)); }

    render_status(frame, status_area, app);
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
