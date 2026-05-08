# Task T-56 — Migrate plugin callbacks onto namespace
Stage: 5
Status: complete (Lookup consolidation deferred — see Scope adjustment)
Depends on: T-30
Blocks:     T-57, T-110, T-111

## Goal
Per-plugin Lua callbacks live at `/plugin/<name>/cb/<u64>`. The
existing per-plugin callback registry becomes a
`Lookup<Resource = LuaCallback>` mounted under each plugin's
namespace root.

## Scope adjustment

Full `Lookup<Resource = LuaCallback>` consolidation is deferred to
T-110 / T-111 (per foundations-review amendment 2026-05-07). The
plugin host's two callback-related maps (`callbacks: Arc<Mutex<
HashMap<u64, RegistryKey>>>` and `pane_callbacks: Arc<Mutex<HashMap<
u64, PaneCallbackKeys>>>`) are not actually two parallel registries —
`pane_callbacks` is a per-pane *index* into the single `callbacks`
registry. Implementing `Lookup` on the registry as-is fights the
`Arc<Mutex<...>>` lifetime (the trait wants `&Resource`; locking
the mutex doesn't outlive the lock). Storage redesign waits for
manifest-driven plugin loading where the API becomes load-bearing.

T-56 ships the namespace-level path encoding so producers and
consumers agree on the wire form today.

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
