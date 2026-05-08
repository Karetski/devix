# Task T-71 — Load built-in commands from manifest
Stage: 7
Status: complete
Depends on: T-53, T-70
Blocks:     T-74

## Goal
Manifest loader registers every `contributes.commands` entry from
`builtin.json` into `CommandRegistry`. Bare ids resolve to Rust
handlers by id at registry-load time (`/cmd/edit.copy` →
`cmd::Copy`).

## In scope
- Loader: builtin manifest commands → `CommandRegistry`.
- Resolver: id → Rust handler (lookup table built at startup).
- Plugin manifests' commands stay unwired here (T-110).
- Tests: every built-in command in the JSON has a Rust handler;
  every Rust handler has a JSON entry (no orphans either way).

## Out of scope
- Removing the source-side `builtins.rs` tables (T-74).
- Plugin command resolution (T-110).

## Files touched
- `crates/devix-core/src/manifest_loader.rs`: command loader
- `crates/devix-core/src/commands/mod.rs`: registration entry point

## Acceptance criteria
- [ ] Every command id in `builtin.json` has a registered handler.
- [ ] No id appears only on one side.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *contributes.commands*.
