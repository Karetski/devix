# Task T-121 — Pulse + filter combinators
Stage: 12
Status: pending
Depends on: T-31
Blocks:     T-122

## Goal
SICP three-step for pulse subscription:
- *Primitive*: `PulseFilter`.
- *Combination*: `PulseFilter::and`, `or`, `not` returning a
  `CombinedFilter` that the bus understands directly. Batched
  subscribe helpers (`bus.subscribe_each(filters, handler)`).
- *Abstraction*: named filter sets shippable from manifest.

## In scope
- Combinator methods on `PulseFilter` (or a wrapper enum) the bus
  matcher honors.
- Helper `bus.subscribe_each` for the common multi-filter case.
- Tests: combined filters behave correctly under `kind` + `field` +
  `path_prefix` interplay.

## Out of scope
- Pre-defined named filter library (consumer responsibility).

## Files touched
- `crates/devix-protocol/src/pulse.rs` (extension)
- `crates/devix-core/src/bus.rs`

## Acceptance criteria
- [ ] `(filterA.and(filterB))` matches only pulses both match.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *SICP*.
- `docs/specs/pulse-bus.md` — *Filter matching*.
