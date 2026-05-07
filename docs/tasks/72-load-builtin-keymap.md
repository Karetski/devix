# Task T-72 — Load built-in keymap from manifest + override list
Stage: 7
Status: pending
Depends on: T-54, T-70, T-71
Blocks:     T-74

## Goal
Manifest loader registers every `contributes.keymaps` entry from
`builtin.json` into `Keymap`. Implement the conflict policy
(refuse-second-with-warning) and the user override list at
`$XDG_CONFIG_HOME/devix/keymap-overrides.json`.

## In scope
- Loader: builtin manifest keymaps → `Keymap`.
- Conflict detection: emit `Pulse::PluginError` and refuse the
  second binding (built-ins load first; plugins cannot silently
  override).
- Override file resolution: `$XDG_CONFIG_HOME/devix/keymap-overrides.json`,
  applied **after** all manifests load.
- Tests: built-in chord ↔ command mapping survives override file
  presence + absence; conflicting plugin chord refused; override
  file binds chord to a manifest-declared command.

## Out of scope
- Plugin manifest registration (T-110).
- v1 conditional bindings (`when` field stays as `null` in v0).

## Files touched
- `crates/devix-core/src/manifest_loader.rs`: keymap loader
- `crates/devix-core/src/commands/keymap.rs`

## Acceptance criteria
- [ ] Every chord in `builtin.json` resolves to its declared
      command path.
- [ ] Override file replaces the binding it names; no other
      bindings change.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *contributes.keymaps*, *Keymap
  conflicts*.
