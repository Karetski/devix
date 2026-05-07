# Task T-62 — Frontend-originated pulses (`InputReceived`, `ViewportChanged`)
Stage: 6
Status: pending
Depends on: T-42, T-44, T-60
Blocks:     T-63

## Goal
Wire frontend-originated events through the bus. The TUI input
thread translates crossterm events to `InputEvent` and publishes
`Pulse::InputReceived { event }` via `publish_async`. Resize / scroll
publishes `Pulse::ViewportChanged { frame, top_line, visible_rows }`.

## In scope
- `crates/devix-tui/src/input.rs`: crossterm → `InputEvent`
  translation. (`Modifiers` order, kebab-case parsing already in
  T-42.)
- `crates/devix-tui/src/input_thread.rs`: pushes
  `InputEvent` → `Pulse::InputReceived` via `publish_async`.
- Resize handler → `Pulse::ViewportChanged`.
- Core's main loop subscribes to these and runs the existing
  command/keymap dispatch under the new pulse-driven flow.

## Out of scope
- Drag-drop / IME / file-drop (deferred per `frontend.md` Q4).
- Removing the legacy direct-dispatch path (T-63).

## Files touched
- `crates/devix-tui/src/{input,input_thread}.rs`
- `crates/devix-core/src/core.rs`: input pulse handler

## Acceptance criteria
- [ ] No `App` dispatches input via direct method call; every
      keypress / mouse / scroll arrives as a Pulse.
- [ ] Resize publishes `ViewportChanged` per frame.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Frontend-originated pulses*.
- `docs/specs/frontend.md` — *InputEvent*, *Lifecycle*.
