# Task T-93 — Confirm Pane / Action trait location (in core)
Stage: 9
Status: pending
Depends on: T-91
Blocks:     T-94

## Goal
Per `crates.md` Q2 (lean: keep `Pane`/`Action` in `devix-core`),
confirm and document the location. Add a top-of-file doc comment
on `pane.rs` and `action.rs` explaining why these traits live in
core (they have non-trivial bodies, not pure data) and what would
trigger a re-evaluation (third-party plugins shipping their own
crates).

## In scope
- Doc comments on `pane.rs` and `action.rs`.
- No code movement; this is a judgment lock.

## Out of scope
- Splitting `Pane` into `PaneSpec` (data) + `PaneRenderer` (trait)
  (deferred until a third-party-crate plugin lands).

## Files touched
- `crates/devix-core/src/pane.rs`: doc comment
- `crates/devix-core/src/action.rs`: doc comment

## Acceptance criteria
- [ ] Both files document the locked decision and re-evaluation
      trigger.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/crates.md` — *Open Q2*.
- `docs/specs/foundations-review.md` — *Gate T-71*.
