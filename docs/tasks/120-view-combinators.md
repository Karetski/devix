# Task T-120 — View combinators (SICP layer for the IR)
Stage: 12
Status: pending
Depends on: T-44, T-95
Blocks:     T-122

## Goal
SICP three-step for the View IR: every `View::*` variant is a
*primitive*; provide *means of combination* via builder
combinators (`stack`, `split_h`, `split_v`, `with_sidebar`, `tab`,
`modal`); provide *means of abstraction* by letting consumers
register named compositions reusable as primitives at the next
level.

## In scope
- `crates/devix-core/src/render/build.rs`: builder fns producing
  `View` trees with sane id minting (per T-90's strategy).
- Documented patterns: how a feature crate composes a sidebar
  + buffer + tab strip into one View tree.
- Tests: each combinator produces structurally correct,
  id-stable output.

## Out of scope
- Lua-side combinator surface (future).

## Files touched
- `crates/devix-core/src/render/build.rs`

## Acceptance criteria
- [ ] Combinator outputs match hand-built View trees for the same
      structural input.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *SICP — primitives, combination,
  abstraction*.
- `docs/specs/foundations-review.md` — *Verification: principles
  vs specs* (SICP row).
