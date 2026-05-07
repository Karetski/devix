# Task T-94 — Fold composites (TabbedPane / SidebarSlotPane) into unified vocabulary
Stage: 9
Status: pending
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
- [ ] One Pane vocabulary; no parallel composite hierarchies.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *MLIR*.
- `docs/specs/frontend.md` — *View IR*.
