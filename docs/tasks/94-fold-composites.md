# Task T-94 — Fold composites (TabbedPane / SidebarSlotPane) into unified vocabulary
Stage: 9
Status: complete — SidebarSlotPane retired; TabbedPane stays as on-stack composite (folds when paint_view becomes the only renderer at T-95)
Depends on: T-91, T-93
Blocks:     T-95

## Goal
With `LayoutNode` collapsed (T-91), the standalone composite Pane
impls (`TabbedPane`, `SidebarSlotPane`) lose their reason to be
parallel concepts. Fold them into the unified Pane vocabulary so
"tab strip" and "sidebar slot" are just Pane impls reachable from
the same walk helpers everything else uses.

## In scope
- Deduplicate the composite Pane code now that the layout enum is
  gone.
- Sidebar `slot` and tab `active` become per-Pane internal state
  (already partly true post-T-91).
- Confirm `View::TabStrip` / `View::Sidebar` paths
  (`/pane/.../tabstrip`, `/pane/.../sidebar/<slot>`) are the
  canonical addresses.

## Out of scope
- New composite Pane kinds.

## Files touched
- `crates/devix-core/src/pane*.rs`
- `crates/devix-core/src/editor/{tree,ops,focus,hittest}.rs`

## Acceptance criteria
- [x] One Pane vocabulary; no parallel composite hierarchies.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — fold

- `SidebarSlotPane` retired. `LayoutSidebar::render` paints chrome +
  optional content unconditionally now; focused styling reads
  `ctx.layout.focused_leaf` when present (default unfocused otherwise).
- `TabbedPane` stays as the on-stack composite for `LayoutFrame`'s
  tab-strip-over-body shape. Its absorption into the View IR happens
  at T-95 when `paint_view` becomes the only paint path.
- Plugin sidebar e2e test (`plugin_e2e::sidebar_slot_pane_renders_lua_pane_inside_chrome`)
  switched to `LayoutSidebar` directly.
- View IR canonical addresses `/pane/.../tabstrip` and
  `/pane/.../sidebar/<slot>` are produced by `editor::view::walk_layout`
  (T-43); confirmed against the post-T-91 walker.

## Spec references
- `docs/principles.md` — *MLIR*.
- `docs/specs/frontend.md` — *View IR*.
