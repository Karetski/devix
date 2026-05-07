# Task T-23 — Test suite reorganization onto new crate layout
Stage: 2
Status: pending
Depends on: T-13
Blocks:     T-31 (pulse bus tests need home), T-50

## Goal
Move integration tests left over in the deleted `app`/`editor`/
`plugin`/`panes` crate test dirs into the right new home
(`devix-core/tests/` or `devix-tui/tests/`). Ensure `cargo test
--workspace` runs every test that ran on main pre-T-10.

## In scope
- Move `crates/app/tests/plugin_sidebar.rs` (and any siblings) to
  `crates/devix-tui/tests/` (rendering-touching) or
  `crates/devix-core/tests/` (model-touching), per dependency.
- Move `crates/plugin/tests/plugin_e2e.rs` to
  `crates/devix-core/tests/`.
- Update test imports to the new crate paths.

## Out of scope
- Adding new tests.
- Refactoring existing tests beyond import path updates.

## Files touched
- `crates/devix-core/tests/**`
- `crates/devix-tui/tests/**`

## Acceptance criteria
- [ ] No `tests/` directory under crates that no longer exist.
- [ ] `cargo test --workspace` passes; the test count >= the
      pre-T-10 baseline.

## Spec references
- `docs/specs/crates.md` — *File-level migration*.
