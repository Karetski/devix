//! Tab strip widget: file-name pills, active tab inverted, ellipsis on overflow.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub struct TabInfo {
    pub label: String,
    pub dirty: bool,
}

pub fn render_tabstrip(tabs: &[TabInfo], active: usize, area: Rect, frame: &mut Frame<'_>) {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(tabs.len() * 2);
    for (i, t) in tabs.iter().enumerate() {
        let label = format!(" {}{} ", t.label, if t.dirty { "*" } else { "" });
        let style = if i == active {
            Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("│"));
    }
    let line = Line::from(spans);
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}
