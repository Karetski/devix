# Task T-44 — TUI View interpreter (walks View; emits ratatui)
Stage: 4
Status: complete
Depends on: T-43
Blocks:     T-92, T-95

## Goal
Add a structural View IR interpreter as a library function in
`devix-tui::view_paint`. Walks the closed `View` tree and emits
ratatui draw calls. Lives alongside the legacy direct-paint Pane
render path until T-95 retires the legacy path and wires the
interpreter into the App's render loop.

## In scope
- `paint_view(view: &View, area: Rect, frame: &mut Frame, theme:
  &Theme)` library function walking every `View::*`.
- Stack / Split: ratatui Layout for proportional area splits;
  recurse on children.
- TabStrip / Sidebar / Buffer / Modal / Popup leaf variants:
  render a minimum-viable representation — exact byte parity with
  the legacy paint path lands at T-95 once the legacy path retires
  and the interpreter is the sole renderer.
- Empty: no-op.
- Capability gating: `Animations`-off skips transition branches.
- Tests: structural walk doesn't panic on a representative tree;
  Stack splits area correctly; Empty draws nothing.

## Out of scope
- Wiring the interpreter into Application's render loop (T-95).
- Byte-equivalence with legacy paint (T-95).
- Buffer-content rendering parity (T-95; reuses devix-core's
  buffer renderer).
- Animations / transitions (T-90+).
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
