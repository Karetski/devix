# Task T-74 — Drop source-side built-in tables; Stage-7 regression gate
Stage: 7
Status: complete
Depends on: T-71, T-72, T-73
Blocks:     all of Stage 8+

## Goal
Now that the manifest is the source of truth, delete the hard-coded
built-in tables (`commands/builtins.rs` registration list,
`commands/keymap.rs` default-keymap table, `panes/theme.rs`
default-theme inline palette). Built-ins now flow exclusively from
`manifests/builtin.json`.

## In scope
- Delete the in-Rust enumerated lists; keep the per-command
  handler structs (each is still registered by id at startup —
  T-71's resolver does the wiring).
- Final regression: build + test + manual run; palette listing,
  settings UI listing, and keymap chord display all read from the
  manifest.

## Out of scope
- Plugin manifest paths (Stage 11).

## Files touched
- `crates/devix-core/src/commands/builtins.rs`: deleted (or empty
  module declaring `pub use cmd::*;` if needed for re-exports)
- `crates/devix-core/src/commands/keymap.rs`: trimmed to just
  the `Keymap` impl
- `crates/devix-core/src/theme.rs`: trimmed to `Theme` registry

## Acceptance criteria
- [ ] No hard-coded built-in command list in source.
- [ ] No hard-coded built-in keymap table in source.
- [ ] No hard-coded built-in theme palette inline.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.
- [ ] Manual: palette opens; chord hints render; theme renders.

## Spec references
- `docs/specs/manifest.md` — *Built-in manifests*.
