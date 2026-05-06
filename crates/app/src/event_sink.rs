//! `EventSink` and `LoopMessage` — the cross-thread handle that services
//! use to push messages back to the run loop.
//!
//! Producers (the input thread, the plugin worker, future LSP transports)
//! receive a clone of `EventSink` from `Service::start`; the run loop owns
//! the receiver. The channel is `std::sync::mpsc::sync_channel` with a
//! finite capacity so a slow main loop applies backpressure on the
//! producer instead of letting messages pile up unbounded.

use std::sync::mpsc::{Receiver, SendError, SyncSender, sync_channel};

use crate::pulse::Pulse;

/// Run-loop channel capacity. Producers block once the run loop is this
/// far behind — backpressure rather than unbounded growth.
const CHANNEL_CAPACITY: usize = 1024;

/// One thing the run loop wakes up to handle.
pub enum LoopMessage {
    Input(crossterm::event::Event),
    Pulse(Box<dyn Pulse>),
    Quit,
}

/// Cross-thread wake handle. Cheap to clone; cloned once per `Service` at
/// `start()`.
#[derive(Clone)]
pub struct EventSink(pub(crate) SyncSender<LoopMessage>);

impl EventSink {
    /// Build a fresh `(EventSink, Receiver<LoopMessage>)` pair. The
    /// binary creates one of these up-front so it can wire the sink
    /// into the editor's disk-watch callback, the plugin runtime's
    /// message sink, and any other producer *before* the run loop
    /// starts.
    pub fn channel() -> (Self, Receiver<LoopMessage>) {
        let (tx, rx) = sync_channel::<LoopMessage>(CHANNEL_CAPACITY);
        (Self(tx), rx)
    }

    /// Push a terminal input event into the run loop. Returns `Err` only
    /// when the receiver has been dropped — i.e. the run loop has exited.
    pub fn input(&self, ev: crossterm::event::Event) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Input(ev))
    }

    /// Push a typed pulse into the run loop.
    pub fn pulse<P: Pulse>(&self, p: P) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Pulse(Box::new(p)))
    }

    /// Ask the run loop to exit from outside the main thread (signal
    /// handlers, test harness). In-loop quit goes through `Effect::Quit`
    /// via `AppContext::quit`.
    pub fn quit(&self) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Quit)
    }
}
