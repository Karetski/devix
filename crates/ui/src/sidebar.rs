//! Sidebar widget: empty bordered placeholder. Body is reserved for plugins.

use devix_core::{Event, HandleCtx, Outcome, Pane, RenderCtx};
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

/// `Pane` wrapper around [`render_sidebar`]. Owns its title `String` so a
/// parent composite (`SidebarSlotPane`) can store it as a field without a
/// self-referential borrow.
pub struct SidebarPane {
    pub title: String,
    pub focused: bool,
}

impl Pane for SidebarPane {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        let info = SidebarInfo { title: &self.title, focused: self.focused };
        render_sidebar(&info, area, ctx.frame);
    }

    fn handle(&mut self, _ev: &Event, _area: Rect, _ctx: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }

    fn is_focusable(&self) -> bool {
        true
    }
}
