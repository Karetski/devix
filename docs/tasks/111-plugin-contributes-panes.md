# Task T-111 — Plugin contributes panes via manifest
Stage: 11
Status: complete — manifest cross-check + Lua → View IR via `pane:set_view` + minimal painter shipped
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
- **2026-05-08 follow-up (full close).**
  - **Lua → View IR marshaling.** New module
    `plugin/view_lua.rs` deserializes Lua tables shaped like the
    JSON wire form into `devix_protocol::view::View`. v0 supports
    `Empty`, `Text`, `Stack` — enough for plugin sidebars to paint
    structured content. Other variants (`Buffer`, `TabStrip`,
    `Sidebar`, `Popup`, `Modal`, `Split`, `List`) are rejected at
    deserialize time because they carry editor-side context plugins
    shouldn't fabricate.
  - `pane:set_view(view_table)` method on `LuaPaneHandle` stores the
    deserialized View into a shared `Arc<Mutex<Option<View>>>`.
  - `LuaPane::render` checks the View first; if set, paints via the
    minimal painter in `view_lua::paint_minimal`. Falls back to the
    line-based path otherwise (back-compat for plugins on the older
    `pane:set_lines` API).
  - The minimal painter mirrors the supported variants (`Stack`
    proportional layout, `Text` with style runs). The full
    `paint_view` walker in `devix-tui` becomes the sole renderer at
    T-95; until then the in-core minimal painter avoids the
    `devix-core ↔ devix-tui` cycle.
  - Test: `pane_set_view_stores_view_ir_for_render` exercises a
    nested stack-of-text from Lua and asserts the deserialized
    structure.
- *Still deferred*:
  - **`/plugin/<name>/pane/<id>` path-based addressing**. Panes
    install on the editor's structural tree by slot today; the
    `id` is documented but unused as a registry key. Lands when a
    consumer of `panes.pane_at(/plugin/<name>/pane/<id>)` arrives.
  - **`StableViewIds` capability gating**. Synthetic IDs are
    generated under `/synthetic/plugin/<kind>-<n>` per render — the
    `StableViewIds` advert is purely informational at v0.

## Spec references
- `docs/specs/manifest.md` — *contributes.panes*.
- `docs/specs/frontend.md` — *Open Q1*.
