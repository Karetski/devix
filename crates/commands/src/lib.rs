//! Editor commands and command-dispatch infrastructure.
//!
//! This crate owns the four pieces that previously lived inside
//! `devix-surface` but had nothing to do with the surface state model:
//!
//! 1. The `Action`-trait command implementations (formerly `surface::cmd`).
//! 2. The `CommandRegistry` (formerly `surface::command`).
//! 3. The `Keymap` and chord parsing (formerly `surface::keymap`).
//! 4. The dispatcher `Context` (formerly `surface::context`).
//!
//! Plus the modal Pane infrastructure (palette, …) which is tightly
//! coupled to commands and registry — the palette runs commands and
//! displays their chord hints from the keymap.

pub mod builtins;
pub mod cmd;
pub mod context;
pub mod dispatch;
pub mod keymap;
pub mod modal;
pub mod registry;

pub use builtins::{build_registry, register_builtins};
pub use cmd::EditorCommand;
pub use context::{Context, Viewport};
pub use keymap::{Chord, Keymap, chord_from_key, default_keymap};
pub use modal::{ModalOutcome, PalettePane, PaletteState, format_chord};
pub use registry::{Command, CommandId, CommandRegistry};
