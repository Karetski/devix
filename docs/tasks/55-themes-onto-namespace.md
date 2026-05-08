# Task T-55 — Migrate themes onto namespace (`/theme/<scope>`)
Stage: 5
Status: complete
Depends on: T-30, T-41
Blocks:     T-57, T-73

## Goal
`Theme` becomes `Lookup<Resource = Style>` mounted at
`/theme/<scope>`, preserving today's dotted scope syntax
(e.g., `/theme/keyword.control`).

## In scope
- `Theme: Lookup<Resource = Style>`.
- Theme location split (per `crates.md` Q3): `ThemeSpec` in
  protocol (manifest schema entry), active `Theme` in core.
- `paths()` enumerates every populated scope.

## Out of scope
- Manifest-driven theme registration (T-73).
- ThemeChanged pulse wiring (T-73).

## Files touched
- `crates/devix-core/src/theme.rs`

## Acceptance criteria
- [ ] `Theme::lookup(&Path::parse("/theme/keyword.control")?)`
      returns the existing scoped style.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Migration table* row for themes.
- `docs/specs/crates.md` — *Open Q3*.
