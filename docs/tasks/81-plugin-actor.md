# Task T-81 — Plugin runtime as supervised actor
Stage: 8
Status: pending
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
- [ ] Forced plugin panic surfaces as `PluginError`; editor still
      responsive.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *Erlang/OTP*.
- `docs/specs/crates.md` — *crates/plugin/src/*.
- `docs/specs/pulse-bus.md` — *Plugin lifecycle*.
