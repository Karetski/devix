# Task T-20 — Workspace dependency hygiene
Stage: 2
Status: complete
Depends on: T-13
Blocks:     T-30 (foundation skeletons need clean dep set)

## Goal
Make every per-crate `Cargo.toml` consume external dependencies
through `workspace.dependencies` (`dep = { workspace = true }`).
Unblocks Stage 3 by ensuring the protocol/core/tui split has a
single source of truth for versions and feature flags.

## In scope
- Audit every crate's `Cargo.toml`. Convert direct `^x.y` deps to
  `{ workspace = true }`.
- Add missing entries to root workspace `Cargo.toml`'s
  `workspace.dependencies` block per the locked list in
  `crates.md` (slotmap, thiserror, anyhow, etc.).
- Confirm `mlua` features in workspace match `crates.md`
  (`["lua54", "vendored", "send"]`).

## Out of scope
- Adding new dependencies (T-21).
- Removing dependencies (T-22).
- Touching version pins.

## Files touched
- `Cargo.toml` (workspace root): expanded `workspace.dependencies`
- `crates/*/Cargo.toml`: convert direct deps to workspace

## Acceptance criteria
- [ ] No crate declares a direct `dep = "x.y"` for an external dep
      that the workspace tracks.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test  --workspace` passes.

## Spec references
- `docs/specs/crates.md` — *Cargo workspace*, *Resolved during initial review*.
