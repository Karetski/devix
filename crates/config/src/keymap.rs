//! Chord → command resolution.
//!
//! The keymap maps a [`Chord`] to either a [`CommandId`] (for discoverable,
//! palette-listable commands registered in [`crate::commands`]) or directly
//! to an [`Action`] (for continuously-fired actions like motion, character
//! insertion, mouse — these never appear in the palette so they bypass the
//! registry).
//!
//! The dispatcher only sees `Action`. The two-arm layering exists so the
//! palette can show "Save File … Ctrl+S" by reverse-looking-up the chord
//! bound to a command id, without polluting the command registry with
//! continuous-action variants that nobody would ever search for.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};
use devix_workspace::{Action, CommandId, CommandRegistry};

use crate::commands as cmd;

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self { code, mods }
    }
}

#[derive(Clone)]
enum Binding {
    Command(CommandId),
    Action(Action),
}

pub struct Keymap {
    bindings: HashMap<Chord, Binding>,
    /// Inverse index: command id → first chord bound to it. Built lazily;
    /// rebuilt on every `bind_command`. Kept as a small map (~30 entries) so
    /// the rebuild cost is irrelevant.
    chord_for: HashMap<CommandId, Chord>,
}

impl Keymap {
    pub fn new() -> Self {
        Self { bindings: HashMap::new(), chord_for: HashMap::new() }
    }

    pub fn bind_command(&mut self, chord: Chord, id: CommandId) {
        self.bindings.insert(chord, Binding::Command(id));
        // Only record the *first* chord we see for an id — palettes typically
        // show one canonical hint, not every alias (Ctrl+W vs Ctrl+F4).
        self.chord_for.entry(id).or_insert(chord);
    }

    pub fn bind_action(&mut self, chord: Chord, action: Action) {
        self.bindings.insert(chord, Binding::Action(action));
    }

    pub fn lookup(&self, chord: Chord, reg: &CommandRegistry) -> Option<Action> {
        match self.bindings.get(&chord)? {
            Binding::Command(id) => reg.resolve(*id),
            Binding::Action(a) => Some(a.clone()),
        }
    }

    /// First chord bound to `id`, for palette / status-line display.
    pub fn chord_for(&self, id: CommandId) -> Option<Chord> {
        self.chord_for.get(&id).copied()
    }
}

impl Default for Keymap {
    fn default() -> Self { Self::new() }
}

const C: KeyModifiers = KeyModifiers::CONTROL;
const S: KeyModifiers = KeyModifiers::SHIFT;
const A: KeyModifiers = KeyModifiers::ALT;
const NONE: KeyModifiers = KeyModifiers::NONE;

fn chord(code: KeyCode, mods: KeyModifiers) -> Chord { Chord::new(code, mods) }
fn ch(c: char) -> KeyCode { KeyCode::Char(c) }

pub fn default_keymap() -> Keymap {
    let mut k = Keymap::new();

    // ---- registered commands (id-bound; palette can find them) ----
    k.bind_command(chord(ch('q'), C),                       cmd::APP_QUIT);
    k.bind_command(chord(ch('s'), C),                       cmd::FILE_SAVE);

    k.bind_command(chord(ch('z'), C),                       cmd::EDIT_UNDO);
    k.bind_command(chord(ch('z'), C | S),                   cmd::EDIT_REDO);
    k.bind_command(chord(ch('y'), C),                       cmd::EDIT_REDO);
    k.bind_command(chord(ch('a'), C),                       cmd::EDIT_SELECT_ALL);
    k.bind_command(chord(ch('c'), C),                       cmd::EDIT_COPY);
    k.bind_command(chord(ch('x'), C),                       cmd::EDIT_CUT);
    k.bind_command(chord(ch('v'), C),                       cmd::EDIT_PASTE);

    k.bind_command(chord(ch('t'), C),                       cmd::TAB_NEW);
    k.bind_command(chord(ch('w'), C),                       cmd::TAB_CLOSE);
    k.bind_command(chord(ch('w'), C | S),                   cmd::TAB_FORCE_CLOSE);
    k.bind_command(chord(KeyCode::Char('['), C | S),        cmd::TAB_PREV);
    k.bind_command(chord(KeyCode::Char(']'), C | S),        cmd::TAB_NEXT);
    // macOS Terminal.app drops the SHIFT bit on Ctrl+Shift+symbol and
    // delivers the shifted character with CTRL alone (Shift+[ = {).
    k.bind_command(chord(KeyCode::Char('{'), C),            cmd::TAB_PREV);
    k.bind_command(chord(KeyCode::Char('}'), C),            cmd::TAB_NEXT);
    k.bind_command(chord(KeyCode::PageUp, C),               cmd::TAB_PREV);
    k.bind_command(chord(KeyCode::PageDown, C),             cmd::TAB_NEXT);

    k.bind_command(chord(ch('\\'), C),                      cmd::SPLIT_VERTICAL);
    k.bind_command(chord(ch('-'), C),                       cmd::SPLIT_HORIZONTAL);

    k.bind_command(chord(ch('b'), C),                       cmd::SIDEBAR_LEFT);
    k.bind_command(chord(ch('b'), C | A),                   cmd::SIDEBAR_RIGHT);

    // Command palette
    k.bind_command(chord(ch('p'), C | S),                   cmd::PALETTE_OPEN);

    // Language server
    k.bind_command(chord(ch('i'), C),                       cmd::LSP_HOVER);
    k.bind_command(chord(KeyCode::F(12), NONE),             cmd::LSP_GOTO_DEFINITION);
    k.bind_command(chord(ch(' '), C),                       cmd::LSP_COMPLETION_TRIGGER);
    k.bind_command(chord(ch('o'), C),                       cmd::LSP_DOCUMENT_SYMBOLS);
    k.bind_command(chord(ch('o'), C | S),                   cmd::LSP_WORKSPACE_SYMBOLS);

    // ---- direct actions (continuous; not registry commands) ----
    // Motion — both extend variants per chord
    for &(extend, sm) in &[(false, NONE), (true, S)] {
        k.bind_action(chord(KeyCode::Left,  C | sm), Action::MoveLineStart { extend });
        k.bind_action(chord(KeyCode::Right, C | sm), Action::MoveLineEnd   { extend });
        k.bind_action(chord(KeyCode::Up,    C | sm), Action::MoveDocStart  { extend });
        k.bind_action(chord(KeyCode::Down,  C | sm), Action::MoveDocEnd    { extend });

        k.bind_action(chord(KeyCode::Left,  A | sm), Action::MoveWordLeft  { extend });
        k.bind_action(chord(KeyCode::Right, A | sm), Action::MoveWordRight { extend });

        k.bind_action(chord(KeyCode::Left,  sm), Action::MoveLeft  { extend });
        k.bind_action(chord(KeyCode::Right, sm), Action::MoveRight { extend });
        k.bind_action(chord(KeyCode::Up,    sm), Action::MoveUp    { extend });
        k.bind_action(chord(KeyCode::Down,  sm), Action::MoveDown  { extend });

        k.bind_action(chord(KeyCode::Home,     sm), Action::MoveLineStart { extend });
        k.bind_action(chord(KeyCode::End,      sm), Action::MoveLineEnd   { extend });
        k.bind_action(chord(KeyCode::PageUp,   sm), Action::PageUp        { extend });
        k.bind_action(chord(KeyCode::PageDown, sm), Action::PageDown      { extend });

        // Ctrl+Home / Ctrl+End — fallback for Ctrl+Up / Ctrl+Down which macOS
        // swallows for Mission Control unless those system shortcuts are off.
        k.bind_action(chord(KeyCode::Home, C | sm), Action::MoveDocStart { extend });
        k.bind_action(chord(KeyCode::End,  C | sm), Action::MoveDocEnd   { extend });
    }

    // ESC b / ESC f for Option+Left/Right on terminals (macOS Terminal.app)
    // that emit the legacy meta encoding. No shift bit in that encoding.
    k.bind_action(chord(ch('b'), A), Action::MoveWordLeft  { extend: false });
    k.bind_action(chord(ch('f'), A), Action::MoveWordRight { extend: false });

    // Directional focus traversal
    k.bind_action(chord(KeyCode::Left,  C | A), Action::FocusDir(devix_workspace::Direction::Left));
    k.bind_action(chord(KeyCode::Right, C | A), Action::FocusDir(devix_workspace::Direction::Right));
    k.bind_action(chord(KeyCode::Up,    C | A), Action::FocusDir(devix_workspace::Direction::Up));
    k.bind_action(chord(KeyCode::Down,  C | A), Action::FocusDir(devix_workspace::Direction::Down));

    // Edits
    k.bind_action(chord(KeyCode::Backspace, NONE), Action::DeleteBack    { word: false });
    k.bind_action(chord(KeyCode::Backspace, A),    Action::DeleteBack    { word: true  });
    k.bind_action(chord(KeyCode::Delete,    NONE), Action::DeleteForward { word: false });
    k.bind_action(chord(KeyCode::Delete,    A),    Action::DeleteForward { word: true  });
    k.bind_action(chord(KeyCode::Enter,     NONE), Action::InsertNewline);
    k.bind_action(chord(KeyCode::Tab,       NONE), Action::InsertTab);

    // Reserved (Phase 7 multicursor):
    //   Shift + Ctrl + Up   → MulticursorAddAbove
    //   Shift + Ctrl + Down → MulticursorAddBelow
    // These chords currently fall through to MoveUp/MoveDown { extend: true }.

    k
}

/// Normalize a `KeyEvent` into a `Chord`. Lowercases ASCII alphabetic chars
/// (so Ctrl+s and Ctrl+S share a chord); preserves all modifier bits.
pub fn chord_from_key(code: KeyCode, mods: KeyModifiers) -> Chord {
    let code = match code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() => KeyCode::Char(c.to_ascii_lowercase()),
        other => other,
    };
    Chord::new(code, mods)
}
