//! View IR types — partial.
//!
//! T-31 stubs the few types pulse payloads need (`Axis`, `SidebarSlot`,
//! `Style`, `Color`, `NamedColor`). T-40 / T-41 land the full View IR
//! and replace the derive-serde Color shape with the canonical
//! string-form per `docs/specs/frontend.md`.

use serde::{Deserialize, Serialize};

/// Layout axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Sidebar slot identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSlot {
    Left,
    Right,
}

/// Resolved style. T-41 replaces `Color` with the canonical
/// string-form serde; the field shape is stable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub reverse: bool,
}

/// Color value. Stub serde at T-31 (default externally-tagged form);
/// T-41 replaces with the canonical string form (`"default"` /
/// `"#rrggbb"` / `"@<n>"` / `"<named>"`) per
/// `docs/specs/frontend.md` § *Color serialization*.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
    Indexed(u8),
    Named(NamedColor),
}

/// Named ANSI/VT100-equivalent colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
}
