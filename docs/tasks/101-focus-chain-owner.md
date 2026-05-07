# Task T-101 — Focus chain owner
Stage: 10
Status: pending
Depends on: T-100
Blocks:     T-104

## Goal
Carve out focus chain ownership (`FocusChain`) from Editor.
Owns the active path + focusable iteration + focus-change
events. Publishes `Pulse::FocusChanged` on transitions.

## In scope
- `FocusChain { active: Option<Path>, ... }` with iter helpers.
- Editor delegates focus operations to `FocusChain`.
- `editor/focus.rs` becomes the FocusChain impl file (or replaces
  it).
- `Pulse::FocusChanged` emitted on transitions only (deduped at
  source).

## Out of scope
- Modal slot ownership (T-103).
- Focus visualization in TUI (already handled by Sidebar.focused +
  Buffer.active fields).

## Files touched
- `crates/devix-core/src/editor/focus.rs`
- `crates/devix-core/src/editor/editor.rs`

## Acceptance criteria
- [ ] FocusChain owns the active path.
- [ ] FocusChanged pulse fires only on actual transitions.
- [ ] `cargo build --workspace` passes.
- [ ] `cargo test --workspace` passes.

## Spec references
- `docs/specs/pulse-bus.md` — *Catalog → FocusChanged*.
