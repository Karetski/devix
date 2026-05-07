//! Command palette rendering.
//!
//! The palette's *state* lives in `devix-commands` (`PaletteState`,
//! `PalettePane`) — pure logic, no rendering. This module owns the
//! ratatui paint pass that turns that state into pixels. Keeping the
//! widget calls here means the model layer (`commands`) can stay free of
//! `ratatui::widgets`.

use crossterm::event::{KeyCode, KeyModifiers};
use crate::{Rect, Theme};
use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

/// One palette row: identifier, label, optional category, optional chord
/// hint. The renderer iterates a slice of these — it does not touch the
/// command registry or keymap directly.
pub struct PaletteRow<'a> {
    pub label: &'a str,
    pub category: &'a str,
    pub chord: &'a str,
    pub selected: bool,
}

/// Compute the centered area the palette occupies inside `parent`.
pub fn palette_area(parent: Rect) -> Rect {
    let w = (parent.width as u32 * 60 / 100).clamp(40, 100) as u16;
    let w = w.min(parent.width);
    let h = (parent.height as u32 * 60 / 100).clamp(8, 24) as u16;
    let h = h.min(parent.height);
    let x = parent.x + (parent.width.saturating_sub(w)) / 2;
    let y = parent.y + (parent.height.saturating_sub(h)) / 3;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Paint a palette into `area`. The caller (app render) projects
/// `PaletteState` into the visible window of `PaletteRow`s and a query
/// string; this function does only the ratatui painting.
pub fn render_palette(
    query: &str,
    rows: &[PaletteRow<'_>],
    selected: usize,
    area: Rect,
    theme: &Theme,
    frame: &mut Frame<'_>,
) {
    Clear.render(area, frame.buffer_mut());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Command Palette ")
        .style(theme.text_style());
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let query_row = Rect { height: 1, ..inner };
    let query_text = format!("> {}", query);
    Paragraph::new(query_text).render(query_row, frame.buffer_mut());

    if inner.height <= 2 {
        return;
    }

    let list_area = Rect {
        y: inner.y + 2,
        height: inner.height - 2,
        ..inner
    };

    let visible = list_area.height as usize;
    if visible == 0 {
        return;
    }

    let total = rows.len();
    let target_row = visible / 3;
    let top = selected
        .saturating_sub(target_row)
        .min(total.saturating_sub(visible.min(total)));

    let select_style = theme.selection_style();
    let dim = Style::default().add_modifier(Modifier::DIM);

    for r in 0..visible {
        let match_idx = top + r;
        if match_idx >= total {
            break;
        }
        let row = &rows[match_idx];
        let label = row.label;
        let category = row.category;
        let chord_str = row.chord;

        let row_rect = Rect {
            x: list_area.x,
            y: list_area.y + r as u16,
            width: list_area.width,
            height: 1,
        };

        let left = if category.is_empty() {
            label.to_string()
        } else {
            format!("{}    {}", label, category)
        };
        let total_w = row_rect.width as usize;
        let chord_w = chord_str.len();
        let left_max = total_w.saturating_sub(chord_w + 1);
        let left_trunc: String = left.chars().take(left_max).collect();
        let pad = total_w.saturating_sub(left_trunc.chars().count() + chord_w);
        let line_text = format!("{}{}{}", left_trunc, " ".repeat(pad), chord_str);

        let style = if row.selected {
            select_style
        } else {
            Style::default()
        };
        let chord_style = if row.selected { select_style } else { dim };

        let label_span = Span::styled(
            line_text[..line_text.len() - chord_str.len()].to_string(),
            style,
        );
        let chord_span = Span::styled(chord_str.to_string(), chord_style);
        Paragraph::new(Line::from(vec![label_span, chord_span]))
            .render(row_rect, frame.buffer_mut());
    }
}

/// Render a `crossterm` chord-like (code, mods) pair as a human-readable
/// shortcut ("Ctrl+Shift+P"). Used to project `commands::Chord` for the
/// palette without dragging the `commands` crate's types into `ui`.
pub fn format_chord(code: KeyCode, mods: KeyModifiers) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(4);
    if mods.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".into());
    }
    if mods.contains(KeyModifiers::ALT) {
        parts.push("Alt".into());
    }
    if mods.contains(KeyModifiers::SHIFT) {
        parts.push("Shift".into());
    }
    let key = match code {
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    };
    parts.push(key);
    parts.join("+")
}
