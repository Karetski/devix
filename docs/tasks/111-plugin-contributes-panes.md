# Task T-111 — Plugin contributes panes via manifest
Stage: 11
Status: partial — manifest pane declarations cross-checked against Lua-side register_pane; full View-IR Lua handle deferred
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
- [x] A test plugin pane renders into the sidebar slot it declared.
- [ ] `StableViewIds` capability check gates pane ids; without it,
      a one-time warn surfaces.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — partial close
- `PluginRuntime::install_with_manifest` now cross-checks each
  `manifest.contributes.panes` entry against the runtime's Lua-side
  `register_pane` registrations by slot. Orphan declarations (manifest
  declares a pane on slot X but no Lua pane registered for X) publish
  `Pulse::PluginError` and the install path skips that slot.
- Tests:
  `manifest_declares_pane_without_matching_lua_pane_publishes_plugin_error`
  and the matching-decl-is-silent counterpart confirm the validation
  path.
- *Deferred from full T-111 spec*:
  - **Lua → Rust `View` IR marshaling**. v0 panes still flow through
    the line-based `LuaPaneHandle` (`pane:set_lines`); the full
    handle-returns-View-tree marshaling is a substantial Lua-glue
    chunk that lands together with the structural `/pane`-tree
    unification (Stage 9 / T-91).
  - **`/plugin/<name>/pane/<id>` path-based addressing**. The
    declared `id` is documented but not yet used as a registry key —
    panes still install onto the editor's structural tree by slot.
    Once T-91 collapses LayoutNode into a unified Pane tree, the
    plugin-pane path becomes its address.
  - **`StableViewIds` capability gating**. Same situation as T-110 /
    T-112: capability negotiation is plugin-side wire work, blocked
    on T-81 full.

## Spec references
- `docs/specs/manifest.md` — *contributes.panes*.
- `docs/specs/frontend.md` — *Open Q1*.
