//! Transient floating UI: command palette today, completion / hover later.
//!
//! Overlays sit above the editor in paint order (no z-buffer in ratatui — we
//! just paint last). Input gates through the overlay first; on `PassThrough`
//! the chord falls back to the keymap.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

use crate::command::{CommandId, CommandRegistry};

pub enum Overlay {
    Palette(PaletteState),
}

pub struct PaletteState {
    query: String,
    /// Cached `Utf32String` per registered command, indexed parallel to
    /// `command_ids`. Built once at open; rebuilt only if the registry
    /// changes (which won't happen mid-overlay in v1).
    haystack: Vec<Utf32String>,
    command_ids: Vec<CommandId>,
    /// Filtered + scored view into `command_ids`. Each entry is an index back
    /// into `command_ids`; the order is best-match-first.
    matches: Vec<usize>,
    selected: usize,
    matcher: Matcher,
}

impl PaletteState {
    /// Snapshot the registry into the palette's haystack and start with an
    /// empty query (which matches every command in registration order).
    pub fn from_registry(reg: &CommandRegistry) -> Self {
        let mut haystack = Vec::with_capacity(reg.len());
        let mut command_ids = Vec::with_capacity(reg.len());
        for cmd in reg.iter() {
            haystack.push(Utf32String::from(cmd.label));
            command_ids.push(cmd.id);
        }
        let matches: Vec<usize> = (0..command_ids.len()).collect();
        Self {
            query: String::new(),
            haystack,
            command_ids,
            matches,
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    pub fn query(&self) -> &str { &self.query }
    pub fn matches(&self) -> &[usize] { &self.matches }
    pub fn selected(&self) -> usize { self.selected }

    pub fn matched_command_id(&self, match_idx: usize) -> Option<CommandId> {
        self.matches.get(match_idx).and_then(|i| self.command_ids.get(*i)).copied()
    }

    /// Currently-highlighted command id, or `None` if there are no matches.
    pub fn selected_command_id(&self) -> Option<CommandId> {
        self.matched_command_id(self.selected)
    }

    pub fn set_query(&mut self, q: String) {
        if q == self.query { return; }
        self.query = q;
        self.refilter();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.matches.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.matches.len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(len);
        self.selected = next as usize;
    }

    fn refilter(&mut self) {
        if self.query.is_empty() {
            self.matches = (0..self.command_ids.len()).collect();
            return;
        }
        let pattern = Pattern::parse(
            &self.query,
            CaseMatching::Smart,
            Normalization::Smart,
        );
        let mut scored: Vec<(usize, u32)> = self
            .haystack
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                pattern.score(s.slice(..), &mut self.matcher).map(|score| (i, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        self.matches = scored.into_iter().map(|(i, _)| i).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::command::{Command, CommandId, CommandRegistry};

    fn reg() -> CommandRegistry {
        let mut r = CommandRegistry::new();
        for (id, label) in [
            ("editor.save", "Save File"),
            ("tab.new", "New Tab"),
            ("tab.close", "Close Tab"),
            ("tab.next", "Next Tab"),
            ("app.quit", "Quit"),
        ] {
            r.register(Command {
                id: CommandId(id), label, category: None, action: Action::Quit,
            });
        }
        r
    }

    #[test]
    fn empty_query_lists_all_in_order() {
        let p = PaletteState::from_registry(&reg());
        assert_eq!(p.matches().len(), 5);
        assert_eq!(p.selected_command_id(), Some(CommandId("editor.save")));
    }

    #[test]
    fn query_filters_and_ranks() {
        let mut p = PaletteState::from_registry(&reg());
        p.set_query("tab".into());
        assert!(p.matches().len() >= 3);
        for i in 0..p.matches().len() {
            let id = p.matched_command_id(i).unwrap();
            assert!(id.0.starts_with("tab.") || id.0.contains("tab"));
        }
    }

    #[test]
    fn move_wraps() {
        let mut p = PaletteState::from_registry(&reg());
        p.move_selection(-1);
        assert_eq!(p.selected(), 4);
        p.move_selection(1);
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn no_match_leaves_selection_at_zero() {
        let mut p = PaletteState::from_registry(&reg());
        p.set_query("zzzzzzzzzz".into());
        assert!(p.matches().is_empty());
        p.move_selection(3);
        assert_eq!(p.selected(), 0);
        assert!(p.selected_command_id().is_none());
    }
}
