# Task T-103 — Modal slot owner
Stage: 10
Status: pending
Depends on: T-100
Blocks:     T-104

## Goal
Carve out the modal slot into its own owner. Holds the
single-modal-at-a-time invariant (palette, picker), publishes
`Pulse::ModalOpened` / `Pulse::ModalDismissed` on transitions, and
exposes the modal as a Pane to the View producer.

## In scope
- `ModalSlot { current: Option<Box<dyn Pane>>, ... }`.
- Editor delegates modal operations to `ModalSlot`.
- T-43's View producer emits `View::Modal` from the slot's content.

## Out of scope
- New modal kinds.
- Z-order beyond tree-position order (per `frontend.md` Q3, locked).

## Files touched
- `crates/devix-core/src/editor/modal.rs`: new
- `crates/devix-core/src/editor/editor.rs`

## Acceptance criteria
- [ ] At most one modal active at a time.
- [ ] `ModalOpened` / `ModalDismissed` fire on transitions.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog → Modal*.
- `docs/specs/frontend.md` — *Open Q3*.
