//! Input events — partial.
//!
//! T-31 stubs `InputEvent` so `Pulse::InputReceived` can carry a typed
//! payload. The canonical kebab-case serde for `Chord` / `KeyCode`
//! lands at T-42 per `docs/specs/frontend.md` § *Chord serialization*.

use serde::{Deserialize, Serialize};

/// Input event from the frontend. T-42 replaces the placeholder
/// structured serde on `Chord` and `KeyCode` with the canonical
/// kebab-case form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    Key {
        chord: Chord,
        text: Option<char>,
    },
    Mouse {
        x: u16,
        y: u16,
        button: Option<MouseButton>,
        #[serde(rename = "press")]
        kind: MouseKind,
        modifiers: Modifiers,
    },
    Scroll {
        x: u16,
        y: u16,
        delta_x: i32,
        delta_y: i32,
        modifiers: Modifiers,
    },
    Paste(String),
    FocusGained,
    FocusLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind {
    Down,
    Up,
    Drag,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    #[serde(rename = "super")]
    pub super_key: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chord {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Esc,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}
