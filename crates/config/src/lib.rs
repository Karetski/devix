//! Settings, themes, keymap. v1 ships baked-in defaults; TOML loading lands
//! once the config-file pipeline is in place.

pub mod keymap;
pub mod theme;

pub use keymap::{Chord, Keymap, chord_from_key, default_keymap};
pub use theme::Theme;
