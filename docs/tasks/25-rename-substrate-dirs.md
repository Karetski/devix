# Task T-25 — Rename `crates/text` and `crates/syntax` to match crate names
Stage: 2
Status: complete
Depends on: T-13
Blocks:     —

## Goal
Resolve `crates.md` Q5 ("Workspace member naming") in the affirmative.
Rename the two substrate crate directories so directory names match
their `[package] name` entries: `crates/text` → `crates/devix-text`,
`crates/syntax` → `crates/devix-syntax`. After this lands every
`crates/<dir>` matches its crate name.

Locked during the post-Stage-1 interactive review on 2026-05-06.

## In scope
- `git mv crates/text crates/devix-text`.
- `git mv crates/syntax crates/devix-syntax`.
- Update workspace `Cargo.toml`: `path = "crates/text"` →
  `path = "crates/devix-text"`; same for syntax.
- Update `docs/specs/crates.md` Q5 status (move from *Open
  questions* to *Resolved during initial review* with a parenthetical
  cite to T-25).
- Add an amendment-log entry to `docs/specs/foundations-review.md`
  recording the resolution.

## Out of scope
- Any code change. The crate names already matched in
  `Cargo.toml`; only the directory names were inconsistent.

## Files touched
- `crates/devix-text/**`: renamed in place
- `crates/devix-syntax/**`: renamed in place
- `Cargo.toml` (workspace): path string updates
- `docs/specs/crates.md`: Q5 → resolved
- `docs/specs/foundations-review.md`: amendment log

## Acceptance criteria
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes (130).
- [x] No `crates/text/` or `crates/syntax/` paths in the workspace.

## Spec references
- `docs/specs/crates.md` — *Open questions* Q5 (now resolved).
- `docs/specs/foundations-review.md` — *Amendment log* 2026-05-06.
