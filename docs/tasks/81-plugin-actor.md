# Task T-81 — Plugin runtime as supervised actor
Stage: 8
Status: complete — module reorg shipped; channel-refresh restart shipped; max_restarts=3
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
- (Original partial-close note retained for history; the
  channel-refresh restart and module reorg landed together in the
  full close, see notes below.)
- `PluginRuntime` gains `shutdown_tx: Option<oneshot::Sender<()>>` and
  a custom `Drop` that fires the signal. Necessary because installed
  plugin panes (held by the editor) keep clones of `input_tx` alive
  past `PluginRuntime` drop, so the supervised loop's `tokio::select!`
  would never observe channel close. The shutdown branch breaks the
  loop cleanly so `SupervisedChild::drop` joins immediately.

## Notes (2026-05-08) — full close

- **Module reorg.** `plugin.rs` (2,300 lines) split into
  `plugin/{mod,host,bridge,pane_handle,runtime}.rs` per
  `crates.md` *crates/plugin/src/*:
  - `host.rs` — `PluginHost`. Owns the Lua VM + callback registry.
    Stays on one thread.
  - `pane_handle.rs` — `LuaPaneHandle` (the userdata), `LuaPane` /
    `PluginPane` (the editor-side wrappers).
  - `bridge.rs` — `LuaAction`, `make_command_action` (action
    invocation bridge).
  - `runtime.rs` — `PluginRuntime` + the supervised worker thread +
    channel topology.
  - `mod.rs` — types (`Contributions`, `CommandSpec`, `PaneSpec`,
    `PluginMsg`, `PluginInput`), helpers, re-exports.
- **Channel-refresh restart.** Editor-held senders are
  [`InvokeSender`] / [`InputSender`] (`Arc<Mutex<UnboundedSender<…>>>`).
  Each spawn of the factory closure:
  1. Creates fresh `(invoke_tx, invoke_rx)` and `(input_tx, input_rx)`
     pairs locally.
  2. Locks the editor-held `Arc<Mutex<…>>` and replaces its inner
     sender with the fresh one.
  3. Uses the fresh receivers in the worker's `tokio::select!`.

  Editor captures (`LuaAction.sender`, `LuaPane.input_tx`) hold the
  `Arc` directly, so they pick up the new sender on restart without
  recompiling the closure. `send_invoke` / `send_input` helpers
  perform the lock + send under Erlang semantics: silent no-op on
  poisoned lock or closed receiver.
- **Restart budget.** `RestartPolicy::max_restarts` lifted from `0`
  to `3` with a `30s` window — three crashes within thirty seconds
  before the supervisor escalates permanently.
- **`shutdown_tx` topology.** Now `Arc<Mutex<Option<oneshot::Sender>>>`
  so each spawn registers its own oneshot and `Drop` signals the
  *current* worker — necessary because the supervisor may be
  mid-restart when the runtime drops.
- **Lua entry determinism.** Because the Lua entry is re-executed on
  each spawn and registers actions in source order against a fresh
  `next_handle` counter, action handle 1 in the restarted host
  refers to the same Lua function as handle 1 in the dead host. The
  editor's `PluginCommandAction(handle=1, …)` keeps working
  transparently — no re-registration into `CommandRegistry` needed.
- **Limitations.** Pane line content set during the dead host's
  lifetime persists in the shared `Arc<Mutex<Vec<String>>>` until
  the new host writes to a *fresh* `Arc<Mutex<Vec<String>>>` it
  allocates in `register_pane`. The editor's installed `LuaPane`
  still points at the *old* `Arc`, so the new host's pane mutations
  are invisible until the editor reinstalls the pane. Auto pane
  reinstall on restart is the next sprint's concern; the topology
  ships with this commit.

## Spec references
- `docs/principles.md` — *Erlang/OTP*.
- `docs/specs/crates.md` — *crates/plugin/src/*.
- `docs/specs/pulse-bus.md` — *Plugin lifecycle*.
