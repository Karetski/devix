# Task T-95 — Stage-9 regression gate
Stage: 9
Status: deferred — depends on full Stage-9 sprint
Depends on: T-90, T-91, T-92, T-93, T-94
Blocks:     all of Stage 10+

## Goal
Verify the LayoutNode → Pane collapse landed cleanly. Retire the
legacy direct-paint path in `devix-tui` (the View-IR interpreter
from T-44 is now the only paint path).

## In scope
- Delete the legacy direct-paint code in `devix-tui` (everything
  the interpreter superseded).
- Full clean build + test.
- Manual sanity: edit, split, sidebar toggle, palette open, plugin
  pane (if configured), theme switch, all work end-to-end with the
  unified Pane tree + View interpreter.

## Out of scope
- New features.

## Files touched
- `crates/devix-tui/src/interpreter.rs` (becomes the only render
  path)
- `crates/devix-tui/src/app.rs` (drops legacy switch flag)

## Acceptance criteria
- [ ] No `LayoutNode` enum survives.
- [ ] No direct-paint code in `devix-tui`.
- [ ] `cargo build --workspace` passes with zero warnings.
- [ ] `cargo test --workspace` passes.
- [ ] Manual sanity passes.

## Spec references
- `docs/specs/foundations-review.md` — *Gate T-71*.
- `docs/specs/frontend.md` — *Lifecycle*.
