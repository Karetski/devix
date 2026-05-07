# Task T-10 — Create devix-protocol crate
Stage: 1
Status: pending
Depends on: —
Blocks:     T-11, T-12, T-13, T-30, T-31, T-32, T-33

## Goal
Create the new `devix-protocol` crate as a pure-data + serde crate.
Move types that already exist as structurally pure-data shapes (or
that we'll soon make pure-data) into it. No type changes; this task
is a relocation. After this lands, `devix-panes` / `devix-editor` /
`devix-app` continue to compile via re-exports.

## In scope
- Create `crates/devix-protocol/` with Cargo.toml + src/lib.rs.
- Module skeletons: `path`, `pulse`, `protocol`, `manifest`, `view`,
  `input` (empty modules with TODO comments — concrete content lands
  in Stage 3).
- Add `devix-protocol = { path = "crates/devix-protocol" }` to
  workspace.dependencies.
- Add re-export of `HighlightSpan` from `devix-syntax`.

## Out of scope
- Implementing `Path`, `Pulse`, `Envelope`, `Manifest`, `View`,
  `InputEvent` — those land in Stage 3 / 4 task files.
- Moving any logic out of `devix-core`-bound crates.
- Touching `devix-panes` / `devix-editor` / `devix-app` apart from
  what's required for them to compile against the new workspace.

## Files touched
- `crates/devix-protocol/Cargo.toml`: new
- `crates/devix-protocol/src/lib.rs`: new (module decls + reexport)
- `crates/devix-protocol/src/path.rs`: empty stub
- `crates/devix-protocol/src/pulse.rs`: empty stub
- `crates/devix-protocol/src/protocol.rs`: empty stub
- `crates/devix-protocol/src/manifest.rs`: empty stub
- `crates/devix-protocol/src/view.rs`: empty stub
- `crates/devix-protocol/src/input.rs`: empty stub
- `Cargo.toml`: workspace deps add `devix-protocol`

## Acceptance criteria
- [ ] `crates/devix-protocol` exists and compiles standalone.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test  --workspace` passes.
- [ ] No type moved into devix-protocol yet (relocation work is
      Stage 3).

## Spec references
- `docs/specs/crates.md` — *devix-protocol (NEW)*, *Stage-1 sequencing*.
