# Task T-21 — Add foundation-skeleton dependencies
Stage: 2
Status: pending
Depends on: T-20
Blocks:     T-30, T-31, T-32, T-33

## Goal
Add the workspace dependencies the Stage-3 skeleton tasks need but
the codebase doesn't currently declare: `thiserror`, `slotmap`,
`schemars` (for manifest JSON Schema generation, manifest.md Q5).
No code consumes them yet; the deps just become available.

## In scope
- `workspace.dependencies` += `thiserror = "1"`, `slotmap = "1"`,
  `schemars = { version = "0.8", optional = true }` (or current
  stable; pin in workspace).
- `devix-protocol` Cargo.toml gains `thiserror = { workspace = true }`
  ahead of `PathError` etc.
- `devix-core` Cargo.toml gains `slotmap = { workspace = true }` if
  not already inherited from existing usage.

## Out of scope
- Defining types that use these deps (Stage 3).
- Changing existing crates' code.

## Files touched
- `Cargo.toml`
- `crates/devix-protocol/Cargo.toml`
- `crates/devix-core/Cargo.toml`

## Acceptance criteria
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.
- [ ] Each declared dep is at least listed in one crate's
      Cargo.toml that will use it; nothing dangles.

## Spec references
- `docs/specs/crates.md` — *devix-protocol*, *devix-core*.
- `docs/specs/manifest.md` — *Open Q5* (schemars).
