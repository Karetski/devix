# Task T-52 — Migrate panes onto namespace (`/pane(/<i>)*`)
Stage: 5
Status: pending
Depends on: T-30
Blocks:     T-57, T-91

## Goal
Address the layout tree at `/pane(/<i>)*`. Replace
`LayoutNode::at_path(&[usize])` with a `Lookup`-style retrieval that
takes a `Path` and walks the structural index list. This is the
namespace-side prep for the Stage-9 collapse.

## In scope
- `Editor::pane_at(&self, &Path) -> Option<&dyn Pane>` walking the
  current `LayoutNode` tree from `/pane` root + structural indices.
- `Editor::pane_at_mut`.
- Path-facing id encoding: `/pane`, `/pane/0`, `/pane/0/1`. Indices
  are 0-based per child position.
- `paths()` enumerator yielding every reachable pane path.

## Out of scope
- Collapsing `LayoutNode` into a unified Pane vocabulary (T-91).
- Moving rect caches to tui (T-92).
- Paths for tab strips / sidebars beyond the basic `/pane/...`
  walk (added during T-91/T-92).

## Files touched
- `crates/devix-core/src/editor/tree.rs`
- `crates/devix-core/src/pane_walk.rs`

## Acceptance criteria
- [ ] `Editor::pane_at(&Path::parse("/pane/0/1")?)` returns the same
      pane that `LayoutNode::at_path(&[0,1])` returned pre-task.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/namespace.md` — *Migration table* row for layout.
