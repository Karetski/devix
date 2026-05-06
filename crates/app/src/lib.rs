//! devix runtime — three named primitives plus the binary glue.
//!
//! - [`Application`] owns the run loop, the terminal, and every long-lived
//!   resource by direct field.
//! - [`Effect`] is runtime-internal deferred work drained between
//!   messages.
//! - [`AppContext`] is the unified `&mut` surface threaded through every
//!   delivery.
//!
//! `EventSink` is the cross-thread handle producers hold; cross-thread
//! messages are boxed `FnOnce(&mut AppContext)` closures — there is no
//! typed pulse trait, no `Service` trait, no god-set of message structs.
//! See `RUNTIME-SPEC.md` and `LAYERING-NOTE.md` at the repo root for
//! design rationale.

pub mod application;
pub mod clipboard;
pub mod context;
pub mod effect;
pub mod event_sink;
pub mod events;
mod input;
mod render;

pub use application::Application;
pub use context::AppContext;
pub use effect::{Effect, EffectFn};
pub use event_sink::{EventSink, LoopMessage, PulseFn};
