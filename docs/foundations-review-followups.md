# Foundations review — follow-up plan

Action plan for the six findings raised against the `refactor/foundations`
branch (3 High, 3 Medium). Each item lists scope, files, concrete
changes, acceptance criteria, and an estimate. Ordered by execution
sequence: safety fixes first, correctness next, architecture last.

Status legend: `pending` / `in-progress` / `complete`.

## Sequence at a glance

| # | Severity | Item | Status | Estimate |
|---|----------|------|--------|----------|
| F-1 | High | Non-blocking `publish_async` + backpressure policy | complete (2026-05-12) | ~0.5d |
| F-2 | High | Panic-safe reentrancy depth (RAII guard) | complete (2026-05-12) | ~0.25d |
| F-3 | Medium | Register plugin settings before runtime load | complete (2026-05-12) | ~0.25d |
| F-4 | Medium | Multi-plugin lifetime ownership | complete (2026-05-12) | ~0.5d |
| F-5 | Medium | Emit missing buffer/command pulses | complete (2026-05-12) | ~1d |
| F-6 | High (arch) | Remove `ratatui` / `crossterm` from `devix-core` | design note landed (2026-05-12); staged tasks pending | multi-PR |

F-1 + F-2 land first as one PR (bus safety). F-3 + F-4 are a plugin
lifecycle PR. F-5 is a pulse-contract PR with its own tests. F-6 is a
multi-stage move and gets its own design note (see §F-6 below).

---

## F-1 — Non-blocking `publish_async` + backpressure policy
Severity: High. Risk class: deadlock.

### Problem
`PulseBus::publish_async` (`crates/devix-core/src/bus.rs:134`) calls
`SyncSender::send`, which blocks on a full queue. The input thread
publishes to the bus *before* sending the wake/input message
(`crates/devix-tui/src/input_thread.rs:50`); the main loop sits in
`rx.recv()` (`crates/devix-tui/src/application.rs:112`). A full bus
therefore blocks the only producer that could wake the loop — classic
producer/consumer deadlock. The unhandled-pulse re-enqueue at
`crates/devix-tui/src/application.rs:219` rides the same bounded
queue and amplifies the risk.

### Change
1. Change `publish_async` signature:
   ```rust
   pub fn publish_async(&self, pulse: Pulse) -> Result<(), PublishError>
   ```
   where `PublishError { Full(Pulse), Disconnected }`. Implement with
   `try_send`.
2. Add an `overflow_count: AtomicU64` on `Inner`. On `Full`, drop the
   pulse, bump the counter, and stash the dropped `PulseKind` in a
   small lock-free ring (size 16) for diagnostics.
3. Document the v0 backpressure policy in
   `docs/specs/pulse-bus.md`: bounded queue, drop-newest on overflow,
   counter is observable via a `bus.overflow_snapshot()` accessor.
4. Update producers:
   - `crates/devix-tui/src/input_thread.rs:51` — drop pulse on `Full`;
     `sink.input(ev)` still wakes the main loop, so dispatch keeps
     working even when the typed observer pulse is dropped.
   - `crates/devix-tui/src/main.rs:64` (`make_msg_sink`) — drop pulse
     on `Full`; `wake_sink.wake()` still fires.
   - `crates/devix-tui/src/application.rs:219` (the `_ => publish_async`
     re-enqueue) — replace with **synchronous** `self.editor.bus.publish(pulse)`.
     The main loop already holds `&mut self`; the cross-thread queue
     was never needed here and is the exact source of the cycle.

### Files touched
- `crates/devix-core/src/bus.rs`
- `crates/devix-tui/src/input_thread.rs`
- `crates/devix-tui/src/main.rs`
- `crates/devix-tui/src/application.rs`
- `docs/specs/pulse-bus.md`

### Acceptance criteria
- [ ] `publish_async` never blocks; signature returns `Result`.
- [ ] New test: spawn a producer that publishes `capacity + 16` pulses
      without draining; main thread never blocks; overflow counter
      reads `16`.
- [ ] New test: simulated wedge — fill the queue, run one input-thread
      iteration, confirm the wake/input still arrives at the main loop
      within 200ms.
- [ ] `cargo test --workspace` green.

---

## F-2 — Panic-safe reentrancy depth (RAII guard)
Severity: High. Risk class: corrupted depth state across publishes.

### Problem
`PulseBus::publish` (`crates/devix-core/src/bus.rs:97`) increments
depth, runs handlers, decrements depth. A panicking subscriber unwinds
past the decrement; the next publish sees a stale depth and trips the
limit (or never trips it, depending on overflow).

### Change
Introduce an RAII guard:

```rust
struct DepthGuard<'a> { depth: &'a Mutex<usize> }
impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut d) = self.depth.lock() {
            *d = d.saturating_sub(1);
        }
    }
}
```

`publish` checks/bumps depth, constructs the guard, runs handlers.
Guard runs on both normal return and unwind. Existing snapshot-then-
invoke logic stays.

### Files touched
- `crates/devix-core/src/bus.rs`

### Acceptance criteria
- [ ] New test: subscriber that panics; the publish call is wrapped in
      `catch_unwind`; the *next* publish (with a non-panicking
      subscriber) succeeds and observes depth=1 inside the handler.
- [ ] `reentrancy_overflow_panics` and `depth_resets_between_publishes`
      still pass.
- [ ] `cargo test --workspace` green.

---

## F-3 — Register plugin settings before runtime load
Severity: Medium. Risk class: plugin sees `nil` for its own defaults.

### Problem
`PluginRuntime::load_supervised_with_settings`
(`crates/devix-tui/src/main.rs:125`) hands the settings store to the
Lua entry, but `register_from_manifest` runs only at
`crates/devix-tui/src/main.rs:181`. A plugin that calls
`devix.setting("my.key")` during startup
(`crates/devix-core/src/plugin/host.rs:295`) sees `nil` instead of the
manifest-declared default.

### Change
Restructure the discovery loop so each plugin's manifest is registered
*before* its runtime loads:

```rust
for manifest_path in manifests {
    let manifest = match load_manifest(&manifest_path) { ... };
    editor.theme_store.register_from_manifest(&manifest);
    editor.settings_store.lock().unwrap().register_from_manifest(&manifest);
    // now load runtime + install
    let runtime = PluginRuntime::load_supervised_with_settings(...);
    runtime.install_with_manifest(...);
}
```

Delete the second discovery pass at `main.rs:181-190`.

Move `apply_overrides_from_file(settings_overrides_path())`
(`main.rs:196`) so it runs **after manifests register** but **before
runtimes load** — plugins should observe user overrides during
startup, not just defaults. This means a two-phase loop:
1. Pass 1: parse every manifest, register settings + theme.
2. Apply user overrides.
3. Pass 2: for each manifest, load runtime + install commands/panes.

### Files touched
- `crates/devix-tui/src/main.rs`

### Acceptance criteria
- [ ] A plugin manifest with `contributes.settings` and a Lua entry that
      reads `devix.setting("<key>")` at startup observes the
      manifest-declared default (test plugin under
      `crates/devix-tui/tests/fixtures/`).
- [ ] When the user's `settings.json` overrides the key, the plugin
      observes the override during startup.
- [ ] `cargo test --workspace` green.

---

## F-4 — Multi-plugin lifetime ownership
Severity: Medium. Risk class: vague shutdown, wrong runtime for
restart events.

### Problem
`Application::plugin: Option<PluginRuntime>`
(`crates/devix-tui/src/application.rs:47`) holds at most one runtime.
Other runtimes are kept alive via `std::mem::forget`
(`crates/devix-tui/src/main.rs:218`), which:
- never drops on shutdown;
- means `PluginLoaded` restart handling at `application.rs:211` only
  calls `reinstall_panes` on the one stored runtime — not necessarily
  the one that restarted.

### Change
1. Replace the field:
   ```rust
   plugins: HashMap<String, PluginRuntime>
   ```
   keyed by manifest `name`.
2. Replace `set_plugin(rt)` with `add_plugin(name, rt)`.
3. Add `PluginRuntime::name()` accessor if not already present.
4. In `main.rs`, drop the `std::mem::forget`; every loaded runtime is
   inserted via `add_plugin`.
5. In `dispatch_typed_pulses`, on `Pulse::PluginLoaded { name, .. }`
   look up the runtime by `name` and call `reinstall_panes` on that
   one specifically.
6. `Application::run`'s shutdown path drops the map (it already drops
   `self`); confirm each runtime's worker thread exits cleanly.

### Files touched
- `crates/devix-tui/src/application.rs`
- `crates/devix-tui/src/main.rs`
- `crates/devix-core/src/plugin/runtime.rs` (or wherever
  `PluginRuntime` lives) — add `name()` if missing.

### Acceptance criteria
- [ ] `std::mem::forget` removed from `main.rs`.
- [ ] Test: load two manifest fixtures (`plug_a`, `plug_b`); send a
      `PluginLoaded { name: "plug_b" }`; only `plug_b`'s
      `reinstall_panes` runs.
- [ ] Shutdown of the application drops both runtime workers within
      `SHUTDOWN_DEADLINE`.
- [ ] `cargo test --workspace` green.

---

## F-5 — Emit missing buffer/command pulses
Severity: Medium. Risk class: typed-bus contract is not the source of
truth.

### Problem
The protocol defines `Pulse::{BufferOpened, BufferChanged, BufferSaved,
CommandInvoked}` (`crates/devix-protocol/src/pulse.rs:30..104`), but
the editor mutates state without publishing them
(`crates/devix-core/src/editor/commands/cmd/file.rs:15`,
`crates/devix-core/src/editor/editor/ops.rs:111`). `rg` only finds
those buffer pulses inside bus tests. Plugins and frontends that
subscribe see nothing.

### Change
Wire pulses at the mutation sites. The `Context` passed to actions
already carries `&mut Editor`, which owns `editor.bus`.

1. **`BufferSaved`** — `crates/devix-core/src/editor/commands/cmd/file.rs:15`
   `Save::invoke`: after `d.buffer.save()` succeeds, publish
   `Pulse::BufferSaved { path: d.path_protocol(), revision: d.buffer.revision() }`.
2. **`BufferOpened`** — `crates/devix-core/src/editor/editor/ops.rs:111`
   `open_path_replace_current`: publish on the *new Document insert*
   branch only; reuse-cached-doc branch must not double-fire.
3. **`BufferChanged`** — find the in-memory edit ops (rope insert /
   delete entry points) and publish there with the post-edit revision.
   The disk-watch path (`install_bus_watcher_for_doc`) already
   publishes for on-disk reloads; in-memory edits are missing.
4. **`CommandInvoked`** — `CommandRegistry::run` (or whatever the
   single dispatch site is). Thread an `InvocationSource` enum
   (`Keymap | Palette | Plugin | Pulse | Test`) through callers; the
   pulse carries the command id + source. Cheap to add — the source is
   already implicit at each caller.

### Files touched
- `crates/devix-core/src/editor/commands/cmd/file.rs`
- `crates/devix-core/src/editor/editor/ops.rs`
- `crates/devix-core/src/editor/document.rs` (edit ops)
- `crates/devix-core/src/editor/commands/registry.rs` (dispatch)
- callers that invoke commands (keymap, palette, plugin host)

### Acceptance criteria
- [ ] Integration test per pulse: subscribe via the bus, run the
      command, assert the pulse fires exactly once with correct fields.
- [ ] `BufferOpened` does **not** fire when opening a path that already
      has a cached Document.
- [ ] `CommandInvoked.source` matches the invocation site under test.
- [ ] No regression in existing bus tests.
- [ ] `cargo test --workspace` green.

---

## F-6 — Remove `ratatui` / `crossterm` from `devix-core`
Severity: High (architectural). Multi-PR. Tracked separately.

### Problem
`crates/devix-core/Cargo.toml:10,12` depends on `ratatui` and
`crossterm`. `crates/devix-core/src/lib.rs:58-71` re-exports
`SidebarPane`, `TabStripPane`, `render_palette`, `render_popup`, etc.
Plugin authors and any future non-TUI frontend inherit terminal-UI
concerns through `devix-core` APIs. The crate is not yet the reusable
engine the spec calls for. Existing tasks T-11 / T-92 / T-95 already
acknowledge this as transitional.

### Approach (separate design doc)
This finding does not get inlined into a normal task. Open a new design
note at `docs/specs/core-decoupling.md` that:

1. **Inventories** every `ratatui::*` / `crossterm::*` reference in
   `devix-core/src/**` (start with
   `rg "ratatui::|crossterm::" crates/devix-core/src`).
2. **Defines neutral types** in `devix-protocol` (or a new
   `devix-render` crate): `Rect`, `Style`, `Color`, `Cell`, `Span`,
   plus a `RenderSurface` trait (`set_cell`, `area`, `flush`).
3. **Stages the move**:
   - Stage A: relocate `widgets::*` to `devix-tui`. `devix-core`
     keeps the *model* (`PaletteState`, `TabInfo`, hit-test results).
   - Stage B: replace the `Frame`-typed `render(...)` methods on
     `Pane` with `paint(&self, &mut dyn RenderSurface)`.
   - Stage C: scrub the plugin host (`plugin/host.rs`) for ratatui
     leakage through `LuaPane`'s public shape.
   - Stage D: drop `ratatui` + `crossterm` from `devix-core/Cargo.toml`;
     fix the resulting compile errors.
4. **Re-numbers under existing stages**: this is the unfinished work
   of T-11 / T-92 / T-95. New sub-tasks land as `T-11a`/`T-11b` (or
   under Stage 9 if cleaner) — decided in the design note. Update
   `docs/tasks/README.md` cross-walk accordingly.

### Acceptance criteria for the design note (gates the implementation)
- [ ] Inventory committed.
- [ ] Neutral-types API sketched, with sign-off on whether they live in
      `devix-protocol` or a new `devix-render` crate.
- [ ] Stage A/B/C/D each have their own task file with `Files touched`
      and acceptance criteria.

### Acceptance criteria for the work itself (post-design)
- [ ] `cargo tree -p devix-core` shows no `ratatui`, no `crossterm`.
- [ ] `devix-core` builds against a stub `RenderSurface` impl in a unit
      test; no terminal types leak.
- [ ] `cargo build --workspace` and `cargo test --workspace` green.

---

## Rollout

- One PR per finding (F-1+F-2 bundled). Each PR cites this document.
- After F-1..F-5 merge, this file's status table moves to `complete`
  for those rows and F-6's design note begins.
- F-6 is tracked through its own task files; this doc keeps the
  pointer.
