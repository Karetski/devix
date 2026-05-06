//! `Service` — a long-lived background subsystem.
//!
//! Each subsystem that owns a thread (or executor) and pushes events back
//! through the run loop is one `impl Service`: today's input reader, disk
//! watcher, and plugin host; tomorrow's LSP and DAP clients.

use std::time::Duration;

use anyhow::Result;

use crate::event_sink::EventSink;

pub trait Service: Send + 'static {
    fn name(&self) -> &'static str;

    /// Take ownership of background resources (spawn threads, register
    /// watchers, open sockets). Called once on the main thread before the
    /// run loop begins. Producers stash a clone of `sink` and use it to
    /// push pulses back into the loop.
    fn start(&mut self, sink: EventSink) -> Result<()>;

    /// Best-effort shutdown. The runtime calls this once after the loop
    /// exits, in service-registration order. Implementations should signal
    /// their threads to exit and wait at most `deadline` before giving up.
    #[allow(unused_variables)]
    fn stop(self: Box<Self>, deadline: Duration) {}
}
