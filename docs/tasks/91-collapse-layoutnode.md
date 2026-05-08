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
- **All `PaneRegistry` read-path methods migrated to Pane-trait
  walks**: `pane_paths`, `find_frame`, `find_frame_mut`,
  `find_sidebar_mut`, `sidebar_present`, `frames`,
  `leaves_with_rects`, `path_to_leaf`, `at_path`, `at_path_mut`,
  `at_path_with_rect`, `pane_at_xy`, `pane_at(&Path)`,
  `pane_at_mut(&Path)`, and `render`. Each walks via
  `Pane::children` / `children_mut` and downcasts at each node to
  the appropriate variant struct or to `LayoutNode`'s wrapping
  variant (transitional dual-downcast — once the enum is retired
  only the direct downcasts remain). `pane_leaf_id` is a public
  helper for callers that need a `LeafRef` from a `&dyn Pane`.
  At-path-family return types changed from `Option<&LayoutNode>` to
  `Option<&dyn Pane>`; `editor::editor`, `editor::hittest`,
  `devix-tui::input`, `devix-tui::application` updated their
  `.leaf_id()` / `.handle_at()` call sites accordingly
  (`pane_leaf_id` / `Pane::handle`).
- **Mutable walks** resolve the borrow-checker conflict (consecutive
  `downcast_mut` calls on the same `Any`) by computing a
  `SelfMatchVariant` enum from an immutable read first, then taking
  exactly one mutable downcast in the matched arm.
- **Three new tests** assert per-variant Pane behavior:
  weighted-rect children for `LayoutSplit`, focusable-leaf no-children
  for `LayoutFrame`, empty-slot Pane shape for `LayoutSidebar`.
- **LayoutNode methods retired**: `render`, `handle_at`,
  `find_frame`, `find_frame_mut`, `find_sidebar_mut`,
  `sidebar_present`, `frames` — all dead code once the registry's
  surface migrated. `LayoutNode` keeps `frame`/`sidebar`
  constructors, `leaf_id`, `is_focusable`, `children_at`,
  `children_at_mut`, `at_path`, `at_path_mut`, `at_path_with_rect`,
  `pane_at` (xy hit-test), `leaves_with_rects`, `path_to_leaf`, and
  the per-variant `Pane` impls. The remaining inherent walkers stay
  for the `tree.rs` test suite that still exercises LayoutNode
  directly.

## Phase-2 work still deferred (its own sprint)
- Change `LayoutSplit.children` from `Vec<(LayoutNode, u16)>` to
  `Vec<(Box<dyn Pane>, u16)>`. Attempted twice in this session — the
  cascade through `LayoutNode::children_at` /
  `children_at_mut` (which return `Vec<(Rect, &LayoutNode)>`),
  remaining `LayoutNode` walk methods that recurse via children
  (`at_path`, `pane_at`, `path_to_leaf`, `leaves_with_rects`),
  the `mutate::*` helpers, the `editor::view` walk-by-variant, the
  `editor::focus` `compute_focus_target` / `walk_into` pattern
  matches, and the `editor/ops.rs` constructor sites all need
  rewriting at once. Reverted; left for the next sprint.
- Rewrite the `mutate::{replace_at, remove_at, collapse_singletons,
  lift_into_horizontal_split}` helpers to walk via
  `Pane::children_mut` + downcast on the composite. Accept
  `Box<dyn Pane>` for the `new` argument so non-LayoutNode panes
  can be inserted. Currently registry wraps these via
  `as_layout_mut()`.
- Rewrite `editor::view::walk_layout` and `editor::focus`'s
  `compute_focus_target` / `walk_into` to walk via Pane trait
  instead of pattern-matching on `LayoutNode::Split`.
- Re-wire the `editor.rs` test bodies that still pattern-match on
  `ws.panes.root()` (e.g. `match ws.panes.root() { LayoutNode::Split(s)
  => ... }`) — switch to typed downcast through `Pane::as_any` or
  use registry helpers.
- Delete the `LayoutNode` enum and the remaining inherent walkers
  on it (`children_at`, `children_at_mut`, `at_path`,
  `at_path_mut`, `at_path_with_rect`, `pane_at`,
  `leaves_with_rects`, `path_to_leaf`, `leaf_id`, `is_focusable`,
  `frame`, `sidebar` constructors); migrate their tree-level
  tests to use the per-variant structs / registry walks.
- Remove the `mutate` module entirely (replace with Pane-trait
  walk helpers).
- T-92 / T-94 / T-95 still gate on this completion.

## Spec references
- `docs/principles.md` — *MLIR — extend one primitive*.
- `docs/specs/namespace.md` — *Migration table* layout row.
- `docs/specs/foundations-review.md` — *Gate T-71*.
