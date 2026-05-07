# Task T-131 — Pulse delivery allocation pass
Stage: 13
Status: pending
Depends on: T-130
Blocks:     T-133

## Goal
Reduce per-publish allocation in the bus. Pre-allocated subscriber
lists per `PulseKind`; reuse iteration buffers across publishes;
avoid Vec-cloning on the dispatch path.

## In scope
- Internal restructure of `PulseBus`'s subscriber storage to keep
  per-kind lists hot and contiguous.
- Benchmarks (criterion or `cargo bench` with a simple harness)
  showing the alloc count drop.

## Out of scope
- API changes visible to callers.
- Per-pulse priority queue (deferred per `pulse-bus.md` Q3).

## Files touched
- `crates/devix-core/src/bus.rs`
- `crates/devix-core/benches/bus.rs` (new; if criterion lands)

## Acceptance criteria
- [ ] Bench shows reduced alloc count per publish in the typical
      case.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *Data-oriented design*.
- `docs/specs/pulse-bus.md` — *The `PulseBus` API*.
