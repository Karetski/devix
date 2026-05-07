# Task T-91 — Collapse LayoutNode into a single Pane tree
Stage: 9
Status: pending
Depends on: T-52, T-90
Blocks:     T-92, T-94, T-95

## Goal
Dissolve the `LayoutNode` enum (Frame / Split / Sidebar variants)
into a single `Pane` tree. Each former variant becomes a Pane impl;
the editor's `root: Box<dyn Pane>` is the only source of layout
truth. Stage-9's MLIR principle payoff: one open primitive, no
closed enum at the layout root.

## In scope
- `Pane`-impl rewrites for the former Frame / Split / Sidebar
  variants. Each Pane owns its previously-enum-tag state
  (tabs/active/scroll for Frame; axis/weights/children for Split;
  slot/content for Sidebar).
- `Editor::pane_at(&Path) -> Option<&dyn Pane>` walks the unified
  tree (already partly there from T-52).
- All ops (`split_active`, `close_active_frame`, `toggle_sidebar`)
  rewritten as Pane-tree mutations.
- `LayoutNode` enum and its match arms deleted.

## Out of scope
- Moving rect caches to tui (T-92).
- Confirming Pane/Action trait location (T-93).
- Folding composites (T-94).

## Files touched
- `crates/devix-core/src/editor/tree.rs`: deleted or trimmed
- `crates/devix-core/src/editor/{ops,focus,hittest}.rs`: rewrites
- `crates/devix-core/src/pane.rs` / `pane_walk.rs`

## Acceptance criteria
- [ ] No `enum LayoutNode` survives.
- [ ] Editor's `root` is `Box<dyn Pane>`.
- [ ] All ops mutate the tree in place via walk helpers.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/principles.md` — *MLIR — extend one primitive*.
- `docs/specs/namespace.md` — *Migration table* layout row.
- `docs/specs/foundations-review.md` — *Gate T-71*.
