# Task T-102 — Ops owner (split / close / toggle)
Stage: 10
Status: pending
Depends on: T-100, T-101
Blocks:     T-104

## Goal
Carve `editor/ops.rs` into a clean owner that mutates the Pane tree
through walk helpers only. Each op is a typed entry (no
direct field access into Editor); produces the right pulses
(`FrameSplit`, `FrameClosed`, `SidebarToggled`).

## In scope
- `editor/ops.rs` becomes a flat list of public free functions
  taking the registry + bus and mutating in place via walk
  helpers.
- Each op publishes the matching pulse.

## Out of scope
- New ops.
- Modal slot (T-103).

## Files touched
- `crates/devix-core/src/editor/ops.rs`

## Acceptance criteria
- [ ] Every op publishes the spec'd pulse.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog → Layout / focus*.
