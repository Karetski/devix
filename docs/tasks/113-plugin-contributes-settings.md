# Task T-113 — Plugin contributes settings via manifest + Lua API
Stage: 11
Status: pending
Depends on: T-110, T-111, T-112
Blocks:     all of Stage 12+

## Goal
Plugin manifests' `contributes.settings` declare typed setting
keys. A user-side settings file resolves values; the runtime
exposes `devix.setting(key)` for plugins. v0 supports flat
boolean/string/number/enum (locked per `manifest.md` Q2).

## In scope
- Setting spec resolution: `boolean | string | number | enum`.
- Setting file resolution:
  `$XDG_CONFIG_HOME/devix/settings.json` →
  `~/.config/devix/settings.json`.
- Lua API `devix.setting(key)` returning the resolved value (or
  the spec's `default` if unset).
- Capability negotiation: `ContributeSettings`.

## Out of scope
- Settings UI rendering (deferred).
- Nested object schemas (deferred per Q2).

## Files touched
- `crates/devix-core/src/manifest_loader.rs`: settings path
- `crates/devix-core/src/plugin/bridge.rs`: `devix.setting` API

## Acceptance criteria
- [ ] A test plugin reads its declared setting via `devix.setting`.
- [ ] Setting file overrides defaults.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *contributes.settings*, *Open Q2*.
