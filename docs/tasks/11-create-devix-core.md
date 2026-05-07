# Task T-11 — Create devix-core; absorb editor + plugin + most of panes
Stage: 1
Status: pending
Depends on: T-10
Blocks:     T-12, T-13

## Goal
Create the new `devix-core` crate. Migrate `crates/editor/`,
`crates/plugin/`, and the non-widget portions of `crates/panes/` into
it per the file-level migration table in `crates.md`. Add `pub use`
re-exports inside the dissolved crates (or leave them as deletion
targets for T-12) so `devix-app` keeps importing through unchanged
paths until its rename.

## In scope
- Create `crates/devix-core/` with Cargo.toml + src/lib.rs.
- Move sources per `crates.md` § *File-level migration*:
  - `crates/panes/src/{pane,action,clipboard,walk,theme}.rs` → core.
  - `crates/panes/src/{geom,layout_geom}.rs` → split per spec
    (`Anchor`/`AnchorEdge` → protocol view stub; `Axis`/`SidebarSlot`
    → protocol view stub; `split_rects` → kept in panes for T-12 to
    move to tui).
  - `crates/editor/src/**` → `crates/devix-core/src/editor/**` plus
    `document.rs`, `cursor.rs`, `commands/**`, `render/buffer.rs`.
  - `crates/plugin/src/lib.rs` → `crates/devix-core/src/plugin/mod.rs`
    (single-file copy now; internal split deferred to T-81).
- Add workspace deps: `mlua`, `nucleo-matcher`, `notify`, `tokio`,
  `slotmap`, `unicode-segmentation` flow into devix-core's Cargo.toml.
- `devix-app` updated to depend on `devix-core` instead of
  `devix-editor` / `devix-plugin` / (most of) `devix-panes`.

## Out of scope
- Renaming `devix-app` → `devix-tui` (T-12).
- Moving widgets / layout primitives (T-12).
- Any internal restructuring inside core (Stages 7+).
- Changing public APIs (relocation only).

## Files touched
- `crates/devix-core/**`: created from migrations above
- `crates/devix-core/Cargo.toml`: workspace dep set per `crates.md`
- `crates/editor/`, `crates/plugin/`: removed (sources moved)
- `crates/panes/`: pruned to widgets + layout; `Cargo.toml` shrinks
- `crates/app/Cargo.toml`: depends on `devix-core` + `devix-protocol`
- `Cargo.toml`: workspace dep `devix-core = { path = ... }`; drop
  `devix-editor`, `devix-plugin` entries

## Acceptance criteria
- [ ] No source under `crates/editor/` or `crates/plugin/`.
- [ ] `crates/devix-core/` builds.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test  --workspace` passes (every existing test).

## Spec references
- `docs/specs/crates.md` — *devix-core (NEW)*, *File-level migration*,
  *Stage-1 sequencing T-11*.
