//! `devix-tui` — the terminal client + binary glue.
//!
//! - [`Application`] owns the run loop, the terminal, and every long-lived
//!   resource by direct field.
//! - [`Effect`] is runtime-internal deferred work drained between
//!   messages. Stage 6 (T-61 / T-63) folds `Effect` into typed pulses.
//! - [`AppContext`] is the unified `&mut` surface threaded through every
//!   delivery.
//!
//! `EventSink` is the cross-thread handle producers hold; cross-thread
//! messages are boxed `FnOnce(&mut AppContext)` closures — there is no
//! typed pulse trait, no `Service` trait, no god-set of message structs.
//! Stage 6 (T-60 / T-63) replaces this with `PulseBus` (`devix-core::bus`)
//! per `docs/specs/pulse-bus.md`.

pub mod application;
pub mod clipboard;
pub mod context;
pub mod effect;
pub mod event_sink;
pub mod input;
mod input_thread;
mod interpreter;
pub mod view_paint;

pub use application::Application;
pub use context::AppContext;
pub use effect::{Effect, EffectFn};
pub use event_sink::{EventSink, LoopMessage, PulseFn};
