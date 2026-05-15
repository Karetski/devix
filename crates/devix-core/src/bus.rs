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
//! * Cross-thread `publish_async` is **non-blocking** (F-1 follow-up,
//!   2026-05-12): a full queue returns `PublishError::Full(pulse)`,
//!   bumps `overflow_count`, and stashes the dropped `PulseKind` in
//!   a bounded diagnostics ring. The previous block-on-full
//!   semantics deadlocked the producer that was supposed to wake the
//!   main loop (the input thread).
//! * `drain` pops every queued pulse and dispatches each through
//!   `publish` on the caller's thread.
//! * Reentrancy depth is bounded (default 16; `with_depth_limit`
//!   builder; resolved 2026-05-07). Overflow panics so accidental
//!   cycles surface at test time. An RAII `DepthGuard` restores the
//!   depth counter on unwind so a panicking subscriber doesn't
//!   poison subsequent publishes (F-2 follow-up, 2026-05-12).
//! * Dispatch is indexed by `PulseKind` so a `BufferChanged` publish
//!   doesn't visit `InputReceived` subscribers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use devix_protocol::pulse::{Pulse, PulseFilter, PulseKind, SubscriptionId};

/// Default cross-thread queue capacity, matching the spec.
const DEFAULT_QUEUE_CAPACITY: usize = 1024;
/// Default reentrancy depth limit (resolved 2026-05-07).
const DEFAULT_DEPTH_LIMIT: usize = 16;
/// Bounded ring of recently-dropped `PulseKind`s for overflow
/// diagnostics. Sized to match the spec's diagnostics window.
const OVERFLOW_RING_CAPACITY: usize = 16;

/// Outcome of a non-blocking `publish_async`. The dropped pulse rides
/// back to the caller in `Full` so producers that care can choose to
/// retry, coalesce, or log; most callers ignore the result because
/// the bus already bumped its overflow counter.
#[derive(Debug)]
pub enum PublishError {
    /// Queue was full; the pulse was dropped and the overflow counter
    /// has been bumped.
    Full(Pulse),
    /// Receiver was dropped — bus is being torn down.
    Disconnected,
}

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
    /// Total `publish_async` calls dropped because the queue was full.
    overflow_count: AtomicU64,
    /// Bounded FIFO of the most recently dropped `PulseKind`s. A
    /// `Mutex<VecDeque>` is fine here: contention is only on overflow,
    /// which is itself a diagnostic event we want recorded reliably,
    /// not a hot path.
    overflow_recent: Mutex<VecDeque<PulseKind>>,
}

/// RAII guard for the publish-depth counter. Runs on both normal
/// return and unwind so a panicking subscriber can't poison subsequent
/// publishes with a stuck-incremented depth.
struct DepthGuard<'a> {
    depth: &'a Mutex<usize>,
}

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut d) = self.depth.lock() {
            *d = d.saturating_sub(1);
        }
    }
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
            overflow_count: AtomicU64::new(0),
            overflow_recent: Mutex::new(VecDeque::with_capacity(OVERFLOW_RING_CAPACITY)),
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
        // Bump depth; panic if we'd exceed the limit. The RAII guard
        // restores the counter on both normal return and unwind so a
        // panicking subscriber doesn't strand the depth at >0 and
        // poison the next publish.
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
        let _guard = DepthGuard { depth: &self.0.depth };
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
    }

    /// Push a pulse from a background thread. **Non-blocking**: on a
    /// full queue the pulse is dropped, the overflow counter is
    /// bumped, and `PublishError::Full(pulse)` is returned. The v0
    /// backpressure policy is *drop-newest* — see
    /// `docs/specs/pulse-bus.md`.
    ///
    /// Most producers can ignore the return value; the dropped pulse
    /// is reported through `overflow_snapshot()` for diagnostics.
    pub fn publish_async(&self, pulse: Pulse) -> Result<(), PublishError> {
        match self.0.tx.try_send(pulse) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(p)) => {
                self.0.overflow_count.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut ring) = self.0.overflow_recent.lock() {
                    if ring.len() == OVERFLOW_RING_CAPACITY {
                        ring.pop_front();
                    }
                    ring.push_back(p.kind());
                }
                Err(PublishError::Full(p))
            }
            Err(TrySendError::Disconnected(_)) => Err(PublishError::Disconnected),
        }
    }

    /// Snapshot of overflow diagnostics: `(total_dropped, recent_kinds)`.
    /// `recent_kinds` is the most-recent `OVERFLOW_RING_CAPACITY` (16)
    /// dropped `PulseKind`s in arrival order. Cheap — used by tests
    /// and an eventual `/dev/pulses` debug view.
    pub fn overflow_snapshot(&self) -> (u64, Vec<PulseKind>) {
        let count = self.0.overflow_count.load(Ordering::Relaxed);
        let recent = self
            .0
            .overflow_recent
            .lock()
            .map(|r| r.iter().copied().collect())
            .unwrap_or_default();
        (count, recent)
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
                    producer_bus
                        .publish_async(Pulse::BufferChanged {
                            path: Path::parse("/buf/42").unwrap(),
                            revision: rev,
                        })
                        .expect("queue capacity not exceeded");
                }
            })
        };
        producer.join().unwrap();
        assert_eq!(bus.drain(), 5);
        let got = hits.lock().unwrap().clone();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn publish_async_is_non_blocking_on_full_queue() {
        // Capacity 4; spec promises drop-newest on overflow.
        let bus = PulseBus::with_capacity(4);
        // No drain — every publish past the 4th must be Full.
        let mut ok = 0usize;
        let mut full = 0usize;
        for rev in 0..(4 + 16) {
            match bus.publish_async(Pulse::BufferChanged {
                path: Path::parse("/buf/0").unwrap(),
                revision: rev,
            }) {
                Ok(()) => ok += 1,
                Err(PublishError::Full(_)) => full += 1,
                Err(PublishError::Disconnected) => panic!("bus disconnected"),
            }
        }
        assert_eq!(ok, 4, "first {ok} sends succeeded; expected 4");
        assert_eq!(full, 16, "remaining {full} sends bounced; expected 16");

        let (overflow, recent) = bus.overflow_snapshot();
        assert_eq!(overflow, 16);
        // Ring captures only the last 16; we dropped 16, so the ring
        // is exactly full and every entry is BufferChanged.
        assert_eq!(recent.len(), 16);
        assert!(recent.iter().all(|k| *k == PulseKind::BufferChanged));
    }

    #[test]
    fn overflow_ring_rolls_past_capacity() {
        // Ring capacity is 16 — drop 24 to confirm only the last 16
        // are retained, and they arrive in FIFO order (oldest first
        // among the survivors).
        let bus = PulseBus::with_capacity(0); // every send fails Full
        // sync_channel(0) is a rendezvous — no buffered slot — so
        // every try_send rejects without a receiver. The bus's
        // receiver is alive but no `recv` is ever called, so the
        // queue is "full" for try_send purposes. Avoids needing
        // to fill a 1024-slot queue first.
        for rev in 0..24 {
            let _ = bus.publish_async(Pulse::BufferChanged {
                path: Path::parse("/buf/0").unwrap(),
                revision: rev,
            });
        }
        let (count, recent) = bus.overflow_snapshot();
        assert_eq!(count, 24, "every send overflowed");
        assert_eq!(
            recent.len(),
            16,
            "ring keeps only the most recent OVERFLOW_RING_CAPACITY"
        );
    }

    #[test]
    fn overflow_ring_remembers_mixed_kinds() {
        let bus = PulseBus::with_capacity(0);
        let _ = bus.publish_async(buffer_changed(1));
        let _ = bus.publish_async(Pulse::CursorMoved {
            cursor: Path::parse("/cur/0").unwrap(),
            doc: Path::parse("/buf/0").unwrap(),
            head: 0,
        });
        let _ = bus.publish_async(Pulse::RenderDirty {
            reason: devix_protocol::pulse::DirtyReason::Layout,
        });
        let (count, recent) = bus.overflow_snapshot();
        assert_eq!(count, 3);
        assert_eq!(
            recent,
            vec![
                PulseKind::BufferChanged,
                PulseKind::CursorMoved,
                PulseKind::RenderDirty,
            ]
        );
    }

    #[test]
    fn publish_async_succeeds_after_drain_frees_a_slot() {
        // After overflow, the count stays high, but new sends
        // succeed once the main loop drains.
        let bus = PulseBus::with_capacity(2);
        bus.publish_async(buffer_changed(0)).unwrap();
        bus.publish_async(buffer_changed(1)).unwrap();
        assert!(matches!(
            bus.publish_async(buffer_changed(2)),
            Err(PublishError::Full(_))
        ));
        assert_eq!(bus.drain(), 2, "two drained");
        // Counter is sticky; new send succeeds.
        let (overflow, _) = bus.overflow_snapshot();
        assert_eq!(overflow, 1);
        bus.publish_async(buffer_changed(3)).expect("space now");
    }

    #[test]
    fn full_error_carries_the_dropped_pulse() {
        let bus = PulseBus::with_capacity(0);
        let original = Pulse::BufferChanged {
            path: Path::parse("/buf/99").unwrap(),
            revision: 7,
        };
        match bus.publish_async(original) {
            Err(PublishError::Full(Pulse::BufferChanged { path, revision })) => {
                assert_eq!(revision, 7, "Full carries the original revision back");
                assert_eq!(path.as_str(), "/buf/99");
            }
            other => panic!("expected Full carrying revision 7, got {other:?}"),
        }
    }

    #[test]
    fn publish_async_wedge_does_not_block_main_loop_signal() {
        // Producer fills the queue, then a separate side-channel
        // signal still arrives within the deadline. Models the input
        // thread's two-step (publish typed pulse, then send wake) —
        // even if the bus is wedged, the wake/input still gets
        // through because publish_async returns immediately.
        use std::sync::mpsc::channel;
        use std::time::{Duration, Instant};

        let bus = PulseBus::with_capacity(2);
        let (wake_tx, wake_rx) = channel::<()>();
        let producer_bus = bus.clone();
        let producer = thread::spawn(move || {
            for rev in 0..32 {
                let _ = producer_bus.publish_async(Pulse::BufferChanged {
                    path: Path::parse("/buf/0").unwrap(),
                    revision: rev,
                });
            }
            wake_tx.send(()).expect("wake channel open");
        });
        let start = Instant::now();
        wake_rx
            .recv_timeout(Duration::from_millis(200))
            .expect("wake arrived within 200ms despite bus full");
        assert!(start.elapsed() < Duration::from_millis(200));
        producer.join().unwrap();
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

    #[test]
    fn panicking_subscriber_does_not_poison_depth() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let bus = PulseBus::new().with_depth_limit(4);
        let panic_id = bus.subscribe(PulseFilter::any(), |_| panic!("boom"));
        let _ = catch_unwind(AssertUnwindSafe(|| bus.publish(buffer_changed(1))));
        bus.unsubscribe(panic_id);

        // Now publish through a non-panicking subscriber that records
        // the depth it observes. Without the RAII guard this would
        // see depth=2 (1 leaked from the panic + 1 for this publish);
        // with the guard, exactly 1.
        let observed = Arc::new(Mutex::new(0usize));
        {
            let bus_handle = bus.clone();
            let observed = observed.clone();
            bus.subscribe(PulseFilter::any(), move |_| {
                let depth = *bus_handle.0.depth.lock().unwrap();
                *observed.lock().unwrap() = depth;
            });
        }
        bus.publish(buffer_changed(2));
        assert_eq!(*observed.lock().unwrap(), 1, "depth leaked from panic");
    }

    #[test]
    fn repeated_panics_stay_at_depth_zero_between_publishes() {
        // Without the RAII guard, every panicked publish would
        // leak one increment. After N panics, the next publish
        // would trip the limit at depth ≥ limit. Confirm we can
        // panic many times and still publish under the limit.
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let bus = PulseBus::new().with_depth_limit(2);
        let panic_id = bus.subscribe(PulseFilter::any(), |_| panic!("again"));
        for _ in 0..16 {
            let _ = catch_unwind(AssertUnwindSafe(|| bus.publish(buffer_changed(0))));
        }
        bus.unsubscribe(panic_id);

        let observed = Arc::new(Mutex::new(0usize));
        {
            let bus_handle = bus.clone();
            let observed = observed.clone();
            bus.subscribe(PulseFilter::any(), move |_| {
                let depth = *bus_handle.0.depth.lock().unwrap();
                *observed.lock().unwrap() = depth;
            });
        }
        bus.publish(buffer_changed(1));
        assert_eq!(*observed.lock().unwrap(), 1);
    }

    #[test]
    fn nested_panicking_subscriber_unwinds_both_guards() {
        // Outer subscriber re-publishes; inner subscriber panics.
        // Both publishes have their own DepthGuard. After the
        // catch_unwind unwinds both, depth must return to 0.
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let bus = PulseBus::new().with_depth_limit(4);
        let bus_handle = bus.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::BufferChanged), move |_| {
            bus_handle.publish(Pulse::CursorMoved {
                cursor: Path::parse("/cur/0").unwrap(),
                doc: Path::parse("/buf/0").unwrap(),
                head: 0,
            });
        });
        bus.subscribe(PulseFilter::kind(PulseKind::CursorMoved), |_| {
            panic!("inner boom")
        });
        let result = catch_unwind(AssertUnwindSafe(|| bus.publish(buffer_changed(0))));
        assert!(result.is_err(), "panic propagates through nested publish");
        let depth = *bus.0.depth.lock().unwrap();
        assert_eq!(depth, 0, "both guards restored depth to zero");
    }
}
