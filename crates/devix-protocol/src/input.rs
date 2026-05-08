//! Input events — `docs/specs/frontend.md` § *InputEvent*,
//! *Chord serialization*.
//!
//! `Chord` and `KeyCode` use **custom string serde** (not derived):
//! the wire form is the canonical kebab-case form
//! `<modifiers>-<key>` defined in `docs/specs/namespace.md`. Modifier
//! order is fixed `ctrl-alt-shift-super` (when present). Named keys
//! and letter keys are lowercase. Out-of-order modifiers, uppercase
//! tokens, and unknown keys are deserialize errors.
//!
//! Chord text-on-Key policy (frontend.md Q5, lean: always set when
//! printable): stored on `InputEvent::Key.text` — keymap dispatch
//! consults `chord`; insertion paths consult `text`.

use serde::{Deserialize, Serialize};

/// Input event from the frontend. `Mouse.press` was previously
/// `Mouse.kind`; renamed at T-31 review (2026-05-07) to avoid the
/// `serde(tag = "kind")` collision.
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
        press: MouseKind,
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub ctrl: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub alt: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub shift: bool,
    #[serde(rename = "super", default, skip_serializing_if = "is_false")]
    pub super_key: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A keypress: a key plus zero or more modifiers. Custom serde to
/// the canonical kebab-case form (`"ctrl-shift-p"`, `"alt-left"`,
/// `"f12"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

// -- Display / Parse --------------------------------------------------------

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.modifiers.ctrl {
            f.write_str("ctrl-")?;
        }
        if self.modifiers.alt {
            f.write_str("alt-")?;
        }
        if self.modifiers.shift {
            f.write_str("shift-")?;
        }
        if self.modifiers.super_key {
            f.write_str("super-")?;
        }
        write!(f, "{}", self.key)
    }
}

impl std::fmt::Display for KeyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyCode::Char(c) => {
                // Letter keys lowercased per spec.
                for lower in c.to_lowercase() {
                    write!(f, "{}", lower)?;
                }
                Ok(())
            }
            KeyCode::Enter => f.write_str("enter"),
            KeyCode::Tab => f.write_str("tab"),
            KeyCode::BackTab => f.write_str("backtab"),
            KeyCode::Esc => f.write_str("esc"),
            KeyCode::Backspace => f.write_str("backspace"),
            KeyCode::Delete => f.write_str("delete"),
            KeyCode::Insert => f.write_str("insert"),
            KeyCode::Left => f.write_str("left"),
            KeyCode::Right => f.write_str("right"),
            KeyCode::Up => f.write_str("up"),
            KeyCode::Down => f.write_str("down"),
            KeyCode::Home => f.write_str("home"),
            KeyCode::End => f.write_str("end"),
            KeyCode::PageUp => f.write_str("pageup"),
            KeyCode::PageDown => f.write_str("pagedown"),
            KeyCode::F(n) => write!(f, "f{}", n),
        }
    }
}

impl Chord {
    /// Parse a chord from its canonical kebab-case form.
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Err("empty chord".into());
        }
        // Split on '-'. Treat each segment as either a modifier (in
        // fixed order ctrl < alt < shift < super) or the key. The
        // key is always the last segment.
        let parts: Vec<&str> = s.split('-').collect();
        let (key_seg, mods) = parts
            .split_last()
            .map(|(last, rest)| (*last, rest))
            .ok_or_else(|| format!("malformed chord `{}`", s))?;
        let mut modifiers = Modifiers::default();
        // Fixed-order acceptance: each new modifier must follow
        // every preceding one in the order ctrl, alt, shift, super.
        let mut idx_seen: i8 = -1;
        for m in mods {
            let idx = match *m {
                "ctrl" => 0i8,
                "alt" => 1,
                "shift" => 2,
                "super" => 3,
                other => return Err(format!("unknown modifier `{}` in chord `{}`", other, s)),
            };
            if idx <= idx_seen {
                return Err(format!(
                    "modifier `{}` out of order in chord `{}` (canonical order is ctrl-alt-shift-super)",
                    m, s
                ));
            }
            idx_seen = idx;
            match idx {
                0 => modifiers.ctrl = true,
                1 => modifiers.alt = true,
                2 => modifiers.shift = true,
                3 => modifiers.super_key = true,
                _ => unreachable!(),
            }
        }
        let key = KeyCode::parse(key_seg)
            .ok_or_else(|| format!("unknown key `{}` in chord `{}`", key_seg, s))?;
        Ok(Chord { key, modifiers })
    }
}

impl KeyCode {
    /// Parse just the key segment of a chord. Single ASCII chars
    /// must be lowercase (uppercase is rejected — the canonical
    /// form is "shift-a", not "A").
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "enter" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "esc" => KeyCode::Esc,
            "backspace" => KeyCode::Backspace,
            "delete" => KeyCode::Delete,
            "insert" => KeyCode::Insert,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            other => {
                // F-key: "f<1..12>".
                if let Some(rest) = other.strip_prefix('f') {
                    if let Ok(n) = rest.parse::<u8>() {
                        if (1..=12).contains(&n) {
                            return Some(KeyCode::F(n));
                        }
                    }
                }
                // Single ASCII char (must be lowercase or non-letter).
                let mut chars = other.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    // multi-char and not a recognized name
                    return None;
                }
                if c.is_ascii_uppercase() {
                    return None;
                }
                KeyCode::Char(c)
            }
        })
    }
}

// -- Serde impls ------------------------------------------------------------

impl schemars::JsonSchema for Chord {
    fn schema_name() -> String {
        "Chord".to_string()
    }
    fn json_schema(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Metadata, SchemaObject};
        SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            metadata: Some(Box::new(Metadata {
                description: Some(
                    "Keyboard chord in kebab-case: 'ctrl-shift-p', 'alt-left', 'f12'."
                        .into(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

impl Serialize for Chord {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Chord;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a canonical kebab-case chord (e.g. `ctrl-shift-p`)")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Chord, E> {
                Chord::parse(v).map_err(serde::de::Error::custom)
            }
        }
        d.deserialize_str(V)
    }
}

impl Serialize for KeyCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for KeyCode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = KeyCode;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a canonical kebab-case key (`p`, `enter`, `f12`, …)")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<KeyCode, E> {
                KeyCode::parse(v).ok_or_else(|| serde::de::Error::custom(format!("unknown key `{}`", v)))
            }
        }
        d.deserialize_str(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord_from(s: &str) -> Chord {
        Chord::parse(s).unwrap_or_else(|e| panic!("parse `{}`: {}", s, e))
    }

    #[test]
    fn chord_round_trips_common_forms() {
        let cases = [
            ("ctrl-s", Chord { key: KeyCode::Char('s'), modifiers: Modifiers { ctrl: true, ..Default::default() } }),
            ("ctrl-shift-p", Chord { key: KeyCode::Char('p'), modifiers: Modifiers { ctrl: true, shift: true, ..Default::default() } }),
            ("alt-left", Chord { key: KeyCode::Left, modifiers: Modifiers { alt: true, ..Default::default() } }),
            ("ctrl-alt-down", Chord { key: KeyCode::Down, modifiers: Modifiers { ctrl: true, alt: true, ..Default::default() } }),
            ("shift-tab", Chord { key: KeyCode::Tab, modifiers: Modifiers { shift: true, ..Default::default() } }),
            ("f12", Chord { key: KeyCode::F(12), modifiers: Modifiers::default() }),
            ("p", Chord { key: KeyCode::Char('p'), modifiers: Modifiers::default() }),
            ("enter", Chord { key: KeyCode::Enter, modifiers: Modifiers::default() }),
        ];
        for (s, expected) in cases {
            assert_eq!(chord_from(s), expected, "input: {}", s);
            assert_eq!(format!("{}", expected), s, "round-trip: {}", s);
        }
    }

    #[test]
    fn chord_rejects_out_of_order_modifiers() {
        assert!(Chord::parse("shift-ctrl-p").is_err());
        assert!(Chord::parse("alt-ctrl-down").is_err());
        assert!(Chord::parse("super-shift-p").is_err());
    }

    #[test]
    fn chord_rejects_uppercase_modifier_or_key() {
        assert!(Chord::parse("Ctrl-s").is_err());
        assert!(Chord::parse("ctrl-S").is_err());
        // Bare uppercase letter is not a chord.
        assert!(Chord::parse("A").is_err());
    }

    #[test]
    fn chord_rejects_unknown_segments() {
        // Unknown modifier.
        assert!(Chord::parse("hyper-p").is_err());
        // Unknown key name.
        assert!(Chord::parse("ctrl-banana").is_err());
        // F-key out of range.
        assert!(Chord::parse("f0").is_err());
        assert!(Chord::parse("f13").is_err());
    }

    #[test]
    fn chord_serde_uses_string_form() {
        let c = chord_from("ctrl-shift-p");
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"ctrl-shift-p\"");
        let back: Chord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn key_code_round_trips_via_serde() {
        let cases = [
            (KeyCode::Char('p'), "\"p\""),
            (KeyCode::Enter, "\"enter\""),
            (KeyCode::F(12), "\"f12\""),
            (KeyCode::PageDown, "\"pagedown\""),
        ];
        for (k, s) in cases {
            assert_eq!(serde_json::to_string(&k).unwrap(), s);
            let back: KeyCode = serde_json::from_str(s).unwrap();
            assert_eq!(back, k);
        }
    }

    #[test]
    fn input_event_key_round_trips() {
        let ev = InputEvent::Key {
            chord: chord_from("ctrl-shift-a"),
            text: Some('A'),
        };
        let json = serde_json::to_string(&ev).unwrap();
        // Chord embeds as the canonical string.
        assert!(json.contains("\"ctrl-shift-a\""));
        let back: InputEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn input_event_mouse_uses_press_field() {
        let ev = InputEvent::Mouse {
            x: 10,
            y: 5,
            button: Some(MouseButton::Left),
            press: MouseKind::Down,
            modifiers: Modifiers { shift: true, ..Default::default() },
        };
        let json = serde_json::to_string(&ev).unwrap();
        // Wire form has `press`, not `kind`.
        assert!(json.contains("\"press\":\"down\""));
        let back: InputEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn modifiers_default_is_empty_object() {
        let m = Modifiers::default();
        assert_eq!(serde_json::to_string(&m).unwrap(), "{}");
    }
}
