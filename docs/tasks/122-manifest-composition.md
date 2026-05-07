# Task T-122 — Manifest declarative composition primitives
Stage: 12
Status: pending
Depends on: T-33, T-110, T-120, T-121
Blocks:     all of Stage 13+

## Goal
SICP three-step for manifest composition: presets, overrides, and
imports. Lets a plugin manifest declare "extend preset X with
overrides Y" rather than restating every contribution.

## In scope
- `presets/`: built-in named presets (e.g., default keymap preset)
  shipped in `crates/devix-core/manifests/presets/`.
- Manifest `extends: "<preset-id>"` field — addition is a minor
  bump per `manifest.md` *Versioning*; record in amendment log.
- Override semantics: child manifest fields shadow parent;
  arrays merge by id.
- Tests: a manifest extending a preset resolves to the merged
  contribution set.

## Out of scope
- Cross-plugin imports (future).
- Configurable preset registries (future).

## Files touched
- `crates/devix-core/manifests/presets/*.json`
- `crates/devix-protocol/src/manifest.rs` (extends field)
- `crates/devix-core/src/manifest_loader.rs`
- `docs/specs/manifest.md` (extends field documented)
- `docs/specs/foundations-review.md` (amendment log entry)

## Acceptance criteria
- [ ] `extends` resolves and the merged manifest validates.
- [ ] Amendment log entry cites this task.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *SICP*.
- `docs/specs/manifest.md` — *Versioning*.
- `docs/specs/foundations-review.md` — *Spec-to-implementation
  feedback loop*.
