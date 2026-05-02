//! Sidebar widget: empty bordered placeholder. Body is reserved for plugins.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders};

pub struct SidebarInfo<'a> {
    pub title: &'a str,
    pub focused: bool,
}

pub fn render_sidebar(info: &SidebarInfo<'_>, area: Rect, frame: &mut Frame<'_>) {
    let style = if info.focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .title(info.title.to_string())
        .borders(Borders::ALL)
        .border_style(style);
    frame.render_widget(block, area);
}
