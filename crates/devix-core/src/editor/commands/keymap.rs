//! Chord → command resolution.
//!
//! The keymap maps a [`Chord`] to either a [`CommandId`] (for discoverable,
//! palette-listable commands registered in [`crate::command`]) or directly
//! to an `Arc<dyn EditorCommand>` (for continuously-fired actions like
//! motion, character insertion, mouse — these never appear in the palette
//! so they bypass the registry).
//!
//! Phase 5: storage migrated from `Action` enum values to trait objects.
//! `lookup` returns `Arc<dyn EditorCommand>` so the caller can drop the
//! immutable keymap/registry borrow before invoking with `&mut Context`.

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::editor::commands::builtins as cmd_id;
use crate::editor::commands::cmd::{self, EditorCommand};
use crate::editor::commands::registry::{CommandId, CommandRegistry};
use crate::{Direction, SidebarSlot};

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

enum Binding {
    Command(CommandId),
    Action(Arc<dyn EditorCommand>),
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

    pub fn bind_action(&mut self, chord: Chord, action: Arc<dyn EditorCommand>) {
        self.bindings.insert(chord, Binding::Action(action));
    }

    pub fn lookup(
        &self,
        chord: Chord,
        reg: &CommandRegistry,
    ) -> Option<Arc<dyn EditorCommand>> {
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
    k.bind_command(chord(ch('q'), C),                       cmd_id::APP_QUIT);
    k.bind_command(chord(ch('s'), C),                       cmd_id::FILE_SAVE);

    k.bind_command(chord(ch('z'), C),                       cmd_id::EDIT_UNDO);
    k.bind_command(chord(ch('z'), C | S),                   cmd_id::EDIT_REDO);
    k.bind_command(chord(ch('y'), C),                       cmd_id::EDIT_REDO);
    k.bind_command(chord(ch('a'), C),                       cmd_id::EDIT_SELECT_ALL);
    k.bind_command(chord(ch('c'), C),                       cmd_id::EDIT_COPY);
    k.bind_command(chord(ch('x'), C),                       cmd_id::EDIT_CUT);
    k.bind_command(chord(ch('v'), C),                       cmd_id::EDIT_PASTE);


    k.bind_command(chord(ch('t'), C),                       cmd_id::TAB_NEW);
    k.bind_command(chord(ch('w'), C),                       cmd_id::TAB_CLOSE);
    k.bind_command(chord(ch('w'), C | S),                   cmd_id::TAB_FORCE_CLOSE);
    k.bind_command(chord(KeyCode::Char('['), C | S),        cmd_id::TAB_PREV);
    k.bind_command(chord(KeyCode::Char(']'), C | S),        cmd_id::TAB_NEXT);
    // macOS Terminal.app drops the SHIFT bit on Ctrl+Shift+symbol and
    // delivers the shifted character with CTRL alone (Shift+[ = {).
    k.bind_command(chord(KeyCode::Char('{'), C),            cmd_id::TAB_PREV);
    k.bind_command(chord(KeyCode::Char('}'), C),            cmd_id::TAB_NEXT);
    k.bind_command(chord(KeyCode::PageUp, C),               cmd_id::TAB_PREV);
    k.bind_command(chord(KeyCode::PageDown, C),             cmd_id::TAB_NEXT);

    k.bind_command(chord(ch('\\'), C),                      cmd_id::SPLIT_VERTICAL);
    k.bind_command(chord(ch('-'), C),                       cmd_id::SPLIT_HORIZONTAL);

    k.bind_command(chord(ch('b'), C),                       cmd_id::SIDEBAR_LEFT);
    k.bind_command(chord(ch('b'), C | A),                   cmd_id::SIDEBAR_RIGHT);

    // Command palette
    k.bind_command(chord(ch('p'), C),                       cmd_id::PALETTE_OPEN);

    // ---- direct actions (continuous; not registry commands) ----
    // Motion — both extend variants per chord
    for &(extend, sm) in &[(false, NONE), (true, S)] {
        k.bind_action(chord(KeyCode::Left,  C | sm), Arc::new(cmd::MoveLineStart { extend }));
        k.bind_action(chord(KeyCode::Right, C | sm), Arc::new(cmd::MoveLineEnd   { extend }));
        k.bind_action(chord(KeyCode::Up,    C | sm), Arc::new(cmd::MoveDocStart  { extend }));
        k.bind_action(chord(KeyCode::Down,  C | sm), Arc::new(cmd::MoveDocEnd    { extend }));

        k.bind_action(chord(KeyCode::Left,  A | sm), Arc::new(cmd::MoveWordLeft  { extend }));
        k.bind_action(chord(KeyCode::Right, A | sm), Arc::new(cmd::MoveWordRight { extend }));

        k.bind_action(chord(KeyCode::Left,  sm), Arc::new(cmd::MoveLeft  { extend }));
        k.bind_action(chord(KeyCode::Right, sm), Arc::new(cmd::MoveRight { extend }));
        k.bind_action(chord(KeyCode::Up,    sm), Arc::new(cmd::MoveUp    { extend }));
        k.bind_action(chord(KeyCode::Down,  sm), Arc::new(cmd::MoveDown  { extend }));

        k.bind_action(chord(KeyCode::Home,     sm), Arc::new(cmd::MoveLineStart { extend }));
        k.bind_action(chord(KeyCode::End,      sm), Arc::new(cmd::MoveLineEnd   { extend }));
        k.bind_action(chord(KeyCode::PageUp,   sm), Arc::new(cmd::PageUp        { extend }));
        k.bind_action(chord(KeyCode::PageDown, sm), Arc::new(cmd::PageDown      { extend }));

        // Ctrl+Home / Ctrl+End — fallback for Ctrl+Up / Ctrl+Down which macOS
        // swallows for Mission Control unless those system shortcuts are off.
        k.bind_action(chord(KeyCode::Home, C | sm), Arc::new(cmd::MoveDocStart { extend }));
        k.bind_action(chord(KeyCode::End,  C | sm), Arc::new(cmd::MoveDocEnd   { extend }));
    }

    // ESC b / ESC f for Option+Left/Right on terminals (macOS Terminal.app)
    // that emit the legacy meta encoding. No shift bit in that encoding.
    k.bind_action(chord(ch('b'), A), Arc::new(cmd::MoveWordLeft  { extend: false }));
    k.bind_action(chord(ch('f'), A), Arc::new(cmd::MoveWordRight { extend: false }));

    // Directional focus traversal
    k.bind_action(chord(KeyCode::Left,  C | A), Arc::new(cmd::FocusDir(Direction::Left)));
    k.bind_action(chord(KeyCode::Right, C | A), Arc::new(cmd::FocusDir(Direction::Right)));
    k.bind_action(chord(KeyCode::Up,    C | A), Arc::new(cmd::FocusDir(Direction::Up)));
    k.bind_action(chord(KeyCode::Down,  C | A), Arc::new(cmd::FocusDir(Direction::Down)));

    // Edits
    k.bind_action(chord(KeyCode::Backspace, NONE), Arc::new(cmd::DeleteBack    { word: false }));
    k.bind_action(chord(KeyCode::Backspace, A),    Arc::new(cmd::DeleteBack    { word: true  }));
    k.bind_action(chord(KeyCode::Delete,    NONE), Arc::new(cmd::DeleteForward { word: false }));
    k.bind_action(chord(KeyCode::Delete,    A),    Arc::new(cmd::DeleteForward { word: true  }));
    k.bind_action(chord(KeyCode::Enter,     NONE), Arc::new(cmd::InsertNewline));
    k.bind_action(chord(KeyCode::Tab,       NONE), Arc::new(cmd::InsertTab));

    // Bind each `SidebarSlot` so the keymap stays self-contained — the only
    // chord-bound place that uses the slot enum.
    let _ = SidebarSlot::Left;

    // Multi-cursor.
    //
    // `Shift+Ctrl+Up/Down` overrides the `MoveDocStart/End { extend: true }`
    // bindings the loop above installed for those chords — extend-to-doc
    // is still reachable via `Shift+Ctrl+Home/End`, which is the standard
    // cross-platform chord. `Esc` collapses multi-cursor back to primary
    // (and an active selection to a point) when no completion or modal is
    // intercepting it.
    k.bind_command(chord(KeyCode::Up,   C | S),             cmd_id::EDIT_ADD_CURSOR_ABOVE);
    k.bind_command(chord(KeyCode::Down, C | S),             cmd_id::EDIT_ADD_CURSOR_BELOW);
    k.bind_command(chord(KeyCode::Esc, NONE),               cmd_id::EDIT_COLLAPSE_SELECTION);

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
