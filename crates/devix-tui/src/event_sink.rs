//! `EventSink` and `LoopMessage` — the cross-thread handle the input
//! thread uses to push terminal events into the run loop.
//!
//! T-63 retired the closure-as-message variant (`LoopMessage::Pulse`)
//! and the producer-side `EventSink::pulse(closure)` API; cross-thread
//! event flow goes through the `PulseBus` (typed pulses, see
//! `docs/specs/pulse-bus.md`) for everything *except* terminal input,
//! which keeps a dedicated channel because the input thread reads
//! crossterm events that aren't a `Pulse` shape.
//!
//! The channel is `std::sync::mpsc::sync_channel` with a finite capacity
//! so a slow main loop applies backpressure on the input thread rather
//! than letting events pile up unbounded.

use std::sync::mpsc::{Receiver, SendError, SyncSender, sync_channel};

/// Run-loop channel capacity. The input thread blocks once the run
/// loop is this far behind — backpressure rather than unbounded
/// growth.
const CHANNEL_CAPACITY: usize = 1024;

/// One thing the run loop wakes up to handle. Just terminal input,
/// out-of-band quit (signal handlers, tests), and a no-op wake
/// signal cross-thread bus producers send to nudge the loop into
/// draining the bus.
pub enum LoopMessage {
    Input(crossterm::event::Event),
    /// No payload — the loop sees this, drains the bus, and
    /// continues. Producers (notably the plugin worker) call
    /// `EventSink::wake` after `bus.publish_async` so the main
    /// loop's `rx.recv` unblocks even when no terminal input
    /// arrives.
    Wake,
    Quit,
}

/// Cross-thread wake handle. Cheap to clone.
#[derive(Clone)]
pub struct EventSink(pub(crate) SyncSender<LoopMessage>);

impl EventSink {
    /// Build a fresh `(EventSink, Receiver<LoopMessage>)` pair.
    pub fn channel() -> (Self, Receiver<LoopMessage>) {
        let (tx, rx) = sync_channel::<LoopMessage>(CHANNEL_CAPACITY);
        (Self(tx), rx)
    }

    /// Push a terminal input event into the run loop. Returns `Err` only
    /// when the receiver has been dropped — i.e. the run loop has exited.
    pub fn input(&self, ev: crossterm::event::Event) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Input(ev))
    }

    /// No-op wake signal. Cross-thread producers (e.g. plugin worker
    /// publishing typed pulses onto `Editor.bus`) call this after a
    /// publish so the main loop's `rx.recv` unblocks and drains the
    /// bus on the next tick.
    pub fn wake(&self) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Wake)
    }

    /// Ask the run loop to exit from outside the main thread (signal
    /// handlers, test harness). In-loop quit flips
    /// `AppContext::quit_request` directly.
    pub fn quit(&self) -> Result<(), SendError<LoopMessage>> {
        self.0.send(LoopMessage::Quit)
    }
}
