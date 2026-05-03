//! Editor view: pure render of buffer text with line-number gutter and
//! selection-range highlight. Vertical scroll comes from the collection
//! primitives — each line is one item in a `UniformLayout`.

use devix_collection::{CollectionPass, CollectionState, UniformLayout};
use ratatui::Frame;
use ratatui::buffer::Buffer as RatBuffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use devix_buffer::{Buffer, Range, Selection};

pub struct EditorView<'a> {
    pub buffer: &'a Buffer,
    pub selection: &'a Selection,
    pub scroll: &'a CollectionState,
}

pub struct EditorRenderResult {
    /// Where to place the terminal cursor (primary range head), if visible.
    pub cursor_screen: Option<(u16, u16)>,
    /// Width of the gutter (line numbers + padding) in cells.
    pub gutter_width: u16,
}

pub fn render_editor(view: EditorView<'_>, area: Rect, frame: &mut Frame<'_>) -> EditorRenderResult {
    let line_count = view.buffer.line_count();
    let num_width = line_count.to_string().len() as u16;
    let gutter_width = num_width + 2; // " 12 "

    let layout = UniformLayout::vertical(line_count, 1, area.width as u32);
    let pass = CollectionPass::new(&layout, view.scroll, area);
    let scroll_top = view.scroll.scroll_y as usize;

    let gutter_style = Style::default().add_modifier(Modifier::DIM);

    // Cap per-line work to what the viewport can show. Long lines (minified
    // code, JSON, logs) would otherwise allocate the full line as a String per
    // visible row per frame — easily megabytes of churn on every navigation
    // key, which makes input feel ignored because the render thread is busy.
    let max_text = (area.width as usize).saturating_sub(gutter_width as usize + 1);
    for (line_idx, geom) in pass.visible_items() {
        let line_text = view.buffer.line_string_truncated(line_idx, max_text);
        let gutter = format!("{:>width$} ", line_idx + 1, width = num_width as usize);
        let line = Line::from(vec![
            Span::styled(gutter, gutter_style),
            Span::raw(" "),
            Span::raw(line_text),
        ]);
        Paragraph::new(line).render(geom.screen, frame.buffer_mut());
    }

    paint_selection(view.buffer, view.selection, area, gutter_width, scroll_top, frame.buffer_mut());

    let primary = view.selection.primary();
    let cur_line = view.buffer.line_of_char(primary.head);
    let cur_col = view.buffer.col_of_char(primary.head);
    let visible_rows = area.height as usize;
    let cursor_screen = if cur_line >= scroll_top && cur_line < scroll_top + visible_rows {
        let y = area.y + (cur_line - scroll_top) as u16;
        let x = area.x + gutter_width + cur_col as u16;
        if x < area.x + area.width && y < area.y + area.height {
            Some((x, y))
        } else {
            None
        }
    } else {
        None
    };

    EditorRenderResult { cursor_screen, gutter_width }
}

fn paint_selection(
    buffer: &Buffer,
    selection: &Selection,
    area: Rect,
    gutter_width: u16,
    scroll_top: usize,
    target: &mut RatBuffer,
) {
    let highlight = Style::default().bg(Color::Rgb(60, 80, 130));
    let visible_rows = area.height as usize;

    let view_end = scroll_top.saturating_add(visible_rows);
    for range in selection.ranges() {
        if range.is_empty() {
            continue;
        }
        let start = range.start();
        let end = range.end();
        let start_line = buffer.line_of_char(start);
        let end_line = buffer.line_of_char(end);

        // Iterate only the slice of selected lines that intersects the
        // viewport. With huge selections (e.g. Ctrl+A on a 1.3M-line file),
        // looping over every selected line per frame stalls the render.
        let first = start_line.max(scroll_top);
        let last = end_line.min(view_end.saturating_sub(1));
        if first > last {
            continue;
        }
        for line_idx in first..=last {
            let line_start = buffer.line_start(line_idx);
            let line_len = buffer.line_len_chars(line_idx);
            let local_start = if line_idx == start_line {
                start - line_start
            } else {
                0
            };
            let local_end = if line_idx == end_line {
                end - line_start
            } else {
                // Multi-line: highlight up through the newline marker (one cell past EOL).
                line_len + 1
            };
            paint_line_span(area, gutter_width, scroll_top, line_idx, local_start, local_end, line_len, highlight, target);
        }
    }
    // Hint to the formatter that this is intentionally a void return.
    let _ = paint_zero_width;
}

#[allow(clippy::too_many_arguments)]
fn paint_line_span(
    area: Rect,
    gutter_width: u16,
    scroll_top: usize,
    line_idx: usize,
    local_start: usize,
    local_end: usize,
    line_len: usize,
    style: Style,
    target: &mut RatBuffer,
) {
    let row = (line_idx - scroll_top) as u16;
    let y = area.y + row;
    if y >= area.y + area.height {
        return;
    }
    let text_x = area.x + gutter_width;
    let max_x = area.x + area.width;
    let mut x = text_x + local_start as u16;
    // Clamp visual end: don't overshoot the line's char width unless we're
    // marking a trailing newline (local_end > line_len).
    let visual_end_chars = if local_end > line_len {
        line_len + 1
    } else {
        local_end
    };
    let mut x_end = text_x + visual_end_chars as u16;
    if x >= max_x { return; }
    if x_end > max_x { x_end = max_x; }
    while x < x_end {
        if let Some(cell) = target.cell_mut((x, y)) {
            cell.set_style(style);
        }
        x += 1;
    }
}

// Reserved for future use: zero-width cursor hint where Vim-style block cursor
// rendering would go. Keeps `Range` import meaningful when we extend.
#[allow(dead_code)]
fn paint_zero_width(_r: Range) {}
