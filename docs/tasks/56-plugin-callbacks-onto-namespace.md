# Task T-56 — Migrate plugin callbacks onto namespace
Stage: 5
Status: pending
Depends on: T-30
Blocks:     T-57, T-110, T-111

## Goal
Per-plugin Lua callbacks live at `/plugin/<name>/cb/<u64>`. The
existing per-plugin callback registry becomes a
`Lookup<Resource = LuaCallback>` mounted under each plugin's
namespace root.

## In scope
- `PluginCallbacks: Lookup<Resource = LuaCallback>` per plugin.
- Both `callbacks(u64)` and `pane_callbacks(u64)` (per
  `namespace.md` *Migration table*) consolidate into the single
  `/plugin/<name>/cb/<u64>` registry.
- Replace bare `u64` handles passed across the Lua boundary with
  `Path`s where ergonomics permit; otherwise keep a thin typed
  wrapper that round-trips to / from the path.

## Out of scope
- Manifest-driven plugin loading (T-110).
- Plugin-contributed pane registration (T-111).

## Files touched
- `crates/devix-core/src/plugin/mod.rs` (and `bridge.rs`,
  `pane_handle.rs` once Stage 8 splits these out)

## Acceptance criteria
- [ ] Per-plugin callback registry implements `Lookup`.
- [ ] No two registries used for callbacks vs pane callbacks.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Migration table* rows for plugin
  callbacks.
