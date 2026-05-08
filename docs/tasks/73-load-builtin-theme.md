# Task T-73 — Load built-in theme from manifest; ThemeChanged wiring
Stage: 7
Status: complete (ThemeChanged publish on activation deferred until activation API exists at T-112)
Depends on: T-55, T-70
Blocks:     T-74, T-112

## Goal
Manifest loader registers `contributes.themes` from `builtin.json`
into the theme registry. Selecting a theme publishes
`Pulse::ThemeChanged { theme, palette }` carrying a
`ThemePalette` for the frontend to interpret scope names against.

## In scope
- Loader: builtin manifest themes → theme registry.
- `Theme::activate(theme: Path)` publishes
  `Pulse::ThemeChanged { theme, palette }`.
- `ThemePalette` build helper that resolves the `Theme`'s scope
  table.
- Tests: switching themes publishes `ThemeChanged`; the carried
  palette matches the registered theme.

## Out of scope
- Plugin theme registration (T-112).
- TUI re-render on `ThemeChanged` (already covered by T-44 via
  Pulse subscription on `RenderDirty`/`ThemeChanged`).

## Files touched
- `crates/devix-core/src/manifest_loader.rs`
- `crates/devix-core/src/theme.rs`

## Acceptance criteria
- [ ] Default theme registers and is the initial active theme.
- [ ] Switching publishes `ThemeChanged` with a non-empty palette.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *contributes.themes*, *Resolved
  during initial review → Theme switching semantics*.
- `docs/specs/pulse-bus.md` — *Catalog → Theme → ThemeChanged*.
