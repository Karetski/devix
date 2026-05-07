# Task T-90 — Pick synthetic-id strategy
Stage: 9
Status: pending
Depends on: T-40, T-43
Blocks:     T-91, T-94

## Goal
Lock the synthetic-id strategy for `View::Stack` / `View::Modal`
(non-resource-bound nodes) per `frontend.md` § *ViewNodeId*. Two
options:
1. *Mint-and-cache*: per-parent cache keyed by structural position.
2. *Deterministic derivation*: id derived from parent's path +
   child structural slot.

Pick one and implement it on the View producer side. Document
the choice; add an amendment-log entry only if the chosen strategy
required spec wording adjustment beyond the existing two-option
description.

## In scope
- Decision write-up in this task file (filled in during task
  execution; the file commits with the resolved strategy
  documented inline).
- Implement the chosen strategy in
  `devix-core::editor::view`'s synthetic-id minting helper.
- Tests: same logical synthetic node yields the same id across two
  renders; structurally different nodes never collide.

## Out of scope
- Animation `transition` field population (lands later when a
  frontend advertises `Capability::Animations`).

## Files touched
- `crates/devix-core/src/editor/view.rs`

## Acceptance criteria
- [ ] Synthetic-id contract (same logical node → same id) holds
      across structural mutations that don't replace the node.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/frontend.md` — *ViewNodeId* (synthetic ids; two
  strategies).
- `docs/specs/foundations-review.md` — *Gate T-71*.
