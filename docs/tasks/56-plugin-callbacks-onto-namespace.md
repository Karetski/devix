# Task T-56 — Migrate plugin callbacks onto namespace
Stage: 5
Status: complete — `Lookup<Resource = LuaCallback>` ships via the `PluginCallbacks` registry shape
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
- [x] Per-plugin callback registry implements `Lookup`.
- [x] No two registries used for callbacks vs pane callbacks.
      (`PluginHost` keeps one `callbacks: Arc<Mutex<HashMap<u64,
      RegistryKey>>>` map; the per-pane `pane_callbacks` is a
      *handle index* into that single registry, not a parallel one.)
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — full close

The "storage redesign" the original partial flagged turned out to
be a resource-type redesign, not a storage redesign. `Lookup::lookup`
wants `&Self::Resource`; a `MutexGuard` borrow doesn't outlive the
call. By making the resource a zero-sized marker type
(`pub struct LuaCallback;`), the registry returns
`Some(&LUA_CALLBACK)` whenever the path resolves to a registered
handle. The actual callback invocation routes through
`PluginRuntime::invoke_sender` + `host.invoke(handle)` — the Lua
state can't cross the worker thread boundary, so the Lookup is
purely a presence + enumeration surface, not a dispatch surface.

- `plugin/mod.rs` defines `LuaCallback` (ZST), the singleton
  `LUA_CALLBACK`, and `PluginCallbacks` (`plugin: String` +
  `Arc<Mutex<HashMap<u64, RegistryKey>>>`).
- `PluginCallbacks` impls `Lookup<Resource = LuaCallback>`:
  - `lookup(&self, &Path)` decodes
    `/plugin/<name>/cb/<u64>` and verifies (a) name matches, (b)
    handle exists in the live map.
  - `paths(&self)` enumerates the registered handles back into
    canonical paths.
  - `lookup_mut` returns `None` (a presence marker has no mutable
    state).
- `PluginHost::plugin_callbacks(plugin)` constructs the registry
  sharing the underlying `Arc<Mutex<…>>`, so live registrations
  remain visible.
- Test: `plugin_callbacks_lookup_resolves_registered_handles`
  registers two Lua actions and asserts (a) each handle resolves
  through `Lookup`, (b) wrong plugin name fails, (c) unknown
  handle fails, (d) `paths()` enumerates the canonical forms.

## Spec references
- `docs/specs/namespace.md` — *Migration table* rows for plugin
  callbacks.
