//! devix runtime — five named primitives plus the binary glue.
//!
//! - [`Application`] owns the run loop, the terminal, and every long-lived
//!   resource by direct field.
//! - [`Service`] is a long-lived background subsystem (input reader, disk
//!   watcher, plugin host, …).
//! - [`Pulse`] is a typed message a service pushes into the loop.
//! - [`Effect`] is runtime-internal deferred work drained between
//!   messages.
//! - [`AppContext`] is the unified `&mut` surface threaded through every
//!   delivery.
//!
//! `EventSink` is the cross-thread handle services hold to push pulses
//! back into the loop. See `RUNTIME-SPEC.md` at the repo root for the
//! full design rationale.

pub mod application;
pub mod clipboard;
pub mod context;
pub mod effect;
pub mod event_sink;
pub mod events;
pub mod pulse;
mod render;
pub mod service;
pub mod services;

pub use application::Application;
pub use context::AppContext;
pub use effect::{Effect, EffectFn};
pub use event_sink::{EventSink, LoopMessage};
pub use pulse::{DiskChanged, PluginEmitted, Pulse, ScrollAccumulated};
pub use service::Service;
pub use services::input::InputService;
pub use services::plugin::PluginService;
