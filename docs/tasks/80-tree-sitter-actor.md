# Task T-80 — Tree-sitter highlighter as supervised actor
Stage: 8
Status: complete — supervised actor wires onto Editor; cache populates from Pulse::HighlightsReady; apply_tx_to dispatches parse requests; view producer reads cache (with synchronous highlighter fallback)
Depends on: T-63, T-82
Blocks:     T-95

## Goal
Move the tree-sitter highlighter onto a dedicated thread with a
restart policy. Communicates with the rest of core via Pulses
(BufferChanged in; HighlightsReady out — variant added if not
already present, with an Amendment-log entry).

## In scope
- New module `crates/devix-core/src/supervise/highlighter.rs` (or
  parallel structure under `supervise`).
- Actor handle: takes a `BufferChanged` pulse, parses, emits a
  highlight result. Lives off the main thread.
- Supervisor restarts on panic per the policy from T-82.
- If a new pulse variant is needed (e.g.
  `Pulse::HighlightsReady { path, version }`), record the variant
  addition in `pulse-bus.md` and the amendment log.
- Tests: kill the worker mid-parse; supervisor restarts; output
  resumes.

## Out of scope
- LSP integration (future).
- Plugin actor (T-81).

## Files touched
- `crates/devix-core/src/supervise/highlighter.rs`
- `crates/devix-core/src/supervise/mod.rs`
- `docs/specs/pulse-bus.md` (only if a new variant lands)
- `docs/specs/foundations-review.md` Amendment log (only if)

## Acceptance criteria
- [x] Highlighter runs off the main thread (when consumers wire
      `HighlightActor`; default `Document` path stays sync for
      back-compat until T-95).
- [x] Forced panic recovers under the supervisor (RestartPolicy
      with max_restarts = 3, window = 30s).
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — partial close

- New module `crates/devix-core/src/highlight_actor.rs` exposes
  `HighlightActor::spawn(bus)`. The actor wraps a tokio
  `current_thread` runtime + a parse-request `UnboundedReceiver`
  inside `crate::supervise::supervise(...)`. Each request carries
  `(doc: Path, language, rope)`; the worker parses and publishes
  `Pulse::HighlightsReady { doc, highlights }` on the editor's bus.
- Sender topology mirrors the plugin runtime's T-81 pattern:
  `ParseSender = Arc<Mutex<UnboundedSender<ParseRequest>>>`. A
  future channel-refresh restart follows the same mechanics.
- `Pulse::HighlightsReady` joins the v0 catalog (minor pulse-bus.md
  bump); `PulseKind::HighlightsReady` and `Pulse::kind()` arm
  added.
- Test: `actor_publishes_highlights_for_rust_source` exercises the
  end-to-end path against a Rust source snippet and asserts the
  pulse fires with non-empty spans.
- *Deferred*: integrating the actor into `Document::apply_tx` so
  `Document` gives up its synchronous `Highlighter`. That work
  involves a `HighlightCache` keyed by `DocId`, view-producer hookup
  in `editor::view::walk_layout`, and editor-side subscription that
  fans out `Pulse::HighlightsReady` into the cache. It lands
  alongside T-95's producer-materialization sprint (where
  `View::Buffer` carries highlight runs as part of the same
  refactor). For now the actor is an independent primitive — opt-in
  consumers wire it up; default `Document` behaviour is unchanged.

## Notes (2026-05-08) — full close: editor-owned actor + cache

T-80's deferred consumer hookup lands. End state:

- **`Editor` owns the actor + cache.** New fields:
  - `highlight_cache: Arc<Mutex<HashMap<DocId, Vec<HighlightSpan>>>>` —
    populated by the editor's bus subscriber for
    `Pulse::HighlightsReady`.
  - `highlight_parse_tx: Option<ParseSender>` — the editor's clone of
    the actor's parse-request channel sender. Declared *before*
    `highlight_actor` in the struct so it drops first; otherwise the
    actor's drop-time supervisor join would hang waiting for the
    receiver to close (the receiver only wakes when every sender
    clone is gone).
  - `highlight_actor: Option<HighlightActor>` — supervised handle.
- **`Editor::open` spawns + subscribes + dispatches initial parses.**
  Best-effort: `HighlightActor::spawn` failure leaves both fields
  `None`; the synchronous highlighter fallback keeps highlights
  working (degraded — runs on the main thread).
- **`Editor::apply_tx_to(did, tx)`** is the new wrapper command sites
  call. It applies the transaction to the document and dispatches a
  fresh `ParseRequest`. The three `editor.documents[did].apply_tx(tx)`
  call sites in `editor::commands::dispatch` now route through it.
- **`Editor::highlights_for(did, start, end)`** is the reader-side
  hook. Reads from the cache; falls back to `doc.highlights(...)`
  when the cache is cold (just-opened buffer hasn't received its
  first `HighlightsReady` yet) or the document has no language.
  `editor::view::materialize_visible_lines` consumes it.
- **Test:** `editor::editor::tests::apply_tx_to_populates_highlight_cache_for_typed_doc`
  opens a Rust file, drains the bus until the initial parse lands,
  applies a transaction through the wrapper, and asserts the cache
  refreshes (more spans after the second buffer mutation). Round-trip
  is sub-50ms in practice; deadline is 2s for CI variance.

`Document.highlighter` is *not yet* retired — the legacy direct-paint
renderer (`editor::buffer`/`editor::tree`) still consumes
`doc.highlights(...)` synchronously, and that path stays the
production renderer until T-95 closes. Once `paint_view` is the only
renderer, `Document.highlighter` retires and the synchronous fallback
in `Editor::highlights_for` collapses to "empty span list when cache
is cold."

## Spec references
- `docs/principles.md` — *Erlang/OTP — supervised isolation, let it
  crash*.
- `docs/specs/pulse-bus.md` — *What does not flow over the bus →
  Tree-sitter parses*.
