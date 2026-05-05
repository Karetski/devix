//! Editor commands and command-dispatch infrastructure.
//!
//! - The `Action`-trait command implementations live in `cmd`.
//! - `CommandRegistry` indexes them by id for the palette.
//! - `Keymap` maps chords to action handles.
//! - `Context` is the dispatcher's per-invocation state bundle.
//! - `modal` holds the palette / picker Pane impls — the modal slot's
//!   typical contents.

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
