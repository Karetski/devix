//! Status line: pure render of a `StatusInfo` value.
//!
//! `StatusInfo` is built by the binary from its `App` state; this module knows
//! nothing about `App`.

use devix_core::{Event, HandleCtx, Outcome, Pane, RenderCtx};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

pub struct StatusInfo<'a> {
    pub path: Option<&'a str>,
    pub dirty: bool,
    pub line: usize,
    pub col: usize,
    pub sel_len: usize,
    pub message: Option<&'a str>,
    pub diag_errors: usize,
    pub diag_warnings: usize,
}

pub fn render_status(info: &StatusInfo<'_>, area: Rect, frame: &mut Frame<'_>) {
    let path = info.path.unwrap_or("[scratch]");
    let dirty = if info.dirty { " [+]" } else { "" };
    let sel = if info.sel_len > 0 {
        format!(" ({} sel)", info.sel_len)
    } else {
        String::new()
    };
    let diags = if info.diag_errors > 0 || info.diag_warnings > 0 {
        format!("  E:{} W:{}", info.diag_errors, info.diag_warnings)
    } else {
        String::new()
    };

    let left = format!(" {}{}  {}:{}{}{}", path, dirty, info.line, info.col, sel, diags);
    let right = info
        .message
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Ctrl+S save · Ctrl+Q quit".to_string());

    let total = area.width as usize;
    let pad = total.saturating_sub(left.chars().count() + right.chars().count() + 1);
    let text = format!("{}{}{} ", left, " ".repeat(pad), right);

    let para = Paragraph::new(text).style(Style::default().bg(Color::DarkGray).fg(Color::White));
    frame.render_widget(para, area);
}

/// Phase-1 adapter: thin `Pane` wrapper over [`render_status`]. Borrows the
/// info struct so callers can build it from per-frame state without
/// allocating. Has no children and ignores events — the status line is
/// purely cosmetic.
pub struct StatusPane<'a> {
    pub info: StatusInfo<'a>,
}

impl<'a> Pane for StatusPane<'a> {
    fn render(&self, area: Rect, ctx: &mut RenderCtx<'_, '_>) {
        render_status(&self.info, area, ctx.frame);
    }

    fn handle(&mut self, _ev: &Event, _area: Rect, _ctx: &mut HandleCtx<'_>) -> Outcome {
        Outcome::Ignored
    }
}
