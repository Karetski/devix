//! Theme = scope-name → style table.
//!
//! Tree-sitter highlights queries emit dotted scope names (e.g.
//! `keyword.control.repeat`). Lookups walk the dotted path from most-specific
//! to least, so a theme can register "keyword" once and have every keyword
//! sub-scope inherit it; a more specific entry like "keyword.control" wins
//! over the bare "keyword".
//!
//! v1 ships a single baked-in default theme (One-Dark-adjacent). TOML loading
//! plugs in here once we wire the config-file pipeline — the in-memory
//! representation won't change.

use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Debug)]
pub struct Theme {
    /// Style applied to plain text (no scope match). Used as the fallback fg.
    text: Style,
    /// Selection-range highlight. Editor renders selection on top, so it
    /// stays here so all visual config lives in one place.
    selection: Style,
    scopes: HashMap<String, Style>,
}

impl Theme {
    pub fn new(text: Style, selection: Style) -> Self {
        Self {
            text,
            selection,
            scopes: HashMap::new(),
        }
    }

    pub fn with_scope(mut self, scope: impl Into<String>, style: Style) -> Self {
        self.scopes.insert(scope.into(), style);
        self
    }

    pub fn text_style(&self) -> Style {
        self.text
    }

    pub fn selection_style(&self) -> Style {
        self.selection
    }

    /// Resolve a dotted scope name to a `Style`. Walks from the full name
    /// down to its first component (`a.b.c` → `a.b.c`, `a.b`, `a`). Returns
    /// `None` if no prefix is registered.
    pub fn style_for(&self, scope: &str) -> Option<Style> {
        let mut cur: &str = scope;
        loop {
            if let Some(s) = self.scopes.get(cur) {
                return Some(*s);
            }
            match cur.rfind('.') {
                Some(i) => cur = &cur[..i],
                None => return None,
            }
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        // Palette inspired by One Dark. Hex literals mirror common terminal
        // themes so the editor looks familiar without a config file.
        let fg = Color::Rgb(0xab, 0xb2, 0xbf);
        let comment = Color::Rgb(0x5c, 0x63, 0x70);
        let red = Color::Rgb(0xe0, 0x6c, 0x75);
        let orange = Color::Rgb(0xd1, 0x9a, 0x66);
        let yellow = Color::Rgb(0xe5, 0xc0, 0x7b);
        let green = Color::Rgb(0x98, 0xc3, 0x79);
        let cyan = Color::Rgb(0x56, 0xb6, 0xc2);
        let blue = Color::Rgb(0x61, 0xaf, 0xef);
        let purple = Color::Rgb(0xc6, 0x78, 0xdd);

        let s = |fg: Color| Style::default().fg(fg);
        Theme::new(
            Style::default().fg(fg),
            Style::default().bg(Color::Rgb(60, 80, 130)),
        )
        // Comments and punctuation lean dim.
        .with_scope("comment", Style::default().fg(comment).add_modifier(Modifier::ITALIC))
        .with_scope("punctuation", s(fg))
        .with_scope("punctuation.delimiter", s(fg))
        .with_scope("punctuation.bracket", s(fg))
        // Keywords / control flow.
        .with_scope("keyword", s(purple))
        .with_scope("keyword.control", s(purple))
        .with_scope("keyword.function", s(purple))
        .with_scope("conditional", s(purple))
        .with_scope("repeat", s(purple))
        .with_scope("operator", s(cyan))
        // Identifiers / types.
        .with_scope("variable", s(fg))
        .with_scope("variable.parameter", s(red))
        .with_scope("property", s(red))
        .with_scope("type", s(yellow))
        .with_scope("type.builtin", s(yellow))
        .with_scope("constructor", s(yellow))
        // Functions.
        .with_scope("function", s(blue))
        .with_scope("function.call", s(blue))
        .with_scope("function.builtin", s(blue))
        .with_scope("function.macro", s(blue))
        .with_scope("method", s(blue))
        // Literals.
        .with_scope("string", s(green))
        .with_scope("string.special", s(green))
        .with_scope("character", s(green))
        .with_scope("number", s(orange))
        .with_scope("boolean", s(orange))
        .with_scope("constant", s(orange))
        .with_scope("constant.builtin", s(orange))
        .with_scope("escape", s(cyan))
        // Attributes / labels.
        .with_scope("attribute", s(yellow))
        .with_scope("label", s(red))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_scope_returns_none() {
        let t = Theme::default();
        assert!(t.style_for("nonsense.thing").is_none());
    }

    #[test]
    fn dotted_scope_falls_back_to_parent() {
        let t = Theme::default();
        // `keyword.unusual` is not registered but `keyword` is — should match.
        let style = t.style_for("keyword.unusual");
        assert_eq!(style, t.style_for("keyword"));
    }

    #[test]
    fn more_specific_wins_over_parent() {
        let theme = Theme::new(Style::default(), Style::default())
            .with_scope("keyword", Style::default().fg(Color::Red))
            .with_scope("keyword.control", Style::default().fg(Color::Blue));
        assert_eq!(theme.style_for("keyword").unwrap().fg, Some(Color::Red));
        assert_eq!(
            theme.style_for("keyword.control").unwrap().fg,
            Some(Color::Blue),
        );
        assert_eq!(
            theme.style_for("keyword.control.repeat").unwrap().fg,
            Some(Color::Blue),
            "deeper unknown leaves should fall back to closest known prefix",
        );
    }

    #[test]
    fn default_theme_resolves_common_scopes() {
        let t = Theme::default();
        assert!(t.style_for("keyword").is_some());
        assert!(t.style_for("string").is_some());
        assert!(t.style_for("function").is_some());
        assert!(t.style_for("comment").is_some());
    }
}
