# Task T-113 — Plugin contributes settings via manifest + Lua API
Stage: 11
Status: partial — store, set/get, Pulse::SettingChanged, manifest seed, override file landed; Lua bridge gated on T-81 full
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
- [ ] A test plugin reads its declared setting via `devix.setting`.
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
- *Still deferred*: the `devix.setting(key)` /
  `devix.on_setting_changed(callback)` Lua bridges. Wiring them
  threads `Arc<Mutex<SettingsStore>>` through `PluginHost::new` and
  installs the closures inside `install_devix_table` — same
  constructor surface T-81 full's module reorg is touching. The
  protocol-side wire form (`SettingValue` in `devix-protocol`,
  `Pulse::SettingChanged` in the catalog) is in place so the bridge
  can light up cleanly when T-81 lands.

## Spec references
- `docs/specs/manifest.md` — *contributes.settings*, *Open Q2*.
