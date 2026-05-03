//! Symbol picker overlay. Same centered floating shape as the palette;
//! list rows show a kind tag + name + (right-aligned, dim) container.
//!
//! `render_symbols` is parameterized only by `&SymbolsState` so the picker
//! works identically for document- and workspace-mode lists; the dispatch
//! layer decides which the user opened.

use lsp_types::SymbolKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use devix_config::Theme;
use devix_workspace::{SymbolsKind, SymbolsState, SymbolsStatus};

pub fn symbols_area(parent: Rect) -> Rect {
    let w = (parent.width as u32 * 60 / 100).clamp(40, 100) as u16;
    let w = w.min(parent.width);
    let h = (parent.height as u32 * 60 / 100).clamp(8, 24) as u16;
    let h = h.min(parent.height);
    let x = parent.x + (parent.width.saturating_sub(w)) / 2;
    let y = parent.y + (parent.height.saturating_sub(h)) / 3;
    Rect { x, y, width: w, height: h }
}

pub fn render_symbols(state: &SymbolsState, theme: &Theme, parent: Rect, frame: &mut Frame<'_>) {
    let area = symbols_area(parent);
    Clear.render(area, frame.buffer_mut());

    let title = match state.kind {
        SymbolsKind::Document => " Document Symbols ",
        SymbolsKind::Workspace => " Workspace Symbols ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme.text_style());
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());
    if inner.width == 0 || inner.height == 0 { return; }

    // Query row.
    let query_row = Rect { height: 1, ..inner };
    let query_text = format!("> {}", state.query);
    Paragraph::new(query_text).render(query_row, frame.buffer_mut());

    if inner.height <= 2 { return; }

    let list_area = Rect {
        y: inner.y + 2,
        height: inner.height - 2,
        ..inner
    };

    let visible = list_area.height as usize;
    if visible == 0 { return; }

    // Status: while pending, show a single placeholder row instead of an
    // empty list. Workspace mode also shows pending while a refetch is in
    // flight even though items may already be populated — the renderer
    // doesn't get to see that distinction; the dispatcher keeps `items`
    // populated through the in-flight window so the user never sees a
    // flicker to empty.
    if matches!(state.status, SymbolsStatus::Pending) && state.items.is_empty() {
        let row_rect = Rect { height: 1, ..list_area };
        let dim = Style::default().add_modifier(Modifier::DIM);
        Paragraph::new(Line::from(Span::styled("…", dim)))
            .render(row_rect, frame.buffer_mut());
        return;
    }

    let total = state.matches.len();
    if total == 0 { return; }
    let target_row = visible / 3;
    let top = state
        .selected
        .saturating_sub(target_row)
        .min(total.saturating_sub(visible.min(total)));

    let select_style = theme.selection_style();
    let dim = Style::default().add_modifier(Modifier::DIM);

    for row in 0..visible {
        let match_idx = top + row;
        if match_idx >= total { break; }
        let Some(sym) = state.matched_symbol(match_idx) else { continue };

        let row_rect = Rect {
            x: list_area.x,
            y: list_area.y + row as u16,
            width: list_area.width,
            height: 1,
        };

        let is_sel = match_idx == state.selected;
        let row_style = if is_sel { select_style } else { Style::default() };

        let kind_tag = symbol_kind_short(sym.kind);
        let indent = "  ".repeat(sym.depth as usize);
        let left = format!("{indent}[{kind_tag}] {name}", indent = indent, kind_tag = kind_tag, name = sym.name);
        let right = sym.container.clone().unwrap_or_default();

        let total_w = row_rect.width as usize;
        let right_w = right.chars().count();
        let left_max = if right_w == 0 { total_w } else { total_w.saturating_sub(right_w + 1) };
        let left_trunc: String = left.chars().take(left_max).collect();
        let pad = total_w.saturating_sub(left_trunc.chars().count() + right_w);
        let right_style = if is_sel { row_style } else { dim };
        Paragraph::new(Line::from(vec![
            Span::styled(left_trunc, row_style),
            Span::styled(" ".repeat(pad), row_style),
            Span::styled(right, right_style),
        ]))
        .render(row_rect, frame.buffer_mut());
    }
}

/// Tight tag for a `SymbolKind` — three-or-four-letter slugs that fit in a
/// terminal cell budget. Fallbacks to the numeric kind for unrecognized
/// values so future LSP revisions don't render as blanks.
fn symbol_kind_short(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::FILE => "file",
        SymbolKind::MODULE => "mod",
        SymbolKind::NAMESPACE => "ns",
        SymbolKind::PACKAGE => "pkg",
        SymbolKind::CLASS => "cls",
        SymbolKind::METHOD => "fn",
        SymbolKind::PROPERTY => "prop",
        SymbolKind::FIELD => "field",
        SymbolKind::CONSTRUCTOR => "ctor",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "iface",
        SymbolKind::FUNCTION => "fn",
        SymbolKind::VARIABLE => "var",
        SymbolKind::CONSTANT => "const",
        SymbolKind::STRING => "str",
        SymbolKind::NUMBER => "num",
        SymbolKind::BOOLEAN => "bool",
        SymbolKind::ARRAY => "arr",
        SymbolKind::OBJECT => "obj",
        SymbolKind::KEY => "key",
        SymbolKind::NULL => "null",
        SymbolKind::ENUM_MEMBER => "evar",
        SymbolKind::STRUCT => "struct",
        SymbolKind::EVENT => "event",
        SymbolKind::OPERATOR => "op",
        SymbolKind::TYPE_PARAMETER => "tparam",
        _ => "sym",
    }
}
