# Task T-33 — Manifest skeleton + reader + JSON Schema
Stage: 3
Status: pending
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
- JSON Schema generation via `schemars` (closing `manifest.md` Q5)
  emitted at build time or via `cargo run -p devix-core
  --bin gen-schema`. Schema written to
  `crates/devix-core/manifests/manifest.schema.json`.
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
- [ ] Every validation row in `manifest.md` is exercised by a test.
- [ ] Generated schema validates the example top-level manifest.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/manifest.md` — *Rust types*, *Top-level schema*,
  *Validation*, *Manifest discovery*, *Resolved during initial review*.
- `docs/specs/foundations-review.md` — *Gate T-23*.
