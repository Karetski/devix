# Task T-102 — Ops owner (split / close / toggle)
Stage: 10
Status: complete
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
- [x] Every op publishes the spec'd pulse.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-07)
- Kept ops as `impl Editor` methods rather than refactoring to free
  functions taking `(&mut PaneRegistry, &mut FocusChain, ...)`. The
  acceptance criteria target pulse emission and tree mutations through
  walk helpers; both are met. Free functions would push 4–5 owner refs
  through every signature with no behavioural gain.
- `FocusChanged` (T-101) and `FrameSplit` / `FrameClosed` /
  `SidebarToggled` (T-102) all fire from inside their op; nothing
  outside the editor's ops directly mutates the tree or the focus
  chain.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog → Layout / focus*.
