# Task T-53 — Migrate commands onto namespace (`/cmd/<dotted-id>`)
Stage: 5
Status: pending
Depends on: T-30
Blocks:     T-57, T-71, T-110

## Goal
`CommandRegistry` becomes `Lookup<Resource = dyn EditorCommand>`
mounted at `/cmd/<dotted-id>`. `CommandId` is replaced by a typed
wrapper around `Path`, or removed entirely if the wrapper buys
nothing.

## In scope
- `CommandRegistry: Lookup<Resource = dyn EditorCommand>`.
- Migrate every command id from a string handle / enum case to a
  `/cmd/<dotted-id>` path.
- Per-root parser `Command::id_from_path` (per `namespace.md` Q3).

## Out of scope
- Manifest-driven registration (T-71).
- Plugin-contributed commands (T-110).

## Files touched
- `crates/devix-core/src/commands/registry.rs`
- `crates/devix-core/src/commands/dispatch.rs`
- `crates/devix-core/src/commands/cmd/*.rs` (id constants)

## Acceptance criteria
- [ ] Every built-in command resolves through `Path`-keyed lookup.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Migration table* row for commands.
