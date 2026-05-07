# Task T-12 — Rename devix-app → devix-tui; absorb widgets + layout + clipboard
Stage: 1
Status: complete
Depends on: T-11
Blocks:     T-13

## Goal
Rename the binary crate `devix-app` to `devix-tui`, rename its
internal files per the spec, and tighten its dep set. The widget /
layout primitive move from `devix-core` to `devix-tui` is **deferred
to Stage 9** (T-92 / T-95) per the foundations-review amendment log;
moving them in T-12 would create a `devix-core ↔ devix-tui` cycle
since `composites.rs`, `editor/tree.rs`, `editor/buffer.rs`,
`editor/editor.rs`, and `editor/commands/modal.rs` all reach into
widgets today. The physical destination matches the migration table;
the timing slips.

## In scope
- Rename directory `crates/app/` → `crates/devix-tui/`.
- `[package] name = "devix-tui"`, retain binary name `devix`.
- File renames per `crates.md` § *File-level migration*:
  - `render.rs` → `interpreter.rs`
  - `events.rs` → `input.rs`
  - `input.rs` → `input_thread.rs`
- `devix-tui` deps: `devix-protocol`, `devix-core`, `ratatui`,
  `crossterm`, `arboard`, `anyhow`. Drop `tokio` (no in-binary
  use). Drop `devix-text` (reach via `devix-core`'s re-export).
- Update workspace `Cargo.toml`: rename `devix-app` member → `devix-tui`.

## Deferred to Stage 9 (per amendment log 2026-05-06)
- `widgets/{popup,palette,sidebar,tabstrip}.rs` → `devix-tui/src/widgets/`
- `widgets/layout.rs` → `devix-tui/src/layout.rs`
- `split_rects` from `layout_geom.rs` → `devix-tui/src/layout.rs`
- `clipboard.rs` arboard impl → `devix-tui/src/clipboard.rs`
  (already at devix-tui post-rename)

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
