# Task T-60 — Replace EventSink + DiskSink + MsgSink + Wakeup with PulseBus
Stage: 6
Status: pending
Depends on: T-31, T-57
Blocks:     T-61, T-63

## Goal
Replace every existing `EventSink::pulse(...)` /
`DiskSink::push(...)` / `MsgSink::send(...)` / `Wakeup::request()`
call site with `PulseBus::publish(...)` (in-thread) or
`PulseBus::publish_async(...)` (cross-thread). The drain happens
once per main-loop tick.

## In scope
- Sweep every `EventSink` / `DiskSink` / `MsgSink` / `Wakeup` call
  site; replace with the typed `Pulse` variant. Cross-thread sites
  (file watcher, plugin worker) use `publish_async`.
- Main loop calls `bus.drain()` once per tick (matches today's
  drain shape per `pulse-bus.md`).
- Backpressure: 1024 cap with block-on-full (locked).
- Tests already passing remain green; the drain ordering matches
  pre-task (FIFO).

## Out of scope
- Removing the `EventSink` / `Effect` types themselves (T-63).
- Frontend-originated pulses (T-62).
- Replacing closure-as-message dispatches (T-61).

## Files touched
- `crates/devix-core/src/core.rs`: hold the bus
- `crates/devix-core/src/**/*.rs`: producer rewrites
- `crates/devix-tui/src/app.rs`: drain per tick

## Acceptance criteria
- [ ] No producer in the workspace calls `EventSink::pulse` or
      friends; every event is a `bus.publish*`.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Delivery semantics*.
- `docs/specs/crates.md` — *crates/app/src/**/* migration row*.
