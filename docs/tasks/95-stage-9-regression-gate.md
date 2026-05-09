# Task T-95 — Stage-9 regression gate
Stage: 9
Status: partial — producer materialization shipped (View::Buffer carries `lines` + `gutter_width`); paint_view consumes them; legacy direct-paint stays as the active path until manual TTY parity verification
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
- [x] No `LayoutNode` enum survives. (T-91 phase-2 close.)
- [ ] No direct-paint code in `devix-tui`. (Pending TTY-verified
      retirement of `Editor::panes.render` from
      `Application::render`.)
- [x] `cargo build --workspace` passes with zero new warnings.
- [x] `cargo test --workspace` passes.
- [ ] Manual sanity passes. (Not actionable in non-TTY env.)

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

## Notes (2026-05-08) — producer-materialization partial close

Design choice **(a) producer materializes full buffer state** is
now in place. What landed:

- `View::Buffer` gained `lines: Vec<BufferLine>` + `gutter_width:
  u32`. The wire form stays back-compat: both fields default to
  empty / 0, so older producers (T-43 minimum-viable) keep
  round-tripping unchanged.
- `BufferLine { line, gutter, spans }` carries the visible line's
  pre-formatted gutter and theme-resolved style runs. The producer
  walks tree-sitter highlights against the active `Theme` and emits
  one `TextSpan` per coalesced style group.
- `editor::view::build_active_buffer` materializes a 200-line
  window starting at the cursor's `scroll_top`. Coalescing same-
  style runs keeps the wire size proportional to syntactic
  complexity rather than character count.
- `paint_view`'s `View::Buffer` arm renders the materialized lines
  via `Frame::buffer_mut().set_stringn` per span, with the
  terminal cursor placed via `set_cursor_position` when the pane
  is `active`. Empty-`lines` falls back to the path-label paint
  for back-compat.
- `Style` ↔ `ratatui::style::Style` round-trip helpers cover the
  full `NamedColor` palette plus `Rgb` / `Indexed` / `Default`.

What's still deferred:

1. **Retiring the legacy direct-paint path.** The active renderer
   stays `Editor::panes.render`. Switching `Application::render` to
   `paint_view` requires byte-parity verification that the
   programmatic `TestBackend` test suite alone can't fully
   establish — long-line truncation, multi-cursor reverse cells,
   selection paint over selection-with-multi-cursor edge cases
   each have surface area easier to spot in a real terminal.
2. ~~**Selection + extra-cursor paint.**~~ *Resolved 2026-05-08*:
   the producer's `editor::view::build_active_buffer` populates
   `selection: Vec<SelectionMark>` (one per range, zero-extent for
   multicursor secondaries); `view_paint::paint_buffer` adds
   `paint_selection_overlay` (reverse-video over the cells in each
   non-empty mark, multi-line marks paint to the row's right edge)
   and `paint_extra_cursor_marks` (reverse cells on every
   zero-extent mark whose coordinate ≠ `cursor`). Mirrors the
   legacy `editor::buffer::paint_selection` + `paint_extra_cursors`.
   Two unit tests in `view_paint::tests` exercise the passes.
3. **Plugin pane content not in View IR.** The View producer's
   `walk_sidebar` returns `View::Sidebar { content: View::Empty,
   .. }`. Switching `Application::render` to `paint_view` would
   render every plugin sidebar (file-tree, etc.) as empty —
   regression. Plugin panes that pushed View IR via T-111's
   `pane:set_view` are also not threaded through `walk_sidebar`.
   Closing T-95 fully needs `walk_sidebar` to consult the plugin
   runtime's installed pane (either via a `Pane::view(area) ->
   View` trait method, or a plugin-runtime side bridge that emits
   `View::Text` from `register_pane`'s `lines = {…}`).
4. **Manual TTY sanity.** Edit/split/sidebar-toggle/palette/plugin
   pane/theme-switch end-to-end pass.

T-95 full close (legacy retirement) lands in a focused sprint with
TTY access. The producer materialization here unblocks the
Stage-8 highlighter actor's downstream consumer — once
`HighlightActor` populates `HighlightCache` and the producer reads
from it, the materialized highlights flow without holding the
editor's main thread.
