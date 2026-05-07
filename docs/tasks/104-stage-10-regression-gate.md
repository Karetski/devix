# Task T-104 — Stage-10 regression gate
Stage: 10
Status: pending
Depends on: T-100, T-101, T-102, T-103
Blocks:     all of Stage 11+

## Goal
Verify Editor's god-struct is meaningfully decomposed: pane
registry, focus chain, ops, modal slot all live in their own
owners with narrow public surfaces. Editor itself becomes a
coordinator over those four owners + DocStore + CursorStore +
CommandRegistry + Keymap + Theme (already shaped from Stage 5).

## In scope
- Final structural sanity: `Editor` is small enough that its public
  fields/methods can fit on one screen.
- Build + test + manual run.

## Out of scope
- New features.

## Files touched
- (no new code; possibly small cleanups)

## Acceptance criteria
- [ ] `Editor` struct has at most ~8 fields, each one a typed owner.
- [ ] `cargo build --workspace` passes with zero warnings.
- [ ] `cargo test --workspace` passes.
- [ ] Manual: every existing feature works end-to-end.

## Spec references
- `docs/principles.md` — *Hickey — simple is not easy*.
