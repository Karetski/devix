# Task T-33 — Manifest skeleton + reader + JSON Schema
Stage: 3
Status: complete — schema gen full close (manifest_json_schema + custom JsonSchema impls + emitted JSON Schema doc)
Depends on: T-30, T-31, T-32
Blocks:     T-70, T-110

## Goal
Implement the manifest schema from `manifest.md` in
`devix-protocol::manifest` and the reader/validator skeleton in
`devix-core::manifest_loader`. The reader can parse a JSON manifest,
validate it against the locked rules, and report failures via
`Pulse::PluginError`. Built-in registration and plugin registration
sites are stubs; concrete wiring lands in Stages 7 / 11.

## In scope
- Rust types per `manifest.md` *Rust types*: `Manifest`, `Engines`,
  `Contributes`, `CommandSpec`, `KeymapSpec`, `PaneSpec`, `ThemeSpec`,
  `SettingSpec`, `SubscriptionSpec`. `serde(deny_unknown_fields)` on
  the top-level structs.
- `engines.devix` ↔ `protocol_version` alias via serde rename per
  spec.
- Validator that runs the full table in `manifest.md` § *Validation*.
- JSON Schema generation via `schemars` (closing `manifest.md` Q5):
  **deferred to a follow-up task.** Producing a usable schema
  requires custom `JsonSchema` impls for `Path`, `Chord`, `Color`,
  `ProtocolVersion` — the canonical-string types whose derive serde
  is also placeholder until T-41 / T-42. Shipping schema generation
  before those land would emit a misleading schema. `schemars` is
  in `workspace.dependencies` (T-21) and ready when the canonical
  serde lands.
- Manifest discovery: `DEVIX_PLUGIN_DIR` env var → fall back to
  `$XDG_CONFIG_HOME/devix/plugins/` → `~/.config/devix/plugins/`.
- Tests: schema validation rejects every error column in the table;
  good manifest from `manifest.md` § *Top-level schema* round-trips.

## Out of scope
- Authoring the built-in manifest file (T-70).
- Loading actual contributions into runtime registries
  (T-71/72/73, T-110).
- Activation events (locked: deferred to post-v0).
- Hot-reload (locked: deferred).
- Lua entry handling beyond storing the path (Stage 11).

## Files touched
- `crates/devix-protocol/src/manifest.rs`: types + serde
- `crates/devix-core/src/manifest_loader.rs`: reader + validator
- `crates/devix-core/manifests/manifest.schema.json`: generated
- `crates/devix-core/src/lib.rs`: re-exports

## Acceptance criteria
- [x] Every validation row in `manifest.md` is exercised by a test.
- [x] Generated schema validates the example top-level manifest.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — schema gen full close

- `schemars` v0.8 dependency added to `devix-protocol`.
- Custom `JsonSchema` impls land next to each canonical-string
  type's serde impl: `Path` (path.rs), `Chord` (input.rs), `Color`
  (view.rs), `ProtocolVersion` (protocol.rs). Each renders as a
  `{ "type": "string" }` schema with a description string.
- `JsonSchema` derive added to `Manifest`, `Engines`,
  `Contributes`, `CommandSpec`, `KeymapSpec`, `PaneSpec`,
  `ThemeSpec`, `SettingSpec`, `SettingValue`, `SubscriptionSpec`,
  `Style`, `SidebarSlot`, `PulseFilter`, `PulseKind`, `PulseField`.
- `pub fn manifest_json_schema() -> serde_json::Value` returns the
  generated schema; re-exported from the crate root.
- New example `crates/devix-protocol/examples/dump_manifest_schema.rs`
  emits the schema to stdout. The committed copy lives at
  `crates/devix-core/manifests/manifest.schema.json` (524 lines).
- Test: `manifest_json_schema_round_trips_serde_value` asserts the
  schema has the expected `definitions` (Path, Chord, Color,
  ProtocolVersion, every manifest type, the transitive view +
  pulse types). `PulseFilter` is omitted from the assertion list
  because `#[serde(flatten)]` in `SubscriptionSpec` causes
  schemars to inline its fields rather than emit a separate
  definition.

## Spec references
- `docs/specs/manifest.md` — *Rust types*, *Top-level schema*,
  *Validation*, *Manifest discovery*, *Resolved during initial review*.
- `docs/specs/foundations-review.md` — *Gate T-23*.
