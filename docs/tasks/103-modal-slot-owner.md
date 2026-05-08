# Task T-103 — Modal slot owner
Stage: 10
Status: complete
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
- [x] At most one modal active at a time.
- [x] `ModalOpened` / `ModalDismissed` fire on transitions.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-07)
- New module `editor/modal_slot.rs` (the chosen filename avoids clash
  with the existing `editor/commands/modal/` palette code).
  `Editor.modal: Option<Box<dyn Pane>>` becomes `Editor.modal:
  ModalSlot`; the `Option`-shaped accessors (`as_ref` / `as_mut` /
  `is_some` / `is_none`) are preserved so existing call sites
  (responder chain, render path, downcast helpers) continue to work.
- Pulse publishing happens in `Editor::open_modal` /
  `Editor::dismiss_modal`; no caller writes to `editor.modal` directly.
- T-43's View producer wiring (`View::Modal`) is unchanged — it still
  reads from `editor.modal.as_ref()`. The slot's invariant is enforced
  in core, not in the producer.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog → Modal*.
- `docs/specs/frontend.md` — *Open Q3*.
