# Task T-110 — Plugin contributes commands via manifest
Stage: 11
Status: pending
Depends on: T-33, T-71, T-81, T-104
Blocks:     T-111, T-112, T-113

## Goal
Plugin manifests' `contributes.commands` register into
`CommandRegistry` at plugin load. Bare ids resolve to
`/plugin/<name>/cmd/<id>`. Each command resolves to a
`LuaAction { handle }` invoking the plugin's registered Lua
function.

## In scope
- Manifest loader for plugin manifests (separate path from built-in
  loader; runs after built-ins per `manifest.md`).
- `LuaAction` impl that drives `PluginToCore::InvokeCommand` round
  trips.
- Capability negotiation: `ContributeCommands` must be in the
  negotiated set; otherwise warn-and-degrade per `protocol.md` Q2.
- Multi-plugin loading: every directory under the plugin root with
  a `manifest.json` is loaded; first-loaded wins on chord conflicts
  (per `manifest.md` *Manifest discovery*).
- Plugin loader publishes `PluginLoaded` / `PluginError`.

## Out of scope
- Pane / theme / settings contributions (T-111 / T-112 / T-113).
- Hot-reload (deferred per `manifest.md` Q4).

## Files touched
- `crates/devix-core/src/manifest_loader.rs`: plugin path
- `crates/devix-core/src/plugin/mod.rs`: LuaAction integration

## Acceptance criteria
- [ ] A test plugin's command registers and invokes its Lua handle.
- [ ] Plugin command paths use `/plugin/<name>/cmd/<id>`.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *contributes.commands*, *Manifest
  discovery*.
- `docs/specs/protocol.md` — *Capability negotiation*, *Open Q2*.
