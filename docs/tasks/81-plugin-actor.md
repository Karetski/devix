# Task T-81 — Plugin runtime as supervised actor
Stage: 8
Status: partial — supervisor wraps plugin thread (escalate-only, no restart); module reorg deferred
Depends on: T-82
Blocks:     T-95

## Goal
Lift `PluginRuntime` / `PluginHost` off the main thread under a
supervisor with a restart policy. Plugin Lua execution still runs
on a dedicated thread (today's `PluginRuntime` shape stays); the
supervisor wraps it with crash recovery.

## In scope
- Move `crates/devix-core/src/plugin/mod.rs` content into
  `host.rs` (Lua VM), `runtime.rs` (thread + channels),
  `bridge.rs` (Lua ↔ Pulse marshaling), `pane_handle.rs`
  (LuaPaneHandle) per `crates.md` *crates/plugin/src/*.
- Supervisor restarts the plugin runtime on panic.
- Plugin output continues to flow through the bus via
  `publish_async`.
- Tests: forced plugin panic doesn't kill the editor; the
  supervisor restarts the runtime; `Pulse::PluginError` is published.

## Out of scope
- Plugin contributions wiring (T-110+).
- Cross-plugin sandboxing.

## Files touched
- `crates/devix-core/src/plugin/{host,runtime,bridge,pane_handle}.rs`
- `crates/devix-core/src/supervise/plugin.rs`

## Acceptance criteria
- [x] Forced plugin panic surfaces as `PluginError`; editor still
      responsive.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — partial close
- `PluginRuntime::load_supervised(path, sink, bus)` wraps the plugin
  worker thread in `crate::supervise::supervise(...)` with
  `RestartPolicy { max_restarts: 0, window: 30s }`. Practical
  consequence: a panic in the Lua VM (or any of the wrapper code on
  that thread) escalates immediately as `Pulse::PluginError` on the
  editor's bus and the plugin thread stops; the editor's main thread
  is unaffected.
- Successful load publishes `Pulse::PluginLoaded { plugin: /plugin/<stem>,
  version }` on the bus. (Stage 11 / T-110 will switch the source of
  `plugin` from filename stem to the manifest's `name` field.)
- Channel-re-acquisition on restart is **deferred**. The receiver
  halves are moved into the factory closure; on respawn the closure
  has nothing to consume from. True "supervisor restarts the plugin
  runtime" needs a channel-topology refactor (e.g., a stable
  `Arc<Mutex<Option<UnboundedSender>>>` indirection so editor-held
  senders refresh across restarts). That work goes alongside the
  module reorg into `host.rs`/`runtime.rs`/`bridge.rs`/`pane_handle.rs`
  per `crates.md`.
- `PluginRuntime` gains `shutdown_tx: Option<oneshot::Sender<()>>` and
  a custom `Drop` that fires the signal. Necessary because installed
  plugin panes (held by the editor) keep clones of `input_tx` alive
  past `PluginRuntime` drop, so the supervised loop's `tokio::select!`
  would never observe channel close. The shutdown branch breaks the
  loop cleanly so `SupervisedChild::drop` joins immediately.

## Spec references
- `docs/principles.md` — *Erlang/OTP*.
- `docs/specs/crates.md` — *crates/plugin/src/*.
- `docs/specs/pulse-bus.md` — *Plugin lifecycle*.
