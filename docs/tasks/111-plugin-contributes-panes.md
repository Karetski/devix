# Task T-111 — Plugin contributes panes via manifest
Stage: 11
Status: pending
Depends on: T-110, T-43
Blocks:     T-113

## Goal
Plugin manifests' `contributes.panes` register sidebar pane
contributions. Each pane is registered at
`/plugin/<name>/pane/<id>`. The pane's `lua_handle` returns a `View`
tree the host marshals into the canonical `View` IR.

## In scope
- Pane spec resolution: `slot: "left" | "right"`. `"floating"`
  remains reserved (locked: overlay panes use top-level
  `View::Popup` per `frontend.md` Q1, not a sidebar slot).
- Lua `View` → Rust `View` marshaling (re-uses serde, Lua tables
  read shape-equivalent to JSON).
- Capability negotiation: `ContributeSidebarPane`.
- Plugin pane is a `Pane` impl wrapping the lua handle; its
  `render` calls into Lua and emits the View under
  `/plugin/<name>/pane/<id>`.

## Out of scope
- Overlay panes (`ContributeOverlayPane`, deferred to v1).
- Status items (`ContributeStatusItem`, deferred to v1).

## Files touched
- `crates/devix-core/src/plugin/pane_handle.rs`
- `crates/devix-core/src/manifest_loader.rs`: pane path

## Acceptance criteria
- [ ] A test plugin pane renders into the sidebar slot it declared.
- [ ] `StableViewIds` capability check gates pane ids; without it,
      a one-time warn surfaces.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *contributes.panes*.
- `docs/specs/frontend.md` — *Open Q1*.
