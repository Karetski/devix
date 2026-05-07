# Task T-54 — Migrate keymap onto namespace (`/keymap/<chord>`)
Stage: 5
Status: pending
Depends on: T-30, T-42, T-53
Blocks:     T-57, T-72

## Goal
`Keymap` becomes `Lookup<Resource = Path>` (chord → command path)
mounted at `/keymap/<chord>` with the canonical kebab-case chord
form.

## In scope
- `Keymap: Lookup<Resource = Path>`.
- Update chord parser to accept canonical kebab-case (today's parser
  in `crates/plugin/src/lib.rs:1180-1235` migrates into the keymap
  module as the single source of truth).
- TUI's display renderer (`format_chord` → `Ctrl+Shift+P`) untouched
  — display is a TUI concern; the canonical wire is the path.
- `paths()` enumerates every bound chord.
- Drop `DEVIX_PLUGIN` env var (locked: removed during T-50/T-54
  per `manifest.md` *Manifest discovery*); plugin discovery
  switches to directory-based.

## Out of scope
- Manifest-driven keymap (T-72).
- User override list (T-72).

## Files touched
- `crates/devix-core/src/commands/keymap.rs`
- `crates/devix-tui/src/widgets/palette.rs` (display renderer call
  sites; visual unchanged)

## Acceptance criteria
- [ ] `Keymap::lookup(&Path::parse("/keymap/ctrl-shift-p")?)`
      returns the palette command path.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Chord segments*, *Migration table*.
- `docs/specs/manifest.md` — *Manifest discovery*.
