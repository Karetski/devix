# Task T-113 — Plugin contributes settings via manifest + Lua API
Stage: 11
Status: complete — store, set/get, Pulse::SettingChanged, manifest seed, override file, Lua bridge (read + observe) all shipped
Depends on: T-110, T-111, T-112
Blocks:     all of Stage 12+

## Goal
Plugin manifests' `contributes.settings` declare typed setting
keys. A user-side settings file resolves values; the runtime
exposes `devix.setting(key)` for plugins. v0 supports flat
boolean/string/number/enum (locked per `manifest.md` Q2).

## In scope
- Setting spec resolution: `boolean | string | number | enum`.
- Setting file resolution:
  `$XDG_CONFIG_HOME/devix/settings.json` →
  `~/.config/devix/settings.json`.
- Lua API `devix.setting(key)` returning the resolved value (or
  the spec's `default` if unset).
- Capability negotiation: `ContributeSettings`.

## Out of scope
- Settings UI rendering (deferred).
- Nested object schemas (deferred per Q2).

## Files touched
- `crates/devix-core/src/manifest_loader.rs`: settings path
- `crates/devix-core/src/plugin/bridge.rs`: `devix.setting` API

## Acceptance criteria
- [x] A test plugin reads its declared setting via `devix.setting`.
- [x] Setting file overrides defaults.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — partial close
- New module `crates/devix-core/src/settings_store.rs` with
  `SettingsStore` (`HashMap<String, SettingValue>` + admissible enum
  ranges), `register_from_manifest(manifest) -> usize`,
  `apply_overrides_from_file(path) -> Result<usize, _>`, and
  `settings_overrides_path()` resolver
  (`$XDG_CONFIG_HOME/devix/settings.json` →
  `~/.config/devix/settings.json`).
- `SettingValue` is `Boolean | String | Number | EnumString`
  matching `manifest.md`'s flat v0 lock. First-loaded-wins on key
  collisions; type mismatches and out-of-list enum overrides surface
  as `SettingsOverrideError`. Unknown override keys (no manifest
  declared them) are silently skipped — they may belong to an
  unloaded plugin.
- Tests cover register-defaults, enum-with-values, first-wins,
  apply-typed-values, type-mismatch-errors, enum-out-of-range-errors,
  and missing-file-is-silent. Seven tests, all green.
- **2026-05-08 follow-up.** `SettingValue` moved from
  `devix-core::settings_store` to `devix-protocol::manifest` so the
  pulse wire form matches the in-memory value. Pulse catalog gains
  `Pulse::SettingChanged { setting: Path, value: SettingValue }`
  (minor pulse-bus.md bump). `SettingsStore::set(key, value, &bus)`
  mutates + publishes; rejects unknown keys, type mismatches, and
  out-of-list enum values without modifying state.
  `SettingsStore` lives on `Editor` (parallel to `theme_store`);
  `main.rs` seeds it from `BUILTIN_MANIFEST` + every discovered
  plugin manifest, then applies overrides from
  `$XDG_CONFIG_HOME/devix/settings.json`.
- **2026-05-08 full close (T-81 unblocked the bridge).**
  - `Editor.settings_store` shifts to `Arc<Mutex<SettingsStore>>`
    so the plugin worker thread shares the store with the editor.
  - `PluginHost::new_with(Option<SharedSettingsStore>)` accepts the
    shared store; `install_devix_table` exposes
    `devix.setting(key) -> value | nil` (lock + read; unknown keys
    return `nil`) and `devix.on_setting_changed(callback)` (registers
    a Lua callback handle keyed in the host's `setting_callbacks`
    list).
  - `PluginInput::SettingChanged { handle, key, value }` carries
    one bus-driven invocation across the worker boundary;
    `dispatch_input` calls `host.invoke_with(handle, (key, value))`
    with the value marshaled into a Lua scalar.
  - `PluginRuntime::load_supervised_with_settings` is the
    settings-aware constructor; on success it subscribes to
    `Pulse::SettingChanged` and fans out one
    `PluginInput::SettingChanged` per registered Lua handle through
    the channel-refresh-aware `input_tx`. On restart, the host
    re-registers callbacks deterministically and the runtime mirrors
    the new handles into its shared list.
  - `main.rs` switches plugin loading to
    `load_supervised_with_settings`, threading
    `editor.settings_store.clone()` into every supervised plugin.
  - Tests: `devix_setting_reads_from_shared_store` exercises the
    read path; `on_setting_changed_dispatches_via_input_channel`
    exercises the observe path (host-level dispatch matching what
    the bus subscriber drives).

## Spec references
- `docs/specs/manifest.md` — *contributes.settings*, *Open Q2*.
