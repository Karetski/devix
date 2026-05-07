# devix — Crate layout spec

Status: working draft. Stage-0 foundation T-06.

## Purpose

Source-of-truth for the Stage-1 crate split. Defines the five-crate layout,
what moves into each, public API per crate, the dependency graph, and
file-level migration mapping.

## Scope

This spec covers:
- Crate boundaries and naming.
- File-level migration (every existing source file → its destination).
- Public API per crate.
- Dependency graph (no cycles).
- Cargo workspace structure.

This spec does **not** cover:
- Cargo feature flags (mechanical; spec-driven).
- Build / packaging concerns.
- Per-file refactor steps inside a crate (those are Stage-7+ tasks).

## Layout

```
devix/
├── Cargo.toml                      (workspace)
└── crates/
    ├── devix-text       (existing) text/buffer/selection/transaction
    ├── devix-syntax     (existing) tree-sitter wrapper
    ├── devix-protocol   (NEW)      pure data + serde, the contract
    ├── devix-core       (NEW)      engine; absorbs devix-editor + devix-plugin
    │                               + most of devix-panes
    └── devix-tui        (renamed from devix-app) terminal client; absorbs
                                    widget code + layout primitives from
                                    devix-panes
```

`devix-panes`, `devix-editor`, `devix-plugin`, and `devix-app` are dissolved
during Stage 1 (T-10/T-11/T-12). Their contents redistribute as documented
below.

The five-crate layout is final for v0; further splits (a separate
`devix-lsp`, a separate `devix-runtime`-supervisor) wait until they earn
their boundary by an actual cross-cutting need.

## Dependency graph

```
                    devix-text  ──┐
                                  │
                    devix-syntax  ┤
                                  │
                    devix-protocol ◄─ depends on devix-text, devix-syntax
                                  │
        devix-core ───────────────┤  depends on devix-protocol,
                                  │  devix-text, devix-syntax
                                  │
        devix-tui  ───────────────┘  depends on devix-core, devix-protocol
```

No cycles. The crates with smallest fan-in (`devix-text`, `devix-syntax`)
are the most stable; `devix-tui` is the leaf and changes most.

`devix-protocol` is special: pure data + serde, dependency footprint as
small as possible. Plugin authors building third-party crates that produce
or consume devix manifests / protocol messages will depend on
`devix-protocol` alone, never on `devix-core`.

## devix-text (unchanged)

Public API: `Buffer`, `Selection`, `Range`, `Transaction`, `Change`,
`replace_selection_tx`, `delete_range_tx`, `delete_each_tx`.

Dependencies: `ropey`, `anyhow`, `thiserror`.

No changes during the refactor. The crate stays as-is; it's the model of
what a clean substrate crate looks like.

## devix-syntax (unchanged)

Public API: `Highlighter`, `HighlightSpan`, `Language`,
`input_edit_for_range`.

Dependencies: `tree-sitter`, `tree-sitter-rust`, `streaming-iterator`,
`anyhow`, `ropey`.

No changes during the refactor (Stage 6 turns `Highlighter` use into a
supervised actor in `devix-core`, but the crate's API stays the same).

## devix-protocol (NEW)

**Pure data + serde. No business logic, no I/O, no traits with
non-trivial bodies (only marker traits and handle traits).**

Internal modules:
- `path`: `Path`, `PathError`, `Lookup` trait — namespace.md.
- `pulse`: `Pulse`, `PulseKind`, `PulseField`, `PulseFilter`,
  `SubscriptionId` — pulse-bus.md *types*. The `PulseBus`
  *implementation* is in `devix-core` because it owns mutex-bounded
  state and a thread-aware queue.
- `protocol`: `Envelope`, `ProtocolVersion`, `Capability`,
  `ClientHello`, `ServerWelcome`, `PluginHello`, `PluginWelcome`,
  `ClientToCore`, `CoreToClient`, `PluginToCore`, `CoreToPlugin`,
  `Request`, `Response`, `RequestError`, `ProtocolError`,
  `FrontendHandle`, `CoreHandle`, `PluginHandle` (handle traits).
- `manifest`: `Manifest`, `Engines`, `Contributes`, `CommandSpec`,
  `KeymapSpec`, `PaneSpec`, `ThemeSpec`, `SettingSpec`,
  `SubscriptionSpec`, `ThemeStyle` — manifest.md.
- `view`: `View`, `ViewNodeId`, `TextSpan`, `WrapMode`, `Color`,
  `NamedColor`, `Style`, `Anchor`, `AnchorEdge`, `PopupChrome`,
  `TabItem`, `SidebarSlot`, `CursorMark`, `SelectionMark`,
  `GutterMode`, `TransitionHint`, `TransitionKind`,
  `Axis` — frontend.md.
- `input`: `InputEvent`, `MouseKind`, `MouseButton`, `Modifiers`,
  `Chord`, `KeyCode` — frontend.md.

Re-exports `HighlightSpan` from `devix-syntax`.

Public API: every type listed above.

Dependencies: `serde`, `serde_json`, `thiserror`, `devix-syntax`
(for re-exporting `HighlightSpan`). **Not** `devix-text`: the protocol's
position-bearing types (`CursorMark`, `SelectionMark`) carry only
`u32` line/col, never `Range` / `Selection`. Buffer ↔ position
translation is a `devix-core` concern.

This crate is the *contract*. It must stay small and stable; every type
in it should be ready to ship to crates.io as a public API for plugin
authors.

## devix-core (NEW)

The engine. All business logic. **No `ratatui`, no `crossterm`.** Owns
all model state, all command handlers, the plugin host, the pulse bus
implementation, the manifest loader, the theme registry.

Internal modules:
- `core`: top-level `Core` struct; entry point. Holds every owner.
- `editor`: `Editor` (the layout tree + focus + modal slot, post-Stage-9
  collapsed into a single Pane tree).
- `document`: `Document`, `DocStore` (the buffer registry; implements
  `Lookup<Resource = Document>`).
- `cursor`: `Cursor`, `CursorStore` (analogous).
- `pane`: `Pane` trait, `Action` trait, walk helpers (`pane_at`,
  `focusable_at`, `focusable_leaves`).
- `commands`: `CommandRegistry`, `Keymap`, command dispatch context.
- `theme`: active `Theme`, scope-resolution helpers.
- `manifest_loader`: reads `manifest.json`, validates, registers
  contributions into the registries above.
- `bus`: `PulseBus` implementation (the queue, subscriber storage,
  drain, depth tracking).
- `plugin`: `PluginRuntime`, `PluginHost`, sandbox, Lua marshaling.
- `supervise`: actor supervision (Stage 8 — tree-sitter worker, future
  LSP client).

Public API:
- `Core` (top-level), `CoreBuilder`.
- `Editor`, `Document`, `Cursor` data types.
- `Pane`, `Action`, `EditorCommand` traits.
- `CommandRegistry`, `Keymap`, `Theme` registries.
- `PluginRuntime`.
- `PulseBus` (the impl; types live in protocol).
- `Lookup`-implementing stores: `DocStore`, `CursorStore`, etc.

Built-in manifest: `crates/devix-core/manifests/builtin.json`, embedded
via `include_str!`.

Dependencies: `devix-protocol`, `devix-text`, `devix-syntax`, `mlua`,
`nucleo-matcher`, `notify`, `serde_json`, `tokio`, `slotmap`, `anyhow`,
`unicode-segmentation`.

## devix-tui (renamed from devix-app)

The terminal client. The **only** crate that uses `ratatui` or
`crossterm`.

Internal modules:
- `app`: `App`, the binary's main; constructs `Core`, attaches itself,
  drives the loop.
- `frontend`: `TuiFrontend` — implements `FrontendHandle`; receives
  `CoreToClient` messages.
- `interpreter`: walks `View` IR and emits ratatui calls.
- `layout`: `LinearLayout`, `UniformLayout`, `CollectionPass`,
  `VRect`, `CellGeometry`, scroll math (moved from
  `crates/panes/src/widgets/layout.rs`).
- `widgets`: ratatui-specific widget code for each `View` variant
  (`tabstrip.rs`, `palette.rs`, `popup.rs`, `sidebar.rs`).
- `input`: crossterm event → `InputEvent` translation.
- `input_thread`: dedicated input poll thread (today's `app/src/input.rs`).
- `clipboard`: `arboard` integration; the only place an OS clipboard
  is touched.

Public API:
- `App` (the binary's entry point).
- `TuiFrontend` (so a future test harness can drive a `Core` against a
  TUI without spawning a real terminal).

Dependencies: `devix-protocol`, `devix-core`, `ratatui`, `crossterm`,
`arboard`, `anyhow`. **Not** `tokio`: the plugin runtime that needs
tokio is internal to `devix-core`; `devix-core`'s public API hides
tokio types behind `devix-protocol` traits and channel-shaped wrappers,
so `devix-tui` never touches a tokio handle directly. (T-21 enforces
this by wrapping `tokio::sync::mpsc::UnboundedSender` behind a
`devix-protocol`-defined trait.)

Binary: `devix` (the user-facing executable).

## File-level migration

Every existing `.rs` file's destination. Stage-1 task T-10 through T-13
execute this mapping.

### `crates/text/**`

Stays as `crates/devix-text/**`. No changes.

### `crates/syntax/**`

Stays as `crates/devix-syntax/**`. No changes.

### `crates/panes/src/**`

| Source | Destination | Notes |
|---|---|---|
| `pane.rs` | `devix-core/src/pane.rs` | `Pane` trait |
| `action.rs` | `devix-core/src/action.rs` | `Action` trait |
| `event.rs` | (deleted) | `InputEvent` lives in `devix-protocol` |
| `clipboard.rs` | `devix-core/src/clipboard_trait.rs` | trait stays in core; impl in tui |
| `composites.rs` | (Stage-9 dissolved) | `TabbedPane`/`SidebarSlotPane` collapse into the unified Pane tree |
| `walk.rs` | `devix-core/src/pane_walk.rs` | `pane_at`, `focusable_at`, `focusable_leaves` |
| `geom.rs` | `devix-protocol/src/view.rs` | `Anchor`, `AnchorEdge` go to view module |
| `layout_geom.rs` | split: `Axis`, `SidebarSlot` → `devix-protocol/src/view.rs`; `split_rects` → `devix-tui/src/layout.rs` |
| `theme.rs` | `devix-core/src/theme.rs` | `Theme` (active state); `Style`/`Color` types in protocol |
| `widgets/mod.rs` | (deleted) | re-export consolidation |
| `widgets/layout.rs` | `devix-tui/src/layout.rs` | TUI virtualization primitives |
| `widgets/popup.rs` | `devix-tui/src/widgets/popup.rs` | renderer for `View::Popup` |
| `widgets/palette.rs` | `devix-tui/src/widgets/palette.rs` | renderer for `View::Modal` (palette case) |
| `widgets/sidebar.rs` | `devix-tui/src/widgets/sidebar.rs` | renderer for `View::Sidebar` |
| `widgets/tabstrip.rs` | `devix-tui/src/widgets/tabstrip.rs` | renderer for `View::TabStrip` |
| `lib.rs` | (deleted) | crate dissolved |

### `crates/editor/src/**`

The whole crate moves into `devix-core/src/editor/**` with internal
restructuring deferred to Stage 7 (LayoutNode collapse) and Stage 8
(Editor split into owners).

| Source | Destination |
|---|---|
| `lib.rs` | `devix-core/src/editor/mod.rs` |
| `editor.rs` | `devix-core/src/editor/editor.rs` (Stage 8 will split this further) |
| `editor/focus.rs` | `devix-core/src/editor/focus.rs` |
| `editor/hittest.rs` | `devix-core/src/editor/hittest.rs` (most logic moves to devix-tui in Stage 9) |
| `editor/ops.rs` | `devix-core/src/editor/ops.rs` |
| `tree.rs` | `devix-core/src/editor/tree.rs` (collapsed in Stage 9) |
| `frame.rs` | `devix-core/src/editor/frame.rs` |
| `document.rs` | `devix-core/src/document.rs` |
| `cursor.rs` | `devix-core/src/cursor.rs` |
| `buffer.rs` | `devix-core/src/render/buffer.rs` (renames; `EditorPane` becomes a Pane impl that emits `View::Buffer`) |
| `commands/**` | `devix-core/src/commands/**` |

### `crates/plugin/src/**`

`lib.rs` moves to `devix-core/src/plugin/mod.rs` with internal modules
split out: `host.rs` (Lua VM), `runtime.rs` (thread + channels), `bridge.rs`
(Lua ↔ Pulse marshaling), `pane_handle.rs` (LuaPaneHandle).

### `crates/app/src/**`

| Source | Destination |
|---|---|
| `main.rs` | `devix-tui/src/main.rs` |
| `lib.rs` | `devix-tui/src/lib.rs` |
| `application.rs` | `devix-tui/src/app.rs` |
| `context.rs` | (dissolved into `Core`) |
| `events.rs` | `devix-tui/src/input.rs` (translates `crossterm::Event` → `InputEvent`) |
| `effect.rs` | (dissolved; `Effect` collapses into `Pulse`) |
| `event_sink.rs` | (dissolved; `EventSink` replaced by `PulseBus`) |
| `render.rs` | `devix-tui/src/interpreter.rs` |
| `input.rs` | `devix-tui/src/input_thread.rs` |
| `clipboard.rs` | `devix-tui/src/clipboard.rs` |

## Public API per crate (summary)

| Crate | Top-level pub | Dep on |
|---|---|---|
| `devix-text` | `Buffer`, `Selection`, `Range`, `Transaction`, `Change`, transaction-builder fns | (external only) |
| `devix-syntax` | `Highlighter`, `HighlightSpan`, `Language`, `input_edit_for_range` | (external only) |
| `devix-protocol` | `Path`, `Lookup`, `Pulse*`, `PulseFilter`, `Envelope`, `Capability`, `ClientToCore`, `CoreToClient`, `PluginToCore`, `CoreToPlugin`, `Request`, `Response`, `Manifest`, `View`, `ViewNodeId`, `Style`, `Color`, `InputEvent`, `Chord`, handle traits | text, syntax |
| `devix-core` | `Core`, `Editor`, `Document`, `Cursor`, `Pane`, `Action`, `CommandRegistry`, `Keymap`, `Theme`, `PluginRuntime`, `PulseBus`, `DocStore`, `CursorStore`, `EditorCommand` | text, syntax, protocol |
| `devix-tui` | `App`, `TuiFrontend` | core, protocol |

## Cargo workspace

Workspace `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
version = "0.1.0"
rust-version = "1.80"

[workspace.dependencies]
# external
anyhow = "1"
thiserror = "1"
ropey = "1.6"
unicode-segmentation = "1.12"
ratatui = "0.29"
crossterm = "0.28"
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
streaming-iterator = "0.1"
nucleo-matcher = "0.3"
notify = "6"
arboard = "3"
mlua = { version = "0.10", features = ["lua54", "vendored", "send"] }
tokio = { version = "1", features = ["rt-multi-thread", "process", "io-util", "sync", "macros"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
slotmap = "1"

# internal
devix-text     = { path = "crates/devix-text" }
devix-syntax   = { path = "crates/devix-syntax" }
devix-protocol = { path = "crates/devix-protocol" }
devix-core     = { path = "crates/devix-core" }
```

(The `mlua` `send` feature is required because `PluginHost` is held
across thread boundaries inside `devix-core`'s supervisor; today's
`crates/plugin/Cargo.toml` already enables it.)

Each crate's `Cargo.toml` consumes via `dep = { workspace = true }`.

Build:
- `cargo build` — full workspace.
- `cargo run` — `devix-tui::main` (the `devix` binary).
- `cargo test` — every crate's tests.

## Stage-1 sequencing

The crate split is itself sequenced — files don't all move at once.
Stage 1 has four tasks (T-10 through T-13):

- **T-10**: Create `devix-protocol`. Move pure-data types into it.
  No type changes; just a relocation. Workspace builds.
- **T-11**: Create `devix-core`. Move editor + plugin + most of panes
  into it. Add explicit `pub use` re-exports so `devix-app` (still
  named) keeps importing the same paths. Workspace builds.
- **T-12**: Rename `devix-app` → `devix-tui`. Move ratatui-specific
  widget code from old `devix-panes/widgets/` into it. Move
  `widgets/layout.rs` into it. Move `clipboard` `arboard` impl into
  it. Workspace builds.
- **T-13**: Regression gate. Every existing test passes; the binary
  launches a working editor end-to-end; no behavior change visible to
  a user. Stage 1 complete.

After T-13, every later stage (mechanical wins, foundation skeletons,
namespace migration, etc.) executes on the new crate layout.

## Interaction with other Stage-0 specs

- **`namespace.md`**: `Path` and `Lookup` live in `devix-protocol`; the
  `Lookup`-implementing registries (`DocStore`, `CommandRegistry`, etc.)
  live in `devix-core`.
- **`pulse-bus.md`**: types in `devix-protocol`, implementation
  (`PulseBus`) in `devix-core`.
- **`protocol.md`**: every type in `devix-protocol`. `FrontendHandle`,
  `CoreHandle`, `PluginHandle` traits also in protocol so consumers
  (`devix-tui`, plugin-side, future LSP) reach for the same surface.
- **`manifest.md`**: schema types in `devix-protocol`. Loader and
  validator in `devix-core`. Built-in manifest data file lives in
  `devix-core/manifests/builtin.json`.
- **`frontend.md`**: View IR, InputEvent, Style, Color in
  `devix-protocol`. TUI interpreter in `devix-tui`.

## Open questions

1. **Crate names.** `devix-core` vs `devix-engine` vs `devix-runtime`.
   Lean: `devix-core` — shortest, mirrors VS Code's "core" naming, and
   "runtime" already implies the supervised-actor runtime sub-system.

2. **`Pane`/`Action` trait location.** Both have behavior (not pure data),
   so they live in `devix-core`. Plugin authors implementing custom
   `Pane`s in third-party crates therefore need a `devix-core` dep, not
   `devix-protocol` alone. Acceptable? Or split `Pane` into a data-only
   `PaneSpec` (in protocol) and a renderer trait (in core)? Lean: keep
   `Pane` in core; if we ever ship plugins as external crates, we'll
   revisit.

3. **`Theme` location.** `ThemeSpec` (declarative manifest entry) lives
   in `devix-protocol` for serde. `Theme` (active state, scope lookup
   table) lives in `devix-core`. Confirm this split.

4. **`devix-protocol` on crates.io.** Eventually publish it so plugin
   authors can `cargo add devix-protocol` and write strongly-typed
   manifest tools or type-safe RPC clients without depending on the
   editor proper. Not in v0; flag as future.

5. **Workspace member naming.** The directory is `crates/<name>` today
   but the crate name in `[package]` doesn't have to match. Match them
   for clarity (`crates/devix-core/Cargo.toml` declares
   `name = "devix-core"`). Confirm.

## Resolved during initial review

- `mlua` is an unconditional dependency of `devix-core`. No Cargo
  feature gate, no separate `devix-plugin-lua` crate. Single boundary
  for plugin code; revisit if/when a non-Lua plugin runtime ships
  (then a feature gate or sub-crate makes sense).