# Task T-110 — Plugin contributes commands via manifest
Stage: 11
Status: partial — manifest commands register at /plugin/<name>/cmd/<id>; capability negotiation + keymap-from-manifest plugin-path deferred
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
- [x] A test plugin's command registers and invokes its Lua handle.
- [x] Plugin command paths use `/plugin/<name>/cmd/<id>`.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — partial close
- `CommandId` widened from tuple-struct `&'static str` to typed
  struct `{ plugin: Option<&'static str>, id: &'static str }` with
  `CommandId::builtin(id)` and `CommandId::plugin(plugin, id)`
  constructors. `to_path()` produces the spec'd shape per kind.
  `CommandRegistry`'s `Lookup` impl resolves both `/cmd/<id>` and
  `/plugin/<name>/cmd/<id>` paths.
- `PluginRuntime::install_with_manifest(commands, keymap, editor,
  manifest, bus)` installs manifest-declared commands at
  `/plugin/<manifest.name>/cmd/<id>`, matching each declaration to a
  Lua-registered handle by id. Manifest declarations without a
  matching Lua handler publish `Pulse::PluginError` and are skipped.
- `main.rs` now discovers plugins under `plugin_dir()` (every
  subdirectory containing `manifest.json`), loads each under the
  supervisor (T-81 partial), and wires its declared commands. The
  legacy `DEVIX_PLUGIN` single-file path stays alive for backward
  compatibility (registers at `/cmd/<id>`).
- *Deferred* from full T-110 spec:
  - **Capability negotiation** (`ContributeCommands` warn-and-degrade
    per `protocol.md` Q2). Capabilities aren't yet exchanged between
    host and plugin; this lands when T-81 introduces the protocol
    envelope on the plugin side.
  - **Keymap-from-manifest with plugin paths**. The manifest schema
    accepts `command: "/plugin/<name>/cmd/<id>"` per
    `manifest.md`, but `register_keymap_contributions` only
    resolves `/cmd/<id>` shape today. Wiring the plugin-path branch
    is small but interacts with the registry's path-shape resolution
    (now in place); deferred so this T-110 partial can land
    cohesively.
  - **First-loaded-wins on chord conflicts**: not enforced
    explicitly; current behavior is "later registration overwrites".
    Same enforcement question lives next to the keymap-manifest
    work.
- Tests: `manifest_driven_commands_register_at_plugin_namespace`
  asserts `/plugin/<name>/cmd/<id>` resolves through the registry's
  `Lookup`; `manifest_declares_unknown_command_id_publishes_plugin_error`
  asserts the orphan-declaration error path.

## Spec references
- `docs/specs/manifest.md` — *contributes.commands*, *Manifest
  discovery*.
- `docs/specs/protocol.md` — *Capability negotiation*, *Open Q2*.
