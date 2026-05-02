//! Chord → Action mapping. The default keymap mirrors the Phase-1/2 binding set.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};
use teditor_workspace::Action;

#[derive(Copy, Clone, Hash, Eq, PartialEq)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Chord {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self { code, mods }
    }
}

pub struct Keymap {
    bindings: HashMap<Chord, Action>,
}

impl Keymap {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    pub fn bind(&mut self, chord: Chord, action: Action) {
        self.bindings.insert(chord, action);
    }

    pub fn lookup(&self, chord: Chord) -> Option<Action> {
        self.bindings.get(&chord).cloned()
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

const C: KeyModifiers = KeyModifiers::CONTROL;
const S: KeyModifiers = KeyModifiers::SHIFT;
const A: KeyModifiers = KeyModifiers::ALT;
const NONE: KeyModifiers = KeyModifiers::NONE;

fn chord(code: KeyCode, mods: KeyModifiers) -> Chord {
    Chord::new(code, mods)
}

fn ch(c: char) -> KeyCode {
    KeyCode::Char(c)
}

pub fn default_keymap() -> Keymap {
    let mut k = Keymap::new();

    // app + file
    k.bind(chord(ch('q'), C), Action::Quit);
    k.bind(chord(ch('s'), C), Action::Save);

    // history
    k.bind(chord(ch('z'), C), Action::Undo);
    k.bind(chord(ch('z'), C | S), Action::Redo);
    k.bind(chord(ch('y'), C), Action::Redo);

    // selection
    k.bind(chord(ch('a'), C), Action::SelectAll);

    // clipboard
    k.bind(chord(ch('c'), C), Action::Copy);
    k.bind(chord(ch('x'), C), Action::Cut);
    k.bind(chord(ch('v'), C), Action::Paste);

    // motion — both extend variants per chord
    for &(extend, sm) in &[(false, NONE), (true, S)] {
        // ctrl + arrows: line/doc bounds
        k.bind(chord(KeyCode::Left, C | sm), Action::MoveLineStart { extend });
        k.bind(chord(KeyCode::Right, C | sm), Action::MoveLineEnd { extend });
        k.bind(chord(KeyCode::Up, C | sm), Action::MoveDocStart { extend });
        k.bind(chord(KeyCode::Down, C | sm), Action::MoveDocEnd { extend });

        // alt + arrows: word motion
        k.bind(chord(KeyCode::Left, A | sm), Action::MoveWordLeft { extend });
        k.bind(chord(KeyCode::Right, A | sm), Action::MoveWordRight { extend });

        // plain arrows
        k.bind(chord(KeyCode::Left, sm), Action::MoveLeft { extend });
        k.bind(chord(KeyCode::Right, sm), Action::MoveRight { extend });
        k.bind(chord(KeyCode::Up, sm), Action::MoveUp { extend });
        k.bind(chord(KeyCode::Down, sm), Action::MoveDown { extend });

        // home / end / pageup / pagedown
        k.bind(chord(KeyCode::Home, sm), Action::MoveLineStart { extend });
        k.bind(chord(KeyCode::End, sm), Action::MoveLineEnd { extend });
        k.bind(chord(KeyCode::PageUp, sm), Action::PageUp { extend });
        k.bind(chord(KeyCode::PageDown, sm), Action::PageDown { extend });
    }

    // edits
    k.bind(chord(KeyCode::Backspace, NONE), Action::DeleteBack { word: false });
    k.bind(chord(KeyCode::Backspace, A), Action::DeleteBack { word: true });
    k.bind(chord(KeyCode::Delete, NONE), Action::DeleteForward { word: false });
    k.bind(chord(KeyCode::Delete, A), Action::DeleteForward { word: true });
    k.bind(chord(KeyCode::Enter, NONE), Action::InsertNewline);
    k.bind(chord(KeyCode::Tab, NONE), Action::InsertTab);

    k
}

/// Normalize a `KeyEvent` into a `Chord` suitable for keymap lookup.
/// Lowercases ASCII alphabetic chars (so Ctrl+s and Ctrl+S share a chord),
/// preserving all modifier bits as crossterm reports them.
pub fn chord_from_key(code: KeyCode, mods: KeyModifiers) -> Chord {
    let code = match code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() => KeyCode::Char(c.to_ascii_lowercase()),
        other => other,
    };
    Chord::new(code, mods)
}
