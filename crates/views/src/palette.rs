//! Command palette overlay. Centered floating box: query line on top,
//! ranked match list below. Painted last in the render pass so it sits over
//! the editor.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use devix_config::Theme;
use devix_workspace::{Chord, CommandRegistry, Keymap, PaletteState};

/// Compute the centered area the palette occupies inside `parent`. ~60% of
/// width, capped to a usable height range so it never dominates a tall window
/// or vanishes in a short one.
pub fn palette_area(parent: Rect) -> Rect {
    let w = (parent.width as u32 * 60 / 100).clamp(40, 100) as u16;
    let w = w.min(parent.width);
    let h = (parent.height as u32 * 60 / 100).clamp(8, 24) as u16;
    let h = h.min(parent.height);
    let x = parent.x + (parent.width.saturating_sub(w)) / 2;
    let y = parent.y + (parent.height.saturating_sub(h)) / 3; // upper third feels right
    Rect { x, y, width: w, height: h }
}

pub fn render_palette(
    state: &PaletteState,
    registry: &CommandRegistry,
    keymap: &Keymap,
    theme: &Theme,
    parent: Rect,
    frame: &mut Frame<'_>,
) {
    let area = palette_area(parent);
    Clear.render(area, frame.buffer_mut());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Command Palette ")
        .style(theme.text_style());
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());
    if inner.width == 0 || inner.height == 0 { return; }

    // Query row: "> {query}_"
    let query_row = Rect { height: 1, ..inner };
    let query_text = format!("> {}", state.query());
    Paragraph::new(query_text).render(query_row, frame.buffer_mut());

    if inner.height <= 2 { return; }

    let list_area = Rect {
        y: inner.y + 2,
        height: inner.height - 2,
        ..inner
    };

    let visible = list_area.height as usize;
    if visible == 0 { return; }

    // Window the match list around the selection so it stays in view as the
    // user arrows through. Simple anchor: keep selection on row `visible/3`
    // when possible, clamp at top/bottom.
    let total = state.matches().len();
    let selected = state.selected();
    let target_row = visible / 3;
    let top = selected.saturating_sub(target_row).min(total.saturating_sub(visible.min(total)));

    let select_style = theme.selection_style();
    let dim = Style::default().add_modifier(Modifier::DIM);

    for row in 0..visible {
        let match_idx = top + row;
        if match_idx >= total { break; }
        let Some(id) = state.matched_command_id(match_idx) else { continue };
        let Some(cmd) = registry.get(id) else { continue };

        let chord_str = keymap.chord_for(id).map(format_chord).unwrap_or_default();
        let label = cmd.label;
        let category = cmd.category.unwrap_or("");

        let row_rect = Rect {
            x: list_area.x,
            y: list_area.y + row as u16,
            width: list_area.width,
            height: 1,
        };

        // Layout columns: "{label}  {category}                    {chord}"
        // We render label + category as one span, chord right-aligned by
        // padding the whole line to the row width.
        let left = if category.is_empty() {
            label.to_string()
        } else {
            format!("{}    {}", label, category)
        };
        let total_w = row_rect.width as usize;
        let chord_w = chord_str.len();
        let left_max = total_w.saturating_sub(chord_w + 1);
        let left_trunc: String = left.chars().take(left_max).collect();
        let pad = total_w
            .saturating_sub(left_trunc.chars().count() + chord_w);
        let line_text = format!(
            "{}{}{}",
            left_trunc,
            " ".repeat(pad),
            chord_str,
        );

        let style = if match_idx == selected { select_style } else { Style::default() };
        let chord_style = if match_idx == selected { select_style } else { dim };

        // Two spans so the chord can be dim while the label uses the normal
        // style. Highlight rows colour both.
        let label_span = Span::styled(
            line_text[..line_text.len() - chord_str.len()].to_string(),
            style,
        );
        let chord_span = Span::styled(chord_str.clone(), chord_style);
        Paragraph::new(Line::from(vec![label_span, chord_span])).render(row_rect, frame.buffer_mut());
    }
}

/// Render `Chord` as a human-readable shortcut string ("Ctrl+Shift+P").
pub fn format_chord(chord: Chord) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(4);
    if chord.mods.contains(KeyModifiers::CONTROL) { parts.push("Ctrl".into()); }
    if chord.mods.contains(KeyModifiers::ALT)     { parts.push("Alt".into()); }
    if chord.mods.contains(KeyModifiers::SHIFT)   { parts.push("Shift".into()); }
    let key = match chord.code {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn format_chord_letter() {
        let c = Chord::new(KeyCode::Char('p'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(format_chord(c), "Ctrl+Shift+P");
    }

    #[test]
    fn format_chord_named() {
        let c = Chord::new(KeyCode::PageUp, KeyModifiers::CONTROL);
        assert_eq!(format_chord(c), "Ctrl+PgUp");
    }
}
