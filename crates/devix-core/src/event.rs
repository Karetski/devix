//! Input events delivered to `Pane::handle`.
//!
//! Re-exported from `crossterm` for the same reason `Rect` comes from
//! `ratatui` — the input task already produces these and a translation
//! layer would just shift work without changing the abstraction. If a
//! non-crossterm backend ever shows up (mock for tests, gui shim, etc.)
//! this is where the wrapper enum gets introduced.

pub use crossterm::event::Event;
