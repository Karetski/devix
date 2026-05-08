//! `devix-tui` — the terminal client + binary glue.
//!
//! - [`Application`] owns the run loop, the terminal, and every long-lived
//!   resource by direct field.
//! - [`AppContext`] is the unified `&mut` surface threaded through every
//!   delivery (input handler, typed-pulse handler).
//!
//! Cross-thread events flow through two channels:
//! * `PulseBus` (`devix-core::bus`, owned by `Editor`) for typed pulses
//!   per `docs/specs/pulse-bus.md` — disk watcher, plugin runtime, future
//!   LSP / clients.
//! * `EventSink` for terminal input from the input thread (crossterm
//!   events aren't a `Pulse` shape; the input thread keeps its dedicated
//!   channel).
//!
//! T-63 retired the closure-as-message `LoopMessage::Pulse(closure)` and
//! the `Effect` enum (`Redraw` / `Quit` / `Run`); typed pulses + direct
//! `AppContext::dirty_request` / `quit_request` flags replace them.

pub mod application;
pub mod clipboard;
pub mod context;
pub mod event_sink;
pub mod input;
mod input_thread;
mod interpreter;
pub mod view_paint;

pub use application::Application;
pub use context::AppContext;
pub use event_sink::{EventSink, LoopMessage};
