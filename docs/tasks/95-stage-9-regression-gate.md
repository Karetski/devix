# Task T-95 — Stage-9 regression gate
Stage: 9
Status: deferred — needs design lock (View IR producer/renderer split) + manual TTY sanity
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

## Notes (2026-05-08) — gate-only deferred; T-90/T-91/T-92/T-93/T-94 all closed

The structural Stage-9 work is done:

- **T-90** — synthetic-id strategy locked (deterministic-derivation).
- **T-91** — `LayoutNode` enum retired; structural tree is
  `Box<dyn Pane>` end-to-end; mutate helpers walk via the trait.
- **T-92** — `RenderCache` lifecycle moved to
  `devix-tui::Application`; Editor APIs take `&RenderCache` as a
  parameter; `commands::Context` carries it through.
- **T-93** — `Pane` / `Action` trait location confirmed in
  `devix-core` (closed at original Stage-9 partial).
- **T-94** — `SidebarSlotPane` retired into `LayoutSidebar`'s
  `Pane` impl; `TabbedPane` remains as the on-stack frame composite.

What's still gating T-95 specifically:

1. **Design choice on the View IR producer/renderer split.** Reaching
   byte-parity with the legacy direct-paint requires either
   (a) materializing full buffer state into `View::Buffer` (line
   content, cursor mark, selection ranges, highlight runs) so the
   renderer is thin, or (b) handing `paint_view` a
   `BufferProvider` that resolves `/buf/<id>` paths back to a live
   `Document`. Both are load-bearing architectural decisions that
   should land deliberately. (a) integrates with Stage-8's
   supervised highlighter; (b) is more pragmatic but spec-impure.
2. **Manual TTY sanity.** Once the renderer is the single paint
   path, edit/split/sidebar-toggle/palette-open/plugin-pane/theme-switch
   all need an interactive end-to-end pass that the test suite does
   not provide.

Both items are deferred to a focused T-95 sprint.
