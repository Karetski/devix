//! `EventSink` and `LoopMessage` — the cross-thread handle producers use
//! to push messages back to the run loop.
//!
//! There is exactly one cross-thread message kind: "do this on the main
//! thread", carried as a boxed `FnOnce(&mut AppContext)`. Producers stay
//! in their own crates and import only `EventSink` and the closure they
//! send; there is no central `Pulse` trait or god-set of typed pulse
//! structs that the runtime must know about.
//!
//! The channel is `std::sync::mpsc::sync_channel` with a finite capacity
//! so a slow main loop applies backpressure on the producer rather than
//! letting messages pile up unbounded.

use std::sync::mpsc::{Receiver, SendError, SyncSender, sync_channel};

use crate::context::AppContext;

/// Run-loop channel capacity. Producers block once the run loop is this
/// far behind — backpressure rather than unbounded growth.
const CHANNEL_CAPACITY: usize = 1024;

/// Boxed closure that the run loop invokes against an `AppContext`. The
/// HRTB on the lifetime lets the closure accept whatever lifetime the
/// context is reborrowed at on the delivery iteration.
pub type PulseFn = Box<dyn for<'a> FnOnce(&mut AppContext<'a>) + Send>;

/// One thing the run loop wakes up to handle.
pub enum LoopMessage {
    Input(crossterm::event::Event),
    Pulse(PulseFn),
    Quit,
}

/// Cross-thread wake handle. Cheap to clone.
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

    /// Push a closure to run on the main thread against `AppContext`.
    pub fn pulse<F>(&self, f: F) -> Result<(), SendError<LoopMessage>>
    where
        F: for<'a> FnOnce(&mut AppContext<'a>) + Send + 'static,
    {
        self.0.send(LoopMessage::Pulse(Box::new(f)))
    }

    /// Ask the run loop to exit from outside the main thread (signal
    /// handlers, test harness). In-loop quit goes through `Effect::Quit`
    /// via `AppContext::quit`.
    pub fn quit(&self) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Quit)
    }
}
