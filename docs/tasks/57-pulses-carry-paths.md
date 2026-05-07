# Task T-57 — Pulses carry `Path`; sweep call sites
Stage: 5
Status: pending
Depends on: T-31, T-50, T-51, T-52, T-53, T-54, T-55, T-56
Blocks:     T-60

## Goal
Convert every existing pulse / event payload that today carries a
typed id (`DocId`, `CursorId`, `FrameId`, `SidebarSlot`,
`CommandId`) to carry a `Path` per the v0 catalog locked in
`pulse-bus.md`. After this lands the pulse types are the canonical
shape Stage 6 starts publishing through.

## In scope
- Every `Pulse::*` variant in code uses `Path`-keyed payloads (the
  enum already does, from T-31; this task sweeps producer call
  sites).
- Replace ad-hoc `Effect::*` variants that carry typed ids with
  payloads that mint `Path`s at the publish site (until T-60
  retires `Effect` entirely).
- Any subscriber that switched on typed ids switches on
  `Path::starts_with` against its registered prefix.

## Out of scope
- Replacing `EventSink::pulse(closure)` with `PulseBus::publish` at
  every call site (T-60).
- Frontend-originated pulses (T-62).
- Removing `Effect` entirely (T-63).

## Files touched
- `crates/devix-core/src/**/*.rs` (sweep)
- `crates/devix-tui/src/**/*.rs` (sweep producer paths)

## Acceptance criteria
- [ ] No Pulse-payload-bearing site uses raw typed ids.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog (v0)*, *Interaction with
  other Stage-0 specs → namespace.md*.
- `docs/specs/namespace.md` — *Interaction with other Stage-0
  specs → pulse-bus.md*.
