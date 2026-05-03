# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

devix is a TUI coding IDE in Rust — tree-sitter + LSP + (planned) Lua plugin host. The phased plan, scope rationale, gotchas, and architectural rules live in `PLAN.md`; read it before any non-trivial work. Reference editors are Helix and Zed; ox is reading material, not a fork target.

## Common commands

```sh
cargo check --workspace        # type-check everything
cargo test --workspace         # run all tests (~136 currently)
cargo test -p devix-buffer     # tests for one crate
cargo test -p devix-buffer transaction_undo_redo   # one test by name
cargo run --bin devix -- path/to/file.rs           # run the editor
cargo run --release --bin devix -- ...             # release build
```

`Cargo.lock` is checked in. The release profile uses `lto = "thin"` + `codegen-units = 1`, so release builds are noticeably slower than dev.

## Crate graph

```
config ←────────── ui ←──────┐
buffer ←──┬─ document ←─ workspace ←─ views ←─ app
syntax ←──┤                ↗
lsp ←─────┘
```

The graph is intentional and enforced by `Cargo.toml` deps. Layering inversions have bitten us three times; before adding a `devix-*` dep, sanity-check the direction.

| Crate | Responsibility | Notable deps |
|---|---|---|
| `buffer` | ropey rope + multi-region `Selection` + `Transaction` (apply/undo/redo) + grapheme cursor + word motions + reload-from-disk helpers | ropey, unicode-segmentation |
| `syntax` | tree-sitter wrapper. `Highlighter` owns parser + tree + compiled highlights query, driven incrementally by buffer transactions. `HighlightSpan` is the renderer-facing output | tree-sitter, tree-sitter-rust |
| `lsp` | JSON-RPC client over child stdio. `Coordinator` runs one client per `(workspace_root, language)`, takes `LspCommand` in and emits `LspEvent` out. `translate.rs` converts buffer edits to LSP content-change events | lsp-types, tokio, serde_json |
| `config` | `Theme` only. Keymap and built-in command tables live in `workspace` because they're inherently coupled to `Action` / `CommandRegistry` | ratatui |
| `document` | `Document` = buffer + path + per-doc syntax highlighter + per-doc LSP wiring + filesystem watcher. Self-contained model layer | buffer, syntax, lsp, notify, tokio, lsp-types, slotmap |
| `workspace` | Layout tree (`Node`/`Frame`), `View`, `Action` enum + `dispatch`, `CommandRegistry`, `Keymap`, palette/symbols overlay state, `Workspace` orchestration. **Has zero view-layer deps** — scroll offsets are stored as `(u32, u32)`, hit-test outputs as a workspace-local `TabHit`. Re-exports `Document`/`DocId`/`DocDiagnostic` from `devix-document` | document, buffer, syntax, lsp, crossterm, tokio, nucleo-matcher, arboard |
| `ui` | **Pure** ratatui widgets — `tabstrip`, `popup`, `status`, `sidebar`. No awareness of workspace/buffer/lsp types. Hosts the `layout` module: `UICollectionView`-style primitives (`CollectionLayout`, `CollectionPass`, `LinearLayout`, `UniformLayout`, `VRect`) plus free scroll-math functions (`scroll_by`, `set_scroll`, `ensure_visible`) operating on `&mut (u32, u32)` so the model side can hold scroll as plain data | config |
| `views` | Workspace-coupled render functions — `editor`, `palette`, `symbols`. Take state from `workspace` and paint with `ui` widgets | ui, workspace, buffer, syntax, lsp-types, ropey, crossterm |
| `app` | Binary. Event loop, terminal lifecycle, LSP wiring, file-watch reconciliation, render orchestration | everything above |
| `plugin` | Empty stub for the future mlua host (Phase 8) | none |

## The three architectural rules from `PLAN.md`

These are load-bearing. Don't break them without explicit discussion.

1. **Buffer mutations are transactions.** Every edit produces a `Transaction { changes, selection_before, selection_after }`. Undo replays inverse transactions. LSP and tree-sitter consume the same stream. The transaction is the source of truth — don't mutate the rope directly past `buffer`'s API.
2. **Layout is a recursive tree.** Splits, panes, and side panels are all `Node`s — sidebars are just panes pinned to an edge with a toggle. Don't introduce hardcoded "left panel" / "right panel" as separate concepts.
3. **Render is pure.** `render(state, frame)` only reads state and writes to `RenderCache`. State mutations happen exclusively in the event-pump phase. `crates/app/src/render.rs` enforces this with a `layout_pass` (mutating, called first) and `paint` (read-only) split — keep it that way.

## Responsiveness commitments

Also from `PLAN.md`. Treat as hard constraints:

- No blocking I/O on the render thread. File open/save, LSP requests, plugin calls all dispatch to tokio tasks; results return via `tokio::sync::mpsc` channels that the event loop drains each tick.
- Frame budget is 16ms. Anything heavier (tree-sitter on huge files, fuzzy match over a large symbol set, LSP response handling) runs off-thread and streams results.
- Keystrokes always preempt pending render or background work.
- External file changes reflect on the next frame. If buffer is clean, swap contents in. If dirty, surface a non-blocking reload prompt — never desync silently, never block.

## UI design philosophy

The split between `ui` and `views` is modeled on UIKit. `ui` is the pure design system: layout/virtualization primitives (`ui::layout`, modeled on `UICollectionView`) plus widgets (`tabstrip`, `popup`, `status`, `sidebar`). `views` is feature-coupled renderers that pull from `workspace` state. Adding a new widget: if it can be expressed without touching `workspace`/`buffer`/`lsp` types, it belongs in `ui`. Otherwise, `views`.

**Critical rule:** the workspace model never names a view type. Scroll positions are `(u32, u32)`; tab-strip hit-test caches use a workspace-local `TabHit`. Layout-aware scroll mutation (`ensure_visible`, etc.) lives in `ui::layout` as free functions that take `&mut (u32, u32)` — render code calls them; the model just stores the offsets. Don't reintroduce shared "model+view primitive" types — that's what made the old `collection` crate a layering hazard.

## When extending the action surface

A new command typically touches three places: `Action` variant in `workspace::action`, a match arm in `workspace::dispatch`, and an entry in `workspace::builtins` (registered command + label) and/or `workspace::keymap` (chord binding). The split between `command.rs` (registry data type) and `builtins.rs` (populated registry instances) is intentional — keep it.
