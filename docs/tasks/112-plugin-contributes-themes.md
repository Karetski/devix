# Task T-112 — Plugin contributes themes via manifest
Stage: 11
Status: partial — store + activate landed; settings-UI / runtime user switch deferred
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
- [x] A test plugin theme registers, can be activated, and emits
      `ThemeChanged`.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — partial close
- New module `crates/devix-core/src/theme_store.rs` with
  `ThemeStore::register_from_manifest(manifest) -> usize` and a free
  `theme_store::activate(store, id, bus) -> Option<Theme>` that
  builds the in-memory `Theme` and publishes `Pulse::ThemeChanged
  { theme: /theme/<id>, palette }`.
- First-loaded-wins on theme-id collisions per `manifest.md`
  *Manifest discovery*. Theme ids stay flat at `/theme/<id>` per
  `pulse-bus.md`'s `ThemeChanged.theme` shape note (plugins are
  expected to namespace their own ids — `acme.dark`, `acme.light`).
- Tests: register-collision wins-first, activate-publishes-pulse,
  activate-unknown-returns-None. The activate test verifies both the
  in-memory `Theme` resolves the manifest's scope and the pulse's
  `palette.scopes` carries the same key.
- *Deferred*: capability negotiation (`ContributeThemes`
  warn-and-degrade) — same situation as T-110; needs T-81 full.
  Runtime user-driven theme switching (palette command, settings
  UI) is its own follow-up. main.rs still loads `/theme/default`
  from `BUILTIN_MANIFEST` at startup (the wire-up landed earlier
  this session). Plugin manifests' themes go into a `ThemeStore`
  but no UI surfaces them yet.

## Spec references
- `docs/specs/manifest.md` — *contributes.themes*.
