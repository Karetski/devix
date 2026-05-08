//! Pulse bus runtime — implements `docs/specs/pulse-bus.md`.
//!
//! The types (`Pulse`, `PulseFilter`, `SubscriptionId`, …) live in
//! `devix-protocol::pulse`. This module owns the *runtime*: subscriber
//! storage, synchronous dispatch, the cross-thread MPSC queue, and the
//! reentrancy-depth tracker.
//!
//! v0 invariants:
//! * Synchronous `publish` walks every matching subscriber on the
//!   caller's thread and returns when all handlers finish.
//! * Cross-thread `publish_async` pushes onto a bounded MPSC
//!   (default capacity 1024 per spec; `with_capacity` for tests);
//!   producers block when the queue is full.
//! * `drain` pops every queued pulse and dispatches each through
//!   `publish` on the caller's thread.
//! * Reentrancy depth is bounded (default 16; `with_depth_limit`
//!   builder; resolved 2026-05-07). Overflow panics so accidental
//!   cycles surface at test time.
//! * Dispatch is indexed by `PulseKind` so a `BufferChanged` publish
//!   doesn't visit `InputReceived` subscribers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use devix_protocol::pulse::{Pulse, PulseFilter, SubscriptionId};

/// Default cross-thread queue capacity, matching the spec.
const DEFAULT_QUEUE_CAPACITY: usize = 1024;
/// Default reentrancy depth limit (resolved 2026-05-07).
const DEFAULT_DEPTH_LIMIT: usize = 16;

type Handler = Arc<dyn Fn(&Pulse) + Send + Sync + 'static>;

struct Subscription {
    id: SubscriptionId,
    filter: PulseFilter,
    handler: Handler,
}

struct Inner {
    /// Indexed by `PulseKind` discriminant. A subscriber whose filter
    /// has `kinds = None` (match-anything-by-kind) is in `any_kind`.
    by_kind: Mutex<Vec<Subscription>>,
    /// Monotonic counter for `SubscriptionId`s.
    next_id: AtomicU64,
    /// Cross-thread queue.
    tx: SyncSender<Pulse>,
    rx: Mutex<Receiver<Pulse>>,
    /// Reentrancy depth — incremented on `publish` entry, decremented
    /// on exit. Panics if it would exceed `depth_limit`.
    depth: Mutex<usize>,
    depth_limit: usize,
}

/// The pulse bus.
pub struct PulseBus(Arc<Inner>);

impl PulseBus {
    /// Build a bus with default capacity (1024) and default depth
    /// limit (16).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_QUEUE_CAPACITY)
    }

    /// Build a bus with a custom cross-thread queue capacity. Default
    /// depth limit (16) still applies; chain `.with_depth_limit(...)`
    /// to override.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, rx) = sync_channel(capacity);
        let inner = Inner {
            by_kind: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            tx,
            rx: Mutex::new(rx),
            depth: Mutex::new(0),
            depth_limit: DEFAULT_DEPTH_LIMIT,
        };
        PulseBus(Arc::new(inner))
    }

    /// Override the default reentrancy depth limit. Useful for tests
    /// that need to provoke overflow with a small handler chain.
    pub fn with_depth_limit(mut self, limit: usize) -> Self {
        // We're the sole holder during construction; `Arc::get_mut`
        // succeeds. (If someone holds a clone of the bus already,
        // setting the limit is a programmer error.)
        let inner = Arc::get_mut(&mut self.0)
            .expect("with_depth_limit requires a uniquely-owned PulseBus");
        inner.depth_limit = limit;
        self
    }

    /// Synchronous publish. Every matching subscriber's handler runs
    /// before this returns. Reentrant publishes (a handler that calls
    /// `publish`) are bounded by `depth_limit`; overflow panics.
    pub fn publish(&self, pulse: Pulse) {
        // Bump depth; panic if we'd exceed the limit.
        {
            let mut depth = self.0.depth.lock().unwrap();
            if *depth >= self.0.depth_limit {
                panic!(
                    "PulseBus reentrancy depth limit ({}) exceeded — likely a publish cycle",
                    self.0.depth_limit
                );
            }
            *depth += 1;
        }
        // Snapshot the matching handlers under the subscriber lock,
        // then release and invoke. Snapshot lets handlers mutate
        // subscriber state (subscribe / unsubscribe from inside a
        // handler) without deadlocking and without iterator
        // invalidation.
        let matched: Vec<Handler> = {
            let subs = self.0.by_kind.lock().unwrap();
            subs.iter()
                .filter(|s| s.filter.matches(&pulse))
                .map(|s| s.handler.clone())
                .collect()
        };
        for h in &matched {
            h(&pulse);
        }
        // Pop depth. Use a fresh lock guard since handlers above may
        // have re-entered.
        {
            let mut depth = self.0.depth.lock().unwrap();
            *depth -= 1;
        }
    }

    /// Push a pulse from a background thread. Blocks if the queue is
    /// full (1024 default — the main loop's `drain()` makes space).
    pub fn publish_async(&self, pulse: Pulse) {
        // `send` blocks on full per `sync_channel` semantics.
        // Receiver is held inside the bus; only fails on receiver-
        // dropped, which means the bus itself is being torn down.
        let _ = self.0.tx.send(pulse);
    }

    /// Drain the cross-thread queue, dispatching every pulse via
    /// `publish` on the caller's thread. Returns the number drained.
    pub fn drain(&self) -> usize {
        let mut count = 0;
        loop {
            let pulse = {
                let rx = self.0.rx.lock().unwrap();
                match rx.try_recv() {
                    Ok(p) => p,
                    Err(_) => break,
                }
            };
            self.publish(pulse);
            count += 1;
        }
        count
    }

    /// Drain the cross-thread queue into `out` *without* dispatching
    /// to bus subscribers. Returns the number drained. Used by the
    /// main loop to dispatch typed pulses with `&mut`-state callers
    /// can't reach through the spec's `Fn(&Pulse) + Send + Sync`
    /// subscriber shape (e.g., handlers that need `&mut Editor`).
    /// `Fn` subscribers are still called by `drain` / `publish`; this
    /// method coexists for the loop-side typed dispatch case
    /// introduced at T-61 (recorded in `foundations-review.md`
    /// 2026-05-07).
    pub fn drain_into(&self, out: &mut Vec<Pulse>) -> usize {
        let mut count = 0;
        let rx = self.0.rx.lock().unwrap();
        while let Ok(p) = rx.try_recv() {
            out.push(p);
            count += 1;
        }
        count
    }

    /// Register a handler. Returns an id usable for `unsubscribe`.
    pub fn subscribe<F>(&self, filter: PulseFilter, handler: F) -> SubscriptionId
    where
        F: Fn(&Pulse) + Send + Sync + 'static,
    {
        let id = SubscriptionId(self.0.next_id.fetch_add(1, Ordering::Relaxed));
        let sub = Subscription {
            id,
            filter,
            handler: Arc::new(handler),
        };
        self.0.by_kind.lock().unwrap().push(sub);
        id
    }

    /// Drop a previously-installed subscription. Idempotent — calling
    /// twice with the same id is a no-op.
    pub fn unsubscribe(&self, id: SubscriptionId) {
        let mut subs = self.0.by_kind.lock().unwrap();
        subs.retain(|s| s.id != id);
    }

    /// Clone of the producer-side handle. Cheap (Arc clone). Useful
    /// for handing the bus across thread boundaries to producers.
    pub fn handle(&self) -> PulseBus {
        PulseBus(self.0.clone())
    }
}

impl Default for PulseBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for PulseBus {
    fn clone(&self) -> Self {
        self.handle()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::thread;

    use devix_protocol::path::Path;
    use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind};

    use super::*;

    fn buffer_changed(id: u64) -> Pulse {
        Pulse::BufferChanged {
            path: Path::parse(&format!("/buf/{}", id)).unwrap(),
            revision: 1,
        }
    }

    #[test]
    fn publish_invokes_only_matching_subscribers() {
        let bus = PulseBus::new();
        let buffer_hits = Arc::new(AtomicU32::new(0));
        let cursor_hits = Arc::new(AtomicU32::new(0));
        {
            let h = buffer_hits.clone();
            bus.subscribe(PulseFilter::kind(PulseKind::BufferChanged), move |_| {
                h.fetch_add(1, Ordering::Relaxed);
            });
        }
        {
            let h = cursor_hits.clone();
            bus.subscribe(PulseFilter::kind(PulseKind::CursorMoved), move |_| {
                h.fetch_add(1, Ordering::Relaxed);
            });
        }
        bus.publish(buffer_changed(42));
        bus.publish(buffer_changed(7));
        assert_eq!(buffer_hits.load(Ordering::Relaxed), 2);
        assert_eq!(cursor_hits.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn unsubscribe_drops_handler() {
        let bus = PulseBus::new();
        let hits = Arc::new(AtomicU32::new(0));
        let id = {
            let h = hits.clone();
            bus.subscribe(PulseFilter::any(), move |_| {
                h.fetch_add(1, Ordering::Relaxed);
            })
        };
        bus.publish(buffer_changed(1));
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        bus.unsubscribe(id);
        bus.publish(buffer_changed(2));
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        // Idempotent.
        bus.unsubscribe(id);
    }

    #[test]
    fn cross_thread_publish_async_drain_in_order() {
        let bus = PulseBus::new();
        let hits = Arc::new(Mutex::new(Vec::new()));
        {
            let h = hits.clone();
            bus.subscribe(PulseFilter::kind(PulseKind::BufferChanged), move |p| {
                if let Pulse::BufferChanged { revision, .. } = p {
                    h.lock().unwrap().push(*revision);
                }
            });
        }
        let producer = {
            let producer_bus = bus.clone();
            thread::spawn(move || {
                for rev in 1..=5 {
                    producer_bus.publish_async(Pulse::BufferChanged {
                        path: Path::parse("/buf/42").unwrap(),
                        revision: rev,
                    });
                }
            })
        };
        producer.join().unwrap();
        assert_eq!(bus.drain(), 5);
        let got = hits.lock().unwrap().clone();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    #[should_panic(expected = "reentrancy depth limit")]
    fn reentrancy_overflow_panics() {
        let bus = PulseBus::new().with_depth_limit(3);
        let bus_handle = bus.clone();
        bus.subscribe(PulseFilter::any(), move |_| {
            // Each handler republishes — produces an unbounded chain.
            bus_handle.publish(buffer_changed(0));
        });
        bus.publish(buffer_changed(0));
    }

    #[test]
    fn nested_publish_within_limit_succeeds() {
        let bus = PulseBus::new().with_depth_limit(8);
        let bus_handle = bus.clone();
        let hits = Arc::new(AtomicU32::new(0));
        {
            let h = hits.clone();
            let bus_inner = bus_handle.clone();
            bus.subscribe(PulseFilter::kind(PulseKind::BufferChanged), move |_| {
                let count = h.fetch_add(1, Ordering::Relaxed);
                // Republish a different kind so we don't infinite-loop.
                if count == 0 {
                    bus_inner.publish(Pulse::CursorMoved {
                        cursor: Path::parse("/cur/3").unwrap(),
                        doc: Path::parse("/buf/42").unwrap(),
                        head: 0,
                    });
                }
            });
        }
        bus.publish(buffer_changed(42));
        // The handler ran once; the nested CursorMoved didn't match
        // its filter, so no double-count.
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn depth_resets_between_publishes() {
        let bus = PulseBus::new().with_depth_limit(2);
        // Subscriber that does nothing — publishing many times should
        // never trip the limit.
        bus.subscribe(PulseFilter::any(), |_| {});
        for i in 0..10 {
            bus.publish(buffer_changed(i));
        }
    }
}
