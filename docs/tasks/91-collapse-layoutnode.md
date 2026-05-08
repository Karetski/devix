# Task T-91 — Collapse LayoutNode into a single Pane tree
Stage: 9
Status: partial (phase 2 in flight) — Editor.panes.root: Box<dyn Pane>; per-variant Pane impls done; enum removal + mutate-helper carve deferred
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
- [x] Editor's `root` is `Box<dyn Pane>`.
- [ ] All ops mutate the tree in place via walk helpers.
- [x] `cargo build --workspace` passes.
- [x] `cargo test --workspace` passes.

## Notes (2026-05-08) — phase 1 partial close
- **Render-context decision locked**: `RenderCtx` widens with
  `layout: Option<&'a LayoutCtx<'a>>`. Structural-render walker
  populates `Some`; chrome panes pass `None` and ignore. Alternatives
  considered (parallel render paths, sub-trait `LayoutPane`)
  recorded in `foundations-review.md` § *Amendment log*.
- **`PaneRegistry.root: Box<dyn Pane>`** lands now.
  `LayoutNode` `impl Pane` (delegating render to its existing
  match-based `render(area, frame, &LayoutCtx)` via `ctx.layout`,
  handle to `handle_at`, children to `children_at`,
  is_focusable to the variant test, `as_any` / `as_any_mut` to
  `Some(self)`). The registry's typed methods (`find_frame`,
  `at_path`, `replace_at`, etc.) keep their signatures by
  recovering `&LayoutNode` through `Pane::as_any` →
  `downcast_ref::<LayoutNode>`. Tests + ops continue to pattern-match
  on `editor.panes.root()`.
## Notes (2026-05-08) — phase 2 partial progress
- **Per-variant Pane impls** landed: `LayoutSplit`, `LayoutFrame`,
  `LayoutSidebar` each impl `Pane` directly. `LayoutNode`'s `Pane`
  impl now delegates to the variant via match. Walks via
  `Pane::children` resolve to the variant's logic without consulting
  the enum kind.
- **`Pane::children_mut`** added (default empty). Sets up the
  mutate-helper rewrite that comes next.
- **`PaneRegistry::pane_paths`** rewritten to walk via
  `Pane::children` rather than pattern-matching on
  `LayoutNode::Split` — generic over the concrete composite.
- **Three new tests** assert per-variant Pane behavior:
  weighted-rect children for `LayoutSplit`, focusable-leaf no-children
  for `LayoutFrame`, empty-slot Pane shape for `LayoutSidebar`.

## Phase-2 work still deferred (its own sprint)
- Change `LayoutSplit.children` from `Vec<(LayoutNode, u16)>` to
  `Vec<(Box<dyn Pane>, u16)>` so the structural tree can hold
  arbitrary Pane impls (not just the three variants).
- Rewrite the `mutate::{replace_at, remove_at, collapse_singletons,
  lift_into_horizontal_split}` helpers to walk via
  `Pane::children_mut` + downcast on the composite.
- Hoist the LayoutNode methods (`find_frame`, `at_path`,
  `frames`, `leaves_with_rects`, `path_to_leaf`,
  `sidebar_present`, `find_sidebar_mut`, etc.) into `PaneRegistry`
  helper functions that walk `Box<dyn Pane>` via `Pane::children` +
  downcast.
- Re-wire `editor::{focus, hittest, ops, view}` and the editor.rs
  test bodies to walk through `PaneRegistry` instead of pattern-
  matching on `LayoutNode`.
- Delete the `LayoutNode` enum, the `mutate` module, and the
  `LayoutCtx`-flavored render path.
- T-92 / T-94 / T-95 still gate on this completion.

## Spec references
- `docs/principles.md` — *MLIR — extend one primitive*.
- `docs/specs/namespace.md` — *Migration table* layout row.
- `docs/specs/foundations-review.md` — *Gate T-71*.
