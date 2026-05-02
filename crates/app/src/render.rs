//! Frame composition: editor area + status line. Translates `App` state into
//! the `StatusInfo` and `EditorView` value types that the `ui` crate consumes.

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

    let cur_line = app.editor.buffer.line_of_char(app.primary().head);
    let visible = editor_area.height as usize;
    if visible > 0 {
        if cur_line < app.editor.scroll_top {
            app.editor.scroll_top = cur_line;
        } else if cur_line >= app.editor.scroll_top + visible {
            app.editor.scroll_top = cur_line + 1 - visible;
        }
    }

    let view = EditorView {
        buffer: &app.editor.buffer,
        selection: &app.editor.selection,
        scroll_top: app.editor.scroll_top,
    };
    let r = render_editor(view, editor_area, frame);
    app.last_gutter_width = r.gutter_width;
    if let Some((x, y)) = r.cursor_screen {
        frame.set_cursor_position((x, y));
    }

    render_status(frame, status_area, app);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let path_str = app.editor.buffer.path().map(|p| p.display().to_string());
    let head = app.primary().head;
    let info = StatusInfo {
        path: path_str.as_deref(),
        dirty: app.editor.buffer.dirty(),
        line: app.editor.buffer.line_of_char(head) + 1,
        col: app.editor.buffer.col_of_char(head) + 1,
        sel_len: app.primary().len(),
        message: app.status.get(),
    };
    render_status_widget(&info, area, frame);
}
