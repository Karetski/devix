//! Floating popup primitive: a bordered box with text or list content,
//! anchored to a screen cell, sized to its content (clamped against max
//! size and the viewport), and flipped above its anchor when below would
//! clip.
//!
//! Used by hover (Text) and completion (CompletionList).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use crate::Theme;

/// Cell the popup attaches to. The renderer prefers placing the box one row
/// below `(col, row)` (so it doesn't cover the symbol the user is pointing
/// at); if that overflows the viewport bottom, it flips above instead.
#[derive(Copy, Clone, Debug)]
pub struct PopupAnchor {
    pub col: u16,
    pub row: u16,
}

/// One row of a completion popup. Rendered as `label   detail` with the
/// detail dim and right-padded; the row is highlighted when its index in
/// the list matches `selected`.
#[derive(Clone, Debug)]
pub struct CompletionLine<'a> {
    pub label: &'a str,
    pub detail: Option<&'a str>,
}

/// Popup payload. Hover ships `Text`; completion ships `CompletionList`.
#[derive(Clone, Debug)]
pub enum PopupContent<'a> {
    /// Pre-split lines. The renderer wraps long lines on word boundaries.
    Text(&'a [String]),
    /// List of completion items. The renderer paints a row per item with
    /// the selected row highlighted; one row per item, no wrapping.
    CompletionList {
        items: &'a [CompletionLine<'a>],
        selected: usize,
    },
}

#[derive(Clone, Debug)]
pub struct Popup<'a> {
    pub anchor: PopupAnchor,
    pub content: PopupContent<'a>,
    /// Outer bounds, including border. The renderer never produces a box
    /// larger than this, even if the content wants more.
    pub max_size: (u16, u16),
}

const DEFAULT_MAX: (u16, u16) = (60, 12);

impl Popup<'_> {
    pub fn with_default_size(anchor: PopupAnchor, content: PopupContent<'_>) -> Popup<'_> {
        Popup { anchor, content, max_size: DEFAULT_MAX }
    }
}

/// Render `popup` inside `viewport`, returning the rect the popup occupied
/// (or `None` if there was no room). Painting is `Clear`ed before drawing
/// so any text underneath is replaced cleanly.
pub fn render_popup(popup: &Popup<'_>, theme: &Theme, viewport: Rect, frame: &mut Frame<'_>) -> Option<Rect> {
    let (max_w, max_h) = popup.max_size;
    let max_w = max_w.min(viewport.width).max(3);
    let max_h = max_h.min(viewport.height).max(3);
    let inner_max_w = max_w.saturating_sub(2);
    let inner_max_h = max_h.saturating_sub(2);
    if inner_max_w == 0 || inner_max_h == 0 {
        return None;
    }

    let (content_w, content_h) = match &popup.content {
        PopupContent::Text(lines) => measure_text(lines, inner_max_w, inner_max_h)?,
        PopupContent::CompletionList { items, .. } => {
            measure_list(items, inner_max_w, inner_max_h)?
        }
    };

    let outer_w = content_w.saturating_add(2).max(3);
    let outer_h = content_h.saturating_add(2).max(3);

    let area = place(popup.anchor, outer_w, outer_h, viewport);
    Clear.render(area, frame.buffer_mut());

    let style = theme.text_style();
    let block = Block::default()
        .borders(Borders::ALL)
        .style(style);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());
    if inner.width == 0 || inner.height == 0 {
        return Some(area);
    }

    match &popup.content {
        PopupContent::Text(lines) => paint_text(lines, inner, frame),
        PopupContent::CompletionList { items, selected } => {
            paint_list(items, *selected, inner, theme, frame);
        }
    }
    Some(area)
}

fn measure_text(lines: &[String], inner_max_w: u16, inner_max_h: u16) -> Option<(u16, u16)> {
    if lines.is_empty() { return None; }
    let mut content_w: u16 = 0;
    let mut content_h: u16 = 0;
    for line in lines {
        let display_w = line.chars().count() as u16;
        let wrapped_rows = display_w.div_ceil(inner_max_w.max(1)).max(1);
        content_w = content_w.max(display_w.min(inner_max_w));
        content_h = content_h.saturating_add(wrapped_rows);
        if content_h >= inner_max_h {
            content_h = inner_max_h;
            break;
        }
    }
    Some((content_w, content_h))
}

fn measure_list(items: &[CompletionLine<'_>], inner_max_w: u16, inner_max_h: u16) -> Option<(u16, u16)> {
    if items.is_empty() { return None; }
    // Width: longest "label  detail" combined, clipped. Height: one row per
    // visible item, clipped to inner_max_h. We paint up to inner_max_h
    // rows starting at the selected item's window.
    let mut content_w: u16 = 0;
    for it in items.iter().take(inner_max_h as usize) {
        let label_w = it.label.chars().count() as u16;
        let detail_w = it
            .detail
            .map(|d| d.chars().count() as u16 + 3) // 3 cells of gap padding
            .unwrap_or(0);
        let row_w = label_w.saturating_add(detail_w).min(inner_max_w);
        content_w = content_w.max(row_w);
    }
    let content_h = (items.len() as u16).min(inner_max_h);
    Some((content_w, content_h))
}

fn place(anchor: PopupAnchor, outer_w: u16, outer_h: u16, viewport: Rect) -> Rect {
    let viewport_right = viewport.x.saturating_add(viewport.width);
    let viewport_bottom = viewport.y.saturating_add(viewport.height);
    let mut x = anchor.col;
    if x + outer_w > viewport_right {
        x = viewport_right.saturating_sub(outer_w);
    }
    if x < viewport.x {
        x = viewport.x;
    }
    let space_below = viewport_bottom.saturating_sub(anchor.row.saturating_add(1));
    let space_above = anchor.row.saturating_sub(viewport.y);
    let y = if space_below >= outer_h {
        anchor.row.saturating_add(1)
    } else if space_above >= outer_h {
        anchor.row.saturating_sub(outer_h)
    } else {
        viewport.y
    };
    Rect { x, y, width: outer_w, height: outer_h }
}

fn paint_text(lines: &[String], inner: Rect, frame: &mut Frame<'_>) {
    let body: Vec<Line<'_>> = lines.iter().map(|s| Line::from(s.as_str())).collect();
    Paragraph::new(body)
        .style(Style::default())
        .wrap(Wrap { trim: false })
        .render(inner, frame.buffer_mut());
}

fn paint_list(
    items: &[CompletionLine<'_>],
    selected: usize,
    inner: Rect,
    theme: &Theme,
    frame: &mut Frame<'_>,
) {
    let visible = inner.height as usize;
    if visible == 0 { return; }
    // Window the list around the selected item: keep selected on the
    // 1/3 row when possible, clamp at top / bottom (mirrors palette).
    let total = items.len();
    let target_row = visible / 3;
    let top = selected
        .saturating_sub(target_row)
        .min(total.saturating_sub(visible.min(total)));

    let select_style = theme.selection_style();
    let dim = Style::default().add_modifier(Modifier::DIM);

    for row in 0..visible {
        let idx = top + row;
        if idx >= total { break; }
        let item = &items[idx];
        let row_rect = Rect {
            x: inner.x,
            y: inner.y + row as u16,
            width: inner.width,
            height: 1,
        };
        let is_sel = idx == selected;
        let row_style = if is_sel { select_style } else { Style::default() };

        // Two-column layout: label left, detail right. Reserve gap space
        // only when there's actually a detail to push right; otherwise the
        // label gets the full row.
        let total_w = row_rect.width as usize;
        let detail_str = item.detail.unwrap_or("");
        let detail_w = detail_str.chars().count();
        let label_max = if detail_w == 0 {
            total_w
        } else {
            total_w.saturating_sub(detail_w + 1)
        };
        let label_trunc: String = item.label.chars().take(label_max).collect();
        let pad = total_w.saturating_sub(label_trunc.chars().count() + detail_w);
        let label_span = Span::styled(label_trunc, row_style);
        let pad_span = Span::styled(" ".repeat(pad), row_style);
        let detail_style = if is_sel { row_style } else { dim };
        let detail_span = Span::styled(detail_str.to_string(), detail_style);
        Paragraph::new(Line::from(vec![label_span, pad_span, detail_span]))
            .render(row_rect, frame.buffer_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn draw(popup: Popup<'_>, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let theme = Theme::default();
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let viewport = Rect { x: 0, y: 0, width: w, height: h };
                let _ = render_popup(&popup, &theme, viewport, f);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn popup_renders_below_anchor_when_room() {
        let lines = vec!["hello".to_string()];
        let popup = Popup::with_default_size(
            PopupAnchor { col: 2, row: 1 },
            PopupContent::Text(&lines),
        );
        let buf = draw(popup, 30, 10);
        // Top border at y=2, content at y=3.
        let cell = buf.cell((3, 3)).unwrap();
        assert_eq!(cell.symbol(), "h");
    }

    #[test]
    fn popup_flips_above_when_no_room_below() {
        let lines = vec!["x".to_string()];
        // anchor near the bottom edge; popup must flip.
        let popup = Popup::with_default_size(
            PopupAnchor { col: 1, row: 9 },
            PopupContent::Text(&lines),
        );
        let buf = draw(popup, 20, 10);
        // Below would have started at row 10 — out of bounds. Flip puts the
        // popup above (rows 6..9 with a 3-row box).
        let cell = buf.cell((2, 7)).unwrap();
        assert_eq!(cell.symbol(), "x");
    }

    #[test]
    fn empty_content_renders_nothing() {
        let lines: Vec<String> = vec![];
        let popup = Popup::with_default_size(
            PopupAnchor { col: 0, row: 0 },
            PopupContent::Text(&lines),
        );
        let buf = draw(popup, 20, 10);
        // Every cell should be unchanged from the empty TestBackend default.
        for x in 0..20 {
            for y in 0..10 {
                assert_eq!(buf.cell((x, y)).unwrap().symbol(), " ");
            }
        }
    }

    #[test]
    fn completion_list_renders_label_and_dim_detail() {
        let items = vec![
            CompletionLine { label: "new", detail: Some("fn() -> Self") },
            CompletionLine { label: "next", detail: None },
        ];
        let popup = Popup::with_default_size(
            PopupAnchor { col: 0, row: 0 },
            PopupContent::CompletionList { items: &items, selected: 0 },
        );
        let buf = draw(popup, 40, 10);
        // Anchor (0,0); popup paints below, so border top at y=1, content
        // starts at inner row y=2. Label "new" begins at (x=1, y=2).
        let cell = buf.cell((1, 2)).unwrap();
        assert_eq!(cell.symbol(), "n");
        // "next" on the next row at (1, 3).
        let next_cell = buf.cell((1, 3)).unwrap();
        assert_eq!(next_cell.symbol(), "n");
    }

    #[test]
    fn completion_list_highlights_selected() {
        let items = vec![
            CompletionLine { label: "a", detail: None },
            CompletionLine { label: "b", detail: None },
        ];
        let popup = Popup::with_default_size(
            PopupAnchor { col: 0, row: 0 },
            PopupContent::CompletionList { items: &items, selected: 1 },
        );
        let buf = draw(popup, 20, 10);
        let theme = Theme::default();
        let sel = theme.selection_style();
        // Selected "b" lives at (1, 3): border top y=1, items at inner y=2,3.
        let cell = buf.cell((1, 3)).unwrap();
        assert_eq!(cell.symbol(), "b");
        if let Some(expected_bg) = sel.bg {
            assert_eq!(cell.bg, expected_bg, "selected row should adopt theme selection bg");
        }
    }
}
