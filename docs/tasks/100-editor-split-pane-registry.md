# Task T-100 — Editor split: pane registry owner
Stage: 10
Status: pending
Depends on: T-95
Blocks:     T-101, T-102, T-103, T-104

## Goal
Carve out the `Pane` registry from the god-Editor struct into its
own owner (e.g., `PaneRegistry`). Editor holds it but does not
reach into its private fields. Reduces the Editor's surface so the
focus chain (T-101) and ops (T-102) and modal (T-103) can take
their pieces cleanly.

## In scope
- New `PaneRegistry` (owner of root pane + lookup helpers).
- Editor holds it; everything that asked Editor for a pane now
  asks `editor.panes()` for the registry.
- Public surface preserved on Editor (delegating accessors),
  shrinking only what's safe.

## Out of scope
- Focus / ops / modal split (T-101..T-103).

## Files touched
- `crates/devix-core/src/editor/registry.rs`: new
- `crates/devix-core/src/editor/editor.rs`: delegate accessors

## Acceptance criteria
- [ ] PaneRegistry owns the pane tree.
- [ ] No external code reaches into Editor's private pane fields.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/crates.md` — *crates/editor/src/* (Stage-7+ internal
  restructuring).
- `docs/principles.md` — *Hickey — simple is not easy*.
