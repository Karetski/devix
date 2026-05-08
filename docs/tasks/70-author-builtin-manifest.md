# Task T-70 — Author `manifests/builtin.json` and embed
Stage: 7
Status: complete
Depends on: T-33
Blocks:     T-71, T-72, T-73, T-74

## Goal
Author the built-in manifest at
`crates/devix-core/manifests/builtin.json`. Embed via
`include_str!` so it loads without a filesystem read at startup.
The schema is the same as plugin manifests (per `manifest.md`).

## In scope
- `manifests/builtin.json`: full v0 contributions for built-ins —
  every command in `commands/builtins.rs`, every keymap in
  `commands/keymap.rs`, the default theme palette ported from
  `panes/theme.rs`. Name is `devix-builtin`; no `entry` field.
- The manifest validates against the JSON Schema generated in T-33.
- A unit test loads the embedded JSON and asserts validation
  passes.

## Out of scope
- Wiring contributions into runtime registries (T-71/72/73).
- Dropping the source-side built-in tables (T-74).

## Files touched
- `crates/devix-core/manifests/builtin.json`: new
- `crates/devix-core/src/lib.rs`: `const BUILTIN_MANIFEST: &str =
  include_str!(...);`
- `crates/devix-core/tests/builtin_manifest.rs`: validation test

## Acceptance criteria
- [ ] Built-in manifest validates against the generated JSON Schema.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *Built-in manifests*, *Manifest
  discovery*.
