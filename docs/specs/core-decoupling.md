# devix — `devix-core` terminal-UI decoupling

Status: design note, **not** a task. Tracks the multi-stage move that
F-6 in `docs/foundations-review-followups.md` calls for. Resolves to
4 staged tasks (A→D) with their own files under `docs/tasks/`.

## Purpose

`devix-core` is supposed to be the reusable engine that hosts any
frontend — terminal, native GUI, web, headless. Today it depends
directly on `ratatui` and `crossterm`:

- `crates/devix-core/Cargo.toml:10` — `ratatui = "0.29"`
- `crates/devix-core/Cargo.toml:12` — `crossterm = "0.28"`

…and re-exports terminal-UI types through public API
(`crates/devix-core/src/lib.rs:58-71` — `SidebarPane`, `TabStripPane`,
`render_palette`, `render_popup`, …). Any non-terminal frontend
inherits these concerns transitively, and plugin authors who depend
on `devix-core` get terminal types in their interface.

This note inventories the leakage, sketches the neutral-types API
the engine should expose, stages the move, and proposes the
re-numbering under existing T-11 / T-92 / T-95 buckets.

## §1 — Inventory

Snapshot taken **2026-05-12** after F-1..F-5 land. Re-run before
each stage commits to catch drift.

Reproduce with:

```
rg "ratatui::|crossterm::" crates/devix-core/src
```

### 1.1 Files referencing `ratatui::*` or `crossterm::*`

| File | Concern | Notes |
|------|---------|-------|
| `crates/devix-core/src/editor/buffer.rs` | ratatui | View-style mapping. |
| `crates/devix-core/src/editor/commands/keymap.rs` | crossterm | `KeyCode`, `KeyModifiers` in `Chord`. |
| `crates/devix-core/src/editor/commands/modal.rs` | crossterm + ratatui | `Frame` render + key event dispatch. |
| `crates/devix-core/src/editor/registry.rs` | ratatui | `Rect` from `layout`. |
| `crates/devix-core/src/editor/tree.rs` | ratatui | `Frame` painting. |
| `crates/devix-core/src/editor/view.rs` | ratatui + crossterm | `Frame` render, event mapping. |
| `crates/devix-core/src/event.rs` | crossterm | `pub use crossterm::event::Event` re-export. |
| `crates/devix-core/src/geom.rs` | ratatui | `pub use ratatui::layout::Rect`. |
| `crates/devix-core/src/layout_geom.rs` | ratatui | `Rect` arithmetic. |
| `crates/devix-core/src/manifest_loader.rs` | crossterm | Manifest → `Chord` parsing. |
| `crates/devix-core/src/pane.rs` | ratatui | `Pane::render(area, frame, …)` trait. |
| `crates/devix-core/src/plugin/host.rs` | ratatui | `Style` resolution for `ThemePalette`. |
| `crates/devix-core/src/plugin/mod.rs` | crossterm | Tests use `KeyCode` / `KeyModifiers`. |
| `crates/devix-core/src/plugin/pane_handle.rs` | crossterm | `KeyEvent`, `MouseButton` in handles. |
| `crates/devix-core/src/plugin/view_lua.rs` | ratatui | `Frame`-based draw of plugin View IR. |
| `crates/devix-core/src/theme.rs` | ratatui | `Style`, `Modifier`, `Color` in catalog. |
| `crates/devix-core/src/widgets/layout.rs` | ratatui | `Frame`, `Rect`. |
| `crates/devix-core/src/widgets/palette.rs` | ratatui | `Frame` + widget composition. |
| `crates/devix-core/src/widgets/popup.rs` | ratatui | `Frame` + widget composition. |
| `crates/devix-core/src/widgets/sidebar.rs` | ratatui | `Frame` + widget composition. |
| `crates/devix-core/src/widgets/tabstrip.rs` | ratatui | `Frame` + widget composition. |

Total: 21 files, ~70 matched references.

### 1.2 Surfaces by concern

Three independent leak categories:

1. **Render surface.** Every `Pane::render(area, frame, …)` and its
   widget helpers (`widgets::*`, `plugin::view_lua`). `Frame` is the
   ratatui rendering context; `Rect` is its geometry; `Style`,
   `Color`, `Modifier` are its styling vocabulary.
2. **Input.** `crossterm::event::{Event, KeyCode, KeyEvent,
   KeyModifiers, MouseButton, MouseEventKind}` flow through
   `event.rs`, `keymap.rs`, `pane_handle.rs`, and the plugin host's
   key/mouse dispatchers.
3. **Theme/style catalog.** `theme.rs` stores `ratatui::style::Style`
   directly. `plugin::host::ThemePalette` resolves to the same.
   Plugins and tests both observe this.

Concerns 1 and 3 are tightly coupled (you can't paint without a
style vocabulary). Concern 2 is separable.

## §2 — Neutral-types API

Goal: every type that crosses the engine↔frontend boundary lives in
a frontend-neutral crate. Two questions to resolve:

1. **Where do the neutral types live?**
2. **What does the engine give a frontend instead of `Frame`?**

### 2.1 Crate placement

Two reasonable homes:

| Option | Where | Pros | Cons |
|--------|-------|------|------|
| A | `devix-protocol` | One stable contract crate; already serializable; plugin authors already depend on it. | Render-surface trait isn't pure data; mixes wire types with runtime API. |
| B | new `devix-render` | Separates wire data from rendering API; keeps `devix-protocol` minimal. | New crate to maintain; cross-crate use between core and tui. |

**Recommendation:** B. The wire-data crate (`devix-protocol`) should
stay free of non-data traits (`RenderSurface` has methods that take
`&mut self`). `devix-render` becomes the engine↔frontend rendering
contract. `devix-protocol` already owns `View`, `Style`, `Color`
(see `crates/devix-protocol/src/view.rs`) — those move with the
render contract or stay co-located depending on what plugin authors
need most.

Final decision sits with the implementer of Stage A; record outcome
inline at the top of `core-decoupling.md` once committed.

### 2.2 Sketched API

```rust
// devix-render (proposed)

/// Frontend-neutral rectangle. Mirrors ratatui's shape so the tui
/// adapter is a zero-cost newtype, but doesn't name `ratatui`.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct Rect { pub x: u16, pub y: u16, pub width: u16, pub height: u16 }

/// A single styled glyph in a surface. Distinguished from
/// `protocol::view::Span` (which carries a *string* fragment + style)
/// because a render surface paints one cell at a time.
#[derive(Clone, Debug)]
pub struct Cell {
    pub symbol: String,   // grapheme cluster
    pub style: Style,
}

/// Subset of ratatui's style: fg/bg + modifier bits we already
/// support in `devix-protocol::view::Style`. The two converge — the
/// view IR's `Style` is the canonical form, the render surface
/// applies it cell-by-cell.
pub use devix_protocol::view::{Color, Modifier, Style};

/// The trait `Pane::paint` writes through. The tui crate's adapter
/// implements this around `ratatui::Frame`; future frontends pick
/// any backing surface (DOM, GPU, headless buffer).
pub trait RenderSurface {
    fn area(&self) -> Rect;
    fn set_cell(&mut self, x: u16, y: u16, cell: Cell);
    /// Optional commit hook (no-op for ratatui — `Frame::render`
    /// owns its flush). Web/GPU surfaces use this to swap buffers.
    fn flush(&mut self) {}
}
```

Input vocabulary lives next to the render surface:

```rust
// Already defined in devix-protocol::input — `InputEvent`, `Chord`,
// `KeyCode`, `Modifiers`, `MouseButton`, `MouseKind` per
// docs/specs/frontend.md. The tui crate's input thread already
// converts crossterm → InputEvent (`input_thread.rs:93`). The
// migration is: stop accepting `crossterm::event::Event` at the
// engine boundary and accept only `InputEvent`.
```

Theme/style:

```rust
// devix-protocol::view already owns `Style`, `Color`. The
// `theme.rs` ratatui::Style storage migrates to that. The tui
// renderer converts `view::Style → ratatui::Style` at the paint
// call site (already partially in place — `plugin/view_lua.rs`
// has `view_style_to_ratatui` and `view_color_to_ratatui`).
```

## §3 — Staged move

Four stages, each its own task. Land in order. Each stage compiles +
tests green on its own merge.

### Stage A — Relocate widget code to `devix-tui`

`widgets/{layout,palette,popup,sidebar,tabstrip}.rs` move to
`devix-tui/src/widgets/`. `devix-core` keeps the *model* — palette
state, tab-info catalog, hit-test results — and exposes that as
plain data. The tui crate consumes the model and paints.

Public API renames:

| Before (in `devix-core::lib`) | After |
|-------------------------------|-------|
| `SidebarPane`, `TabStripPane` | model lives in core; widget moves to `devix-tui::widgets`. |
| `render_palette`, `render_popup` | move wholesale to `devix-tui`. |
| `widgets::*` re-exports | dropped from `devix-core::lib`. |

Acceptance: `devix-core/src/widgets/` deleted; the file count there
drops to zero. `cargo test --workspace` green.

### Stage B — Replace `Pane::render(frame, …)` with `paint(surface)`

`crates/devix-core/src/pane.rs:Pane` trait swaps `render(area: Rect,
frame: &mut Frame, ctx: &LayoutCtx)` for `paint(surface: &mut dyn
RenderSurface, ctx: &LayoutCtx)`. The tui crate provides a
`FrameSurface(&'a mut ratatui::Frame)` adapter that implements
`RenderSurface`.

The same flip applies to:
- `widgets::*::render` callers
- `editor/tree.rs` recursive pane render
- `plugin/view_lua.rs`'s View IR painter
- `editor/buffer.rs`'s buffer renderer

This is the largest stage by line count. A single-PR cutover is
realistic because the trait signature change forces every implementor
to migrate in one move.

Acceptance: `Frame` no longer named under `devix-core/src/`.
`cargo test --workspace` green.

### Stage C — Scrub plugin host

`crates/devix-core/src/plugin/host.rs:310-365` resolves
`ThemePalette` against ratatui's `Style` and color enum. The Lua
bridge surfaces no ratatui types to scripts, but the host's internal
table types still do. Replace with `devix_protocol::view::Style` /
`Color` (already serializable; what plugins observe).

Touch points:
- `plugin/host.rs` resolves `ThemePalette` against `view::Style`
- `plugin/pane_handle.rs` shapes `LuaPane` against `InputEvent` only
  (not raw `crossterm::KeyEvent`)
- `plugin/view_lua.rs` paints View IR through `RenderSurface`

Acceptance: `crossterm` and `ratatui` removed from
`plugin/mod.rs`-level tests; tests construct `InputEvent` directly.
`devix-core`'s plugin module references neither crate.

### Stage D — Drop `ratatui` / `crossterm` from `devix-core/Cargo.toml`

Mechanical: `cargo build -p devix-core` after stages A–C should
already pass once references are gone. Stage D removes the
manifest entries and resolves any straggling compile errors. The
tui crate inherits both dependencies from its own `Cargo.toml`.

Acceptance:
- `cargo tree -p devix-core` shows no `ratatui`, no `crossterm`.
- `devix-core` builds against a stub `RenderSurface` impl in a unit
  test (proves the surface trait is enough; no terminal-typed
  leakage through the engine API).
- `cargo build --workspace` and `cargo test --workspace` green.

## §4 — Task renumbering

This work is the long-tail of T-11 (Stage-1 crate split), T-92
(layout cache carve-out), and T-95 (paint_view). All three stage
checkpoints flagged it as transitional and explicitly deferred to a
follow-up. The cleanest spot is Stage 11 (plugin contributions) +
Stage 9 (layout/rendering) — Stage A is plugin-adjacent (widget
relocation that the plugin host depends on), Stages B–D are
core-rendering hygiene.

Proposed numbering:

| Stage | New task | Bucket |
|-------|----------|--------|
| A | `docs/tasks/115-stage-a-widgets-to-tui.md` | Stage 11 follow-up to T-111 (plugin panes). |
| B | `docs/tasks/95a-paint-surface-trait.md` | Stage 9 follow-up to T-95. |
| C | `docs/tasks/115a-plugin-host-neutral-types.md` | Stage 11 follow-up. |
| D | `docs/tasks/95b-drop-terminal-deps-from-core.md` | Stage 9 close-out. |

`docs/tasks/README.md` cross-walk gets these entries when Stage A's
task file lands.

## §5 — Risks / open questions

1. **`Rect` shape divergence.** ratatui's `Rect` carries derived
   helpers (`area`, `inner`, `split`) we lean on. The neutral
   `Rect` either ports them (one file of code) or the layout
   primitives live in `devix-render` and import the neutral `Rect`
   from there. Decide during Stage A.
2. **Plugin author churn.** Plugin authors who happened to pull
   re-exported `SidebarPane` from `devix-core` need a migration
   note. Realistically the only consumer is the built-in manifest.
3. **`Style` collision.** `devix-protocol::view::Style` already
   exists. The Stage B work consolidates on it; storing
   `ratatui::Style` in the theme catalog goes away. The tui crate
   converts at the paint call site (the conversion is already
   present in `view_lua.rs:287` — generalize it).
4. **Order of A vs B.** A is independently mergeable; B has more
   ripple. A first reduces B's scope. Document the order in
   Stage A's task file so subsequent reviewers don't reorder.

## §6 — Acceptance for *this design note*

(Gates the implementation work.)

- [x] Inventory committed (§1).
- [x] Neutral-types API sketched (§2). Crate-placement recommendation
      is B (`devix-render`); final decision deferred to Stage A.
- [x] Stages A/B/C/D each have proposed task numbers and per-stage
      acceptance criteria (§3, §4).
- [ ] Task files land under `docs/tasks/` when their stage opens —
      not now. This note is the spec.

## §7 — Pointer

This note is the source of truth for F-6 in
`docs/foundations-review-followups.md`. When Stage A opens, copy its
acceptance into the new task file, link back here for context, and
leave this note in place — it documents the cumulative plan.
