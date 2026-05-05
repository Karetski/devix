//! Modal panes and their owned state.
//!
//! The architecture target: a head-of-tree slot on `Editor` holds
//! `Option<Box<dyn Pane>>`. When set, the modal sits at the head of the
//! responder chain — the dispatcher gives it first crack at every input
//! event before the focused-leaf path. There is no closed `Overlay` enum
//! and no per-modal type-tagging in the framework.
//!
//! Rendering for these panes lives in `devix-ui` (e.g.
//! `ui::palette::render_palette`). This module owns only the state and
//! input handling so the model layer stays free of `ratatui::widgets`.

use std::any::Any;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use devix_panes::{HandleCtx, Outcome, Pane, Rect, RenderCtx};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

use crate::commands::keymap::Chord;
use crate::commands::registry::{CommandId, CommandRegistry};

/// Side-effect requested by a modal Pane during input handling. The host
/// reads and clears this after `Pane::handle` returns; modals signal
/// what they cannot do themselves (close themselves out of the slot,
/// invoke another command).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModalOutcome {
    None,
    Dismiss,
    Accept,
}

// ---------------------------------------------------------------------------
// Palette state
// ---------------------------------------------------------------------------

pub struct PaletteState {
    query: String,
    haystack: Vec<Utf32String>,
    command_ids: Vec<CommandId>,
    matches: Vec<usize>,
    selected: usize,
    matcher: Matcher,
}

impl PaletteState {
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

    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn matches(&self) -> &[usize] {
        &self.matches
    }
    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn matched_command_id(&self, match_idx: usize) -> Option<CommandId> {
        self.matches
            .get(match_idx)
            .and_then(|i| self.command_ids.get(*i))
            .copied()
    }

    pub fn selected_command_id(&self) -> Option<CommandId> {
        self.matched_command_id(self.selected)
    }

    pub fn set_query(&mut self, q: String) {
        self.query = q;
        self.refilter();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.matches.is_empty() {
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
        let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
        let mut scored: Vec<(usize, u32)> = self
            .haystack
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                pattern
                    .score(s.slice(..), &mut self.matcher)
                    .map(|score| (i, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        self.matches = scored.into_iter().map(|(i, _)| i).collect();
    }
}

// ---------------------------------------------------------------------------
// PalettePane — owned modal Pane (state + input only; rendering in `devix-ui`)
// ---------------------------------------------------------------------------

pub struct PalettePane {
    pub state: PaletteState,
    outcome: ModalOutcome,
}

impl PalettePane {
    pub fn from_registry(reg: &CommandRegistry) -> Self {
        Self {
            state: PaletteState::from_registry(reg),
            outcome: ModalOutcome::None,
        }
    }

    pub fn drain_outcome(&mut self) -> ModalOutcome {
        std::mem::replace(&mut self.outcome, ModalOutcome::None)
    }

    fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Outcome {
        match (code, mods) {
            (KeyCode::Esc, _) => {
                self.outcome = ModalOutcome::Dismiss;
                Outcome::Consumed
            }
            (KeyCode::Enter, _) => {
                self.outcome = ModalOutcome::Accept;
                Outcome::Consumed
            }
            (KeyCode::Up, _) => {
                self.state.move_selection(-1);
                Outcome::Consumed
            }
            (KeyCode::Down, _) => {
                self.state.move_selection(1);
                Outcome::Consumed
            }
            (KeyCode::Backspace, _) => {
                let mut q = self.state.query().to_string();
                q.pop();
                self.state.set_query(q);
                Outcome::Consumed
            }
            (KeyCode::Char(c), m)
                if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) =>
            {
                let mut q = self.state.query().to_string();
                q.push(c);
                self.state.set_query(q);
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }
}

impl Pane for PalettePane {
    /// PalettePane render is a no-op: rendering is driven externally
    /// from the app's render module via `devix_panes::render_palette`,
    /// which gets the typed registry/keymap context the trait can't
    /// carry. Keeping this empty preserves the closed `dyn Pane` shape
    /// without coupling commands to ratatui.
    fn render(&self, _area: Rect, _ctx: &mut RenderCtx<'_, '_>) {}

    fn handle(&mut self, ev: &Event, _area: Rect, _ctx: &mut HandleCtx<'_>) -> Outcome {
        match ev {
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind,
                ..
            }) if matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key(*code, *modifiers)
            }
            _ => Outcome::Ignored,
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Render `Chord` as a human-readable shortcut string ("Ctrl+Shift+P").
/// Thin wrapper over `devix_panes::palette::format_chord` so callers can
/// pass a `Chord` directly. Lives here because `Chord` is a commands type.
pub fn format_chord(chord: Chord) -> String {
    devix_panes::format_chord(chord.code, chord.mods)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cmd::Quit;
    use crate::commands::registry::{Command, CommandId, CommandRegistry};
    use std::sync::Arc;

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
                id: CommandId(id),
                label,
                category: None,
                action: Arc::new(Quit),
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

    #[test]
    fn palette_pane_arrow_keys_move_selection_and_consume() {
        let mut p = PalettePane::from_registry(&reg());
        let r = p.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(r, Outcome::Consumed);
        assert_eq!(p.state.selected(), 1);
        assert_eq!(p.drain_outcome(), ModalOutcome::None);
    }

    #[test]
    fn palette_pane_esc_signals_dismiss() {
        let mut p = PalettePane::from_registry(&reg());
        let r = p.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(r, Outcome::Consumed);
        assert_eq!(p.drain_outcome(), ModalOutcome::Dismiss);
    }

    #[test]
    fn palette_pane_enter_signals_accept() {
        let mut p = PalettePane::from_registry(&reg());
        let r = p.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(r, Outcome::Consumed);
        assert_eq!(p.drain_outcome(), ModalOutcome::Accept);
    }

    #[test]
    fn palette_pane_typing_updates_query() {
        let mut p = PalettePane::from_registry(&reg());
        p.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        p.handle_key(KeyCode::Char('a'), KeyModifiers::NONE);
        p.handle_key(KeyCode::Char('b'), KeyModifiers::NONE);
        assert_eq!(p.state.query(), "tab");
    }

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
