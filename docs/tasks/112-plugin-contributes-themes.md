# Task T-112 — Plugin contributes themes via manifest
Stage: 11
Status: pending
Depends on: T-73, T-110
Blocks:     T-113

## Goal
Plugin manifests' `contributes.themes` register themes the user
can switch to. Themes are fully declarative; no Lua code touches
them.

## In scope
- Theme spec resolution: scope key → `Style`. Re-uses the canonical
  string serde for `Color`.
- Capability negotiation: `ContributeThemes`.
- Theme switching publishes `Pulse::ThemeChanged { theme, palette }`
  same as built-in themes (T-73).

## Out of scope
- Settings UI for theme selection (T-113 / future).

## Files touched
- `crates/devix-core/src/manifest_loader.rs`: theme path

## Acceptance criteria
- [ ] A test plugin theme registers, can be activated, and emits
      `ThemeChanged`.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *contributes.themes*.
