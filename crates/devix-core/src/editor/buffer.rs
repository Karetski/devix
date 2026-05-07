//! Editor view: pure render of buffer text with line-number gutter,
//! syntax-scope styling, and selection-range highlight. Vertical scroll
//! comes from the layout primitives in `crate::layout` — each line is
//! one item in a `UniformLayout`.
//!
//! Highlights are passed in by the caller (resolved from the document's
//! tree-sitter Highlighter for the visible byte range). The renderer is
//! agnostic to how that list was produced; it just paints scope styles
//! over visible text.

use crate::widgets::layout::{CollectionPass, UniformLayout};
use ratatui::Frame;
use ratatui::buffer::Buffer as RatBuffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use ropey::Rope;

use devix_text::{Buffer, Range, Selection};
use crate::{Event, HandleCtx, Outcome, Pane, RenderCtx, Theme};
use devix_syntax::HighlightSpan;

pub struct BufferRender<'a> {
    pub buffer: &'a Buffer,
    pub selection: &'a Selection,
    pub scroll: (u32, u32),
    pub theme: &'a Theme,
    /// Highlight spans intersecting the visible byte range. May include spans
    /// straddling the viewport edges; the renderer clips per line. Order is
    /// significant: tree-sitter emits captures in source order so later spans
    /// override earlier ones (last-write paint), letting more-specific scopes
    /// win over their parents.
    pub highlights: &'a [HighlightSpan],
}

pub struct EditorRenderResult {
    /// Where to place the terminal cursor (primary range head), if visible.
    pub cursor_screen: Option<(u16, u16)>,
    /// Width of the gutter (line numbers + padding) in cells.
    pub gutter_width: u16,
}

pub fn render_buffer(view: BufferRender<'_>, area: Rect, frame: &mut Frame<'_>) -> EditorRenderResult {
    let line_count = view.buffer.line_count();
    let num_width = line_count.to_string().len() as u16;
    let gutter_width = num_width + 2; // " 12 "

    let layout = UniformLayout::vertical(line_count, 1, area.width as u32);
    let pass = CollectionPass::new(&layout, view.scroll, area);
    let scroll_top = view.scroll.1 as usize;

    let gutter_style = Style::default().add_modifier(Modifier::DIM);
    let text_style = view.theme.text_style();
    let rope = view.buffer.rope();

    // Cap per-line work to what the viewport can show. Long lines (minified
    // code, JSON, logs) would otherwise allocate the full line as a String per
    // visible row per frame — easily megabytes of churn on every navigation
    // key, which makes input feel ignored because the render thread is busy.
    let max_text = (area.width as usize).saturating_sub(gutter_width as usize + 1);
    for (line_idx, geom) in pass.visible_items() {
        let line_text = view.buffer.line_string_truncated(line_idx, max_text);
        let gutter = format!("{:>width$} ", line_idx + 1, width = num_width as usize);
        let mut spans = Vec::with_capacity(2 + 1);
        spans.push(Span::styled(gutter, gutter_style));
        spans.push(Span::raw(" "));
        styled_line_spans(
            &line_text,
            line_idx,
            rope,
            view.highlights,
            view.theme,
            text_style,
            &mut spans,
        );
        Paragraph::new(Line::from(spans)).render(geom.screen, frame.buffer_mut());
    }

    paint_selection(
        view.buffer,
        view.selection,
        area,
        gutter_width,
        scroll_top,
        view.theme.selection_style(),
        frame.buffer_mut(),
    );

    paint_extra_cursors(
        view.buffer,
        view.selection,
        area,
        gutter_width,
        scroll_top,
        frame.buffer_mut(),
    );

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

/// Phase-2 of the architecture refactor: the editor body as a `Pane`.
pub struct EditorPane<'a> {
    pub buffer: &'a Buffer,
    pub selection: &'a Selection,
    pub scroll: (u32, u32),
    pub theme: &'a Theme,
    /// Owned (not borrowed) so a parent composite — `TabbedPane` —
    /// can store an `EditorPane` as a field without a self-referential
    /// borrow against a sibling `Vec<HighlightSpan>`. The cost is one
    /// `Vec<HighlightSpan>` move per frame, which is dominated by the
    /// tree-sitter query that produced the spans.
    pub highlights: Vec<HighlightSpan>,
    /// True only for the editor's focused frame. Drives the terminal
    /// cursor (`Frame::set_cursor_position`); inactive editor panes paint
    /// their text but do not steal the cursor.
    pub active: bool,
}

impl<'a> Pane for EditorPane<'a> {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        let view = BufferRender {
            buffer: self.buffer,
            selection: self.selection,
            scroll: self.scroll,
            theme: self.theme,
            highlights: &self.highlights,
        };
        let r = render_buffer(view, area, ctx.frame);
        if self.active {
            if let Some((x, y)) = r.cursor_screen {
                ctx.frame.set_cursor_position((x, y));
            }
        }
    }

    fn handle(&mut self, _ev: &Event, _area: Rect, _ctx: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }

    fn is_focusable(&self) -> bool {
        true
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_selection(
    buffer: &Buffer,
    selection: &Selection,
    area: Rect,
    gutter_width: u16,
    scroll_top: usize,
    highlight: Style,
    target: &mut RatBuffer,
) {
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

#[allow(dead_code)]
fn paint_zero_width(_r: Range) {}

/// Paint the head of every non-primary cursor as a reverse-video block.
fn paint_extra_cursors(
    buffer: &Buffer,
    selection: &Selection,
    area: Rect,
    gutter_width: u16,
    scroll_top: usize,
    target: &mut RatBuffer,
) {
    if !selection.is_multi() {
        return;
    }
    let visible_rows = area.height as usize;
    let view_end = scroll_top.saturating_add(visible_rows);
    let primary_idx = selection.primary_index();
    let max_x = area.x + area.width;
    let text_x = area.x + gutter_width;
    let style = Style::default().add_modifier(Modifier::REVERSED);

    for (i, range) in selection.ranges().iter().enumerate() {
        if i == primary_idx { continue; }
        if !range.is_empty() { continue; }
        let line = buffer.line_of_char(range.head);
        if line < scroll_top || line >= view_end { continue; }
        let col = buffer.col_of_char(range.head);
        let y = area.y + (line - scroll_top) as u16;
        let x = text_x.saturating_add(col as u16);
        if x >= max_x || y >= area.y + area.height { continue; }
        if let Some(cell) = target.cell_mut((x, y)) {
            cell.set_style(style);
        }
    }
}

/// Build per-line ratatui spans coloured by the highlight list.
fn styled_line_spans(
    line_text: &str,
    line_idx: usize,
    rope: &Rope,
    highlights: &[HighlightSpan],
    theme: &Theme,
    default_style: Style,
    out: &mut Vec<Span<'static>>,
) {
    let chars: Vec<char> = line_text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return;
    }

    let line_char_start = rope.line_to_char(line_idx);
    let line_byte_start = rope.char_to_byte(line_char_start);
    let line_byte_end = rope.char_to_byte(line_char_start + n);

    let mut styles = vec![default_style; n];
    for span in highlights {
        if span.end_byte <= line_byte_start || span.start_byte >= line_byte_end {
            continue;
        }
        let Some(scope_style) = theme.style_for(&span.scope) else { continue };
        let s_byte = span.start_byte.max(line_byte_start);
        let e_byte = span.end_byte.min(line_byte_end);
        let s_char = rope.byte_to_char(s_byte).saturating_sub(line_char_start);
        let e_char = rope.byte_to_char(e_byte).saturating_sub(line_char_start);
        let s = s_char.min(n);
        let e = e_char.min(n);
        for slot in &mut styles[s..e] {
            *slot = scope_style;
        }
    }

    let mut i = 0;
    while i < n {
        let st = styles[i];
        let mut j = i + 1;
        while j < n && styles[j] == st {
            j += 1;
        }
        let chunk: String = chars[i..j].iter().collect();
        out.push(Span::styled(chunk, st));
        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devix_text::{Selection, replace_selection_tx};
    use devix_syntax::{Highlighter, Language};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_rust(text: &str, width: u16, height: u16) -> (ratatui::buffer::Buffer, Theme) {
        let mut buf = devix_text::Buffer::empty();
        let tx = replace_selection_tx(&buf, &Selection::point(0), text);
        buf.apply(tx);

        let mut h = Highlighter::new(Language::Rust).unwrap();
        h.parse(buf.rope());
        let highlights = h.highlights(buf.rope(), 0, buf.rope().len_bytes());

        let theme = Theme::default();
        let scroll = (0u32, 0u32);
        let selection = Selection::point(0);

        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect { x: 0, y: 0, width, height };
                let _ = render_buffer(
                    BufferRender {
                        buffer: &buf,
                        selection: &selection,
                        scroll,
                        theme: &theme,
                        highlights: &highlights,
                    },
                    area,
                    f,
                );
            })
            .unwrap();

        (terminal.backend().buffer().clone(), theme)
    }

    #[test]
    fn rust_keyword_renders_with_keyword_style() {
        let (rendered, theme) = render_rust("fn main() {}", 40, 3);
        let kw_fg = theme
            .style_for("keyword.function")
            .or_else(|| theme.style_for("keyword"))
            .and_then(|s| s.fg)
            .expect("default theme registers a keyword color");
        let cell = rendered.cell((3, 0)).expect("keyword cell exists");
        assert_eq!(cell.symbol(), "f", "expected `fn` to land at column 3");
        assert_eq!(
            cell.fg, kw_fg,
            "fn keyword cell should adopt the theme's keyword fg, got {:?}",
            cell.fg,
        );
    }

    #[test]
    fn plaintext_renders_with_default_text_style() {
        let mut buf = devix_text::Buffer::empty();
        let tx = replace_selection_tx(&buf, &Selection::point(0), "hello");
        buf.apply(tx);
        let theme = Theme::default();
        let scroll = (0u32, 0u32);
        let selection = Selection::point(0);

        let backend = TestBackend::new(40, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect { x: 0, y: 0, width: 40, height: 3 };
                let _ = render_buffer(
                    BufferRender {
                        buffer: &buf,
                        selection: &selection,
                        scroll,
                        theme: &theme,
                        highlights: &[],
                    },
                    area,
                    f,
                );
            })
            .unwrap();
        let rendered = terminal.backend().buffer();
        let cell = rendered.cell((3, 0)).unwrap();
        assert_eq!(cell.symbol(), "h");
        let expected_fg = theme.text_style().fg.expect("default text style sets fg");
        assert_eq!(cell.fg, expected_fg);
    }
}
