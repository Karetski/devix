# Task T-44 — TUI View interpreter (walks View; emits ratatui)
Stage: 4
Status: pending
Depends on: T-43
Blocks:     T-92, T-95

## Goal
Implement the View interpreter in `devix-tui::interpreter`. Walks
the `View` tree from T-43 and paints via the existing ratatui widget
adapters. Coexists with the current direct-paint code path until
T-95 retires it.

## In scope
- `interpret(view: &View, area: Rect, frame: &mut Frame, ctx)` walking
  every `View::*` and delegating to widget adapters in
  `devix-tui::widgets`.
- Layout primitives (`LinearLayout`, `UniformLayout`, scroll math
  from T-12) used to translate weights/axis into rects.
- Capability gating: when `Animations` is off, skip transition
  branches; when `TruecolorStyles` is off, quantize `Color::Rgb` →
  indexed at paint time.
- Frontend drives: `Request::View { root: "/pane" }` →
  `Response::View(...)` → interpreter paints.
- Tests: golden-View inputs render byte-equivalent ratatui buffers
  vs. the legacy direct-paint path on a fixture buffer/cursor/state.

## Out of scope
- Removing the legacy direct-paint path (T-95 regression gate
  at end of Stage 9).
- Animations (T-90+).
- New widget kinds.

## Files touched
- `crates/devix-tui/src/interpreter.rs`
- `crates/devix-tui/src/widgets/{tabstrip,sidebar,palette,popup}.rs`
  (small adapter signature changes)
- `crates/devix-tui/src/app.rs`: optional flag to switch render path
  during dev

## Acceptance criteria
- [ ] Golden-View fixtures render to identical ratatui buffer state
      as legacy direct-paint.
- [ ] Manual: `cargo run --bin devix` with the View path enabled
      edits a file end-to-end without behavioral change.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/frontend.md` — *View IR*, *Style*, *Animation hints*.
- `docs/specs/crates.md` — *devix-tui*.
