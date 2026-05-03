//! Floating popup primitive: a bordered box with text content, anchored to
//! a screen cell, sized to its content (clamped against max size and the
//! viewport), and flipped above its anchor when below would clip.
//!
//! Used by hover today; completion (slice 3) will reuse this surface with a
//! different content payload.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use devix_config::Theme;

/// Cell the popup attaches to. The renderer prefers placing the box one row
/// below `(col, row)` (so it doesn't cover the symbol the user is pointing
/// at); if that overflows the viewport bottom, it flips above instead.
#[derive(Copy, Clone, Debug)]
pub struct PopupAnchor {
    pub col: u16,
    pub row: u16,
}

/// Popup payload. Today only `Text`; completion will add a list variant.
#[derive(Clone, Debug)]
pub enum PopupContent<'a> {
    /// Pre-split lines. The renderer wraps long lines on word boundaries.
    Text(&'a [String]),
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
    let PopupContent::Text(lines) = &popup.content;
    let lines: &[String] = lines;
    if lines.is_empty() {
        return None;
    }

    let (max_w, max_h) = popup.max_size;
    let max_w = max_w.min(viewport.width).max(3);
    let max_h = max_h.min(viewport.height).max(3);
    let inner_max_w = max_w.saturating_sub(2);
    if inner_max_w == 0 {
        return None;
    }

    // Content sizing pass: width = longest line clipped to inner_max_w;
    // height = wrapped-row count clipped to inner_max_h.
    let inner_max_h = max_h.saturating_sub(2);
    let mut content_w: u16 = 0;
    let mut content_h: u16 = 0;
    for line in lines {
        // Char count is a coarse cell estimate — fine for ASCII source text;
        // CJK / emoji are over-counted, but the popup gets clipped anyway.
        let display_w = line.chars().count() as u16;
        let wrapped_rows = display_w.div_ceil(inner_max_w.max(1)).max(1);
        content_w = content_w.max(display_w.min(inner_max_w));
        content_h = content_h.saturating_add(wrapped_rows);
        if content_h >= inner_max_h {
            content_h = inner_max_h;
            break;
        }
    }
    let outer_w = content_w.saturating_add(2).max(3);
    let outer_h = content_h.saturating_add(2).max(3);

    // Anchor-relative placement. Prefer below; flip above if the box would
    // overflow the bottom edge.
    let viewport_right = viewport.x.saturating_add(viewport.width);
    let viewport_bottom = viewport.y.saturating_add(viewport.height);
    let mut x = popup.anchor.col;
    if x + outer_w > viewport_right {
        x = viewport_right.saturating_sub(outer_w);
    }
    if x < viewport.x {
        x = viewport.x;
    }
    let space_below = viewport_bottom.saturating_sub(popup.anchor.row.saturating_add(1));
    let space_above = popup.anchor.row.saturating_sub(viewport.y);
    let y = if space_below >= outer_h {
        popup.anchor.row.saturating_add(1)
    } else if space_above >= outer_h {
        popup.anchor.row.saturating_sub(outer_h)
    } else {
        // Neither fits cleanly — clamp to viewport top, let the box take what
        // it can. The wrap will hide overflow.
        viewport.y
    };

    let area = Rect { x, y, width: outer_w, height: outer_h };
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

    let body: Vec<Line<'_>> = lines.iter().map(|s| Line::from(s.as_str())).collect();
    Paragraph::new(body)
        .style(Style::default())
        .wrap(Wrap { trim: false })
        .render(inner, frame.buffer_mut());

    Some(area)
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
}
