# Task T-12 — Rename devix-app → devix-tui; absorb widgets + layout + clipboard
Stage: 1
Status: pending
Depends on: T-11
Blocks:     T-13

## Goal
Rename the binary crate `devix-app` to `devix-tui`. Move all
ratatui/crossterm-touching code from the now-shrunken `devix-panes`
crate into it. Move `widgets/layout.rs` (LinearLayout / UniformLayout /
CollectionPass / VRect / CellGeometry / scroll math) into devix-tui's
`layout` module. Move arboard clipboard impl into devix-tui. After
this, `devix-panes` is gone entirely.

## In scope
- Rename directory `crates/app/` → `crates/devix-tui/`.
- `[package] name = "devix-tui"`, retain binary name `devix`.
- Move sources per `crates.md` § *File-level migration*:
  - `crates/panes/src/widgets/{popup,palette,sidebar,tabstrip}.rs`
    → `crates/devix-tui/src/widgets/`.
  - `crates/panes/src/widgets/layout.rs` → `crates/devix-tui/src/layout.rs`.
  - `split_rects` from `crates/panes/src/layout_geom.rs` →
    `crates/devix-tui/src/layout.rs`.
  - `crates/app/src/{render,events,input,clipboard}.rs` → renamed
    per spec (`render.rs` → `interpreter.rs`, `events.rs` →
    `input.rs`, `input.rs` → `input_thread.rs`).
- `devix-tui` deps: `devix-protocol`, `devix-core`, `ratatui`,
  `crossterm`, `arboard`, `anyhow`. **Not** tokio.
- Delete `crates/panes/` entirely after content moves.
- Update workspace `Cargo.toml`: drop `devix-panes`; rename
  `devix-app` member → `devix-tui`.

## Out of scope
- Implementing the View IR interpreter (Stage 4).
- Replacing existing render path (Stage 4).
- Touching tokio leakage outside what `crates.md` already requires
  hidden.

## Files touched
- `crates/devix-tui/**`: created from `crates/app/**` + widget moves
- `crates/panes/`: deleted
- `Cargo.toml`: workspace member rename, drop panes
- All `Cargo.toml`s downstream lose `devix-panes` dep

## Acceptance criteria
- [ ] `crates/panes/` does not exist.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test  --workspace` passes.
- [ ] `cargo run --bin devix` opens an editable buffer (sanity).

## Spec references
- `docs/specs/crates.md` — *devix-tui*, *File-level migration*,
  *Stage-1 sequencing T-12*.
