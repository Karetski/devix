//! Settings, themes, keymap. Phase 4+ will add settings/themes; today we
//! expose a hardcoded default keymap.

pub mod keymap;

pub use keymap::{Chord, Keymap, chord_from_key, default_keymap};
