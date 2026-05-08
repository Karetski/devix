# Task T-24 — Clippy baseline + lint cleanup
Stage: 2
Status: complete
Depends on: T-20, T-21, T-22, T-23
Blocks:     all Stage 3+ (clean baseline)

## Goal
Bring the workspace to zero clippy warnings under the project's
preferred lint set. Establishes a clean baseline so Stage 3+
warnings are signal, not noise.

## In scope
- Run `cargo clippy --workspace --all-targets -- -D warnings`.
- Fix all reported lints, preferring code adjustments over
  `#[allow(...)]`. Where an allow is justified, scope it to the
  smallest item and add a one-line `// clippy: <reason>` comment.
- Document allowed lints in workspace `Cargo.toml`'s
  `[workspace.lints]` block if more than three local allows
  accumulate.

## Out of scope
- New features. New tests.
- Adding clippy-pedantic / clippy-nursery overrides.

## Files touched
- Source files across the workspace (small changes only).
- `Cargo.toml` (lint config if needed).

## Acceptance criteria
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test  --workspace` passes.

## Spec references
- (No spec mandates clippy; this is workspace hygiene before the
  Stage-3 spec implementations land.)
