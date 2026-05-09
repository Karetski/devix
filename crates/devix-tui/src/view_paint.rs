//! Structural View IR interpreter.
//!
//! Walks the closed `View` tree from `devix-protocol::view` and
//! emits ratatui draw calls. Lives alongside the legacy direct-paint
//! Pane render path; T-95 retires the legacy path and wires this
//! interpreter into the App's render loop.
//!
//! Scope at T-44 is structural — Stack / Split partition area
//! correctly, Empty is a no-op, and leaf variants render a
//! minimum-viable representation. Byte parity with the legacy
//! renderer comes when T-95 makes this the sole renderer.

use devix_protocol::path::Path;
use devix_protocol::view::{
    Anchor, AnchorEdge, Axis, BufferLine, Color as VColor, CursorMark, NamedColor as VNamed,
    PopupChrome, SelectionMark, Style as VStyle, View,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style as TuiStyle};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Paint a `View` tree into `area` of `frame`.
///
/// `theme` is unused at T-44 but threaded through for T-95's
/// byte-parity work; the legacy path consumes it.
pub fn paint_view(view: &View, area: Rect, frame: &mut Frame<'_>, _theme: &devix_core::Theme) {
    paint_inner(view, area, frame);
}

fn paint_inner(view: &View, area: Rect, frame: &mut Frame<'_>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match view {
        View::Empty => {}
        View::Stack {
            axis,
            weights,
            children,
            ..
        }
        | View::Split {
            axis,
            weights,
            children,
            ..
        } => {
            paint_stack_or_split(*axis, weights, children, area, frame);
        }
        View::Text { spans, .. } => {
            let line: Line = spans
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join("")
                .into();
            frame.render_widget(Paragraph::new(line), area);
        }
        View::List { items, selected, .. } => {
            paint_list(items, *selected, area, frame);
        }
        View::Buffer { lines, gutter_width, path, cursor, selection, active, .. } => {
            paint_buffer(
                lines,
                *gutter_width,
                path,
                cursor.as_ref(),
                selection,
                *active,
                area,
                frame,
            );
        }
        View::TabStrip { tabs, active, .. } => {
            paint_tab_strip(tabs, *active, area, frame);
        }
        View::Sidebar {
            title,
            focused,
            content,
            ..
        } => {
            paint_sidebar(title, *focused, content, area, frame);
        }
        View::Popup {
            anchor,
            content,
            max_size,
            chrome,
            ..
        } => {
            paint_popup(*anchor, content, *max_size, *chrome, area, frame);
        }
        View::Modal { title, content, .. } => {
            paint_modal_view(title, content, area, frame);
        }
    }
}

fn paint_stack_or_split(
    axis: Axis,
    weights: &[u16],
    children: &[View],
    area: Rect,
    frame: &mut Frame<'_>,
) {
    if children.is_empty() {
        return;
    }
    let direction = match axis {
        Axis::Horizontal => Direction::Horizontal,
        Axis::Vertical => Direction::Vertical,
    };
    let constraints: Vec<Constraint> = weights
        .iter()
        .copied()
        .chain(std::iter::repeat(1u16))
        .take(children.len())
        .map(|w| Constraint::Ratio(w.max(1) as u32, 1))
        .collect();
    let rects = Layout::default()
        .direction(direction)
        .constraints(constraints)
        .split(area);
    for (rect, child) in rects.iter().zip(children.iter()) {
        paint_inner(child, *rect, frame);
    }
}

fn paint_list(items: &[View], selected: Option<u32>, area: Rect, frame: &mut Frame<'_>) {
    if items.is_empty() {
        return;
    }
    // Each item gets one row. T-95 plugs in proper virtualization.
    let rows = area.height as usize;
    let mut y = area.y;
    for (idx, item) in items.iter().take(rows).enumerate() {
        let row_rect = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
        if Some(idx as u32) == selected {
            frame.render_widget(
                Paragraph::new("").style(TuiStyle::default().add_modifier(Modifier::REVERSED)),
                row_rect,
            );
        }
        paint_inner(item, row_rect, frame);
        y = y.saturating_add(1);
    }
}

fn paint_tab_strip(tabs: &[devix_protocol::view::TabItem], active: u32, area: Rect, frame: &mut Frame<'_>) {
    let mut buf = String::new();
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            buf.push_str(" │ ");
        }
        if i as u32 == active {
            buf.push('[');
        }
        buf.push_str(&tab.label);
        if tab.dirty {
            buf.push('*');
        }
        if i as u32 == active {
            buf.push(']');
        }
    }
    frame.render_widget(Paragraph::new(buf), area);
}

fn paint_sidebar(title: &str, focused: bool, content: &View, area: Rect, frame: &mut Frame<'_>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string())
        .style(if focused {
            TuiStyle::default().add_modifier(Modifier::BOLD)
        } else {
            TuiStyle::default()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    paint_inner(content, inner, frame);
}

fn paint_popup(
    anchor: Anchor,
    content: &View,
    max_size: Option<(u16, u16)>,
    chrome: PopupChrome,
    area: Rect,
    frame: &mut Frame<'_>,
) {
    let (w, h) = max_size.unwrap_or((40, 10));
    let popup_rect = match anchor.edge {
        AnchorEdge::Below => Rect {
            x: anchor.col,
            y: anchor.row.saturating_add(1),
            width: w.min(area.width),
            height: h.min(area.height),
        },
        AnchorEdge::Above => Rect {
            x: anchor.col,
            y: anchor.row.saturating_sub(h),
            width: w.min(area.width),
            height: h.min(area.height),
        },
        AnchorEdge::Left => Rect {
            x: anchor.col.saturating_sub(w),
            y: anchor.row,
            width: w.min(area.width),
            height: h.min(area.height),
        },
        AnchorEdge::Right => Rect {
            x: anchor.col.saturating_add(1),
            y: anchor.row,
            width: w.min(area.width),
            height: h.min(area.height),
        },
    };
    if let PopupChrome::Bordered = chrome {
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(popup_rect);
        frame.render_widget(block, popup_rect);
        paint_inner(content, inner, frame);
    } else {
        paint_inner(content, popup_rect, frame);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_buffer(
    lines: &[BufferLine],
    gutter_width: u32,
    path: &Path,
    cursor: Option<&CursorMark>,
    selection: &[SelectionMark],
    active: bool,
    area: Rect,
    frame: &mut Frame<'_>,
) {
    if lines.is_empty() {
        // Producer didn't materialize. Fall back to the path label
        // so the renderer is non-fatal during back-compat paths.
        let label = format!("[buffer {}]", path.as_str());
        frame.render_widget(Paragraph::new(label), area);
        return;
    }
    let buf = frame.buffer_mut();
    let gutter_w = gutter_width.min(u16::MAX as u32) as u16;
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    let visible = area.height as usize;
    for (row_idx, line) in lines.iter().take(visible).enumerate() {
        let y = area.y + row_idx as u16;
        if y >= area.y + area.height {
            break;
        }
        // Gutter — dim style. Truncated to gutter_w.
        if !line.gutter.is_empty() {
            let _ = buf.set_stringn(
                area.x,
                y,
                &line.gutter,
                gutter_w as usize,
                dim,
            );
        }
        // Spans — concatenated after the gutter, each with its own
        // style.
        let mut x = area.x.saturating_add(gutter_w);
        let max_x = area.x.saturating_add(area.width);
        for span in &line.spans {
            if x >= max_x {
                break;
            }
            let style = view_style_to_ratatui(&span.style);
            let remaining = (max_x - x) as usize;
            let next = buf.set_stringn(x, y, &span.text, remaining, style);
            x = next.0;
        }
    }
    // Selection background (reverse video) over non-empty marks +
    // extra-cursor reverse cells over zero-extent marks that aren't
    // the primary caret. Mirrors the legacy renderer's
    // `paint_selection` + `paint_extra_cursors` byte-for-byte; the
    // primary's caret is placed last (`set_cursor_position`) so it
    // wins over a selection cell painted on the same coordinate.
    paint_selection_overlay(lines, selection, gutter_w, area, frame);
    paint_extra_cursor_marks(lines, selection, cursor, gutter_w, area, frame);

    // Place the terminal cursor when the pane is active.
    if active {
        if let Some(cm) = cursor {
            // Producer's CursorMark is buffer-relative (line, col);
            // first materialized line is the scroll top, so the
            // visible row offset is `cm.line - lines[0].line`.
            let top = lines.first().map(|l| l.line).unwrap_or(0);
            if cm.line >= top {
                let row = cm.line - top;
                if (row as usize) < lines.len() && row < area.height as u32 {
                    let x = area
                        .x
                        .saturating_add(gutter_w)
                        .saturating_add(cm.col.min(u16::MAX as u32) as u16);
                    let y = area.y.saturating_add(row.min(u16::MAX as u32) as u16);
                    if x < area.x.saturating_add(area.width)
                        && y < area.y.saturating_add(area.height)
                    {
                        frame.set_cursor_position((x, y));
                    }
                }
            }
        }
    }
}

/// Paint reverse-video selection background for every non-empty
/// mark in `selection`. Single-line marks paint `[start_col, end_col)`
/// on `start_line`; multi-line marks paint `start_col..line_end` on
/// the first line, the full visible row on intermediate lines, and
/// `0..end_col` on the last line. Mirrors the legacy
/// `editor::buffer::paint_selection` shape.
fn paint_selection_overlay(
    lines: &[BufferLine],
    selection: &[SelectionMark],
    gutter_w: u16,
    area: Rect,
    frame: &mut Frame<'_>,
) {
    if selection.is_empty() || lines.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    let style = TuiStyle::default().add_modifier(Modifier::REVERSED);
    let top = lines[0].line;
    let bottom = top + lines.len() as u32; // exclusive
    let text_x = area.x.saturating_add(gutter_w);
    let max_x = area.x.saturating_add(area.width);
    let max_y = area.y.saturating_add(area.height);
    for mark in selection {
        let start_line = mark.start_line;
        let end_line = mark.end_line;
        let start_col = mark.start_col;
        let end_col = mark.end_col;
        if start_line == end_line && start_col == end_col {
            // zero-extent — handled by `paint_extra_cursor_marks`.
            continue;
        }
        // Clamp to the materialized window.
        let first = start_line.max(top);
        let last = end_line.min(bottom.saturating_sub(1));
        if first > last {
            continue;
        }
        for line_idx in first..=last {
            let row = line_idx - top;
            let y = area.y.saturating_add(row.min(u16::MAX as u32) as u16);
            if y >= max_y {
                break;
            }
            let local_start = if line_idx == start_line { start_col } else { 0 };
            let local_end = if line_idx == end_line {
                end_col
            } else {
                // Multi-line: paint to the row's right edge so
                // intermediate lines look continuous in the selection.
                area.width as u32
            };
            let mut x = text_x.saturating_add(local_start.min(u16::MAX as u32) as u16);
            let x_end = text_x.saturating_add(local_end.min(u16::MAX as u32) as u16);
            let x_end = x_end.min(max_x);
            while x < x_end {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(style);
                }
                x = x.saturating_add(1);
            }
        }
    }
}

/// Paint reverse-cell extra-cursor marks for every zero-extent
/// `SelectionMark` that isn't coincident with the primary `cursor`.
/// Multicursor secondaries appear as zero-extent ranges whose
/// `start == end`; the primary caret is the one that matches the
/// `cursor` field, and gets painted by `set_cursor_position` instead
/// of a reverse cell.
fn paint_extra_cursor_marks(
    lines: &[BufferLine],
    selection: &[SelectionMark],
    cursor: Option<&CursorMark>,
    gutter_w: u16,
    area: Rect,
    frame: &mut Frame<'_>,
) {
    if selection.is_empty() || lines.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    let style = TuiStyle::default().add_modifier(Modifier::REVERSED);
    let top = lines[0].line;
    let bottom = top + lines.len() as u32;
    let text_x = area.x.saturating_add(gutter_w);
    let max_x = area.x.saturating_add(area.width);
    let max_y = area.y.saturating_add(area.height);
    for mark in selection {
        if mark.start_line != mark.end_line || mark.start_col != mark.end_col {
            continue;
        }
        if let Some(c) = cursor {
            if c.line == mark.start_line && c.col == mark.start_col {
                continue;
            }
        }
        if mark.start_line < top || mark.start_line >= bottom {
            continue;
        }
        let row = mark.start_line - top;
        let y = area.y.saturating_add(row.min(u16::MAX as u32) as u16);
        let x = text_x.saturating_add(mark.start_col.min(u16::MAX as u32) as u16);
        if x >= max_x || y >= max_y {
            continue;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_style(style);
        }
    }
}

fn view_style_to_ratatui(style: &VStyle) -> TuiStyle {
    use ratatui::style::Color as RColor;
    fn to_ratatui(c: VColor) -> RColor {
        match c {
            VColor::Default => RColor::Reset,
            VColor::Rgb(r, g, b) => RColor::Rgb(r, g, b),
            VColor::Indexed(n) => RColor::Indexed(n),
            VColor::Named(n) => match n {
                VNamed::Black => RColor::Black,
                VNamed::Red => RColor::Red,
                VNamed::Green => RColor::Green,
                VNamed::Yellow => RColor::Yellow,
                VNamed::Blue => RColor::Blue,
                VNamed::Magenta => RColor::Magenta,
                VNamed::Cyan => RColor::Cyan,
                VNamed::White => RColor::White,
                VNamed::DarkGray => RColor::DarkGray,
                VNamed::LightRed => RColor::LightRed,
                VNamed::LightGreen => RColor::LightGreen,
                VNamed::LightYellow => RColor::LightYellow,
                VNamed::LightBlue => RColor::LightBlue,
                VNamed::LightMagenta => RColor::LightMagenta,
                VNamed::LightCyan => RColor::LightCyan,
            },
        }
    }
    let mut s = TuiStyle::default();
    if let Some(c) = style.fg {
        s = s.fg(to_ratatui(c));
    }
    if let Some(c) = style.bg {
        s = s.bg(to_ratatui(c));
    }
    let mut mods = Modifier::empty();
    if style.bold {
        mods |= Modifier::BOLD;
    }
    if style.italic {
        mods |= Modifier::ITALIC;
    }
    if style.underline {
        mods |= Modifier::UNDERLINED;
    }
    if style.dim {
        mods |= Modifier::DIM;
    }
    if style.reverse {
        mods |= Modifier::REVERSED;
    }
    s.add_modifier(mods)
}

fn paint_modal_view(title: &str, content: &View, area: Rect, frame: &mut Frame<'_>) {
    // Center a 60% × 60% box in `area` (placeholder geometry; T-95
    // refines).
    let w = (area.width as u32 * 6 / 10) as u16;
    let h = (area.height as u32 * 6 / 10) as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let modal_rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    let block = Block::default().borders(Borders::ALL).title(title.to_string());
    let inner = block.inner(modal_rect);
    frame.render_widget(block, modal_rect);
    paint_inner(content, inner, frame);
}

#[cfg(test)]
mod tests {
    use devix_core::Theme;
    use devix_protocol::path::Path;
    use devix_protocol::view::{Axis, View, ViewNodeId};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    fn id(s: &str) -> ViewNodeId {
        ViewNodeId(Path::parse(s).unwrap())
    }

    fn run<F: FnOnce(&mut Frame<'_>)>(width: u16, height: u16, f: F) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|frame| f(frame)).unwrap();
        term.backend().buffer().clone()
    }

    #[test]
    fn empty_paints_nothing() {
        let theme = Theme::default();
        let buf = run(10, 3, |frame| {
            paint_view(&View::Empty, frame.area(), frame, &theme);
        });
        // A blank buffer's cells are all default — assert by
        // checking the first cell is not styled / has empty symbol.
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), " ");
    }

    #[test]
    fn vertical_stack_partitions_area() {
        let theme = Theme::default();
        let view = View::Stack {
            id: id("/synthetic/stack/test"),
            axis: Axis::Vertical,
            weights: vec![1, 1],
            children: vec![
                View::Text {
                    id: id("/synthetic/text/top"),
                    spans: vec![devix_protocol::view::TextSpan {
                        text: "TOP".into(),
                        style: Default::default(),
                    }],
                    wrap: devix_protocol::view::WrapMode::NoWrap,
                    transition: None,
                },
                View::Text {
                    id: id("/synthetic/text/bot"),
                    spans: vec![devix_protocol::view::TextSpan {
                        text: "BOT".into(),
                        style: Default::default(),
                    }],
                    wrap: devix_protocol::view::WrapMode::NoWrap,
                    transition: None,
                },
            ],
            spacing: 0,
            transition: None,
        };
        let buf = run(10, 4, |frame| {
            paint_view(&view, frame.area(), frame, &theme);
        });
        // Top row contains "TOP", bottom half contains "BOT".
        let top = (0..3).map(|x| buf.cell((x, 0)).unwrap().symbol()).collect::<String>();
        let bot = (0..3).map(|x| buf.cell((x, 2)).unwrap().symbol()).collect::<String>();
        assert_eq!(top, "TOP");
        assert_eq!(bot, "BOT");
    }

    /// Selection background overlays the cells inside a non-empty
    /// `SelectionMark` with `Modifier::REVERSED`. Cells outside the
    /// selection stay default-styled.
    #[test]
    fn buffer_paints_selection_background() {
        use devix_protocol::view::{BufferLine, GutterMode, SelectionMark, TextSpan};
        let theme = Theme::default();
        let line = BufferLine {
            line: 0,
            gutter: String::new(),
            spans: vec![TextSpan {
                text: "abcdef".into(),
                style: Default::default(),
            }],
        };
        let view = View::Buffer {
            id: id("/buf/42"),
            path: Path::parse("/buf/42").unwrap(),
            scroll_top_line: 0,
            cursor: None,
            selection: vec![SelectionMark {
                start_line: 0,
                start_col: 1,
                end_line: 0,
                end_col: 4,
            }],
            highlights: Vec::new(),
            gutter: GutterMode::None,
            active: false,
            lines: vec![line],
            gutter_width: 0,
            transition: None,
        };
        let buf = run(10, 1, |frame| {
            paint_view(&view, frame.area(), frame, &theme);
        });
        // Cells [1, 4) should be reverse-styled; cells [0, 1) and
        // [4, 6) should not.
        let reversed = |x: u16| {
            buf.cell((x, 0)).unwrap().style().add_modifier
                .contains(Modifier::REVERSED)
        };
        assert!(!reversed(0), "cell 0 outside selection");
        assert!(reversed(1), "cell 1 selected");
        assert!(reversed(2), "cell 2 selected");
        assert!(reversed(3), "cell 3 selected");
        assert!(!reversed(4), "cell 4 just past selection (end is exclusive)");
        assert!(!reversed(5), "cell 5 outside selection");
    }

    /// Zero-extent SelectionMarks that aren't coincident with the
    /// primary `cursor` paint reverse cells (multicursor secondary
    /// caret).
    #[test]
    fn buffer_paints_extra_cursor_marks() {
        use devix_protocol::view::{BufferLine, CursorMark, GutterMode, SelectionMark, TextSpan};
        let theme = Theme::default();
        let line = BufferLine {
            line: 0,
            gutter: String::new(),
            spans: vec![TextSpan {
                text: "abcdef".into(),
                style: Default::default(),
            }],
        };
        let view = View::Buffer {
            id: id("/buf/42"),
            path: Path::parse("/buf/42").unwrap(),
            scroll_top_line: 0,
            // Primary at col 0; the zero-extent mark at col 0 is
            // suppressed; the one at col 3 paints a reverse cell.
            cursor: Some(CursorMark { line: 0, col: 0 }),
            selection: vec![
                SelectionMark { start_line: 0, start_col: 0, end_line: 0, end_col: 0 },
                SelectionMark { start_line: 0, start_col: 3, end_line: 0, end_col: 3 },
            ],
            highlights: Vec::new(),
            gutter: GutterMode::None,
            active: false,
            lines: vec![line],
            gutter_width: 0,
            transition: None,
        };
        let buf = run(10, 1, |frame| {
            paint_view(&view, frame.area(), frame, &theme);
        });
        let reversed = |x: u16| {
            buf.cell((x, 0)).unwrap().style().add_modifier
                .contains(Modifier::REVERSED)
        };
        assert!(!reversed(0), "primary's coordinate is suppressed");
        assert!(reversed(3), "secondary multicursor head paints reverse");
        assert!(!reversed(2), "non-cursor cells stay default");
    }

    #[test]
    fn buffer_renders_path_label() {
        let theme = Theme::default();
        let view = View::Buffer {
            id: id("/buf/42"),
            path: Path::parse("/buf/42").unwrap(),
            scroll_top_line: 0,
            cursor: None,
            selection: Vec::new(),
            highlights: Vec::new(),
            gutter: devix_protocol::view::GutterMode::None,
            active: true,
            lines: Vec::new(),
            gutter_width: 0,
            transition: None,
        };
        let buf = run(20, 1, |frame| {
            paint_view(&view, frame.area(), frame, &theme);
        });
        let row = (0..20).map(|x| buf.cell((x, 0)).unwrap().symbol()).collect::<String>();
        assert!(row.contains("buffer /buf/42"));
    }
}
