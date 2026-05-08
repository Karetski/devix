# Task T-31 — Pulse bus skeleton (types in protocol; impl in core)
Stage: 3
Status: complete
Depends on: T-30, T-22
Blocks:     T-32, T-57, T-60, T-61

## Goal
Implement `Pulse` (closed enum, v0 catalog), `PulseKind`,
`PulseField`, `PulseFilter`, `SubscriptionId` types in
`devix-protocol::pulse`; implement `PulseBus` (queue + drain +
subscribers + depth tracking) in `devix-core::bus`. After this lands
the bus is callable but no producer publishes through it yet (that's
Stage 6).

## In scope
- `Pulse` enum with the 27 v0 variants from `pulse-bus.md`. Every
  payload `Path`-keyed; `Clone + Debug + Serialize + Deserialize`.
- `PulseKind`, `PulseField` (typed enum), `PulseFilter` with
  `kinds`, `path_prefix`, `field`. Constructors: `any`, `kind`,
  `kinds`, `under`, `under_field`.
- `Pulse::kind(&self)` returning `PulseKind`. Hand-maintained at v0
  per *Open Q4*.
- `SubscriptionId(u64)` newtype, monotonic.
- `ThemePalette` struct (in pulse module per spec).
- `InvocationSource`, `ModalKind`, `DirtyReason` enums.
- `devix-core::bus::PulseBus`: `new`, `with_capacity`,
  `with_depth_limit`, `publish`, `publish_async` (bounded MPSC,
  block-on-full, default 1024), `drain`, `subscribe`, `unsubscribe`.
  Dispatch indexed by `PulseKind`; depth tracked per re-entrancy
  (panic on overflow at default 16).
- Add `Pulse::ClientConnected` / `ClientDisconnected` per
  `foundations-review.md` *Gate T-22 → Session lifecycle pulses*
  (locked: yes, lands in T-31).
- Unit tests: synchronous publish reaches matching subscribers;
  cross-thread publish_async + drain ordering; reentrancy depth
  panic; filter `under_field` exercises every `PulseField` variant.

## Out of scope
- Replacing existing `EventSink`/`Effect`/`Wakeup` call sites
  (T-60).
- Per-pulse priority queue (deferred per `pulse-bus.md` Q3).
- Wall-clock timestamps (locked: no, per Q2).
- Macro-derived `PulseKind` (locked: hand-maintained at v0 per Q4).

## Files touched
- `crates/devix-protocol/src/pulse.rs`: types + filters
- `crates/devix-core/src/bus.rs`: PulseBus impl
- `crates/devix-core/src/lib.rs`: re-export `bus::PulseBus`

## Acceptance criteria
- [ ] Every variant in the v0 catalog (incl. `ClientConnected` /
      `ClientDisconnected`) compiles and round-trips serde.
- [ ] `PulseFilter::under_field` matches against every `PulseField`
      role; mismatched field is a no-op match (returns false).
- [ ] Reentrancy panic fires at the configured depth.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog (v0)*, *Filter matching*,
  *The `PulseBus` API*, *Delivery semantics*, *Resolved during
  initial review*.
- `docs/specs/foundations-review.md` — *Gate T-22 → Session
  lifecycle pulses*, *Gate T-21*.
