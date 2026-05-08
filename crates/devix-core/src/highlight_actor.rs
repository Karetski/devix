//! Supervised tree-sitter highlighter (T-80).
//!
//! A long-running worker thread takes parse requests over a channel,
//! re-parses against a `Highlighter`, and publishes
//! `Pulse::HighlightsReady { doc, highlights }` so view producers can
//! consume the result without holding the editor's main thread.
//! The worker is wrapped in [`supervise`](crate::supervise::supervise)
//! with a real restart budget — a panic inside `tree_sitter` escalates
//! as `Pulse::PluginError` and the supervisor respawns up to
//! `max_restarts` times before giving up.
//!
//! Consumer integration (a `HighlightCache` keyed by `DocId` that
//! `Editor::view`'s `walk_layout` reads) lands together with T-95's
//! producer materialization. This module ships the supervised worker
//! + the request channel; the cache + view-producer hook-up is the
//! follow-on.
//!
//! ## Channel topology
//!
//! Editor-held senders are [`ParseSender`]
//! (`Arc<Mutex<UnboundedSender<ParseRequest>>>`) — same shape as the
//! plugin runtime's [`super::plugin::InvokeSender`] so a future
//! channel-refresh restart of this actor follows the established
//! pattern.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use devix_protocol::path::Path;
use devix_protocol::pulse::Pulse;

use devix_syntax::{HighlightSpan, Highlighter, Language};

use crate::supervise::{RestartPolicy, SupervisedChild, supervise};
use crate::PulseBus;

/// Editor-held handle for the highlighter's parse-request channel.
/// Wrapped in `Arc<Mutex<…>>` so a future channel-refresh restart
/// can swap in a fresh sender without rewiring captured callers.
pub type ParseSender = Arc<Mutex<UnboundedSender<ParseRequest>>>;

/// One parse request the highlighter actor consumes. Producers send
/// these from any thread; the worker drains them serially.
pub struct ParseRequest {
    /// Path of the buffer the spans are for. Echoed back on
    /// `Pulse::HighlightsReady.doc` so subscribers can route results
    /// to the right cache entry.
    pub doc: Path,
    /// Language to parse with — the actor builds a fresh
    /// `Highlighter` per request rather than caching one per
    /// document, so no cross-request state survives a panic.
    pub language: Language,
    /// Snapshot of the buffer's text. Cloning a `Rope` is O(log n)
    /// and shares storage; we hand the actor a snapshot so the editor
    /// can keep mutating `buffer` without a lock-step stall.
    pub rope: ropey::Rope,
}

/// Handle for the supervised highlighter actor. Holding the handle
/// keeps the worker alive; `Drop` joins it.
pub struct HighlightActor {
    sender: ParseSender,
    #[allow(dead_code)]
    supervised: Option<SupervisedChild>,
}

impl HighlightActor {
    /// Spawn a supervised highlighter actor on `bus`. Returns the
    /// handle the editor keeps; `parse_sender()` exposes the channel
    /// producers send `ParseRequest`s through.
    pub fn spawn(bus: PulseBus) -> std::io::Result<Self> {
        let (tx, rx) = unbounded_channel::<ParseRequest>();
        let sender: ParseSender = Arc::new(Mutex::new(tx));

        let bus_for_factory = bus.clone();
        let mut rx_holder: Option<tokio::sync::mpsc::UnboundedReceiver<ParseRequest>> =
            Some(rx);
        let factory = move || {
            // Single-shot factory state: if the actor restarts after a
            // panic, the receiver was already moved into the previous
            // run. Subsequent calls noop — we deliberately don't try
            // to recover mid-parse state. (Channel-refresh restart is
            // the next sprint, mirroring T-81.)
            let Some(rx) = rx_holder.take() else { return };
            let bus_local = bus_for_factory.clone();
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return,
            };
            runtime.block_on(async move {
                let mut rx = rx;
                while let Some(req) = rx.recv().await {
                    let highlights = parse_one(req.language, &req.rope);
                    bus_local.publish(Pulse::HighlightsReady {
                        doc: req.doc,
                        highlights,
                    });
                }
            });
        };

        // Real restart budget — a tree-sitter panic is rare but not
        // unheard-of (queries can stack-overflow on adversarial input);
        // surviving a few crashes keeps the editor lit.
        let policy = RestartPolicy {
            max_restarts: 3,
            window: Duration::from_secs(30),
        };
        let supervised = supervise("highlighter", bus, policy, factory)
            .context("spawning supervised highlighter actor")
            .map_err(std::io::Error::other)?;

        Ok(Self {
            sender,
            supervised: Some(supervised),
        })
    }

    /// Sender clone for callers (`Document::apply_tx` follow-up,
    /// integration tests). Producers wrap in [`send_parse`] for the
    /// silent-no-op-on-poison Erlang semantics.
    pub fn parse_sender(&self) -> ParseSender {
        self.sender.clone()
    }
}

/// Push a `ParseRequest` through `sender`. Silent no-op on poisoned
/// lock or closed receiver — same Erlang shape as the plugin
/// runtime's `send_invoke`.
pub fn send_parse(sender: &ParseSender, req: ParseRequest) -> bool {
    match sender.lock() {
        Ok(tx) => tx.send(req).is_ok(),
        Err(_) => false,
    }
}

fn parse_one(language: Language, rope: &ropey::Rope) -> Vec<HighlightSpan> {
    // A panic inside `Highlighter::new` / `parse` / `highlights`
    // would crash the worker thread, which is exactly what the
    // supervisor catches. We don't catch_unwind here — the
    // supervisor's wrapper is the recovery boundary.
    let Ok(mut h) = Highlighter::new(language) else {
        return Vec::new();
    };
    h.parse(rope);
    let len = rope.len_bytes();
    h.highlights(rope, 0, len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use devix_protocol::pulse::{PulseFilter, PulseKind};

    #[test]
    fn actor_publishes_highlights_for_rust_source() {
        let bus = PulseBus::new();
        let captured = Arc::new(Mutex::new(Vec::<Pulse>::new()));
        let cap = captured.clone();
        bus.subscribe(PulseFilter::kind(PulseKind::HighlightsReady), move |p| {
            cap.lock().unwrap().push(p.clone());
        });

        let actor = HighlightActor::spawn(bus.clone()).unwrap();
        let mut rope = ropey::Rope::new();
        rope.insert(0, "fn main() { let x = 1; }");
        let path = Path::parse("/buf/1").unwrap();
        send_parse(
            &actor.parse_sender(),
            ParseRequest {
                doc: path.clone(),
                language: Language::Rust,
                rope,
            },
        );

        // Spin until the subscriber sees the result. With a working
        // tree-sitter `Rust` query, parse + highlights take < 5ms;
        // give the actor up to 2s before declaring failure.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            bus.drain();
            if !captured.lock().unwrap().is_empty() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("highlighter actor did not publish HighlightsReady");
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let pulses = captured.lock().unwrap();
        match &pulses[0] {
            Pulse::HighlightsReady { doc, highlights } => {
                assert_eq!(doc.as_str(), "/buf/1");
                assert!(!highlights.is_empty(), "rust source produced no highlights");
            }
            other => panic!("unexpected pulse: {other:?}"),
        }
    }
}
